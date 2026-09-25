//! Service controllers (`specs/TOOLS_SPEC.md` §6.2, §7.2, §7.4) and the
//! exchange between a master and its slaves: the master's state reaches
//! every slave controller as inputs, a slave's granted requests act on the
//! master, and bound PTO and valve joints on the slave follow the master's
//! services. Every working part is a link of the tree; values live on links
//! and a controller that drives a joint reads the values of the link that
//! joint moves.

use std::collections::{HashMap, HashSet};

use crate::physics::PhysicsWorld;
use bevy::prelude::*;
use gearbox_api::GearboxBus;
use crate::physics::backend::{BodyId, DeviceCommand, JointAxis, JointId, JointMut, MotorModel};
use usd_bevy::UsdPrimRef;

use crate::attach::Attachments;
use crate::controller::{
    CmdVel, ControllerInventory, ControllerKey, ControllerSpec, ControllerStates, MachineAgentKeys,
    MachineInstanceSpec, body_forward_vector, find_prim_entity,
};
use crate::links::{CouplingSide, LinkSpec, LinkTree};

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

/// Live named values per link: (machine id, link name, value name) → value.
#[derive(Resource, Default)]
pub struct LinkValues(pub HashMap<(String, String, String), f64>);

impl LinkValues {
    pub fn get(&self, machine: &str, link: &str, name: &str) -> Option<f64> {
        self.0
            .get(&(machine.to_string(), link.to_string(), name.to_string()))
            .copied()
    }

    pub fn set(&mut self, machine: &str, link: &str, name: &str, value: f64) {
        self.0.insert(
            (machine.to_string(), link.to_string(), name.to_string()),
            value,
        );
    }

    /// Every value of one link, by name.
    pub fn of_link(&self, machine: &str, link: &str) -> HashMap<String, f64> {
        self.0
            .iter()
            .filter(|((m, l, _), _)| m == machine && l == link)
            .map(|((_, _, n), v)| (n.clone(), *v))
            .collect()
    }

    /// Every value of one machine, sorted by link then name.
    pub fn of_machine(&self, machine: &str) -> Vec<(String, String, f64)> {
        let mut out: Vec<(String, String, f64)> = self
            .0
            .iter()
            .filter(|((m, _, _), _)| m == machine)
            .map(|((_, l, n), v)| (l.clone(), n.clone(), *v))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        out
    }
}

#[derive(Resource, Default)]
struct WarnedOnce(HashSet<String>);

/// Machines whose authored link values have been copied into `LinkValues`.
#[derive(Resource, Default)]
pub(crate) struct SeededLinkValues(pub(crate) HashSet<String>);

pub struct ServicesPlugin;

#[cfg(test)]
pub(crate) fn benchmark_schedule(app: &mut App) -> bevy::ecs::schedule::Schedule {
    app.init_resource::<MasterInputs>().init_resource::<ServiceCommands>().init_resource::<WarnedOnce>();
    let mut schedule = bevy::ecs::schedule::Schedule::default();
    schedule.add_systems(apply_service_controllers);
    schedule
}

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ServiceCommandSet;

impl Plugin for ServicesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MasterInputs>()
            .init_resource::<TimRequests>()
            .init_resource::<ServiceCommands>()
            .init_resource::<LinkValues>()
            .init_resource::<WarnedOnce>()
            .init_resource::<SeededLinkValues>()
            .add_systems(
                Update,
                (
                    seed_link_values,
                    drain_service_commands.after(crate::attach::serve_attachments).in_set(ServiceCommandSet),
                    apply_service_controllers,
                    apply_process_controllers,
                )
                    .chain(),
            )
            .add_systems(PostUpdate, feed_master_inputs);
    }
}

fn machine_for<'a>(
    inventory: &'a ControllerInventory,
    keys: &MachineAgentKeys,
    machine_id: &str,
) -> Option<&'a MachineInstanceSpec> {
    let key = keys.0.get(machine_id)?;
    inventory
        .machines
        .iter()
        .find(|m| m.scene_root == Some(key.scene_root) && m.id == key.machine_id)
}

fn has_agent(keys: &MachineAgentKeys, machine: &MachineInstanceSpec) -> bool {
    machine.scene_root.is_some_and(|root| {
        keys.0
            .values()
            .any(|k| k.scene_root == root && k.machine_id == machine.id)
    })
}

fn num(props: &HashMap<String, String>, key: &str) -> Option<f64> {
    props.get(key).and_then(|v| v.parse().ok())
}

fn flag(props: &HashMap<String, String>, key: &str) -> Option<bool> {
    props
        .get(key)
        .map(|v| matches!(v.as_str(), "1" | "true" | "on" | "yes"))
}

/// Values authored on the links become the live values once per machine.
fn seed_link_values(
    inventory: Res<ControllerInventory>,
    keys: Res<MachineAgentKeys>,
    mut values: ResMut<LinkValues>,
    mut seeded: ResMut<SeededLinkValues>,
) {
    for machine in &inventory.machines {
        if !has_agent(&keys, machine) || !seeded.0.insert(machine.id.clone()) {
            continue;
        }
        for link in &machine.links.links {
            for (name, value) in &link.values {
                if values.get(&machine.id, &link.name, name).is_none() {
                    values.set(&machine.id, &link.name, name, *value);
                }
            }
        }
    }
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
/// attached slave act on the master when granted; `link=` + `name=` set a
/// value on a link of the tree.
fn drain_service_commands(
    inventory: Res<ControllerInventory>,
    keys: Res<MachineAgentKeys>,
    bus: Option<ResMut<GearboxBus>>,
    attachments: Res<Attachments>,
    mut service: ResMut<ServiceCommands>,
    mut tim: ResMut<TimRequests>,
    mut values: ResMut<LinkValues>,
    mut warned: ResMut<WarnedOnce>,
) {
    let Some(mut bus) = bus else { return };
    let machine_ids: Vec<String> = bus.machines.keys().cloned().collect();
    for machine_id in machine_ids {
        let Some(machine) = machine_for(&inventory, &keys, &machine_id) else {
            continue;
        };
        let scene_root = machine.scene_root.expect("keyed machines have a root");
        let Some(agent) = bus.machines.get_mut(&machine_id) else {
            continue;
        };
        let mut keep = Vec::new();
        for cmd in agent.commands.drain(..) {
            let mut props: HashMap<String, String> = cmd.props().iter().into_iter().collect();
            if props.contains_key("tool") {
                keep.push(cmd);
                continue;
            }
            let value = num(&props, "value").unwrap_or(cmd.value);
            if let Some(scope) = props.get("tyre_pressure_scope") {
                if let Err(error) = crate::controller::wheel_forces::set_pressure_group(machine, &mut values, scope, value) {
                    warn!("gearbox-services: pressure request rejected for {machine_id}: {error}");
                }
                continue;
            }
            if let Some(request) = props.get("request").cloned() {
                let Some(att) = attachments.0.iter().find(|a| a.slave_id == machine_id) else {
                    if warned.0.insert(format!("{machine_id}:request:{request}")) {
                        warn!("gearbox-services: `{machine_id}` requested `{request}` but is not attached");
                    }
                    continue;
                };
                let Some(master) = machine_for(&inventory, &keys, &att.master_id) else {
                    continue;
                };
                if !master.grants.iter().any(|g| *g == request) {
                    if warned.0.insert(format!("{machine_id}:denied:{request}")) {
                        warn!(
                            "gearbox-services: `{}` does not grant `{request}` to `{machine_id}`; ignored",
                            att.master_id
                        );
                    }
                    continue;
                }
                if !apply_request(&request, value, master, &mut service, &mut tim)
                    && warned.0.insert(format!("{machine_id}:norequest:{request}"))
                {
                    warn!(
                        "gearbox-services: `{}` grants `{request}` but has no controller for it",
                        att.master_id
                    );
                }
                continue;
            }
            if let Some(link) = props.get("link").cloned() {
                let Some(name) = props.get("name").cloned() else {
                    if warned.0.insert(format!("{machine_id}:link:noname")) {
                        warn!("gearbox-services: `{machine_id}`: a link value needs `name`");
                    }
                    continue;
                };
                if machine.links.get(&link).is_some() {
                    values.set(&machine.id, &link, &name, value);
                } else if warned.0.insert(format!("{machine_id}:link:{link}")) {
                    warn!("gearbox-services: `{machine_id}` has no link `{link}`; value dropped");
                }
                continue;
            }
            let Some(instance) = props.remove("controller") else {
                continue;
            };
            let Some(controller) = machine.controllers.iter().find(|c| c.instance == instance)
            else {
                if warned.0.insert(format!("{machine_id}:controller:{instance}")) {
                    warn!("gearbox-services: `{machine_id}` has no controller `{instance}`");
                }
                continue;
            };
            if !SERVICE_TYPES.contains(&controller.controller_type.as_str()) {
                continue;
            }
            if !props.contains_key("value") {
                props.insert("value".to_string(), cmd.value.to_string());
            }
            // The link this controller's joint moves takes the same values,
            // so `cmd` and `set-value` agree on what the link is doing.
            for prim in controller_joints(machine, controller) {
                let Some(link) = moved_link(&machine.links, prim) else {
                    continue;
                };
                for (k, v) in &props {
                    if let Some(v) = numeric(v) {
                        values.set(&machine.id, &link.name, k, v);
                    }
                }
            }
            let key = ControllerKey::new(scene_root, &machine.id, &instance);
            service.0.entry(key).or_default().extend(props);
        }
        agent.commands = keep;
    }
}

fn numeric(v: &str) -> Option<f64> {
    match v {
        "true" | "on" | "yes" => Some(1.0),
        "false" | "off" | "no" => Some(0.0),
        _ => v.parse().ok(),
    }
}

/// The joint a controller drives: the bodies it connects and its kind.
struct JointRef {
    body0: BodyId,
    body1: BodyId,
    axis: JointAxis,
}

fn resolve_joint(
    scene_root: Entity,
    prim_path: &str,
    joints: &Query<(
        Entity,
        &UsdPrimRef,
        &crate::physics::markers::UsdPhysicsJoint,
    )>,
    parents: &Query<&ChildOf>,
    physics: &PhysicsWorld,
) -> Option<JointRef> {
    let (_, _, joint) = joints.iter().find(|(entity, prim, _)| {
        prim.path == prim_path && crate::controller::is_descendant_of(*entity, scene_root, parents)
    })?;
    let body0 = physics.entity_to_body.get(&joint.body0?).copied()?;
    let body1 = physics.entity_to_body.get(&joint.body1?).copied()?;
    let axis = match joint.kind {
        crate::physics::markers::UsdJointKind::Prismatic => JointAxis::LinX,
        _ => JointAxis::AngX,
    };
    Some(JointRef { body0, body1, axis })
}

/// What a service controller asks of its joint.
#[derive(Clone, Copy, Debug)]
enum Actuation {
    /// Hold a position (rad or m).
    Position(f64),
    /// Run at a velocity (rad/s or m/s); `max_force` caps the runtime's own
    /// servo, a motor device keeps its own limit.
    Velocity { target: f64, max_force: f64 },
    /// Brake damping (N·m·s/rad or N·s/m).
    Brake(f64),
}

/// The joint between two bodies; a constraint joint wins over a
/// reduced-coordinate one when both exist.
fn joint_between(physics: &PhysicsWorld, j: &JointRef) -> Option<JointId> {
    let between = physics.joints_between(j.body0, j.body1);
    between
        .iter()
        .copied()
        .find(|id| !physics.joint_is_reduced(*id))
        .or(between.first().copied())
}

/// Drives a joint through its motor device when it has one, otherwise with
/// the runtime's own servo. Brakes go through the joint's brake channel,
/// which the drive's motors leave alone.
fn actuate(physics: &mut PhysicsWorld, j: &JointRef, act: Actuation) -> bool {
    let Some(id) = joint_between(physics, j) else {
        return false;
    };
    let device = physics.has_motor(id);
    match act {
        Actuation::Brake(damping) => physics.set_joint_brake(id, damping).is_ok(),
        Actuation::Position(target) if device => physics.command_motor(id, DeviceCommand::Position(target)).is_ok(),
        Actuation::Velocity { target, .. } if device => {
            physics.command_motor(id, DeviceCommand::Velocity(target)).is_ok()
        }
        Actuation::Position(target) => servo(physics, id, j.axis, |g| {
            g.set_motor_position(j.axis, target, POSITION_STIFFNESS, POSITION_DAMPING);
            g.set_motor_max_force(j.axis, MOTOR_MAX_FORCE);
        }),
        Actuation::Velocity { target, max_force } => servo(physics, id, j.axis, |g| {
            g.set_motor_velocity(j.axis, target, VELOCITY_FACTOR);
            g.set_motor_max_force(j.axis, max_force);
        }),
    }
}

/// The runtime's servo on a joint without a motor device.
fn servo(physics: &mut PhysicsWorld, id: JointId, axis: JointAxis, f: impl FnOnce(&mut dyn JointMut)) -> bool {
    let Some(joint) = physics.joint_mut(id, true) else {
        return false;
    };
    joint.set_motor_model(axis, MotorModel::Acceleration);
    f(joint);
    true
}

const POSITION_STIFFNESS: f64 = 4_000.0;
const POSITION_DAMPING: f64 = 400.0;
const MOTOR_MAX_FORCE: f64 = 50_000.0;
const VELOCITY_FACTOR: f64 = 200.0;
/// Brake damping at full level when the joint's brake authors no
/// `maxDamping` (N·m·s/rad).
const BRAKE_DAMPING: f64 = 1.0e6;
const DEFAULT_PTO_RPM: f64 = 540.0;
const MAX_PTO_RPM: f64 = 1200.0;
/// A PTO stub weighs a few kilograms; the hitch torque cap would throw the
/// machine.
const PTO_MAX_TORQUE: f64 = 150.0;
const JOINT_VELOCITY_MAX_TORQUE: f64 = 2_000.0;
const DEFAULT_VALVE_RATE: f64 = 0.5;
const DEFAULT_TRAILER_STEER_DEG: f64 = 35.0;

pub(crate) fn controller_joints<'a>(
    machine: &'a MachineInstanceSpec,
    c: &'a ControllerSpec,
) -> Vec<&'a str> {
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

/// The link a joint moves: the tree link connected through that joint prim.
pub(crate) fn moved_link<'a>(tree: &'a LinkTree, joint_prim: &str) -> Option<&'a LinkSpec> {
    tree.links
        .iter()
        .find(|l| l.joint_prim.as_deref() == Some(joint_prim))
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
    values: Res<LinkValues>,
    inputs: Res<MasterInputs>,
    active: Res<gearbox_api::PhysicsActive>,
    mut physics: ResMut<PhysicsWorld>,
    joints: Query<(
        Entity,
        &UsdPrimRef,
        &crate::physics::markers::UsdPhysicsJoint,
    )>,
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
        if !has_agent(&keys, machine) {
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
                // Controller props, overlaid by the values of the link this
                // joint moves, so a link addressed from the tree moves.
                let mut props: HashMap<String, String> =
                    service.0.get(&key).cloned().unwrap_or_default();
                if let Some(link) = moved_link(&machine.links, prim) {
                    for (name, value) in values.of_link(&machine.id, &link.name) {
                        props.insert(name, value.to_string());
                    }
                }
                let props = &props;
                let ok = match controller.controller_type.as_str() {
                    "builtin:joint_position" | "builtin:hitch" => {
                        let position = num(props, "position")
                            .or_else(|| num(props, "value"))
                            .unwrap_or(0.0)
                            .clamp(0.0, 1.0);
                        let range = num(props, "range").unwrap_or(1.0);
                        actuate(&mut physics, &j, Actuation::Position(position * range))
                    }
                    "builtin:joint_velocity" => {
                        let bound = bound_pto.as_deref() == Some(prim);
                        let vel = match (num(props, "velocity"), bound, master) {
                            (Some(v), _, _) => v,
                            (None, true, Some(m)) => m.pto_rad_s(),
                            _ => 0.0,
                        };
                        let act = Actuation::Velocity { target: vel, max_force: JOINT_VELOCITY_MAX_TORQUE };
                        actuate(&mut physics, &j, act)
                    }
                    "builtin:pto" => {
                        let rpm = num(props, "rpm")
                            .unwrap_or(DEFAULT_PTO_RPM)
                            .clamp(0.0, MAX_PTO_RPM);
                        let engaged = flag(props, "engaged").unwrap_or(false);
                        let vel = if engaged {
                            rpm * std::f64::consts::TAU / 60.0
                        } else {
                            0.0
                        };
                        actuate(&mut physics, &j, Actuation::Velocity { target: vel, max_force: PTO_MAX_TORQUE })
                    }
                    "builtin:hydraulic_valve" => {
                        let flow = num(props, "flow")
                            .or_else(|| num(props, "value"))
                            .unwrap_or(0.0)
                            .clamp(-1.0, 1.0);
                        let rate = num(props, "rate").unwrap_or(DEFAULT_VALVE_RATE);
                        let act = Actuation::Velocity { target: flow * rate, max_force: MOTOR_MAX_FORCE };
                        actuate(&mut physics, &j, act)
                    }
                    "builtin:brake" => {
                        // Uncoupled and uncommanded, a trailer holds itself
                        // like a parking brake; hitched, it rolls free.
                        let parked = if master.is_none() { 1.0 } else { 0.0 };
                        let level = num(props, "level")
                            .or_else(|| num(props, "value"))
                            .unwrap_or(parked)
                            .clamp(0.0, 1.0);
                        let full = crate::devices::brake_capacity(machine, prim).unwrap_or(BRAKE_DAMPING);
                        actuate(&mut physics, &j, Actuation::Brake(level * full))
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
                        actuate(&mut physics, &j, Actuation::Position(angle))
                    }
                    _ => true,
                };
                if !ok
                    && warned
                        .0
                        .insert(format!("{}:{}:{prim}", machine.id, controller.instance))
                {
                    warn!(
                        "gearbox-services: no physics joint for {prim} on `{}`; `{}` cannot drive it",
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
            actuate(&mut physics, &j, Actuation::Velocity { target: m.pto_rad_s(), max_force: PTO_MAX_TORQUE });
        }
        for (n, valve_joint) in bound_valves.iter().enumerate() {
            if controlled.contains(valve_joint.as_str()) {
                continue;
            }
            let flow = m.valves.get(n).copied().unwrap_or(0.0);
            if let Some(j) = resolve_joint(scene_root, valve_joint, &joints, &parents, &physics) {
                let act = Actuation::Velocity { target: flow * DEFAULT_VALVE_RATE, max_force: MOTOR_MAX_FORCE };
                actuate(&mut physics, &j, act);
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
        .and_then(|h| physics.body(*h))?;
    let heading = body_forward_vector(body).map(|f| f.x.atan2(f.z))?;
    let diff = master.heading_rad - heading;
    let articulation =
        (diff + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI;
    // Forced steering: the steered axle sits behind the coupler, so it turns
    // against the tractor's turn and the trailer tracks the tractor's path. A
    // positive angle turns the wheels to the trailer's left.
    Some(-TRAILER_STEER_GAIN * articulation)
}

/// Trailer steer angle per radian of articulation between tractor and trailer.
const TRAILER_STEER_GAIN: f64 = 1.0;

fn element_kind(link: &LinkSpec) -> Option<&str> {
    link.element.as_ref().map(|e| e.kind.as_str())
}

/// Section and rate control over the link tree (`TOOLS_SPEC.md` §7.4):
/// work state follows the setpoint while the master moves, totals and tank
/// content follow speed × active width. Every value lives on a link.
fn apply_process_controllers(
    inventory: Res<ControllerInventory>,
    keys: Res<MachineAgentKeys>,
    inputs: Res<MasterInputs>,
    active: Res<gearbox_api::PhysicsActive>,
    time: Res<Time>,
    mut values: ResMut<LinkValues>,
) {
    if !active.0 {
        return;
    }
    let dt = time.delta_secs_f64();
    for machine in &inventory.machines {
        if !has_agent(&keys, machine) {
            continue;
        }
        let speed = inputs
            .0
            .get(&machine.id)
            .map(|m| m.ground_speed_mps)
            .unwrap_or(0.0);
        let id = machine.id.as_str();
        let tree = &machine.links;
        for controller in &machine.controllers {
            if !controller.enabled || !PROCESS_TYPES.contains(&controller.controller_type.as_str())
            {
                continue;
            }
            let target = controller.target.as_deref().and_then(|t| tree.by_prim(t));
            match controller.controller_type.as_str() {
                "builtin:section_control" => {
                    let function = target
                        .filter(|l| element_kind(l) == Some("function"))
                        .or_else(|| {
                            tree.links
                                .iter()
                                .find(|l| element_kind(l) == Some("function"))
                        });
                    let Some(function) = function else { continue };
                    let sections: Vec<&LinkSpec> = tree
                        .links
                        .iter()
                        .filter(|l| {
                            element_kind(l) == Some("section")
                                && l.parent.as_deref() == Some(function.name.as_str())
                        })
                        .collect();
                    let control_on = values
                        .get(id, &function.name, "SectionControlState")
                        .unwrap_or(1.0)
                        > 0.5;
                    let mut active_width = 0.0;
                    for s in &sections {
                        let setpoint = values.get(id, &s.name, "SetpointWorkState").unwrap_or(0.0);
                        let working = control_on && setpoint > 0.5 && speed > 0.05;
                        values.set(id, &s.name, "ActualWorkState", working as u8 as f64);
                        if working {
                            active_width +=
                                values.get(id, &s.name, "ActualWorkingWidth").unwrap_or(0.0);
                        }
                    }
                    if !sections.is_empty() {
                        values.set(
                            id,
                            &function.name,
                            "ActualWorkState",
                            (active_width > 0.0) as u8 as f64,
                        );
                    }
                    let area = values.get(id, &function.name, "TotalArea").unwrap_or(0.0);
                    values.set(
                        id,
                        &function.name,
                        "TotalArea",
                        area + speed * active_width * dt / 10_000.0,
                    );
                    let dist = values
                        .get(id, &function.name, "EffectiveTotalDistance")
                        .unwrap_or(0.0);
                    values.set(
                        id,
                        &function.name,
                        "EffectiveTotalDistance",
                        dist + if active_width > 0.0 { speed * dt } else { 0.0 },
                    );
                }
                "builtin:rate_control" => {
                    let bin = target
                        .filter(|l| element_kind(l) == Some("bin"))
                        .or_else(|| tree.links.iter().find(|l| element_kind(l) == Some("bin")));
                    let Some(bin) = bin else { continue };
                    let setpoint = values
                        .get(id, &bin.name, "SetpointVolumePerAreaApplicationRate")
                        .unwrap_or(0.0);
                    let active_width: f64 = tree
                        .links
                        .iter()
                        .filter(|l| element_kind(l) == Some("section"))
                        .filter(|l| values.get(id, &l.name, "ActualWorkState").unwrap_or(0.0) > 0.5)
                        .map(|l| values.get(id, &l.name, "ActualWorkingWidth").unwrap_or(0.0))
                        .sum();
                    let content = values
                        .get(id, &bin.name, "ActualVolumeContent")
                        .unwrap_or(0.0);
                    let applying = active_width > 0.0 && content > 0.0 && speed > 0.05;
                    let actual = if applying { setpoint } else { 0.0 };
                    values.set(id, &bin.name, "ActualVolumePerAreaApplicationRate", actual);
                    if applying {
                        // l/ha × ha/s → litres drained this step.
                        let drained = actual * speed * active_width * dt / 10_000.0;
                        values.set(
                            id,
                            &bin.name,
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
        let Some(master) = machine_for(&inventory, &keys, &a.master_id) else {
            continue;
        };
        let Some(slave) = machine_for(&inventory, &keys, &a.slave_id) else {
            continue;
        };
        let Some(scene_root) = master.scene_root else {
            continue;
        };
        let mut state = MasterState {
            master_id: master.id.clone(),
            ..Default::default()
        };
        if let Some(key) = keys.0.get(&a.master_id)
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
                    let rpm = props
                        .and_then(|p| num(p, "rpm"))
                        .unwrap_or(DEFAULT_PTO_RPM)
                        .clamp(0.0, MAX_PTO_RPM);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::backend::{BodyDesc, ColliderDesc, DVec3, DeviceLimits, JointDesc, JointKind, Pose, Shape};

    /// An arm on a revolute joint to a fixed post, in zero gravity.
    fn arm() -> (PhysicsWorld, JointRef, JointId) {
        let mut world = PhysicsWorld::default();
        world.set_gravity(DVec3::ZERO);
        let mut body = |desc: BodyDesc, at: DVec3| {
            let id = world.insert_body(desc.pose(Pose::from_translation(at)));
            world.insert_collider(ColliderDesc::new(Shape::Ball { radius: 0.1 }).density(1000.0).parent(id)).unwrap();
            id
        };
        let (post, arm) = (body(BodyDesc::fixed(), DVec3::ZERO), body(BodyDesc::dynamic(), DVec3::X));
        let desc = JointDesc::new(JointKind::Revolute { axis: DVec3::Y }, Pose::IDENTITY, Pose::from_translation(DVec3::NEG_X));
        let joint = world.backend.insert_joint(post, arm, desc);
        (world, JointRef { body0: post, body1: arm, axis: JointAxis::AngX }, joint)
    }

    #[test]
    fn service_commands_go_to_the_joints_motor_device() {
        let (mut world, j, joint) = arm();
        let limits = DeviceLimits { max_velocity: 0.5, ..Default::default() };
        world.insert_motor(joint, limits, 500.0).unwrap();
        assert!(actuate(&mut world, &j, Actuation::Position(0.3)));
        world.step();
        let motor = world.motor_output(joint).unwrap();
        assert_eq!(motor.command, DeviceCommand::Position(0.3));
        assert!((motor.commanded_velocity - 0.5).abs() < 1e-9, "{motor:?}");
        for _ in 0..240 {
            world.step();
        }
        assert!((world.motor_output(joint).unwrap().position - 0.3).abs() < 1e-3);
        assert!(actuate(&mut world, &j, Actuation::Velocity { target: -0.2, max_force: 1.0 }));
        world.step();
        assert_eq!(world.motor_output(joint).unwrap().command, DeviceCommand::Velocity(-0.2));
    }

    #[test]
    fn joints_without_a_device_keep_the_runtime_servo_and_brakes_use_damping() {
        let (mut world, j, joint) = arm();
        assert!(!world.has_motor(joint));
        assert!(actuate(&mut world, &j, Actuation::Position(0.3)));
        for _ in 0..600 {
            world.step();
        }
        let angle = world.joint(joint).unwrap().motor_position(JointAxis::AngX).unwrap();
        assert!((angle - 0.3).abs() < 1e-2, "{angle}");
        assert!(actuate(&mut world, &j, Actuation::Brake(250.0)));
        world.step();
        assert_eq!(world.motor_output(joint).unwrap().brake_damping, 250.0);
    }
}
