//! The versioned machine endpoints: the manifest, direct leases under a
//! claimed session, joint command batches through the arbiter, and joint
//! state and frames sampled after the physics step.

use std::collections::HashMap;
use std::time::Instant;

use bevy::prelude::*;
use gearbox_api::{
    ControlRequest, ControlResponse, GearboxBus, JointCommandAck, JointCommand, JointState, Props, Tf, code, command_status,
    control_op, joint_mode, names_from_bytes, names_to_bytes,
};

use super::{Batch, Effective, Grant, Setpoint, Validity};
use crate::controller::{ControllerInventory, MachineAgentKeys, MachineInstanceSpec};
use crate::machine_model::{DofToken, MachineDescription, RigidTransform, manifest};
use crate::physics::PhysicsWorld;

/// Direct leases by machine id and the session that holds them.
#[derive(Resource, Default)]
pub struct DirectLeases(pub HashMap<(String, u64), (u64, Grant)>);

fn machine_of<'a>(inventory: &'a ControllerInventory, keys: &MachineAgentKeys, agent: &str) -> Option<&'a MachineInstanceSpec> {
    let key = keys.0.get(agent)?;
    inventory.machines.iter().find(|m| m.id == key.machine_id && m.scene_root == Some(key.scene_root))
}

fn dof_keys(model: &MachineDescription, machine_id: &str, names: &[String]) -> Result<Vec<super::DofKey>, String> {
    names
        .iter()
        .map(|name| {
            let r = model.dof_by_name(name).ok_or_else(|| format!("no DOF named `{name}`"))?;
            Ok(super::DofKey {
                machine: machine_id.to_string(),
                generation: model.generation,
                joint: model.joints[r.joint].path.clone(),
                dof: model.dofs[r.dof].token,
            })
        })
        .collect()
}

fn refuse(code: u32, message: &str) -> ControlResponse {
    ControlResponse { code, props: Props::from_pairs(&[("message", message)]).into_bytes(), ..Default::default() }
}

fn ack(status: u32, code: u32, command: &JointCommand, tick: u64, message: &str) -> JointCommandAck {
    JointCommandAck { sequence: command.sequence, generation: command.generation, tick, status, code, message: message.as_bytes().to_vec() }
}

/// A command batch as the arbiter's batch under `grant`, or the wire
/// error code and message that rejects it whole.
pub fn batch_from_command(model: &MachineDescription, machine_id: &str, grant: &Grant, command: &JointCommand) -> Result<Batch, (u32, String)> {
    if command.generation != model.generation {
        return Err((code::REFUSED, format!("generation {} is stale; the machine is at {}", command.generation, model.generation)));
    }
    let names = command.names();
    let n = names.len();
    if command.modes.len() != n || command.values.len() != n || (!command.feedforward.is_empty() && command.feedforward.len() != n) {
        return Err((code::USAGE, "names, modes, values and feedforward must have one entry per DOF".into()));
    }
    let dofs = dof_keys(model, machine_id, &names).map_err(|e| (code::NOT_FOUND, e))?;
    let mut commands = Vec::with_capacity(n);
    for (i, dof) in dofs.into_iter().enumerate() {
        let value = command.values[i];
        let feedforward = command.feedforward.get(i).copied().filter(|v| !v.is_nan());
        let setpoint = match command.modes[i] {
            joint_mode::POSITION => Setpoint::Position { q: value, qdot: feedforward },
            joint_mode::VELOCITY => Setpoint::Velocity { qdot: value },
            joint_mode::EFFORT => Setpoint::Effort { force: value },
            joint_mode::HOLD => Setpoint::Hold,
            joint_mode::FREE => Setpoint::Free,
            other => return Err((code::UNSUPPORTED, format!("unknown joint mode {other}"))),
        };
        commands.push((dof, setpoint));
    }
    let validity = if command.validity_ms == 0 { model.command_validity_ms } else { u64::from(command.validity_ms) };
    if validity > crate::machine_model::MAX_COMMAND_VALIDITY_MS {
        return Err((code::USAGE, format!("validity {validity} ms exceeds {} ms", crate::machine_model::MAX_COMMAND_VALIDITY_MS)));
    }
    Ok(Batch {
        owner: grant.owner.clone(),
        epoch: command.epoch,
        sequence: command.sequence,
        commands,
        validity: Validity::WallMs(validity),
        requested_cap: (!command.max_effort.is_nan()).then_some(command.max_effort),
    })
}

pub fn serve(
    bus: Option<ResMut<GearboxBus>>,
    keys: Res<MachineAgentKeys>,
    inventory: Res<ControllerInventory>,
    mut physics: ResMut<PhysicsWorld>,
    mut leases: ResMut<DirectLeases>,
) {
    let Some(mut bus) = bus else { return };
    let live: Vec<(String, u64)> = inventory
        .machines
        .iter()
        .filter_map(|m| m.model.as_ref().map(|model| (m.id.clone(), model.generation)))
        .collect();
    let stale: Vec<(String, u64)> = leases
        .0
        .iter()
        .filter(|((machine, _), (generation, _))| !live.contains(&(machine.clone(), *generation)))
        .map(|(k, _)| k.clone())
        .collect();
    for key in stale {
        if let Some((_, grant)) = leases.0.remove(&key) {
            physics.control.arbiter.revoke(&grant.owner);
        }
    }
    for (agent_name, agent) in bus.machines.iter_mut() {
        let Some(machine) = machine_of(&inventory, &keys, agent_name) else { continue };
        let Some(model) = machine.model.as_ref().filter(|_| machine.accepted()) else { continue };
        if agent.description.generation != model.generation || agent.description.manifest.is_empty() {
            agent.description.generation = model.generation;
            agent.description.manifest = manifest::to_string(&manifest::machine(model)).into_bytes();
        }
        let session = agent.session_id();
        revoke_lapsed(&mut physics, &mut leases, &machine.id, session);
        let holder = agent.session().map(|s| s.holder.clone()).unwrap_or_default();
        for (request, token) in std::mem::take(&mut agent.pending_control) {
            let response = handle_control(&mut physics, &mut leases, model, &machine.id, session, &holder, &request);
            agent.respond_control(token, &response);
        }
        let now = Instant::now();
        for (command, token) in std::mem::take(&mut agent.pending_joint_commands) {
            let response = handle_joint_command(&mut physics, &leases, model, &machine.id, session, &command, now);
            agent.respond_joint_command(token, &response);
        }
    }
}

/// Leases of `machine` held under any session but its current one are
/// revoked: their DOFs go to their safe setpoints.
pub fn revoke_lapsed(physics: &mut PhysicsWorld, leases: &mut DirectLeases, machine: &str, session: u64) {
    let lapsed: Vec<(String, u64)> = leases.0.keys().filter(|(m, s)| m == machine && *s != session).cloned().collect();
    for key in lapsed {
        if let Some((_, grant)) = leases.0.remove(&key) {
            physics.control.arbiter.revoke(&grant.owner);
        }
    }
}

/// One `control` request from the agent's current claimed `session`.
pub fn handle_control(
    physics: &mut PhysicsWorld,
    leases: &mut DirectLeases,
    model: &MachineDescription,
    machine: &str,
    session: u64,
    holder: &str,
    request: &ControlRequest,
) -> ControlResponse {
    let key = (machine.to_string(), session);
    let done = |epoch: u64, tick: u64| ControlResponse { epoch, generation: model.generation, tick, ..Default::default() };
    if request.session == 0 || request.session != session {
        return refuse(code::REFUSED, "claim the machine first; the lease is bound to that session");
    }
    if request.generation != model.generation {
        return refuse(code::REFUSED, &format!("generation {} is stale; the machine is at {}", request.generation, model.generation));
    }
    match request.op {
        control_op::ACQUIRE => {
            let dofs = match dof_keys(model, machine, &names_from_bytes(&request.names)) {
                Ok(dofs) => dofs,
                Err(e) => return refuse(code::NOT_FOUND, &e),
            };
            if let Some((_, previous)) = leases.0.remove(&key) {
                physics.control.arbiter.release(&previous, false);
            }
            match physics.control.arbiter.acquire(holder, machine, &dofs) {
                Ok(grant) => {
                    let response = ControlResponse {
                        props: Props::from_pairs(&[("owner", grant.owner.to_string().as_str())]).into_bytes(),
                        ..done(grant.epoch, physics.ticks + 1)
                    };
                    leases.0.insert(key, (model.generation, grant));
                    response
                }
                Err(e) => refuse(code::REFUSED, &e.to_string()),
            }
        }
        control_op::RELEASE => match leases.0.get(&key) {
            Some((_, grant)) if grant.epoch == request.epoch => {
                let grant = grant.clone();
                physics.control.arbiter.release(&grant, request.resume != 0);
                leases.0.remove(&key);
                done(grant.epoch, physics.ticks + 1)
            }
            _ => refuse(code::REFUSED, "no lease of that epoch under this session"),
        },
        control_op::RESET => {
            leases.0.retain(|(m, _), _| m != machine);
            physics.control.arbiter.reset_machine(machine);
            done(0, physics.ticks + 1)
        }
        _ => refuse(code::USAGE, "unknown control op"),
    }
}

/// One `joint_command` batch from the agent's current claimed `session`.
pub fn handle_joint_command(
    physics: &mut PhysicsWorld,
    leases: &DirectLeases,
    model: &MachineDescription,
    machine: &str,
    session: u64,
    command: &JointCommand,
    now: Instant,
) -> JointCommandAck {
    let tick = physics.ticks;
    let reply = (|| -> Result<(), (u32, String)> {
        if command.session == 0 || command.session != session {
            return Err((code::REFUSED, "claim the machine first".into()));
        }
        let Some((_, grant)) = leases.0.get(&(machine.to_string(), session)) else {
            return Err((code::REFUSED, "no lease: acquire the DOFs on control first".into()));
        };
        let batch = batch_from_command(model, machine, grant, command)?;
        physics.submit_batch(batch, now).map_err(|e| (code::REFUSED, e.to_string()))
    })();
    match reply {
        Ok(()) => ack(command_status::ACCEPTED, code::OK, command, tick, ""),
        Err((c, message)) => ack(command_status::REJECTED, c, command, tick, &message),
    }
}

/// Bevy's Y-up world to the manifest's right-handed Z-up world.
fn to_z_up(pose: crate::physics::backend::Pose) -> RigidTransform {
    let r = bevy::math::DQuat::from_rotation_x(std::f64::consts::FRAC_PI_2);
    let t = pose.translation;
    let q = pose.rotation;
    RigidTransform::new(
        r * bevy::math::DVec3::new(t.x, t.y, t.z),
        (r * bevy::math::DQuat::from_xyzw(q.x, q.y, q.z, q.w)).normalize(),
    )
}

/// World pose (Z-up) of every body of a machine by model index.
pub fn body_poses(
    machine: &MachineInstanceSpec,
    model: &MachineDescription,
    physics: &PhysicsWorld,
    index: &crate::prim_index::PrimIndex,
) -> Vec<Option<RigidTransform>> {
    let Some(root) = machine.scene_root else { return vec![None; model.bodies.len()] };
    model
        .bodies
        .iter()
        .map(|b| {
            let entity = index.find(root, &b.path)?;
            let handle = physics.entity_to_body.get(&entity)?;
            Some(to_z_up(physics.body(*handle)?.position()))
        })
        .collect()
}

/// Every reporting-tree edge as a parent-relative transform.
pub fn tf_edges(model: &MachineDescription, bodies: &[Option<RigidTransform>]) -> Vec<(String, String, RigidTransform)> {
    let world = |frame: usize| {
        let (body, pose) = model.frame_in_body(frame)?;
        Some(bodies.get(body).copied().flatten()?.mul(&pose))
    };
    model
        .tree
        .iter()
        .filter_map(|edge| {
            let (parent, child) = (world(edge.parent)?, world(edge.child)?);
            Some((model.frames[edge.parent].name.clone(), model.frames[edge.child].name.clone(), parent.inverse().mul(&child)))
        })
        .collect()
}

#[derive(Default)]
pub struct Sampled {
    last: HashMap<(String, String), (f64, u64)>,
}

/// A machine's joint state after the last physics tick: measured
/// coordinates, rates from consecutive samples, the backend's measured effort
/// where it has one (NaN elsewhere), and the committed effort and target.
pub fn sample_joint_state(
    machine_id: &str,
    model: &MachineDescription,
    physics: &PhysicsWorld,
    last: &mut HashMap<(String, String), (f64, u64)>,
) -> JointState {
    let tick = physics.ticks;
    let dt = physics.dt();
    let order: Vec<usize> = match &model.reporting {
        Some(r) if !r.joints.is_empty() => r.joints.iter().flat_map(|&j| model.joints[j].dofs.clone()).collect(),
        _ => (0..model.dofs.len()).collect(),
    };
    let mut state = JointState { generation: model.generation, tick, sim_time_s: physics.simulated_seconds, valid: 1, ..Default::default() };
    let mut names = Vec::new();
    for index in order {
        let dof = &model.dofs[index];
        let key = super::DofKey { machine: machine_id.to_string(), generation: model.generation, joint: model.joints[dof.joint].path.clone(), dof: dof.token };
        let binding = physics.dof_binding(&key);
        let q = binding.and_then(|(j, axis)| physics.joint(j).and_then(|joint| joint.motor_position(axis))).unwrap_or(f64::NAN);
        let qdot = match last.insert((machine_id.to_string(), dof.name.clone()), (q, tick)) {
            Some((previous, at)) if at < tick && q.is_finite() && previous.is_finite() => (q - previous) / ((tick - at) as f64 * dt),
            _ => f64::NAN,
        };
        let committed = binding.and_then(|b| physics.control.committed.get(&b));
        let measured = binding.and_then(|(j, _)| physics.motor_output(j)).map_or(f64::NAN, |o| o.force);
        state.position.push(q);
        state.velocity.push(if dof.token == DofToken::Distance { f64::NAN } else { qdot });
        state.measured_effort.push(measured);
        state.commanded_effort.push(match committed.map(|s| s.effective) {
            Some(Effective::Effort { force }) => force,
            _ => f64::NAN,
        });
        state.target.push(match committed.map(|s| s.effective) {
            Some(Effective::Servo { q: Some(q), .. }) => q,
            Some(Effective::Servo { q: None, qdot, .. }) => qdot,
            _ => f64::NAN,
        });
        names.push(dof.name.clone());
    }
    state.names = names_to_bytes(&names);
    state
}

/// Joint state and frames after the physics steps of the frame.
pub fn publish_state(
    bus: Option<ResMut<GearboxBus>>,
    keys: Res<MachineAgentKeys>,
    inventory: Res<ControllerInventory>,
    physics: Res<PhysicsWorld>,
    index: Res<crate::prim_index::PrimIndex>,
    mut sampled: Local<Sampled>,
) {
    let Some(mut bus) = bus else { return };
    let tick = physics.ticks;
    for (agent_name, agent) in bus.machines.iter_mut() {
        let Some(machine) = machine_of(&inventory, &keys, agent_name) else { continue };
        let Some(model) = machine.model.as_ref().filter(|_| machine.accepted()) else { continue };
        let state = sample_joint_state(&machine.id, model, &physics, &mut sampled.last);
        agent.publish_joint_state(&state);

        let bodies = body_poses(machine, model, &physics, &index);
        let edges = tf_edges(model, &bodies);
        let mut tf = Tf { generation: model.generation, tick, sim_time_s: physics.simulated_seconds, ..Default::default() };
        let (mut parent_names, mut child_names) = (Vec::new(), Vec::new());
        for (parent, child, pose) in edges {
            tf.translation.extend([pose.translation.x, pose.translation.y, pose.translation.z]);
            let q = if pose.rotation.w < 0.0 { -pose.rotation } else { pose.rotation };
            tf.rotation.extend([q.w, q.x, q.y, q.z]);
            parent_names.push(parent);
            child_names.push(child);
        }
        let root = model.base_frame.and_then(|b| {
            let (body, offset) = model.frame_in_body(b)?;
            Some((model.frames[b].name.clone(), bodies.get(body).copied().flatten()?.mul(&offset)))
        });
        let mut props = vec![("convention".to_string(), "right_handed_z_up".to_string())];
        if let Some((name, pose)) = root {
            props.push(("root".into(), name));
            props.push(("root_translation".into(), format!("{} {} {}", pose.translation.x, pose.translation.y, pose.translation.z)));
            props.push(("root_rotation_wxyz".into(), format!("{} {} {} {}", pose.rotation.w, pose.rotation.x, pose.rotation.y, pose.rotation.z)));
        }
        tf.parents = names_to_bytes(&parent_names);
        tf.children = names_to_bytes(&child_names);
        let pairs: Vec<(&str, &str)> = props.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        tf.props = Props::from_pairs(&pairs).into_bytes();
        agent.publish_tf(&tf);
    }
}
