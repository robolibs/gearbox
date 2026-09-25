//! Actuator devices on the Molla rigid world's device layer.

use molla_solvers::devices as md;
use molla_solvers::rigid_world::RigidWorld;

use super::convert;
use super::MollaBackend;
use crate::physics::backend::*;

/// The Molla device behind a gearbox `DeviceId`.
#[derive(Clone, Copy, Debug)]
pub(super) enum MollaDevice {
    Propeller(md::PropellerId),
    Belt(md::BeltId),
    Connector(md::ConnectorId),
    Copter(md::CopterId),
    Drag(md::DragId),
}

fn copter_command(c: CopterCommand) -> md::CopterCommand {
    match c {
        CopterCommand::Off => md::CopterCommand::Off,
        CopterCommand::Velocity { forward, left, up, yaw_rate } => {
            md::CopterCommand::Velocity { forward, left, up, yaw_rate }
        }
        CopterCommand::Attitude { roll, pitch, yaw_rate, climb } => {
            md::CopterCommand::Attitude { roll, pitch, yaw_rate, climb }
        }
    }
}

fn copter_command_back(c: md::CopterCommand) -> CopterCommand {
    match c {
        md::CopterCommand::Off => CopterCommand::Off,
        md::CopterCommand::Velocity { forward, left, up, yaw_rate } => {
            CopterCommand::Velocity { forward, left, up, yaw_rate }
        }
        md::CopterCommand::Attitude { roll, pitch, yaw_rate, climb } => {
            CopterCommand::Attitude { roll, pitch, yaw_rate, climb }
        }
    }
}

fn vec(v: DVec3) -> molla_math::Vec3 {
    molla_math::Vec3::new(v.x, v.y, v.z)
}

fn dvec(v: molla_math::Vec3) -> DVec3 {
    DVec3::new(v.x, v.y, v.z)
}

fn limits(l: DeviceLimits) -> md::MotorLimits {
    md::MotorLimits {
        max_velocity: l.max_velocity,
        acceleration: l.acceleration,
        pid: l.pid,
        position_limits: l.position_limits,
    }
}

fn command(c: DeviceCommand) -> md::MotorCommand {
    match c {
        DeviceCommand::Position(v) => md::MotorCommand::Position(v),
        DeviceCommand::Velocity(v) => md::MotorCommand::Velocity(v),
        DeviceCommand::Force(v) => md::MotorCommand::Force(v),
    }
}

fn error(e: molla_core::Error) -> String {
    e.to_string()
}

impl MollaBackend {
    fn joint_handle(&self, joint: JointId) -> Result<molla_sim::runtime::JointHandle, String> {
        Ok(self.joints.get(&joint).ok_or("unknown motor joint")?.handle)
    }

    fn device(&self, id: DeviceId) -> Result<MollaDevice, String> {
        self.devices.get(&id).copied().ok_or_else(|| "unknown device".into())
    }

    fn add_device(&mut self, device: MollaDevice) -> DeviceId {
        self.next_device += 1;
        let id = DeviceId(self.next_device);
        self.devices.insert(id, device);
        id
    }

    fn connector_device(&self, id: md::ConnectorId) -> Option<DeviceId> {
        self.devices
            .iter()
            .find(|(_, d)| matches!(d, MollaDevice::Connector(c) if *c == id))
            .map(|(&device, _)| device)
    }

    pub(super) fn device_insert_motor(&mut self, joint: JointId, l: DeviceLimits, max_force: f64) -> Result<(), String> {
        let handle = self.joint_handle(joint)?;
        let mut world = self.shared.world();
        let RigidWorld { scene, devices, .. } = &mut *world;
        devices
            .motors
            .insert(scene, handle, md::MotorSpec { limits: limits(l), max_force })
            .map_err(error)
    }

    pub(super) fn device_remove_motor(&mut self, joint: JointId) {
        let Ok(handle) = self.joint_handle(joint) else { return };
        let mut world = self.shared.world();
        let RigidWorld { scene, devices, .. } = &mut *world;
        super::apply(devices.motors.remove(scene, handle));
    }

    pub(super) fn device_has_motor(&self, joint: JointId) -> bool {
        self.joint_handle(joint).is_ok_and(|handle| self.shared.world().devices.motors.contains(handle))
    }

    pub(super) fn device_command_motor(&mut self, joint: JointId, c: DeviceCommand) -> Result<(), String> {
        let handle = self.joint_handle(joint)?;
        self.shared.world().devices.motors.command(handle, command(c)).map_err(error)
    }

    pub(super) fn device_configure_motor(&mut self, joint: JointId, setting: DeviceSetting) -> Result<(), String> {
        let handle = self.joint_handle(joint)?;
        let mut world = self.shared.world();
        let motors = &mut world.devices.motors;
        match setting {
            DeviceSetting::Speed(v) => motors.set_speed(handle, v),
            DeviceSetting::Acceleration(a) => motors.set_acceleration(handle, a),
            DeviceSetting::Pid(pid) => motors.set_pid(handle, pid),
            DeviceSetting::MaxForce(f) => motors.set_max_force(handle, f),
        }
        .map_err(error)
    }

    pub(super) fn device_set_brake(&mut self, joint: JointId, damping: f64) -> Result<(), String> {
        let handle = self.joint_handle(joint)?;
        let mut world = self.shared.world();
        let RigidWorld { scene, devices, .. } = &mut *world;
        devices.motors.set_brake(scene, handle, damping).map_err(error)
    }

    pub(super) fn device_motor_output(&self, joint: JointId) -> Option<MotorOutput> {
        let handle = self.joint_handle(joint).ok()?;
        let r = self.shared.world().devices.motors.reading(handle)?;
        Some(MotorOutput {
            position: r.position,
            velocity: r.velocity,
            command: match r.command {
                md::MotorCommand::Position(v) => DeviceCommand::Position(v),
                md::MotorCommand::Velocity(v) => DeviceCommand::Velocity(v),
                md::MotorCommand::Force(v) => DeviceCommand::Force(v),
            },
            commanded_velocity: r.commanded_velocity,
            force: r.force,
            brake_damping: r.brake_damping,
            brake_force: r.brake_force,
        })
    }

    pub(super) fn device_insert_propeller(&mut self, desc: PropellerDesc) -> Result<DeviceId, String> {
        let body = self.bodies.get(&desc.body).ok_or("unknown propeller body")?.handle;
        let id = {
            let mut world = self.shared.world();
            let RigidWorld { scene, devices, .. } = &mut *world;
            devices
                .propellers
                .insert(scene, md::PropellerSpec {
                    body,
                    frame: convert::transform(desc.frame),
                    thrust: desc.thrust,
                    torque: desc.torque,
                    limits: limits(desc.limits),
                    max_torque: desc.max_torque,
                })
                .map_err(error)?
        };
        Ok(self.add_device(MollaDevice::Propeller(id)))
    }

    pub(super) fn device_set_propeller_speed(&mut self, id: DeviceId, omega: f64) -> Result<(), String> {
        let MollaDevice::Propeller(p) = self.device(id)? else {
            return Err("device is not a propeller".into());
        };
        self.shared.world().devices.propellers.set_speed(p, omega).map_err(error)
    }

    pub(super) fn device_propeller_output(&self, id: DeviceId) -> Option<PropellerOutput> {
        let MollaDevice::Propeller(p) = self.device(id).ok()? else { return None };
        let r = self.shared.world().devices.propellers.reading(p)?;
        Some(PropellerOutput { omega: r.omega, thrust: r.thrust, torque: r.torque, advance: r.advance })
    }

    pub(super) fn device_insert_belt(&mut self, desc: BeltDesc) -> Result<DeviceId, String> {
        let colliders = desc
            .colliders
            .iter()
            .map(|c| self.colliders.get(c).map(|a| a.handle).ok_or("unknown belt collider"))
            .collect::<Result<Vec<_>, _>>()?;
        let spec = md::BeltSpec {
            colliders,
            direction: vec(desc.direction),
            limits: limits(desc.limits),
        };
        let id = {
            let mut world = self.shared.world();
            let RigidWorld { scene, devices, .. } = &mut *world;
            devices.belts.insert(scene, spec).map_err(error)?
        };
        Ok(self.add_device(MollaDevice::Belt(id)))
    }

    pub(super) fn device_command_belt(&mut self, id: DeviceId, c: DeviceCommand) -> Result<(), String> {
        let MollaDevice::Belt(b) = self.device(id)? else {
            return Err("device is not a belt".into());
        };
        self.shared.world().devices.belts.command(b, command(c)).map_err(error)
    }

    pub(super) fn device_configure_belt(&mut self, id: DeviceId, setting: DeviceSetting) -> Result<(), String> {
        let MollaDevice::Belt(b) = self.device(id)? else {
            return Err("device is not a belt".into());
        };
        let mut world = self.shared.world();
        let belts = &mut world.devices.belts;
        match setting {
            DeviceSetting::Speed(v) => belts.set_speed(b, v),
            DeviceSetting::Acceleration(a) => belts.set_acceleration(b, a),
            DeviceSetting::Pid(_) | DeviceSetting::MaxForce(_) => {
                return Err("belts take a speed and an acceleration".into());
            }
        }
        .map_err(error)
    }

    pub(super) fn device_belt_output(&self, id: DeviceId) -> Option<BeltOutput> {
        let MollaDevice::Belt(b) = self.device(id).ok()? else { return None };
        let r = self.shared.world().devices.belts.reading(b)?;
        Some(BeltOutput { position: r.position, velocity: r.velocity })
    }

    pub(super) fn device_insert_connector(&mut self, desc: ConnectorDesc) -> Result<DeviceId, String> {
        let body = match desc.body {
            Some(body) => Some(self.bodies.get(&body).ok_or("unknown connector body")?.handle),
            None => None,
        };
        let spec = md::ConnectorSpec {
            body,
            frame: convert::transform(desc.frame),
            model: desc.model,
            kind: match desc.kind {
                ConnectorKind::Symmetric => md::ConnectorKind::Symmetric,
                ConnectorKind::Active => md::ConnectorKind::Active,
                ConnectorKind::Passive => md::ConnectorKind::Passive,
            },
            auto_lock: desc.auto_lock,
            unilateral_lock: desc.unilateral_lock,
            unilateral_unlock: desc.unilateral_unlock,
            distance_tolerance: desc.distance_tolerance,
            axis_tolerance: desc.axis_tolerance,
            rotation_tolerance: desc.rotation_tolerance,
            rotations: desc.rotations,
            snap: desc.snap,
            tensile_strength: desc.tensile_strength,
            shear_strength: desc.shear_strength,
            stiffness: desc.stiffness,
        };
        let id = {
            let mut world = self.shared.world();
            let RigidWorld { scene, devices, .. } = &mut *world;
            devices.connectors.insert(scene, spec).map_err(error)?
        };
        Ok(self.add_device(MollaDevice::Connector(id)))
    }

    pub(super) fn device_lock_connector(&mut self, id: DeviceId, lock: bool) -> Result<(), String> {
        let MollaDevice::Connector(c) = self.device(id)? else {
            return Err("device is not a connector".into());
        };
        let mut world = self.shared.world();
        let connectors = &mut world.devices.connectors;
        if lock { connectors.lock(c) } else { connectors.unlock(c) }.map_err(error)
    }

    pub(super) fn device_connector_output(&self, id: DeviceId) -> Option<ConnectorOutput> {
        let MollaDevice::Connector(c) = self.device(id).ok()? else { return None };
        let r = self.shared.world().devices.connectors.reading(c)?;
        Some(ConnectorOutput {
            presence: r.presence.and_then(|p| self.connector_device(p)),
            locked: r.locked,
            linked: r.linked.and_then(|p| self.connector_device(p)),
            tensile: r.tensile,
            shear: r.shear,
        })
    }

    pub(super) fn device_insert_copter(&mut self, desc: CopterDesc) -> Result<DeviceId, String> {
        let body = self.bodies.get(&desc.body).ok_or("unknown copter body")?.handle;
        let propellers = desc
            .propellers
            .iter()
            .map(|&id| match self.device(id)? {
                MollaDevice::Propeller(p) => Ok(p),
                _ => Err("copters fly propellers".to_string()),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let l = desc.limits;
        let spec = md::CopterSpec {
            body,
            frame: convert::transform(desc.frame),
            propellers,
            limits: md::CopterLimits {
                max_tilt: l.max_tilt,
                max_speed: l.max_speed,
                max_climb: l.max_climb,
                max_yaw_rate: l.max_yaw_rate,
                velocity_gain: l.velocity_gain,
                disturbance_time: l.disturbance_time,
                attitude_frequency: l.attitude_frequency,
                yaw_gain: l.yaw_gain,
            },
        };
        let id = {
            let mut world = self.shared.world();
            let RigidWorld { scene, devices, .. } = &mut *world;
            devices.copters.insert(scene, &devices.propellers, spec).map_err(error)?
        };
        Ok(self.add_device(MollaDevice::Copter(id)))
    }

    pub(super) fn device_command_copter(&mut self, id: DeviceId, c: CopterCommand) -> Result<(), String> {
        let MollaDevice::Copter(f) = self.device(id)? else {
            return Err("device is not a copter".into());
        };
        self.shared.world().devices.copters.command(f, copter_command(c)).map_err(error)
    }

    pub(super) fn device_insert_drag(&mut self, desc: DragDesc) -> Result<DeviceId, String> {
        let body = self.bodies.get(&desc.body).ok_or("unknown drag body")?.handle;
        let spec = md::DragSpec {
            body,
            frame: convert::transform(desc.frame),
            area: vec(desc.area),
            density: desc.density,
        };
        let id = {
            let mut world = self.shared.world();
            let RigidWorld { scene, devices, .. } = &mut *world;
            devices.aero.insert(scene, spec).map_err(error)?
        };
        Ok(self.add_device(MollaDevice::Drag(id)))
    }

    pub(super) fn device_drag_output(&self, id: DeviceId) -> Option<DragOutput> {
        let MollaDevice::Drag(d) = self.device(id).ok()? else { return None };
        let r = self.shared.world().devices.aero.reading(d)?;
        Some(DragOutput { airspeed: dvec(r.airspeed), force: dvec(r.force), wind: dvec(r.wind) })
    }

    pub(super) fn device_set_wind(&mut self, wind: DVec3) {
        super::apply(self.shared.world().devices.aero.set_wind(vec(wind)));
    }

    pub(super) fn device_set_body_wind(&mut self, body: BodyId, wind: Option<DVec3>) {
        let Some(handle) = self.bodies.get(&body).map(|b| b.handle) else { return };
        super::apply(self.shared.world().devices.aero.set_body_wind(handle, wind.map(vec)));
    }

    pub(super) fn device_copter_output(&self, id: DeviceId) -> Option<CopterOutput> {
        let MollaDevice::Copter(f) = self.device(id).ok()? else { return None };
        let r = self.shared.world().devices.copters.reading(f)?;
        Some(CopterOutput {
            command: copter_command_back(r.command),
            roll: r.roll,
            pitch: r.pitch,
            velocity: r.velocity,
            yaw_rate: r.yaw_rate,
            thrust: r.thrust,
            saturated: r.saturated,
        })
    }

    pub(super) fn device_remove(&mut self, id: DeviceId) {
        let Some(device) = self.devices.remove(&id) else { return };
        let mut world = self.shared.world();
        let RigidWorld { scene, devices, .. } = &mut *world;
        match device {
            MollaDevice::Propeller(p) => {
                devices.propellers.remove(p);
            }
            MollaDevice::Belt(b) => super::apply(devices.belts.remove(scene, b).map(|_| ())),
            MollaDevice::Connector(c) => super::apply(devices.connectors.remove(scene, c).map(|_| ())),
            MollaDevice::Copter(f) => {
                devices.copters.remove(f);
            }
            MollaDevice::Drag(d) => {
                devices.aero.remove(d);
            }
        }
    }
}
