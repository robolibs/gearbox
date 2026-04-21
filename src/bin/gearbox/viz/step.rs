//! Per-frame physics stepping.
//!
//! Uses a **fixed timestep accumulator** (as recommended by rapier's
//! docs) so the raycast-suspension vehicle controller sees the same
//! `dt` every step regardless of rendering FPS. Variable dt was the
//! cause of the high-frequency tractor shake at speed — the
//! suspension solver is sensitive to step-size noise.

use bevy::prelude::*;

use super::GearboxSim;

/// Physics rate — 60 Hz matches rapier's default recommendation and
/// keeps the suspension visibly bouncy (120 Hz over-damped it into
/// feeling like a go-kart). Still fixed so integration stays stable.
const PHYSICS_HZ: f32 = 60.0;
/// Cap on substeps per render frame so a stall / debugger pause can't
/// send the accumulator spiralling.
const MAX_SUBSTEPS: u32 = 8;

pub fn step_sim_system(
    mut sim: ResMut<GearboxSim>,
    time: Res<Time>,
    mut accumulator: Local<f32>,
) {
    let dt_fixed = 1.0 / PHYSICS_HZ;
    *accumulator += time.delta_secs();

    let mut steps = 0;
    while *accumulator >= dt_fixed && steps < MAX_SUBSTEPS {
        sim.0.step(dt_fixed);
        *accumulator -= dt_fixed;
        steps += 1;
    }
    // Drop the carried-over fraction if we couldn't keep up — better
    // than letting the accumulator grow unboundedly.
    if *accumulator > dt_fixed * MAX_SUBSTEPS as f32 {
        *accumulator = 0.0;
    }
}
