//! Skid-steer / differential-drive (Husky, tracked robots).
//!
//! `steer` biases left-vs-right wheel throttle instead of a steering
//! angle. `TURN_GAIN` amplifies the steer component so turn-in-place
//! feels crisp even at the low `max_engine_force` values a small
//! outdoor robot needs for a sensible straight-line top speed.

use super::{DriveContext, DriveController, GroundFrame};

/// Throttle multiplier applied to the steer contribution. Higher =
/// snappier turns without boosting straight-line top speed.
const TURN_GAIN: f64 = 6.0;

#[derive(Debug, Default, Clone, Copy)]
pub struct DifferentialController;

impl DriveController for DifferentialController {
    fn apply(&self, ctx: &mut DriveContext) {
        let ctrl = ctx.control;
        let specs = &ctx.spec.wheels;
        let frame = GroundFrame::compute(ctx);

        // Power gate: zero drive when out of battery.
        if !ctx.spec.power.is_engine_live() {
            for (idx, spec) in specs.iter().enumerate().take(ctx.wheels.len()) {
                if let Some(mut w) = ctx.wheels.get_mut(idx) {
                    w.set_engine_force(0.0);
                    w.set_brake(ctrl.brake * spec.max_brake * frame.brake_gate);
                    w.set_steering(0.0);
                }
            }
            return;
        }

        // +X is the right side in our lateral convention. Positive
        // `steer` (A key) pivots the vehicle LEFT, so right-side
        // wheels push backward while left-side wheels push forward.
        let t = ctrl.throttle;
        let s = ctrl.steer * TURN_GAIN;
        let left_cmd = t + s;
        let right_cmd = t - s;

        for (idx, spec) in specs.iter().enumerate().take(ctx.wheels.len()) {
            let Some(mut w) = ctx.wheels.get_mut(idx) else {
                continue;
            };
            w.set_engine_force(if spec.driven {
                let cmd = if spec.chassis_connection.x < 0.0 {
                    left_cmd
                } else {
                    right_cmd
                };
                cmd * spec.max_engine_force
            } else {
                0.0
            });
            w.set_brake(ctrl.brake * spec.max_brake * frame.brake_gate);
            w.set_steering(0.0);
        }
    }
}
