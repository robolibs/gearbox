//! 4-wheel-independent-steer (4WIS / omni) drive — Robotti.
//!
//! Input mapping:
//!   W/S (throttle) → drive force along each wheel's current facing.
//!   A/D (steer)    → Ackermann on front wheels, small same-direction
//!                    assist on the rear wheels, with a kinematic
//!                    drive-force differential so inner/outer wheels
//!                    don't scrub.
//!   Z/X (lift)     → all four wheels same angle (crab / strafe).
//!   Q/E (yaw)      → pivot in place — each wheel points tangentially
//!                    to a circle centred on the body. Yaw overrides
//!                    steer+lift when active because the target
//!                    direction depends on wheel position.
//!
//! Wheel steering is rate-limited here so pressing a key doesn't snap
//! every wheel from 0° → 90° in a single tick (which would shove the
//! body sideways on sticky tyres).

use super::util::ackermann_steer;
use super::{DriveContext, DriveController, GroundFrame};

/// Max A/D steering angle (leaves headroom for crab to combine).
const TURN_LIMIT: f64 = 40.0 * core::f64::consts::PI / 180.0;
/// Rear-wheel same-direction assist, as a fraction of the front nominal angle.
const REAR_ASSIST: f64 = 0.18;
/// How fast a wheel is allowed to reach its target steering angle.
const STEER_RATE: f64 = 3.0; // rad/s

#[derive(Debug, Default, Clone, Copy)]
pub struct OmniController;

impl DriveController for OmniController {
    fn apply(&self, ctx: &mut DriveContext) {
        let ctrl = ctx.control;
        let specs = &ctx.spec.wheels;
        let frame = GroundFrame::compute(ctx);
        // Power gate: zero all drive + hold steering if any battery/
        // fuel source is empty. The wheels coast; body can still drift.
        if !ctx.spec.power.is_engine_live() {
            for idx in 0..ctx.wheels.len() {
                let spec = &specs[idx];
                if let Some(mut w) = ctx.wheels.get_mut(idx) {
                    w.set_engine_force(0.0);
                    w.set_brake(ctrl.brake * spec.max_brake * frame.brake_gate);
                    // Keep the last steered angle — feels more natural
                    // than snapping back to 0° the instant power dies.
                }
            }
            return;
        }

        // Divide front vs rear by the mean wheel Z so asymmetric
        // layouts still work.
        let mid_z = (frame.z_min + frame.z_max) * 0.5;
        let use_yaw = ctrl.yaw.abs() > 1e-3;

        let mut target_steer = vec![0.0_f64; specs.len()];
        let mut engine_force = vec![0.0_f64; specs.len()];

        for (idx, spec) in specs.iter().enumerate() {
            let x_i = spec.chassis_connection.x;
            let z_i = spec.chassis_connection.z;
            let is_front = z_i > mid_z;
            let max_steer = spec.max_steer_rad;

            if use_yaw {
                // Pivot in place: positive `yaw` (Q) turns LEFT, so
                // flip the sign to match right-handed rotation.
                let omega_y = -ctrl.yaw;
                let vx = omega_y * z_i;
                let vz = -omega_y * x_i;
                let mag = (vx * vx + vz * vz).sqrt();
                let raw = if mag > 1e-4 { vx.atan2(vz) } else { 0.0 };
                let (folded, sign) = if raw > core::f64::consts::FRAC_PI_2 {
                    (raw - core::f64::consts::PI, -1.0)
                } else if raw < -core::f64::consts::FRAC_PI_2 {
                    (raw + core::f64::consts::PI, -1.0)
                } else {
                    (raw, 1.0)
                };
                target_steer[idx] = folded.clamp(-max_steer, max_steer);
                engine_force[idx] = if spec.driven {
                    mag.min(1.0) * spec.max_engine_force * sign
                } else {
                    0.0
                };
            } else {
                // Ackermann on front + gentle same-direction rear assist.
                let turn_cap = max_steer.min(TURN_LIMIT);
                let front_angle = if is_front && spec.steered {
                    ackermann_steer(ctrl.steer, turn_cap, x_i, frame.wheelbase)
                } else {
                    0.0
                };
                let rear_angle = if !is_front {
                    ctrl.steer * turn_cap * REAR_ASSIST
                } else {
                    0.0
                };
                let crab = ctrl.lift * max_steer;
                target_steer[idx] = (front_angle + rear_angle + crab).clamp(-max_steer, max_steer);

                // Kinematic diff — each wheel's engine force scaled
                // by its radius from the Ackermann turn centre vs the
                // rear-axle centre's radius.
                let steer_mag = ctrl.steer.abs();
                let diff_scale = if steer_mag > 1e-3 && frame.wheelbase > 1e-3 {
                    let delta_nominal = ctrl.steer * turn_cap;
                    let x_o = -frame.wheelbase / delta_nominal.tan();
                    let rear_axle_z = frame.z_min;
                    let dx = x_i - x_o;
                    let dz = z_i - rear_axle_z;
                    let r_i = (dx * dx + dz * dz).sqrt();
                    let r_rear = x_o.abs().max(1e-3);
                    let ratio = r_i / r_rear;
                    // 60 % kinematic / 40 % uniform, clamped to keep
                    // imbalance controllable at large steer.
                    (1.0 + 0.6 * (ratio - 1.0)).clamp(0.4, 1.6)
                } else {
                    1.0
                };
                engine_force[idx] = if spec.driven {
                    ctrl.throttle * spec.max_engine_force * diff_scale
                } else {
                    0.0
                };
            }
        }

        let max_delta = STEER_RATE * ctx.dt;
        for idx in 0..ctx.wheels.len() {
            let spec = &specs[idx];
            let Some(mut w) = ctx.wheels.get_mut(idx) else {
                continue;
            };
            w.set_engine_force(engine_force[idx]);
            w.set_brake(ctrl.brake * spec.max_brake * frame.brake_gate);
            // Rate-limit so an instant 0°→90° doesn't shove the body.
            let current = w.steering();
            let delta = (target_steer[idx] - current).clamp(-max_delta, max_delta);
            w.set_steering(current + delta);
        }
    }
}
