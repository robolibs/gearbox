//! Motor devices on joints with an authored `PhysicsDriveAPI`. The device
//! moves the drive's targets, no faster than its speed and no sharper than
//! its acceleration; the drive's stiffness, damping and force limit move the
//! joint, as in any UsdPhysics simulator.

use super::backend::{DeviceCommand, DeviceLimits, DeviceSetting, JointAxis, JointMut, MotorModel, MotorOutput};
use super::world::AuthoredDrive;

/// A force command runs the drive toward this speed with this damping, so
/// its force limit is the force applied.
const FORCE_COMMAND_VELOCITY: f64 = 1.0e6;
const FORCE_COMMAND_DAMPING: f64 = 1.0e10;

#[derive(Clone, Copy, Debug)]
pub(crate) struct DriveMotor {
    pub axis: JointAxis,
    drive: AuthoredDrive,
    limits: DeviceLimits,
    max_force: f64,
    command: DeviceCommand,
    /// Speed position control runs at, up to `limits.max_velocity`.
    speed: f64,
    /// The drive's targets.
    target: f64,
    velocity: f64,
    integral: f64,
    previous_error: Option<f64>,
    /// The joint's coordinate and rate after the last step.
    position: f64,
    rate: f64,
}

impl DriveMotor {
    /// A motor on the joint at `position`, heading for its drive's authored
    /// target: a spring's target position (else `position`), a damper's
    /// target velocity (else rest). The drive's force limit, else
    /// `max_force`, caps it.
    pub(crate) fn new(
        axis: JointAxis,
        drive: AuthoredDrive,
        limits: DeviceLimits,
        max_force: f64,
        position: f64,
    ) -> Result<Self, String> {
        let max_force = drive.max_force.unwrap_or(max_force);
        if !(0.0..).contains(&max_force) || !(0.0..).contains(&limits.max_velocity) {
            return Err("motor force and velocity limits must be nonnegative".into());
        }
        let mut motor = Self {
            axis,
            drive,
            limits,
            max_force,
            command: DeviceCommand::Position(position),
            speed: limits.max_velocity,
            target: position,
            velocity: 0.0,
            integral: 0.0,
            previous_error: None,
            position,
            rate: 0.0,
        };
        motor.command(if drive.stiffness > 0.0 {
            DeviceCommand::Position(drive.target_position.unwrap_or(position))
        } else {
            DeviceCommand::Velocity(drive.target_velocity.unwrap_or(0.0))
        })?;
        Ok(motor)
    }

    pub(crate) fn command(&mut self, command: DeviceCommand) -> Result<(), String> {
        let (DeviceCommand::Position(value) | DeviceCommand::Velocity(value) | DeviceCommand::Force(value)) = command;
        if !value.is_finite() {
            return Err("motor command must be finite".into());
        }
        let command = match command {
            DeviceCommand::Position(p) => DeviceCommand::Position(match self.limits.position_limits {
                Some([lo, hi]) => p.clamp(lo, hi),
                None => p,
            }),
            DeviceCommand::Velocity(v) => {
                DeviceCommand::Velocity(v.clamp(-self.limits.max_velocity, self.limits.max_velocity))
            }
            force => force,
        };
        if !matches!((self.command, command), (DeviceCommand::Position(_), DeviceCommand::Position(_))) {
            self.integral = 0.0;
            self.previous_error = None;
        }
        // Out of a force, a spring takes up from where the joint went, not
        // from the target it held before.
        if matches!(self.command, DeviceCommand::Force(_))
            && !matches!(command, DeviceCommand::Force(_))
        {
            self.target = self.position;
            self.velocity = 0.0;
        }
        self.command = command;
        Ok(())
    }

    pub(crate) fn configure(&mut self, setting: DeviceSetting) -> Result<(), String> {
        match setting {
            DeviceSetting::Speed(speed) => self.speed = speed.abs().min(self.limits.max_velocity),
            DeviceSetting::Acceleration(acceleration) => self.limits.acceleration = acceleration.map(f64::abs),
            DeviceSetting::Pid(pid) => {
                self.limits.pid = pid;
                self.integral = 0.0;
                self.previous_error = None;
            }
            DeviceSetting::MaxForce(force) if force >= 0.0 => self.max_force = force,
            DeviceSetting::MaxForce(_) => return Err("motor force limit must be nonnegative".into()),
        }
        Ok(())
    }

    /// Moves the targets one step of `dt`; whether they changed.
    pub(crate) fn advance(&mut self, dt: f64) -> bool {
        let before = (self.target, self.velocity);
        let spring = self.drive.stiffness > 0.0;
        let want = match self.command {
            DeviceCommand::Force(_) => return false,
            DeviceCommand::Velocity(v) => v,
            // A spring's target travels to the goal at the speed, braking
            // in time to stop on it.
            DeviceCommand::Position(goal) if spring => {
                let error = goal - self.target;
                let mut speed = self.speed.min(error.abs() / dt);
                if let Some(a) = self.limits.acceleration {
                    speed = speed.min((2.0 * a * error.abs()).sqrt());
                }
                speed.copysign(error)
            }
            // A damper runs at the speed the PID asks of the position error.
            DeviceCommand::Position(goal) => {
                let error = goal - self.position;
                self.integral += error * dt;
                let derivative = self.previous_error.map_or(0.0, |previous| (error - previous) / dt);
                self.previous_error = Some(error);
                let [p, i, d] = self.limits.pid;
                (p * error + i * self.integral + d * derivative).clamp(-self.speed, self.speed)
            }
        };
        self.velocity = match self.limits.acceleration {
            Some(a) => self.velocity + (want - self.velocity).clamp(-a * dt, a * dt),
            None => want,
        };
        if spring {
            self.target += self.velocity * dt;
            match self.command {
                DeviceCommand::Position(goal) if (goal - self.target) * (goal - before.0) <= 0.0 => {
                    self.target = goal;
                    self.velocity = 0.0;
                }
                // A running spring's target stays within the reach of its
                // force limit.
                DeviceCommand::Velocity(_) => {
                    let reach = self.max_force / self.drive.stiffness;
                    self.target = self.target.clamp(self.position - reach, self.position + reach);
                }
                _ => {}
            }
        }
        (self.target, self.velocity) != before
    }

    /// Writes this step's drive onto the joint.
    pub(crate) fn write(&self, joint: &mut dyn JointMut) {
        let (stiffness, damping) = (self.drive.stiffness, self.drive.damping);
        if let DeviceCommand::Force(force) = self.command {
            joint.set_motor_model(self.axis, MotorModel::Force);
            joint.set_motor_velocity(self.axis, FORCE_COMMAND_VELOCITY.copysign(force), FORCE_COMMAND_DAMPING);
            joint.set_motor_max_force(self.axis, force.abs().min(self.max_force));
            return;
        }
        let model = if self.drive.force { MotorModel::Force } else { MotorModel::Acceleration };
        joint.set_motor_model(self.axis, model);
        if stiffness > 0.0 {
            joint.set_motor(self.axis, self.target, self.velocity, stiffness, damping);
        } else {
            joint.set_motor_velocity(self.axis, self.velocity, damping);
        }
        if self.max_force.is_finite() {
            joint.set_motor_max_force(self.axis, self.max_force);
        }
    }

    /// Records the joint's coordinate after a step of `dt`.
    pub(crate) fn observe(&mut self, position: f64, dt: f64) {
        self.rate = (position - self.position) / dt;
        self.position = position;
    }

    /// The motor's reading; `brake` is the backend's reading of a brake on
    /// the same joint.
    pub(crate) fn output(&self, brake: Option<MotorOutput>) -> MotorOutput {
        let force = match self.command {
            DeviceCommand::Force(force) => force,
            _ => {
                let spring = if self.drive.stiffness > 0.0 { self.drive.stiffness * (self.target - self.position) } else { 0.0 };
                spring + self.drive.damping * (self.velocity - self.rate)
            }
        };
        MotorOutput {
            position: self.position,
            velocity: self.rate,
            command: self.command,
            commanded_velocity: self.velocity,
            force: force.clamp(-self.max_force, self.max_force),
            brake_damping: brake.map_or(0.0, |b| b.brake_damping),
            brake_force: brake.map_or(0.0, |b| b.brake_force),
        }
    }
}
