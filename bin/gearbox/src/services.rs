//! Service controllers (`specs/TOOLS_SPEC.md` §6.2, §7.2) and the ISOBUS
//! shaped exchange between a master and its slaves: the master's state
//! reaches every slave controller as inputs, and a slave's granted requests
//! (`speed`, `steering`) drive the master when its session is quiet.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use gearbox_api::GearboxBus;
use rapier3d::prelude::{GenericJoint, JointAxis, RigidBodyHandle};
use usd_bevy::UsdPrimRef;
use usd_bevy::physics::PhysicsWorld;

use crate::attach::Attachments;
use crate::controller::{
    CmdVel, ControllerInventory, ControllerKey, ControllerSpec, ControllerStates, MachineAgentKeys,
    MachineInstanceSpec, body_forward_vector, find_prim_entity,
};

pub const SERVICE_TYPES: [&str; 6] = [
    "builtin:hitch",
    "builtin:pto",
    "builtin:hydraulic_valve",
    "builtin:joint_position",
    "builtin:brake",
    "builtin:trailer_steer",
];

/// What a slave controller sees of its master each step (§5.2).
#[derive(Debug, Clone, Default)]
pub struct MasterState {
    pub master_id: String,
    pub ground_speed_mps: f64,
    pub heading_rad: f64,
    pub roll_rad: f64,
    pub pitch_rad: f64,
    /// `builtin:hitch` instance → position 0..1.
    pub hitch: HashMap<String, f64>,
    /// `builtin:pto` instance → (rpm, engaged).
    pub pto: HashMap<String, (f64, bool)>,
}

/// Slave machine id → its master's state.
#[derive(Resource, Default)]
pub struct MasterInputs(pub HashMap<String, MasterState>);

/// Master machine id → the twist a granted slave request asks for.
#[derive(Resource, Default)]
pub struct TimRequests(pub HashMap<String, CmdVel>);

/// Latest `/cmd` props per service controller, merged over time.
#[derive(Resource, Default)]
pub struct ServiceCommands(pub HashMap<ControllerKey, HashMap<String, String>>);

#[derive(Resource, Default)]
struct WarnedOnce(HashSet<String>);

pub struct ServicesPlugin;

impl Plugin for ServicesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MasterInputs>()
            .init_resource::<TimRequests>()
            .init_resource::<ServiceCommands>()
            .init_resource::<WarnedOnce>()
            .add_systems(
                Update,
                (
                    drain_service_commands.after(crate::attach::serve_attachments),
                    apply_service_controllers,
                )
                    .chain(),
            )
            .add_systems(PostUpdate, feed_master_inputs);
    }
}

fn machine_for<'a>(
    inventory: &'a ControllerInventory,
    keys: &MachineAgentKeys,
    ns: &str,
) -> Option<&'a MachineInstanceSpec> {
    let key = keys.0.get(ns)?;
    inventory
        .machines
        .iter()
        .find(|m| m.scene_root == Some(key.scene_root) && m.id == key.machine_id)
}

fn num(props: &HashMap<String, String>, key: &str) -> Option<f64> {
    props.get(key).and_then(|v| v.parse().ok())
}

fn flag(props: &HashMap<String, String>, key: &str) -> Option<bool> {
    props
        .get(key)
        .map(|v| matches!(v.as_str(), "1" | "true" | "on" | "yes"))
}

/// Commands from the bus land per controller; `request=` commands from an
/// attached slave become TIM requests when the master grants them.
fn drain_service_commands(
    inventory: Res<ControllerInventory>,
    keys: Res<MachineAgentKeys>,
    bus: Option<ResMut<GearboxBus>>,
    attachments: Res<Attachments>,
    mut service: ResMut<ServiceCommands>,
    mut tim: ResMut<TimRequests>,
    mut warned: ResMut<WarnedOnce>,
) {
    let Some(mut bus) = bus else { return };
    let namespaces: Vec<String> = bus.machines.keys().cloned().collect();
    for ns in namespaces {
        let Some(machine) = machine_for(&inventory, &keys, &ns) else {
            continue;
        };
        let scene_root = machine.scene_root.expect("keyed machines have a root");
        let Some(agent) = bus.machines.get_mut(&ns) else {
            continue;
        };
        let mut keep = Vec::new();
        for cmd in agent.commands.drain(..) {
            let mut props: HashMap<String, String> = cmd.props().iter().into_iter().collect();
            if props.contains_key("tool") {
                keep.push(cmd);
                continue;
            }
            if let Some(request) = props.get("request").cloned() {
                let value = num(&props, "value").unwrap_or(cmd.value);
                let Some(att) = attachments.0.iter().find(|a| a.slave_ns == ns) else {
                    if warned.0.insert(format!("{ns}:request:{request}")) {
                        warn!("gearbox-services: `{ns}` requested `{request}` but is not attached");
                    }
                    continue;
                };
                let Some(master) = machine_for(&inventory, &keys, &att.master_ns) else {
                    continue;
                };
                if !master.grants.iter().any(|g| *g == request) {
                    if warned.0.insert(format!("{ns}:denied:{request}")) {
                        warn!(
                            "gearbox-services: `{}` does not grant `{request}` to `{ns}`; ignored",
                            att.master_ns
                        );
                    }
                    continue;
                }
                let entry = tim.0.entry(master.id.clone()).or_default();
                match request.as_str() {
                    "speed" => entry.linear_mps = value as f32,
                    "steering" => entry.angular_rps = value as f32,
                    _ => {}
                }
                continue;
            }
            let Some(instance) = props.remove("controller") else {
                continue;
            };
            let Some(controller) = machine.controllers.iter().find(|c| c.instance == instance)
            else {
                if warned.0.insert(format!("{ns}:controller:{instance}")) {
                    warn!("gearbox-services: `{ns}` has no controller `{instance}`");
                }
                continue;
            };
            if !SERVICE_TYPES.contains(&controller.controller_type.as_str()) {
                continue;
            }
            if !props.contains_key("value") {
                props.insert("value".to_string(), cmd.value.to_string());
            }
            let key = ControllerKey::new(scene_root, &machine.id, &instance);
            service.0.entry(key).or_default().extend(props);
        }
        agent.commands = keep;
    }
}

/// The joint a controller drives: the bodies it connects and its kind.
struct JointRef {
    body0: RigidBodyHandle,
    body1: RigidBodyHandle,
    axis: JointAxis,
}

fn resolve_joint(
    scene_root: Entity,
    prim_path: &str,
    joints: &Query<(Entity, &UsdPrimRef, &usd_bevy::UsdPhysicsJoint)>,
    parents: &Query<&ChildOf>,
    physics: &PhysicsWorld,
) -> Option<JointRef> {
    let (_, _, joint) = joints.iter().find(|(entity, prim, _)| {
        prim.path == prim_path && crate::controller::is_descendant_of(*entity, scene_root, parents)
    })?;
    let body0 = physics.entity_to_body.get(&joint.body0?).copied()?;
    let body1 = physics.entity_to_body.get(&joint.body1?).copied()?;
    let axis = match joint.kind {
        usd_bevy::UsdJointKind::Prismatic => JointAxis::LinX,
        _ => JointAxis::AngX,
    };
    Some(JointRef { body0, body1, axis })
}

/// Apply `f` to the rapier joint between two bodies, wherever it lives.
fn with_joint(physics: &mut PhysicsWorld, j: &JointRef, f: impl FnOnce(&mut GenericJoint)) -> bool {
    let pair = |a: RigidBodyHandle, b: RigidBodyHandle| {
        (a == j.body0 && b == j.body1) || (a == j.body1 && b == j.body0)
    };
    let impulse = physics
        .impulse_joints
        .iter()
        .find(|(_, joint)| pair(joint.body1, joint.body2))
        .map(|(h, _)| h);
    if let Some(h) = impulse {
        if let Some(joint) = physics.impulse_joints.get_mut(h, true) {
            f(&mut joint.data);
            return true;
        }
    }
    let multibody = physics
        .multibody_joints
        .attached_joints(j.body1)
        .find(|(a, b, _)| pair(*a, *b))
        .map(|(_, _, h)| h);
    if let Some(h) = multibody
        && let Some((mb, id)) = physics.multibody_joints.get_mut(h)
        && let Some(link) = mb.link_mut(id)
    {
        f(&mut link.joint.data);
        return true;
    }
    false
}

const POSITION_STIFFNESS: f64 = 4_000.0;
const POSITION_DAMPING: f64 = 400.0;
const MOTOR_MAX_FORCE: f64 = 50_000.0;
const VELOCITY_FACTOR: f64 = 200.0;
const BRAKE_FACTOR: f64 = 400.0;
const DEFAULT_PTO_RPM: f64 = 540.0;
const DEFAULT_VALVE_RATE: f64 = 0.5;
const DEFAULT_TRAILER_STEER_DEG: f64 = 35.0;

fn controller_joints<'a>(machine: &'a MachineInstanceSpec, c: &'a ControllerSpec) -> Vec<&'a str> {
    match c.controller_type.as_str() {
        "builtin:brake" => {
            let mut v: Vec<&str> = c.wheel_joints.iter().map(String::as_str).collect();
            if v.is_empty() {
                v.extend(machine.brake_joints.iter().map(String::as_str));
            }
            if v.is_empty() {
                v.extend(machine.powered_wheel_joints.iter().map(String::as_str));
                v.extend(machine.passive_wheel_joints.iter().map(String::as_str));
            }
            v
        }
        "builtin:trailer_steer" => {
            let mut v: Vec<&str> = c.steer_joints.iter().map(String::as_str).collect();
            if v.is_empty() {
                v.extend(machine.steering_joints.iter().map(String::as_str));
            }
            v
        }
        _ => c
            .target
            .as_deref()
            .into_iter()
            .chain(machine.tool_joints.iter().map(String::as_str).take(1))
            .take(1)
            .collect(),
    }
}

fn apply_service_controllers(
    inventory: Res<ControllerInventory>,
    keys: Res<MachineAgentKeys>,
    service: Res<ServiceCommands>,
    inputs: Res<MasterInputs>,
    active: Res<usd_bevy::physics::PhysicsActive>,
    mut physics: ResMut<PhysicsWorld>,
    joints: Query<(Entity, &UsdPrimRef, &usd_bevy::UsdPhysicsJoint)>,
    prims: Query<(Entity, &UsdPrimRef)>,
    parents: Query<&ChildOf>,
    mut warned: ResMut<WarnedOnce>,
) {
    if !active.0 {
        return;
    }
    for machine in &inventory.machines {
        let Some(scene_root) = machine.scene_root else {
            continue;
        };
        if !keys
            .0
            .values()
            .any(|k| k.scene_root == scene_root && k.machine_id == machine.id)
        {
            continue;
        }
        for controller in &machine.controllers {
            if !controller.enabled || !SERVICE_TYPES.contains(&controller.controller_type.as_str())
            {
                continue;
            }
            let key = ControllerKey::new(scene_root, &machine.id, &controller.instance);
            let empty = HashMap::new();
            let props = service.0.get(&key).unwrap_or(&empty);
            let joint_prims = controller_joints(machine, controller);
            if joint_prims.is_empty() {
                if warned
                    .0
                    .insert(format!("{}:{}:nojoint", machine.id, controller.instance))
                {
                    warn!(
                        "gearbox-services: {} controller `{}` on `{}` names no joint (target or role)",
                        controller.controller_type, controller.instance, machine.id
                    );
                }
                continue;
            }
            for prim in joint_prims {
                let Some(j) = resolve_joint(scene_root, prim, &joints, &parents, &physics) else {
                    continue;
                };
                let ok = match controller.controller_type.as_str() {
                    "builtin:joint_position" | "builtin:hitch" => {
                        let position = num(props, "position")
                            .or_else(|| num(props, "value"))
                            .unwrap_or(0.0)
                            .clamp(0.0, 1.0);
                        let range = num(props, "range").unwrap_or(1.0);
                        with_joint(&mut physics, &j, |g| {
                            g.set_motor_position(
                                j.axis,
                                position * range,
                                POSITION_STIFFNESS,
                                POSITION_DAMPING,
                            );
                            g.set_motor_max_force(j.axis, MOTOR_MAX_FORCE);
                        })
                    }
                    "builtin:pto" => {
                        let rpm = num(props, "rpm").unwrap_or(DEFAULT_PTO_RPM);
                        let engaged = flag(props, "engaged").unwrap_or(false);
                        let vel = if engaged {
                            rpm * std::f64::consts::TAU / 60.0
                        } else {
                            0.0
                        };
                        with_joint(&mut physics, &j, |g| {
                            g.set_motor_velocity(j.axis, vel, VELOCITY_FACTOR);
                            g.set_motor_max_force(j.axis, MOTOR_MAX_FORCE);
                        })
                    }
                    "builtin:hydraulic_valve" => {
                        let flow = num(props, "flow")
                            .or_else(|| num(props, "value"))
                            .unwrap_or(0.0)
                            .clamp(-1.0, 1.0);
                        let rate = num(props, "rate").unwrap_or(DEFAULT_VALVE_RATE);
                        with_joint(&mut physics, &j, |g| {
                            g.set_motor_velocity(j.axis, flow * rate, VELOCITY_FACTOR);
                            g.set_motor_max_force(j.axis, MOTOR_MAX_FORCE);
                        })
                    }
                    "builtin:brake" => {
                        let level = num(props, "level")
                            .or_else(|| num(props, "value"))
                            .unwrap_or(0.0)
                            .clamp(0.0, 1.0);
                        with_joint(&mut physics, &j, |g| {
                            g.set_motor_velocity(j.axis, 0.0, level * BRAKE_FACTOR);
                            g.set_motor_max_force(j.axis, MOTOR_MAX_FORCE * level);
                        })
                    }
                    "builtin:trailer_steer" => {
                        let max = controller
                            .max_steer_deg
                            .map(|d| d as f64)
                            .unwrap_or(DEFAULT_TRAILER_STEER_DEG)
                            .to_radians();
                        let angle = match num(props, "angle_rad") {
                            Some(a) => a,
                            None => auto_trailer_steer(
                                machine, scene_root, &inputs, &physics, &prims, &parents,
                            )
                            .unwrap_or(0.0),
                        }
                        .clamp(-max, max);
                        with_joint(&mut physics, &j, |g| {
                            g.set_motor_position(
                                j.axis,
                                angle,
                                POSITION_STIFFNESS,
                                POSITION_DAMPING,
                            );
                            g.set_motor_max_force(j.axis, MOTOR_MAX_FORCE);
                        })
                    }
                    _ => true,
                };
                if !ok
                    && warned
                        .0
                        .insert(format!("{}:{}:{prim}", machine.id, controller.instance))
                {
                    warn!(
                        "gearbox-services: no rapier joint for {prim} on `{}`; `{}` cannot drive it",
                        machine.id, controller.instance
                    );
                }
            }
        }
    }
}

/// A steered trailer axle follows the master's heading when nobody commands
/// an angle.
fn auto_trailer_steer(
    machine: &MachineInstanceSpec,
    scene_root: Entity,
    inputs: &MasterInputs,
    physics: &PhysicsWorld,
    prims: &Query<(Entity, &UsdPrimRef)>,
    parents: &Query<&ChildOf>,
) -> Option<f64> {
    let master = inputs.0.get(&machine.id)?;
    let body_prim = machine
        .body
        .as_deref()
        .or_else(|| machine.links.base().and_then(|b| b.body_prim.as_deref()))?;
    let entity = find_prim_entity(scene_root, body_prim, prims, parents)?;
    let body = physics
        .entity_to_body
        .get(&entity)
        .and_then(|h| physics.bodies.get(*h))?;
    let heading = body_forward_vector(body).map(|f| f.x.atan2(f.z))?;
    let diff = master.heading_rad - heading;
    Some((diff + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI)
}

/// Every attached slave learns its master's state, hitch and PTO values.
fn feed_master_inputs(
    attachments: Res<Attachments>,
    inventory: Res<ControllerInventory>,
    keys: Res<MachineAgentKeys>,
    states: Res<ControllerStates>,
    service: Res<ServiceCommands>,
    mut inputs: ResMut<MasterInputs>,
) {
    inputs.0.clear();
    for a in &attachments.0 {
        let Some(master) = machine_for(&inventory, &keys, &a.master_ns) else {
            continue;
        };
        let Some(slave) = machine_for(&inventory, &keys, &a.slave_ns) else {
            continue;
        };
        let Some(scene_root) = master.scene_root else {
            continue;
        };
        let mut state = MasterState {
            master_id: master.id.clone(),
            ..Default::default()
        };
        if let Some(key) = keys.0.get(&a.master_ns)
            && let Some(s) = states.states.get(key)
        {
            state.ground_speed_mps = s.linear_speed_mps;
            state.heading_rad = s.heading_rad;
            state.roll_rad = s.roll_rad;
            state.pitch_rad = s.pitch_rad;
        }
        for c in &master.controllers {
            let key = ControllerKey::new(scene_root, &master.id, &c.instance);
            let props = service.0.get(&key);
            match c.controller_type.as_str() {
                "builtin:hitch" => {
                    let position = props
                        .and_then(|p| num(p, "position").or_else(|| num(p, "value")))
                        .unwrap_or(0.0);
                    state.hitch.insert(c.instance.clone(), position);
                }
                "builtin:pto" => {
                    let rpm = props.and_then(|p| num(p, "rpm")).unwrap_or(DEFAULT_PTO_RPM);
                    let engaged = props.and_then(|p| flag(p, "engaged")).unwrap_or(false);
                    state.pto.insert(c.instance.clone(), (rpm, engaged));
                }
                _ => {}
            }
        }
        inputs.0.insert(slave.id.clone(), state);
    }
}
