//! Service and process-data controllers (`specs/TOOLS_SPEC.md` §6.2, §7.2,
//! §7.4) and the ISOBUS shaped exchange between a master and its slaves:
//! the master's state reaches every slave controller as inputs, a slave's
//! granted requests act on the master, and bound PTO and valve joints on
//! the slave follow the master's services.

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
use crate::elements::ElementKind;
use crate::links::CouplingSide;

pub const SERVICE_TYPES: [&str; 7] = [
    "builtin:hitch",
    "builtin:pto",
    "builtin:hydraulic_valve",
    "builtin:joint_position",
    "builtin:joint_velocity",
    "builtin:brake",
    "builtin:trailer_steer",
];

pub const PROCESS_TYPES: [&str; 2] = ["builtin:section_control", "builtin:rate_control"];

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
    /// `builtin:hydraulic_valve` flows −1..1 in controller order.
    pub valves: Vec<f64>,
}

impl MasterState {
    /// The first engaged PTO's shaft speed in rad/s, else zero.
    pub fn pto_rad_s(&self) -> f64 {
        self.pto
            .values()
            .find(|(_, engaged)| *engaged)
            .map(|(rpm, _)| rpm * std::f64::consts::TAU / 60.0)
            .unwrap_or(0.0)
    }
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

/// Live process data: (machine id, element number, DDI name) → value.
#[derive(Resource, Default)]
pub struct ProcessData(pub HashMap<(String, u32, String), f64>);

impl ProcessData {
    pub fn get(&self, machine: &str, element: u32, ddi: &str) -> Option<f64> {
        self.0
            .get(&(machine.to_string(), element, ddi.to_string()))
            .copied()
    }

    pub fn set(&mut self, machine: &str, element: u32, ddi: &str, value: f64) {
        self.0
            .insert((machine.to_string(), element, ddi.to_string()), value);
    }

    /// Every value of one machine, sorted by element then DDI.
    pub fn of_machine(&self, machine: &str) -> Vec<(u32, String, f64)> {
        let mut out: Vec<(u32, String, f64)> = self
            .0
            .iter()
            .filter(|((m, _, _), _)| m == machine)
            .map(|((_, e, d), v)| (*e, d.clone(), *v))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        out
    }
}

#[derive(Resource, Default)]
struct WarnedOnce(HashSet<String>);

/// Machines whose authored process data has been copied into `ProcessData`.
#[derive(Resource, Default)]
struct SeededProcessData(HashSet<String>);

pub struct ServicesPlugin;

impl Plugin for ServicesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MasterInputs>()
            .init_resource::<TimRequests>()
            .init_resource::<ServiceCommands>()
            .init_resource::<ProcessData>()
            .init_resource::<WarnedOnce>()
            .init_resource::<SeededProcessData>()
            .add_systems(
                Update,
                (
                    drain_service_commands.after(crate::attach::serve_attachments),
                    apply_service_controllers,
                    apply_process_controllers,
                    draw_link_frames,
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

/// Write a granted slave request onto the master (`TOOLS_SPEC.md` §5.3).
fn apply_request(
    request: &str,
    value: f64,
    master: &MachineInstanceSpec,
    service: &mut ServiceCommands,
    tim: &mut TimRequests,
) -> bool {
    let Some(scene_root) = master.scene_root else {
        return false;
    };
    match request {
        "speed" => {
            tim.0.entry(master.id.clone()).or_default().linear_mps = value as f32;
            true
        }
        "steering" => {
            tim.0.entry(master.id.clone()).or_default().angular_rps = value as f32;
            true
        }
        _ => {
            let (kind, name) = request.split_once(':').unwrap_or((request, ""));
            let controller = match kind {
                "hitch" => master
                    .controllers
                    .iter()
                    .find(|c| c.controller_type == "builtin:hitch" && c.instance == name),
                "pto" => master
                    .controllers
                    .iter()
                    .find(|c| c.controller_type == "builtin:pto" && c.instance == name),
                "aux_valve" => name.parse::<usize>().ok().and_then(|n| {
                    master
                        .controllers
                        .iter()
                        .filter(|c| c.controller_type == "builtin:hydraulic_valve")
                        .nth(n)
                }),
                _ => None,
            };
            let Some(controller) = controller else {
                return false;
            };
            let key = ControllerKey::new(scene_root, &master.id, &controller.instance);
            let props = service.0.entry(key).or_default();
            match kind {
                "hitch" => {
                    props.insert("position".into(), value.to_string());
                }
                "pto" => {
                    props.insert("rpm".into(), value.abs().to_string());
                    props.insert("engaged".into(), (value > 0.0).to_string());
                }
                _ => {
                    props.insert("flow".into(), value.to_string());
                }
            }
            true
        }
    }
}

/// Commands from the bus land per controller; `request=` commands from an
/// attached slave act on the master when granted; `ddi=` commands set
/// process data on an element.
fn drain_service_commands(
    inventory: Res<ControllerInventory>,
    keys: Res<MachineAgentKeys>,
    bus: Option<ResMut<GearboxBus>>,
    attachments: Res<Attachments>,
    mut service: ResMut<ServiceCommands>,
    mut tim: ResMut<TimRequests>,
    mut pd: ResMut<ProcessData>,
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
                if !apply_request(&request, value, master, &mut service, &mut tim)
                    && warned.0.insert(format!("{ns}:norequest:{request}"))
                {
                    warn!(
                        "gearbox-services: `{}` grants `{request}` but has no controller for it",
                        att.master_ns
                    );
                }
                continue;
            }
            if let Some(ddi) = props.get("ddi").cloned() {
                let value = num(&props, "value").unwrap_or(cmd.value);
                if machine.elements.iter().any(|e| e.number == cmd.element) {
                    pd.set(&machine.id, cmd.element, &ddi, value);
                } else if warned.0.insert(format!("{ns}:element:{}", cmd.element)) {
                    warn!(
                        "gearbox-services: `{ns}` has no element {}; process data dropped",
                        cmd.element
                    );
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

/// The slave-side coupler binding this machine's joints to its master's
/// PTO and valves, when attached.
fn coupler_bindings(machine: &MachineInstanceSpec) -> Option<(Option<String>, Vec<String>)> {
    machine
        .links
        .couplings()
        .find(|(_, c)| c.side == CouplingSide::Coupler)
        .map(|(_, c)| (c.pto_joint.clone(), c.valve_joints.clone()))
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
        let master = inputs.0.get(&machine.id);
        let bindings = coupler_bindings(machine);
        let bound_pto = bindings.as_ref().and_then(|(p, _)| p.clone());
        let bound_valves = bindings.map(|(_, v)| v).unwrap_or_default();

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
                    "builtin:joint_velocity" => {
                        // A joint bound to the master's PTO turns with it
                        // unless a velocity was commanded directly.
                        let bound = bound_pto.as_deref() == Some(prim);
                        let vel = match (num(props, "velocity"), bound, master) {
                            (Some(v), _, _) => v,
                            (None, true, Some(m)) => m.pto_rad_s(),
                            _ => 0.0,
                        };
                        with_joint(&mut physics, &j, |g| {
                            g.set_motor_velocity(j.axis, vel, VELOCITY_FACTOR);
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

        // Bound joints with no controller of their own follow the master's
        // services directly (§6.2): the PTO stub spins, valve joints move.
        let Some(m) = master else {
            continue;
        };
        let controlled: HashSet<&str> = machine
            .controllers
            .iter()
            .flat_map(|c| controller_joints(machine, c))
            .collect();
        if let Some(pto) = bound_pto.as_deref()
            && !controlled.contains(pto)
            && let Some(j) = resolve_joint(scene_root, pto, &joints, &parents, &physics)
        {
            let vel = m.pto_rad_s();
            with_joint(&mut physics, &j, |g| {
                g.set_motor_velocity(j.axis, vel, VELOCITY_FACTOR);
                g.set_motor_max_force(j.axis, MOTOR_MAX_FORCE);
            });
        }
        for (n, valve_joint) in bound_valves.iter().enumerate() {
            if controlled.contains(valve_joint.as_str()) {
                continue;
            }
            let flow = m.valves.get(n).copied().unwrap_or(0.0);
            if let Some(j) = resolve_joint(scene_root, valve_joint, &joints, &parents, &physics) {
                with_joint(&mut physics, &j, |g| {
                    g.set_motor_velocity(j.axis, flow * DEFAULT_VALVE_RATE, VELOCITY_FACTOR);
                    g.set_motor_max_force(j.axis, MOTOR_MAX_FORCE);
                });
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

/// Section and rate control over the element tree (`TOOLS_SPEC.md` §7.4):
/// work state follows the setpoint while the master moves, totals and tank
/// content follow speed × active width.
fn apply_process_controllers(
    inventory: Res<ControllerInventory>,
    keys: Res<MachineAgentKeys>,
    inputs: Res<MasterInputs>,
    active: Res<usd_bevy::physics::PhysicsActive>,
    time: Res<Time>,
    mut pd: ResMut<ProcessData>,
    mut seeded: ResMut<SeededProcessData>,
) {
    let dt = time.delta_secs_f64();
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
        if seeded.0.insert(machine.id.clone()) {
            for e in &machine.elements {
                for (ddi, value) in &e.process_data {
                    if pd.get(&machine.id, e.number, ddi).is_none() {
                        pd.set(&machine.id, e.number, ddi, *value);
                    }
                }
            }
        }
        if !active.0 {
            continue;
        }
        let speed = inputs
            .0
            .get(&machine.id)
            .map(|m| m.ground_speed_mps)
            .unwrap_or(0.0);
        let id = machine.id.as_str();
        for controller in &machine.controllers {
            if !controller.enabled || !PROCESS_TYPES.contains(&controller.controller_type.as_str())
            {
                continue;
            }
            let target = controller
                .target
                .as_deref()
                .and_then(|t| machine.elements.iter().find(|e| e.prim_path == t));
            match controller.controller_type.as_str() {
                "builtin:section_control" => {
                    let function =
                        target
                            .filter(|e| e.kind == ElementKind::Function)
                            .or_else(|| {
                                machine
                                    .elements
                                    .iter()
                                    .find(|e| e.kind == ElementKind::Function)
                            });
                    let Some(function) = function else { continue };
                    let sections: Vec<_> = machine
                        .elements
                        .iter()
                        .filter(|e| {
                            e.kind == ElementKind::Section && e.parent == Some(function.number)
                        })
                        .collect();
                    let control_on = pd
                        .get(id, function.number, "SectionControlState")
                        .unwrap_or(1.0)
                        > 0.5;
                    let mut active_width = 0.0;
                    for s in &sections {
                        let setpoint = pd.get(id, s.number, "SetpointWorkState").unwrap_or(0.0);
                        let working = control_on && setpoint > 0.5 && speed > 0.05;
                        pd.set(id, s.number, "ActualWorkState", working as u8 as f64);
                        if working {
                            active_width +=
                                pd.get(id, s.number, "ActualWorkingWidth").unwrap_or(0.0);
                        }
                    }
                    if !sections.is_empty() {
                        pd.set(
                            id,
                            function.number,
                            "ActualWorkState",
                            (active_width > 0.0) as u8 as f64,
                        );
                    }
                    let area = pd.get(id, function.number, "TotalArea").unwrap_or(0.0);
                    pd.set(
                        id,
                        function.number,
                        "TotalArea",
                        area + speed * active_width * dt / 10_000.0,
                    );
                    let dist = pd
                        .get(id, function.number, "EffectiveTotalDistance")
                        .unwrap_or(0.0);
                    pd.set(
                        id,
                        function.number,
                        "EffectiveTotalDistance",
                        dist + if active_width > 0.0 { speed * dt } else { 0.0 },
                    );
                }
                "builtin:rate_control" => {
                    let bin = target
                        .filter(|e| e.kind == ElementKind::Bin)
                        .or_else(|| machine.elements.iter().find(|e| e.kind == ElementKind::Bin));
                    let Some(bin) = bin else { continue };
                    let setpoint = pd
                        .get(id, bin.number, "SetpointVolumePerAreaApplicationRate")
                        .unwrap_or(0.0);
                    let active_width: f64 = machine
                        .elements
                        .iter()
                        .filter(|e| e.kind == ElementKind::Section)
                        .filter(|e| pd.get(id, e.number, "ActualWorkState").unwrap_or(0.0) > 0.5)
                        .map(|e| pd.get(id, e.number, "ActualWorkingWidth").unwrap_or(0.0))
                        .sum();
                    let content = pd.get(id, bin.number, "ActualVolumeContent").unwrap_or(0.0);
                    let applying = active_width > 0.0 && content > 0.0 && speed > 0.05;
                    let actual = if applying { setpoint } else { 0.0 };
                    pd.set(id, bin.number, "ActualVolumePerAreaApplicationRate", actual);
                    if applying {
                        // l/ha × ha/s → litres drained this step.
                        let drained = actual * speed * active_width * dt / 10_000.0;
                        pd.set(
                            id,
                            bin.number,
                            "ActualVolumeContent",
                            (content - drained).max(0.0),
                        );
                    }
                }
                _ => {}
            }
        }
    }
}

/// Every attached slave learns its master's state, hitch, PTO and valve
/// values.
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
                "builtin:hydraulic_valve" => {
                    let flow = props
                        .and_then(|p| num(p, "flow").or_else(|| num(p, "value")))
                        .unwrap_or(0.0);
                    state.valves.push(flow);
                }
                _ => {}
            }
        }
        // A nested slave inherits its master's inherited inputs.
        if let Some(inherited) = inputs.0.get(&master.id).cloned() {
            if state.pto.is_empty() {
                state.pto = inherited.pto;
            }
            if state.ground_speed_mps == 0.0 {
                state.ground_speed_mps = inherited.ground_speed_mps;
            }
        }
        inputs.0.insert(slave.id.clone(), state);
    }
}

/// Draw a frame at every link of a machine whose tf stream is on, so
/// `gearbox machine tf` shows the same thing in the window.
fn draw_link_frames(
    mut gizmos: Gizmos,
    inventory: Res<ControllerInventory>,
    keys: Res<MachineAgentKeys>,
    bus: Option<Res<GearboxBus>>,
    prims: Query<(Entity, &UsdPrimRef)>,
    parents: Query<&ChildOf>,
    transforms: Query<&GlobalTransform>,
) {
    let Some(bus) = bus else { return };
    for machine in &inventory.machines {
        let Some(scene_root) = machine.scene_root else {
            continue;
        };
        let on = keys
            .0
            .iter()
            .find(|(_, k)| k.scene_root == scene_root && k.machine_id == machine.id)
            .and_then(|(ns, _)| bus.machines.get(ns))
            .is_some_and(|agent| agent.tf_enabled());
        if !on {
            continue;
        }
        for link in &machine.links.links {
            let Some(entity) = find_prim_entity(scene_root, &link.prim_path, &prims, &parents)
            else {
                continue;
            };
            let Ok(gt) = transforms.get(entity) else {
                continue;
            };
            let size = if link.role == crate::links::LinkRole::Base {
                0.8
            } else {
                0.35
            };
            gizmos.axes(*gt, size);
        }
    }
}
