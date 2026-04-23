//! Standard front-steer Ackermann drive (cars, tractors).
//!
//! `throttle` drives all `driven` wheels with a weight-transfer-aware
//! open differential per axle (unloaded inside wheel gets less torque,
//! loaded outside gets more). `steer` applies per-wheel Ackermann
//! correction to every `steered` wheel. `brake` cuts all wheels.

use std::collections::BTreeMap;

use super::util::ackermann_steer;
use super::{DriveContext, DriveController, GroundFrame};

#[derive(Debug, Default, Clone, Copy)]
pub struct AckermannController;

impl DriveController for AckermannController {
    fn apply(&self, ctx: &mut DriveContext) {
        let ctrl = ctx.control;
        let specs = &ctx.spec.wheels;
        let frame = GroundFrame::compute(ctx);

        // Power gate — when out of fuel/battery, the vehicle can only
        // coast. Zero every wheel's engine force AND steering angle;
        // keep the (passive) brake so the operator can still stop.
        if !ctx.spec.power.is_engine_live() {
            for idx in 0..ctx.wheels.len() {
                let spec = &specs[idx];
                if let Some(mut w) = ctx.wheels.get_mut(idx) {
                    w.set_engine_force(0.0);
                    w.set_brake(ctrl.brake * spec.max_brake * frame.brake_gate);
                    w.set_steering(0.0);
                }
            }
            return;
        }

        // Axle buckets for the weight-transfer differential.
        let mut axles: BTreeMap<i32, Vec<usize>> = BTreeMap::new();
        for (idx, spec) in specs.iter().enumerate() {
            if !spec.driven {
                continue;
            }
            let z_key = (spec.chassis_connection.z * 100.0).round() as i32;
            axles.entry(z_key).or_default().push(idx);
        }

        // Default: every driven wheel gets full throttle × its max engine force.
        let mut engine_force: Vec<f64> = specs
            .iter()
            .map(|s| if s.driven { ctrl.throttle * s.max_engine_force } else { 0.0 })
            .collect();

        // Weight-transfer-aware open diff within each axle.
        for wheel_indices in axles.values() {
            if wheel_indices.len() < 2 {
                continue;
            }
            let total_n: f64 = wheel_indices
                .iter()
                .map(|&i| frame.normal_forces[i])
                .sum();
            if total_n < 1.0 {
                continue;
            }
            let axle_total: f64 = wheel_indices
                .iter()
                .map(|&i| ctrl.throttle * specs[i].max_engine_force)
                .sum();
            for &idx in wheel_indices {
                let share = frame.normal_forces[idx] / total_n;
                engine_force[idx] = axle_total * share;
            }
        }

        for idx in 0..ctx.wheels.len() {
            let spec = &specs[idx];
            let Some(mut w) = ctx.wheels.get_mut(idx) else { continue };
            w.set_engine_force(engine_force[idx]);
            w.set_brake(ctrl.brake * spec.max_brake * frame.brake_gate);
            w.set_steering(if spec.steered {
                ackermann_steer(
                    ctrl.steer,
                    spec.max_steer_rad,
                    spec.chassis_connection.x,
                    frame.wheelbase,
                )
            } else {
                0.0
            });
        }
    }
}
