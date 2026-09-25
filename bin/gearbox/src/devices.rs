//! Actuator devices of machines, after Webots: motors and brakes on joints,
//! propellers, belts (conveyors, tank tracks) and connectors.
//!
//! The physics backend steps every device inside its step. Gearbox reads
//! them from the machine's USD, registers them once the physics objects
//! exist, applies the commands controllers send to
//! `/machines/<id>/actuate`, and publishes each device's reading on
//! `/machines/<id>/actuators/<device>`.

use std::collections::HashMap;

use bevy::prelude::*;
use gearbox_api::{ActuatorCommand, GearboxBus, Measurement, Props, measurement_kind as kind};
use openusd::sdf::{Path as SdfPath, Value};
use usd_bevy::UsdPrimRef;

use crate::controller::{ControllerInventory, MachineAgentKeys, find_prim_entity, read_attr, read_bool, read_float, read_token, type_name};
use crate::physics::backend::{
    BeltDesc, ConnectorDesc, ConnectorKind, DVec3, DeviceCommand, DeviceId, DeviceLimits, DeviceSetting, JointId,
    PhysicsBackend, Pose, PropellerDesc,
};
use crate::physics::PhysicsWorld;
use crate::usd_ext::StageExt;

pub struct DevicesPlugin;

impl Plugin for DevicesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MachineDevices>()
            .add_systems(Update, drive_devices.before(crate::physics::step_physics))
            .add_systems(PostUpdate, publish_devices);
    }
}

// ── Authoring ──────────────────────────────────────────────────────────────

/// What a device prim authors.
#[derive(Clone, Debug, PartialEq)]
pub enum DeviceKind {
    /// A joint's motor and/or brake: the motor's limits and available force,
    /// the brake's initial damping.
    Joint {
        motor: Option<(DeviceLimits, f64)>,
        brake: Option<f64>,
    },
    Propeller {
        thrust: [f64; 2],
        torque: [f64; 2],
        limits: DeviceLimits,
        max_torque: f64,
    },
    /// Running direction in the prim's frame.
    Belt { direction: [f64; 3], limits: DeviceLimits },
    /// `desc.body` and `desc.frame` are filled at registration; `locked`
    /// latches it from the start.
    Connector { desc: ConnectorDesc, locked: bool },
}

/// An authored device of a machine.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceSpec {
    pub name: String,
    pub prim: String,
    pub kind: DeviceKind,
}

fn floats(stage: &openusd::usd::Stage, prim: &SdfPath, name: &str) -> Option<Vec<f64>> {
    Some(match read_attr(stage, prim, name)? {
        Value::Vec2f(v) => vec![v.x as f64, v.y as f64],
        Value::Vec2d(v) => vec![v.x, v.y],
        Value::Vec3f(v) => vec![v.x as f64, v.y as f64, v.z as f64],
        Value::Vec3d(v) => vec![v.x, v.y, v.z],
        Value::FloatVec(v) => v.iter().map(|&x| x as f64).collect(),
        Value::DoubleVec(v) => v,
        _ => return None,
    })
}

/// Speed limit, acceleration, PID and soft position limits under `ns`.
fn limits(stage: &openusd::usd::Stage, prim: &SdfPath, ns: &str, default_speed: f64) -> DeviceLimits {
    let float = |name: &str| read_float(stage, prim, &format!("gearbox:{ns}:{name}")).map(f64::from);
    let pid = floats(stage, prim, &format!("gearbox:{ns}:controlPID"))
        .filter(|v| v.len() == 3)
        .map_or([10.0, 0.0, 0.0], |v| [v[0], v[1], v[2]]);
    let position_limits = match (float("minPosition"), float("maxPosition")) {
        (Some(lo), Some(hi)) if lo < hi => Some([lo, hi]),
        _ => None,
    };
    DeviceLimits {
        max_velocity: float("maxVelocity").unwrap_or(default_speed).abs(),
        acceleration: float("acceleration").filter(|a| *a > 0.0),
        pid,
        position_limits,
    }
}

fn has_namespace(names: &[String], ns: &str) -> bool {
    let prefix = format!("gearbox:{ns}:");
    names.iter().any(|n| n.starts_with(&prefix))
}

/// Every device authored under `machine`, or the errors that stop one.
pub fn discover(
    stage: &openusd::usd::Stage,
    machine: &SdfPath,
    prims: &[SdfPath],
) -> (Vec<DeviceSpec>, Vec<String>) {
    let (mut devices, mut errors) = (Vec::new(), Vec::new());
    let root = format!("{}/", machine.as_str());
    for prim in prims.iter().filter(|p| p.as_str().starts_with(&root)) {
        let names = stage.prim_properties(prim).unwrap_or_default();
        if !names.iter().any(|n| n.starts_with("gearbox:")) {
            continue;
        }
        let name = read_token(stage, prim, "gearbox:device:name")
            .unwrap_or_else(|| prim.as_str().rsplit('/').next().unwrap_or_default().to_string());
        let float = |key: &str| read_float(stage, prim, key).map(f64::from);
        let is_joint = type_name(stage, prim).is_some_and(|t| t.starts_with("Physics") && t.ends_with("Joint"));
        let (motor, brake) = (has_namespace(&names, "motor"), has_namespace(&names, "brake"));
        let kind = if motor || brake {
            if !is_joint {
                errors.push(format!("{}: motors and brakes go on a physics joint", prim.as_str()));
                continue;
            }
            DeviceKind::Joint {
                motor: motor.then(|| {
                    (limits(stage, prim, "motor", 10.0), float("gearbox:motor:maxForce").unwrap_or(10.0).abs())
                }),
                brake: brake.then(|| float("gearbox:brake:damping").unwrap_or(0.0).max(0.0)),
            }
        } else if has_namespace(&names, "propeller") {
            let pair = |key: &str| {
                floats(stage, prim, key).filter(|v| v.len() == 2).map(|v| [v[0], v[1]])
            };
            let Some(thrust) = pair("gearbox:propeller:thrustConstants") else {
                errors.push(format!("{}: a propeller needs gearbox:propeller:thrustConstants", prim.as_str()));
                continue;
            };
            DeviceKind::Propeller {
                thrust,
                torque: pair("gearbox:propeller:torqueConstants").unwrap_or([0.0, 0.0]),
                limits: limits(stage, prim, "propeller", 100.0),
                max_torque: float("gearbox:propeller:maxTorque").map_or(f64::INFINITY, f64::abs),
            }
        } else if has_namespace(&names, "belt") {
            let direction = floats(stage, prim, "gearbox:belt:direction")
                .filter(|v| v.len() == 3)
                .map_or([1.0, 0.0, 0.0], |v| [v[0], v[1], v[2]]);
            DeviceKind::Belt { direction, limits: limits(stage, prim, "belt", 10.0) }
        } else if has_namespace(&names, "connector") {
            let flag = |key: &str, default: bool| read_bool(stage, prim, &format!("gearbox:connector:{key}")).unwrap_or(default);
            let value = |key: &str| float(&format!("gearbox:connector:{key}"));
            let strength = |key: &str| value(key).filter(|s| *s >= 0.0);
            let base = ConnectorDesc::default();
            let kind = match read_token(stage, prim, "gearbox:connector:type").as_deref() {
                None | Some("symmetric") => ConnectorKind::Symmetric,
                Some("active") => ConnectorKind::Active,
                Some("passive") => ConnectorKind::Passive,
                Some(other) => {
                    errors.push(format!("{}: connector type `{other}` is not symmetric, active or passive", prim.as_str()));
                    continue;
                }
            };
            DeviceKind::Connector {
                desc: ConnectorDesc {
                    model: read_token(stage, prim, "gearbox:connector:model").unwrap_or_default(),
                    kind,
                    auto_lock: flag("autoLock", base.auto_lock),
                    unilateral_lock: flag("unilateralLock", base.unilateral_lock),
                    unilateral_unlock: flag("unilateralUnlock", base.unilateral_unlock),
                    distance_tolerance: value("distanceTolerance").unwrap_or(base.distance_tolerance),
                    axis_tolerance: value("axisTolerance").unwrap_or(base.axis_tolerance),
                    rotation_tolerance: value("rotationTolerance").unwrap_or(base.rotation_tolerance),
                    rotations: value("numberOfRotations").map_or(base.rotations, |n| n.max(0.0) as u32),
                    snap: flag("snap", base.snap),
                    tensile_strength: strength("tensileStrength"),
                    shear_strength: strength("shearStrength"),
                    stiffness: value("stiffness").filter(|s| *s > 0.0).unwrap_or(base.stiffness),
                    ..base
                },
                locked: flag("isLocked", false),
            }
        } else {
            continue;
        };
        if devices.iter().any(|d: &DeviceSpec| d.name == name) {
            errors.push(format!("{}: device name `{name}` is taken", prim.as_str()));
            continue;
        }
        devices.push(DeviceSpec { name, prim: prim.as_str().to_string(), kind });
    }
    (devices, errors)
}

// ── Runtime ────────────────────────────────────────────────────────────────

/// A registered device and the backend object behind it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Live {
    Joint(JointId),
    Propeller(DeviceId),
    Belt(DeviceId),
    Connector(DeviceId),
}

struct Registered {
    spec: DeviceSpec,
    live: Live,
}

/// Devices registered per machine id.
#[derive(Resource, Default)]
pub struct MachineDevices {
    machines: HashMap<String, Vec<Registered>>,
    failed: HashMap<String, String>,
}

impl MachineDevices {
    /// `<machine>/<device>` of a connector's backend id.
    fn connector_name(&self, id: DeviceId) -> Option<String> {
        self.machines.iter().find_map(|(machine, devices)| {
            devices
                .iter()
                .find(|d| d.live == Live::Connector(id))
                .map(|d| format!("{machine}/{}", d.spec.name))
        })
    }
}

/// The nearest entity at or above `entity` that carries a physics body.
fn body_entity(entity: Entity, physics: &PhysicsWorld, parents: &Query<&ChildOf>) -> Option<Entity> {
    std::iter::once(entity)
        .chain(parents.iter_ancestors(entity))
        .find(|e| physics.entity_to_body.contains_key(e))
}

/// `child`'s pose in `parent`'s frame, in physics axes.
fn relative_pose(parent: &GlobalTransform, child: &GlobalTransform) -> Option<Pose> {
    let local = parent.affine().inverse() * child.affine();
    let (_, r, t) = local.to_scale_rotation_translation();
    (r.is_finite() && t.is_finite()).then(|| {
        Pose::new(
            DVec3::new(t.x as f64, t.y as f64, t.z as f64),
            glam::DQuat::from_xyzw(r.x as f64, r.y as f64, r.z as f64, r.w as f64).normalize(),
        )
    })
}

/// Registers one device with the backend; `Ok(None)` until its physics
/// objects exist.
fn register(
    spec: &DeviceSpec,
    scene_root: Entity,
    physics: &mut PhysicsWorld,
    prims: &Query<(Entity, &UsdPrimRef)>,
    parents: &Query<&ChildOf>,
    children: &Query<&Children>,
    transforms: &Query<&GlobalTransform>,
) -> Result<Option<Live>, String> {
    let Some(entity) = find_prim_entity(scene_root, &spec.prim, prims, parents) else {
        return Ok(None);
    };
    let placed = |physics: &PhysicsWorld| -> Option<(crate::physics::backend::BodyId, Pose)> {
        let body = body_entity(entity, physics, parents)?;
        let frame = relative_pose(transforms.get(body).ok()?, transforms.get(entity).ok()?)?;
        Some((*physics.entity_to_body.get(&body)?, frame))
    };
    match &spec.kind {
        DeviceKind::Joint { motor, brake } => {
            let Some(&joint) = physics.entity_to_joint.get(&entity) else {
                return Ok(None);
            };
            if let Some((limits, max_force)) = motor {
                physics.insert_motor(joint, *limits, *max_force)?;
            }
            if let Some(damping) = brake {
                physics.set_joint_brake(joint, *damping)?;
            }
            Ok(Some(Live::Joint(joint)))
        }
        DeviceKind::Propeller { thrust, torque, limits, max_torque } => {
            let Some((body, frame)) = placed(physics) else { return Ok(None) };
            let id = physics.insert_propeller(PropellerDesc {
                body,
                frame,
                thrust: *thrust,
                torque: *torque,
                limits: *limits,
                max_torque: *max_torque,
            })?;
            Ok(Some(Live::Propeller(id)))
        }
        DeviceKind::Belt { direction, limits } => {
            let colliders: Vec<(Entity, _)> = std::iter::once(entity)
                .chain(children.iter_descendants(entity))
                .filter_map(|e| physics.entity_to_collider.get(&e).map(|&c| (e, c)))
                .collect();
            let Some(&(first, _)) = colliders.first() else { return Ok(None) };
            let (Ok(prim_gt), Ok(collider_gt)) = (transforms.get(entity), transforms.get(first)) else {
                return Ok(None);
            };
            let in_prim = Vec3::new(direction[0] as f32, direction[1] as f32, direction[2] as f32);
            let d = collider_gt.rotation().inverse() * (prim_gt.rotation() * in_prim);
            let id = physics.insert_belt(BeltDesc {
                colliders: colliders.into_iter().map(|(_, c)| c).collect(),
                direction: DVec3::new(d.x as f64, d.y as f64, d.z as f64),
                limits: *limits,
            })?;
            Ok(Some(Live::Belt(id)))
        }
        DeviceKind::Connector { desc, locked } => {
            let Some((body, frame)) = placed(physics) else { return Ok(None) };
            let id = physics.insert_connector(ConnectorDesc { body: Some(body), frame, ..desc.clone() })?;
            if *locked {
                physics.lock_connector(id, true)?;
            }
            Ok(Some(Live::Connector(id)))
        }
    }
}

fn number(props: &Props, key: &str) -> Option<f64> {
    props.get(key).and_then(|v| v.trim().parse::<f64>().ok())
}

/// Applies one command to a registered device.
fn apply(physics: &mut dyn PhysicsBackend, device: &Registered, command: &ActuatorCommand) -> Result<(), String> {
    let props = command.props();
    let has = |key: &str| props.get(key).is_some();
    let flag = |key: &str| props.get(key).map(|v| matches!(v.trim(), "1" | "true" | "on" | "yes"));
    match device.live {
        Live::Joint(joint) => {
            if let Some(damping) = number(&props, "brake") {
                physics.set_joint_brake(joint, damping)?;
            }
            let settings = [
                number(&props, "speed").map(DeviceSetting::Speed),
                has("acceleration").then(|| {
                    DeviceSetting::Acceleration(number(&props, "acceleration").filter(|a| *a > 0.0))
                }),
                number(&props, "max_force").or(number(&props, "max_torque")).map(DeviceSetting::MaxForce),
                props.get("pid").and_then(|v| {
                    let gains: Vec<f64> = v.split(',').filter_map(|g| g.trim().parse().ok()).collect();
                    (gains.len() == 3).then(|| DeviceSetting::Pid([gains[0], gains[1], gains[2]]))
                }),
            ];
            for setting in settings.into_iter().flatten() {
                physics.configure_motor(joint, setting)?;
            }
            let command = number(&props, "position")
                .map(DeviceCommand::Position)
                .or(number(&props, "velocity").map(DeviceCommand::Velocity))
                .or(number(&props, "force").or(number(&props, "torque")).map(DeviceCommand::Force));
            if let Some(command) = command {
                physics.command_motor(joint, command)?;
            }
            Ok(())
        }
        Live::Propeller(id) => {
            let omega = number(&props, "velocity").or(number(&props, "speed")).ok_or("propellers take a velocity")?;
            physics.set_propeller_speed(id, omega)
        }
        Live::Belt(id) => {
            if let Some(speed) = number(&props, "speed") {
                physics.configure_belt(id, DeviceSetting::Speed(speed))?;
            }
            if has("acceleration") {
                physics.configure_belt(id, DeviceSetting::Acceleration(number(&props, "acceleration").filter(|a| *a > 0.0)))?;
            }
            let command = number(&props, "position")
                .map(DeviceCommand::Position)
                .or(number(&props, "velocity").map(DeviceCommand::Velocity));
            if let Some(command) = command {
                physics.command_belt(id, command)?;
            }
            Ok(())
        }
        Live::Connector(id) => {
            let lock = flag("lock").or(flag("unlock").map(|u| !u)).ok_or("connectors take lock or unlock")?;
            physics.lock_connector(id, lock)
        }
    }
}

/// Registers the devices of loaded machines, drops those of removed ones,
/// and applies this frame's actuator commands before the physics steps.
#[allow(clippy::too_many_arguments)]
fn drive_devices(
    mut registry: ResMut<MachineDevices>,
    inventory: Res<ControllerInventory>,
    keys: Res<MachineAgentKeys>,
    bus: Option<ResMut<GearboxBus>>,
    mut physics: ResMut<PhysicsWorld>,
    prims: Query<(Entity, &UsdPrimRef)>,
    parents: Query<&ChildOf>,
    children: Query<&Children>,
    transforms: Query<&GlobalTransform>,
) {
    let registry = &mut *registry;
    let live: Vec<&str> = inventory.machines.iter().map(|m| m.id.as_str()).collect();
    let gone: Vec<String> = registry.machines.keys().filter(|id| !live.contains(&id.as_str())).cloned().collect();
    for id in gone {
        for device in registry.machines.remove(&id).unwrap_or_default() {
            match device.live {
                Live::Joint(joint) => physics.remove_motor(joint),
                Live::Propeller(id) | Live::Belt(id) | Live::Connector(id) => physics.remove_device(id),
            }
        }
    }
    for machine in &inventory.machines {
        if machine.devices.is_empty() || registry.machines.contains_key(&machine.id) {
            continue;
        }
        let Some(scene_root) = machine.scene_root else { continue };
        let mut registered = Vec::new();
        let mut ready = true;
        for spec in &machine.devices {
            match register(spec, scene_root, &mut physics, &prims, &parents, &children, &transforms) {
                Ok(Some(live)) => registered.push(Registered { spec: spec.clone(), live }),
                Ok(None) => ready = false,
                Err(error) => {
                    if registry.failed.insert(format!("{}/{}", machine.id, spec.name), error.clone()).is_none() {
                        warn!("gearbox-devices: `{}` device `{}`: {error}", machine.id, spec.name);
                    }
                }
            }
        }
        if ready {
            info!("gearbox-devices: `{}` runs {} device(s)", machine.id, registered.len());
            registry.machines.insert(machine.id.clone(), registered);
        } else {
            for device in registered {
                match device.live {
                    Live::Joint(joint) => physics.remove_motor(joint),
                    Live::Propeller(id) | Live::Belt(id) | Live::Connector(id) => physics.remove_device(id),
                }
            }
        }
    }
    let Some(mut bus) = bus else { return };
    for machine in &inventory.machines {
        let Some(agent) = bus_key(&keys, machine).and_then(|key| bus.machines.get_mut(&key)) else {
            continue;
        };
        let commands = std::mem::take(&mut agent.actuations);
        let devices = registry.machines.get(&machine.id).map_or(&[][..], Vec::as_slice);
        for command in commands {
            let name = command.device();
            let Some(device) = devices.iter().find(|d| d.spec.name == name) else {
                warn!("gearbox-devices: `{}` has no device `{name}`", machine.id);
                continue;
            };
            if let Err(error) = apply(&mut **physics, device, &command) {
                warn!("gearbox-devices: `{}` device `{name}`: {error}", machine.id);
            }
        }
    }
}

/// The bus key of a loaded machine's agent.
fn bus_key(keys: &MachineAgentKeys, machine: &crate::controller::MachineInstanceSpec) -> Option<String> {
    let scene_root = machine.scene_root?;
    keys.0
        .iter()
        .find(|(_, k)| k.scene_root == scene_root && k.machine_id == machine.id)
        .map(|(key, _)| key.clone())
}

/// Publishes every registered device's reading after the physics steps.
fn publish_devices(
    registry: Res<MachineDevices>,
    inventory: Res<ControllerInventory>,
    keys: Res<MachineAgentKeys>,
    bus: Option<ResMut<GearboxBus>>,
    physics: Res<PhysicsWorld>,
    time: Res<Time>,
) {
    let Some(mut bus) = bus else { return };
    let stamp_ms = (time.elapsed_secs_f64() * 1000.0) as u32;
    for machine in &inventory.machines {
        let Some(devices) = registry.machines.get(&machine.id) else { continue };
        let Some(agent) = bus_key(&keys, machine).and_then(|key| bus.machines.get_mut(&key)) else {
            continue;
        };
        for device in devices {
            let Some((measurement_kind, values, peer)) = reading(&**physics, device, &registry) else {
                continue;
            };
            let mut pairs = vec![("name", device.spec.name.as_str())];
            if let Some(peer) = &peer {
                pairs.push(("peer", peer.as_str()));
            }
            let measurement = Measurement {
                sim_time_s: physics.simulated_seconds,
                kind: measurement_kind,
                stamp_ms,
                count: 1,
                values,
                props: Props::from_pairs(&pairs).into_bytes(),
                ..Default::default()
            };
            agent.publish_actuator_measurement(&device.spec.name, &measurement);
        }
    }
}

/// A device's measurement kind, record and peer name.
fn reading(physics: &dyn PhysicsBackend, device: &Registered, registry: &MachineDevices) -> Option<(u32, Vec<f64>, Option<String>)> {
    Some(match device.live {
        Live::Joint(joint) => {
            let m = physics.motor_output(joint)?;
            let (mode, target) = match m.command {
                DeviceCommand::Position(p) => (0.0, p),
                DeviceCommand::Velocity(v) => (1.0, v),
                DeviceCommand::Force(f) => (2.0, f),
            };
            let values = vec![m.position, m.velocity, m.commanded_velocity, m.force, m.brake_damping, m.brake_force, mode, target];
            (kind::MOTOR, values, None)
        }
        Live::Propeller(id) => {
            let p = physics.propeller_output(id)?;
            (kind::PROPELLER, vec![p.omega, p.thrust, p.torque, p.advance], None)
        }
        Live::Belt(id) => {
            let b = physics.belt_output(id)?;
            (kind::BELT, vec![b.position, b.velocity], None)
        }
        Live::Connector(id) => {
            let c = physics.connector_output(id)?;
            let peer = c.linked.or(c.presence).and_then(|p| registry.connector_name(p));
            let bit = |b: bool| f64::from(u8::from(b));
            let values = vec![bit(c.presence.is_some()), bit(c.locked), bit(c.linked.is_some()), c.tensile, c.shear];
            (kind::CONNECTOR, values, peer)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::backend::{BodyDesc, ColliderDesc, JointDesc, JointKind, Shape};
    use crate::physics::molla::MollaBackend;

    const MACHINE: &str = r#"#usda 1.0
(
    defaultPrim = "robot"
    upAxis = "Z"
)

def Xform "robot" (prepend apiSchemas = ["GearboxMachineAPI"])
{
    token gearbox:machine:kind = "test"
    rel gearbox:machine:body = </robot/chassis>
    def Xform "chassis" (prepend apiSchemas = ["PhysicsRigidBodyAPI"])
    {
        def Xform "fan"
        {
            double2 gearbox:propeller:thrustConstants = (0.2, 0.01)
            double2 gearbox:propeller:torqueConstants = (0.02, 0)
            float gearbox:propeller:maxVelocity = 150
        }
        def Cube "roof" (prepend apiSchemas = ["PhysicsCollisionAPI"])
        {
            float3 gearbox:belt:direction = (0, 1, 0)
            float gearbox:belt:acceleration = 2
        }
        def Xform "nose"
        {
            token gearbox:connector:model = "hitch"
            token gearbox:connector:type = "active"
            bool gearbox:connector:autoLock = true
            bool gearbox:connector:isLocked = true
            float gearbox:connector:tensileStrength = 5000
            float gearbox:connector:numberOfRotations = 1
        }
    }
    def Xform "boom" (prepend apiSchemas = ["PhysicsRigidBodyAPI"]) {}
    def Xform "stick" (prepend apiSchemas = ["PhysicsRigidBodyAPI"]) {}
    def Scope "Joints"
    {
        def PhysicsRevoluteJoint "boom_lift"
        {
            rel physics:body0 = </robot/chassis>
            rel physics:body1 = </robot/boom>
            float gearbox:motor:maxForce = 5000
            float gearbox:motor:maxVelocity = 0.5
            float3 gearbox:motor:controlPID = (4, 0.25, 0)
            float gearbox:motor:minPosition = -0.25
            float gearbox:motor:maxPosition = 1.25
            float gearbox:brake:damping = 300
        }
        def PhysicsRevoluteJoint "hinge"
        {
            rel physics:body0 = </robot/boom>
            rel physics:body1 = </robot/stick>
            float gearbox:brake:damping = 0
        }
    }
}
"#;

    #[test]
    fn devices_are_discovered_from_their_attributes() {
        let path = std::env::temp_dir().join(format!("gearbox-devices-{}.usda", std::process::id()));
        std::fs::write(&path, MACHINE).unwrap();
        let machines = crate::controller::discover_machines_from_usd(&path).expect("scan");
        let _ = std::fs::remove_file(&path);
        let machine = machines.into_iter().next().expect("one machine");
        assert!(machine.links.errors.is_empty(), "{:?}", machine.links.errors);
        let by = |name: &str| machine.devices.iter().find(|d| d.name == name).unwrap_or_else(|| panic!("{name}")).kind.clone();
        let DeviceKind::Joint { motor: Some((limits, max_force)), brake: Some(brake) } = by("boom_lift") else {
            panic!("{:?}", by("boom_lift"));
        };
        assert_eq!((max_force, brake, limits.max_velocity), (5000.0, 300.0, 0.5));
        assert_eq!((limits.pid, limits.position_limits), ([4.0, 0.25, 0.0], Some([-0.25, 1.25])));
        assert!(matches!(by("hinge"), DeviceKind::Joint { motor: None, brake: Some(0.0) }));
        let DeviceKind::Propeller { thrust, torque, limits, .. } = by("fan") else { panic!() };
        assert_eq!((thrust, torque, limits.max_velocity), ([0.2, 0.01], [0.02, 0.0], 150.0));
        let DeviceKind::Belt { direction, limits } = by("roof") else { panic!() };
        assert_eq!((direction, limits.acceleration), ([0.0, 1.0, 0.0], Some(2.0)));
        let DeviceKind::Connector { desc, locked } = by("nose") else { panic!() };
        assert!(locked && desc.auto_lock && desc.kind == ConnectorKind::Active);
        assert_eq!((desc.model.as_str(), desc.tensile_strength, desc.rotations), ("hitch", Some(5000.0), 1));
        assert_eq!(machine.devices.len(), 5);
    }

    fn body(world: &mut PhysicsWorld, desc: BodyDesc, at: DVec3) -> crate::physics::backend::BodyId {
        let body = world.insert_body(desc.pose(Pose::from_translation(at)));
        world
            .insert_collider(ColliderDesc::new(Shape::Ball { radius: 0.1 }).density(1000.0).parent(body))
            .unwrap();
        body
    }

    #[test]
    fn the_molla_backend_steps_motors_propellers_and_connectors() {
        let mut world = PhysicsWorld::with_backend(Box::new(MollaBackend::default()));
        world.set_gravity(DVec3::ZERO);
        let post = body(&mut world, BodyDesc::fixed(), DVec3::new(0.0, 5.0, 0.0));
        let arm = body(&mut world, BodyDesc::dynamic(), DVec3::new(1.0, 5.0, 0.0));
        let joint = world.backend.insert_joint(
            post,
            arm,
            JointDesc::new(JointKind::Revolute { axis: DVec3::Y }, Pose::IDENTITY, Pose::from_translation(DVec3::NEG_X)),
        );
        let limits = DeviceLimits { max_velocity: 1.0, ..Default::default() };
        world.insert_motor(joint, limits, 500.0).unwrap();
        world.command_motor(joint, DeviceCommand::Position(0.4)).unwrap();

        let drone = body(&mut world, BodyDesc::dynamic(), DVec3::new(0.0, 5.0, 10.0));
        let fan = world
            .insert_propeller(PropellerDesc {
                body: drone,
                frame: Pose::IDENTITY,
                thrust: [0.01, 0.0],
                torque: [0.0, 0.0],
                limits: DeviceLimits::default(),
                max_torque: 10.0,
            })
            .unwrap();
        world.set_propeller_speed(fan, 10.0).unwrap();

        // The dock faces +Z from the post; the plug on the pod faces back.
        let turn = |angle: f64| glam::DQuat::from_rotation_y(angle);
        let dock = world
            .insert_connector(ConnectorDesc {
                body: Some(post),
                frame: Pose::new(DVec3::Z * 0.5, turn(-std::f64::consts::FRAC_PI_2)),
                ..Default::default()
            })
            .unwrap();
        let pod = body(&mut world, BodyDesc::dynamic(), DVec3::new(0.0, 5.0, 0.505));
        let plug = world
            .insert_connector(ConnectorDesc {
                body: Some(pod),
                frame: Pose::new(DVec3::ZERO, turn(std::f64::consts::FRAC_PI_2)),
                ..Default::default()
            })
            .unwrap();
        world.lock_connector(dock, true).unwrap();
        for _ in 0..240 {
            world.step();
        }
        let motor = world.motor_output(joint).unwrap();
        assert!((motor.position - 0.4).abs() < 1e-3, "{motor:?}");
        let prop = world.propeller_output(fan).unwrap();
        assert!((prop.thrust - 1.0).abs() < 1e-9, "{prop:?}");
        assert!(world.body(drone).unwrap().linvel().x > 0.0);
        let docked = world.connector_output(dock).unwrap();
        assert_eq!((docked.linked, world.connector_output(plug).unwrap().linked), (Some(plug), Some(dock)));
    }
}
