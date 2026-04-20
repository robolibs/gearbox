//! Per-frame physics stepping.

use bevy::prelude::*;

use super::GearboxSim;

/// Steps the gearbox `Sim` by the current frame's delta time.
///
/// Uses a clamped `dt` so a debugger breakpoint or hitch doesn't produce a
/// multi-second step that would blow the solver up.
pub fn step_sim_system(mut sim: ResMut<GearboxSim>, time: Res<Time>) {
    let dt = time.delta_secs().clamp(0.0, 1.0 / 30.0);
    sim.0.step(dt);
}
