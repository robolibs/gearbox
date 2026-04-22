//! Arcade quad-copter drive — direct body forces + PD tilt controller.
//!
//! A real quadrotor is an inverted pendulum: differential rotor thrust
//! tilts the body, gravity then pulls it horizontally. Simulating that
//! literally would need a PID attitude stack to stop the drone
//! flipping (nothing holds an inverted pendulum up by itself). Instead
//! we cheat in the standard game-engine way:
//!
//!   - Horizontal translation is driven by direct forces at the
//!     centre of mass — always stable, easy to steer.
//!   - Tilt is produced by a PD controller that drives pitch/roll
//!     toward a target proportional to the stick input. Release the
//!     stick and it levels out again.
//!
//! Control mapping:
//!   - `throttle` (W/S) → forward / backward force + nose-down/up
//!     visual tilt.
//!   - `steer`    (A/D) → strafe force + bank-right/left tilt.
//!   - `lift`     (Z/X) → extra vertical force on top of a constant
//!                        hover force that cancels gravity.
//!   - `yaw`      (Q/E) → yaw torque around world +Y. Positive `yaw`
//!                        (Q) = turn LEFT.

use rapier3d::prelude::Vec3;

use super::{DriveContext, DriveController};

// Tunables — per-mass, so scaling the drone up keeps the feel.
const HORIZ_ACCEL: f32 = 6.0; // m/s² at full stick
const LIFT_ACCEL: f32 = 10.0; // m/s² at full lift
const YAW_ACCEL: f32 = 2.7; // rad/s² at full yaw
const MAX_TILT: f32 = 0.30; // rad (~17°) at full stick
/// PD gains in angular-acceleration space (rad/s² per rad of error,
/// and per rad/s of rate). Scaled by the body's actual principal
/// inertia at apply-time so small drones don't get torques sized for
/// a refrigerator. ω_n ≈ 8 rad/s, ζ ≈ 0.9 → near-critical damping.
const TILT_OMEGA: f32 = 8.0;
const TILT_ZETA: f32 = 0.9;

#[derive(Debug, Default, Clone, Copy)]
pub struct DroneController;

impl DriveController for DroneController {
    fn is_airborne(&self) -> bool {
        true
    }

    fn apply(&self, ctx: &mut DriveContext) {
        let ctrl = ctx.control;

        // Power gate: when the battery runs flat the drone stops
        // applying *any* force — hover cancels too, so it falls under
        // gravity like a real dead quadrotor.
        if !ctx.spec.power.is_engine_live() {
            ctx.body.reset_forces(true);
            ctx.body.reset_torques(true);
            return;
        }

        let mass = ctx.body.mass();
        let rot = ctx.body.rotation();

        // World-frame basis vectors from the body's current rotation.
        let fwd_world = rot * Vec3::Z;
        let right_world = rot * Vec3::X;
        let up_body_world = rot * Vec3::Y;

        // Horizontal projections so the drone doesn't dive when tilted.
        let fwd_h = Vec3::new(fwd_world.x, 0.0, fwd_world.z).normalize_or_zero();
        let right_h = Vec3::new(right_world.x, 0.0, right_world.z).normalize_or_zero();

        // --- Linear forces ---
        // Hover goes along WORLD +Y (not body +Y) so altitude is
        // maintained even when the drone tilts for W/S forward motion
        // — the previous body-local hover let it sink by `1 − cos(θ)`
        // of mg every tilt. Flipped drones still fall, because the
        // hover is gated on `up_body_world.y > 0.3` — body +Y pointing
        // actually up. Once you roll past ~73° hover cuts out and
        // gravity takes the machine down, same as the "dead drone"
        // behaviour we already had.
        let upright = up_body_world.y > 0.3;
        let hover = if upright {
            Vec3::new(0.0, -ctx.gravity.y * mass, 0.0)
        } else {
            Vec3::ZERO
        };
        // Z/X (lift) is also world-vertical so a tilted drone doesn't
        // strafe when the operator asks for altitude.
        let lift = Vec3::new(0.0, ctrl.lift * mass * LIFT_ACCEL, 0.0);
        let fore = fwd_h * ctrl.throttle * mass * HORIZ_ACCEL;
        let side = right_h * ctrl.steer * mass * HORIZ_ACCEL;

        ctx.body.reset_forces(true);
        ctx.body.reset_torques(true);
        ctx.body.add_force(hover + lift + fore + side, true);

        // --- Tilt PD controller ---
        // Measure pitch/roll from how the body's local +Y projects
        // into world. Small-angle OK; the gains compensate.
        let pitch_angle = up_body_world.z.atan2(up_body_world.y);
        let roll_angle = up_body_world.x.atan2(up_body_world.y);
        let target_pitch = ctrl.throttle * MAX_TILT;
        let target_roll = ctrl.steer * MAX_TILT;

        let angvel_world = ctx.body.angvel();
        let angvel_local = rot.inverse() * angvel_world;

        let kp = TILT_OMEGA * TILT_OMEGA;
        let kd = 2.0 * TILT_ZETA * TILT_OMEGA;
        let pitch_alpha = kp * (target_pitch - pitch_angle) - kd * angvel_local.x;
        let roll_alpha = kp * (target_roll - roll_angle) - kd * (-angvel_local.z);

        // Convert α → Nm using the body's actual inertia tensor.
        let local_inertia = ctx.body.principal_inertia();
        let pitch_torque_local = pitch_alpha * local_inertia.x;
        let roll_torque_local = -roll_alpha * local_inertia.z;

        let yaw_torque_world_y = -ctrl.yaw * YAW_ACCEL * local_inertia.y;

        let torque_local = Vec3::new(pitch_torque_local, 0.0, roll_torque_local);
        let torque_world = rot * torque_local + Vec3::new(0.0, yaw_torque_world_y, 0.0);
        ctx.body.add_torque(torque_world, true);
    }
}
