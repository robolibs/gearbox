//! The commit boundary of the joint command contract. Producers submit
//! setpoints for exact DOFs; once per physics tick the arbiter resolves one
//! actuator state per DOF and this writes it once: to the backend joint, to
//! the motor device that realizes it, or to the drive motor that shapes it.

use std::collections::HashMap;
use std::time::Instant;

use super::backend::{DeviceCommand, DeviceSetting, JointAxis, JointId, JointMut, MotorModel};
use super::world::PhysicsWorld;
use crate::joint_control::{
    ActuatorState, Arbiter, Batch, DofKey, DofSpec, DriveSpec, Effective, OwnerId, Rejection, Setpoint, Validity,
};
use crate::machine_model::{DofToken, DriveType, Interface, MachineDescription};

#[derive(Debug, Default)]
pub struct Control {
    pub arbiter: Arbiter,
    bindings: HashMap<DofKey, (JointId, JointAxis)>,
    by_joint: HashMap<(JointId, JointAxis), DofKey>,
    sequences: HashMap<OwnerId, u64>,
    /// Transactions each DOF took in the last committed tick.
    pub writes: HashMap<(JointId, JointAxis), u32>,
    /// The state each DOF was last committed to.
    pub committed: HashMap<(JointId, JointAxis), ActuatorState>,
    /// Machine id → its bound generation.
    pub machines: HashMap<String, u64>,
}

/// The backend axis a DOF token drives: scalar joints move about their
/// joint X, D6 axes about their own.
pub fn dof_axis(token: DofToken) -> Option<JointAxis> {
    Some(match token {
        DofToken::Angular | DofToken::RotX => JointAxis::AngX,
        DofToken::Linear | DofToken::TransX => JointAxis::LinX,
        DofToken::RotY => JointAxis::AngY,
        DofToken::RotZ => JointAxis::AngZ,
        DofToken::TransY => JointAxis::LinY,
        DofToken::TransZ => JointAxis::LinZ,
        DofToken::Distance => return None,
    })
}

fn token_of(axis: JointAxis) -> DofToken {
    match axis {
        JointAxis::AngX => DofToken::Angular,
        JointAxis::LinX => DofToken::Linear,
        JointAxis::AngY => DofToken::RotY,
        JointAxis::AngZ => DofToken::RotZ,
        JointAxis::LinY => DofToken::TransY,
        JointAxis::LinZ => DofToken::TransZ,
    }
}

fn model_of(drive: DriveType) -> MotorModel {
    match drive {
        DriveType::Force => MotorModel::Force,
        DriveType::Acceleration => MotorModel::Acceleration,
    }
}

/// The joint's drive as it was built: the authored drive with its targets,
/// idle without a target, or no motor without a drive.
fn restore_authored(joint: &mut dyn JointMut, axis: JointAxis, drive: Option<super::world::AuthoredDrive>) {
    let Some(d) = drive else {
        joint.clear_motor(axis);
        return;
    };
    joint.clear_motor(axis);
    joint.set_motor_model(axis, if d.force { MotorModel::Force } else { MotorModel::Acceleration });
    match (d.target_position, d.target_velocity) {
        (Some(q), qdot) => joint.set_motor(axis, q, qdot.unwrap_or(0.0), d.stiffness, d.damping),
        (None, Some(qdot)) => joint.set_motor_velocity(axis, qdot, d.damping),
        (None, None) => {}
    }
    joint.set_motor_max_force(axis, d.max_force.unwrap_or(f64::INFINITY));
}

/// One DOF's state onto its joint, every motor field set so nothing stale survives.
pub fn write_state(joint: &mut dyn JointMut, axis: JointAxis, state: &ActuatorState) {
    let cap = state.cap.unwrap_or(f64::INFINITY);
    match state.effective {
        Effective::Free => joint.clear_motor(axis),
        Effective::Effort { force } => joint.set_motor_effort(axis, force, cap),
        Effective::Servo { q: Some(q), qdot, stiffness, damping, drive } => {
            joint.set_motor_model(axis, model_of(drive));
            joint.set_motor(axis, q, qdot, stiffness, damping);
            joint.set_motor_max_force(axis, cap);
        }
        Effective::Servo { q: None, qdot, damping, drive, .. } => {
            joint.set_motor_model(axis, model_of(drive));
            joint.set_motor_velocity(axis, qdot, damping);
            joint.set_motor_max_force(axis, cap);
        }
    }
}

/// What a motor device is told to realize a state.
fn device_command(state: &ActuatorState) -> DeviceCommand {
    match state.effective {
        Effective::Free => DeviceCommand::Force(0.0),
        Effective::Effort { force } => DeviceCommand::Force(force),
        Effective::Servo { q: Some(q), .. } => DeviceCommand::Position(q),
        Effective::Servo { q: None, qdot, .. } => DeviceCommand::Velocity(qdot),
    }
}

impl PhysicsWorld {
    /// Bind an accepted machine generation's DOFs to their physics joints and
    /// register their owners, all or none. `joint_of` maps a joint prim path
    /// to its built joint.
    pub fn bind_machine(
        &mut self,
        machine_id: &str,
        model: &MachineDescription,
        joint_of: impl Fn(&str) -> Option<JointId>,
    ) -> Result<usize, Vec<String>> {
        let mut specs = crate::joint_control::dof_specs(machine_id, model);
        specs.retain(|s| dof_axis(s.key.dof).is_some());
        let mut bound = Vec::new();
        let mut missing = Vec::new();
        for spec in &specs {
            let (Some(joint), Some(axis)) = (joint_of(&spec.key.joint), dof_axis(spec.key.dof)) else {
                missing.push(spec.key.to_string());
                continue;
            };
            bound.push((spec.key.clone(), joint, axis));
        }
        if !missing.is_empty() {
            return Err(missing);
        }
        self.unbind_machine(machine_id);
        for spec in &mut specs {
            let joint = bound.iter().find(|(k, ..)| *k == spec.key).map(|(_, j, _)| *j).expect("bound");
            if spec.drive.is_none()
                && let Some(drive) = self.authored_drives.get(&joint)
            {
                spec.drive = Some(DriveSpec {
                    stiffness: drive.stiffness,
                    damping: drive.damping,
                    drive: if drive.force { DriveType::Force } else { DriveType::Acceleration },
                    max_force: drive.max_force,
                });
            }
        }
        let count = specs.len();
        self.control.arbiter.register(specs).map_err(|e| e.iter().map(ToString::to_string).collect::<Vec<_>>())?;
        for (key, joint, axis) in bound {
            if let Some(previous) = self.control.by_joint.insert((joint, axis), key.clone()).filter(|p| *p != key) {
                self.control.arbiter.remove(&previous);
                self.control.bindings.remove(&previous);
            }
            self.control.bindings.insert(key, (joint, axis));
        }
        self.control.machines.insert(machine_id.to_string(), model.generation);
        Ok(count)
    }

    /// Bind one DOF directly: fixtures and the conformance harness.
    pub fn bind_dof(&mut self, spec: DofSpec, joint: JointId, axis: JointAxis) {
        let key = spec.key.clone();
        self.control.arbiter.insert(spec);
        if let Some(previous) = self.control.by_joint.insert((joint, axis), key.clone()).filter(|p| *p != key) {
            self.control.arbiter.remove(&previous);
            self.control.bindings.remove(&previous);
        }
        self.control.bindings.insert(key, (joint, axis));
    }

    /// Forget a machine's DOFs, owners and commands.
    pub fn unbind_machine(&mut self, machine_id: &str) {
        self.control.arbiter.unregister(machine_id);
        let control = &mut self.control;
        control.bindings.retain(|k, _| k.machine != machine_id);
        control.by_joint.retain(|_, k| k.machine != machine_id);
        control.machines.remove(machine_id);
        control.sequences.retain(|o, _| o.machine != machine_id);
    }

    pub fn dof_binding(&self, key: &DofKey) -> Option<(JointId, JointAxis)> {
        self.control.bindings.get(key).copied()
    }

    pub fn dof_key(&self, joint: JointId, axis: JointAxis) -> Option<&DofKey> {
        self.control.by_joint.get(&(joint, axis))
    }

    /// The joints between two bodies whose `axis` DOF `owner` holds. When no
    /// machine bound any of them, all of them: the unbound legacy path.
    pub fn owned_joints_between(&self, owner: &OwnerId, a: super::backend::BodyId, b: super::backend::BodyId, axis: JointAxis) -> Vec<JointId> {
        let between = self.backend.joints_between(a, b);
        let bound: Vec<JointId> = between.iter().copied().filter(|j| self.control.by_joint.contains_key(&(*j, axis))).collect();
        if bound.is_empty() {
            return between;
        }
        bound
            .into_iter()
            .filter(|j| self.control.by_joint.get(&(*j, axis)).and_then(|k| self.control.arbiter.owner(k)) == Some(owner))
            .collect()
    }

    /// The owner of `joint`'s `axis` DOF, when one is bound.
    pub fn joint_owner(&self, joint: JointId, axis: JointAxis) -> Option<&OwnerId> {
        self.control.by_joint.get(&(joint, axis)).and_then(|k| self.control.arbiter.owner(k))
    }

    /// The axis a scalar joint's motor drives.
    pub fn motor_axis(&self, joint: JointId) -> Option<JointAxis> {
        if let Some(motor) = self.drive_motors.get(&joint) {
            return Some(motor.axis);
        }
        let j = self.backend.joint(joint)?;
        [JointAxis::AngX, JointAxis::LinX].into_iter().find(|axis| j.motor_position(*axis).is_some())
    }

    /// The setpoint the owner of `joint`'s `axis` DOF last asked for.
    pub fn requested_setpoint(&self, joint: JointId, axis: JointAxis) -> Option<Setpoint> {
        self.control.by_joint.get(&(joint, axis)).and_then(|k| self.control.arbiter.requested(k))
    }

    /// Whether a DOF of `joint` had a state committed last tick.
    pub fn has_command(&self, joint: JointId) -> bool {
        self.control.committed.keys().any(|(j, _)| *j == joint)
    }

    /// An internal producer's setpoint for one DOF, latched until it sends
    /// another. A joint no machine bound is leased to its first writer.
    pub fn submit(&mut self, owner: &OwnerId, joint: JointId, axis: JointAxis, setpoint: Setpoint, cap: Option<f64>) -> Result<(), Rejection> {
        let key = match self.control.by_joint.get(&(joint, axis)) {
            Some(key) => key.clone(),
            None => {
                let key = DofKey { machine: format!("unbound:{}", owner.machine), generation: 0, joint: format!("#{}", joint.0), dof: token_of(axis) };
                let drive = self.authored_drives.get(&joint).map(|d| DriveSpec {
                    stiffness: d.stiffness,
                    damping: d.damping,
                    drive: if d.force { DriveType::Force } else { DriveType::Acceleration },
                    max_force: d.max_force,
                });
                self.control.arbiter.insert(DofSpec {
                    key: key.clone(),
                    interfaces: Interface::ALL.to_vec(),
                    lower: None,
                    upper: None,
                    drive,
                    hardware_cap: None,
                    default_owner: Some(owner.clone()),
                    legacy_caps: true,
                });
                self.control.by_joint.insert((joint, axis), key.clone());
                self.control.bindings.insert(key.clone(), (joint, axis));
                key
            }
        };
        let sequence = self.control.sequences.entry(owner.clone()).or_default();
        *sequence += 1;
        let batch = Batch { owner: owner.clone(), epoch: 0, sequence: *sequence, commands: vec![(key, setpoint)], validity: Validity::Latched, requested_cap: cap };
        self.control.arbiter.submit(batch, Instant::now())
    }

    /// An external or replay batch, validated whole.
    pub fn submit_batch(&mut self, batch: Batch, now: Instant) -> Result<(), Rejection> {
        self.control.arbiter.submit(batch, now)
    }

    /// One commit without a physics step: what the next step would write.
    pub fn commit_pending(&mut self) {
        self.commit_actuators();
    }

    /// Resolve and write every commanded DOF once; drive motors without a
    /// command advance on their own last command.
    pub(super) fn commit_actuators(&mut self) {
        let dt = self.dt();
        let states = {
            let bindings = &self.control.bindings;
            let backend = &self.backend;
            self.control.arbiter.resolve(Instant::now(), |key| {
                bindings.get(key).and_then(|(joint, axis)| backend.joint(*joint).and_then(|j| j.motor_position(*axis)))
            })
        };
        self.control.writes.clear();
        let mut committed = HashMap::with_capacity(states.len());
        for (key, state) in states {
            let Some(&(joint, axis)) = self.control.bindings.get(&key) else { continue };
            let changed = self.control.committed.get(&(joint, axis)) != Some(&state);
            if let Some(motor) = self.drive_motors.get_mut(&joint) {
                if changed {
                    let _ = motor.command(device_command(&state));
                    if let Some(cap) = state.cap {
                        let _ = motor.configure(DeviceSetting::MaxForce(cap));
                    }
                }
                let moved = motor.advance(dt);
                if let Some(j) = self.backend.joint_mut(joint, moved || changed) {
                    motor.write(j);
                }
            } else if self.backend.has_motor(joint) {
                if changed {
                    let _ = self.backend.command_motor(joint, device_command(&state));
                    if let Some(cap) = state.cap {
                        let _ = self.backend.configure_motor(joint, DeviceSetting::MaxForce(cap));
                    }
                }
            } else if let Some(j) = self.backend.joint_mut(joint, changed) {
                write_state(j, axis, &state);
            }
            *self.control.writes.entry((joint, axis)).or_default() += 1;
            committed.insert((joint, axis), state);
        }
        // A DOF whose command is gone (reset, revoked owner) returns to its
        // load-time actuator once: the authored drive and targets, the
        // device's rest command, or none.
        let dropped: Vec<(JointId, JointAxis)> = self.control.committed.keys().copied().filter(|k| !committed.contains_key(k)).collect();
        for (joint, axis) in dropped {
            if let Some(motor) = self.drive_motors.get_mut(&joint) {
                let _ = motor.command(motor.rest_command());
                continue;
            }
            if self.backend.has_motor(joint) {
                if let Some(q) = self.backend.joint(joint).and_then(|j| j.motor_position(axis)) {
                    let _ = self.backend.command_motor(joint, DeviceCommand::Position(q));
                }
                *self.control.writes.entry((joint, axis)).or_default() += 1;
                continue;
            }
            let drive = self.authored_drives.get(&joint).or_else(|| self.authored_axis_drives.get(&(joint, axis))).copied();
            if let Some(j) = self.backend.joint_mut(joint, true) {
                restore_authored(j, axis, drive);
                *self.control.writes.entry((joint, axis)).or_default() += 1;
            }
        }
        let backend = &mut self.backend;
        let writes = &mut self.control.writes;
        for (joint, motor) in &mut self.drive_motors {
            if committed.contains_key(&(*joint, motor.axis)) {
                continue;
            }
            let moved = motor.advance(dt);
            if let Some(j) = backend.joint_mut(*joint, moved) {
                motor.write(j);
            }
            *writes.entry((*joint, motor.axis)).or_default() += 1;
        }
        let gone: Vec<JointId> = self.drive_motors.keys().copied().filter(|j| self.backend.joint(*j).is_none()).collect();
        for joint in gone {
            self.drive_motors.remove(&joint);
        }
        self.control.committed = committed;
    }
}
