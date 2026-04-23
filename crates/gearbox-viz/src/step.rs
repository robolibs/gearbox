//! Per-frame physics stepping.
//!
//! Uses a **fixed timestep accumulator** (as recommended by rapier's
//! docs) so the raycast-suspension vehicle controller sees the same
//! `dt` every step regardless of rendering FPS. Variable dt was the
//! cause of the high-frequency tractor shake at speed — the
//! suspension solver is sensitive to step-size noise.
//!
//! A [`SimClock`] resource drives play/pause + speed multiplier,
//! driven by the viewport transport bar.

use bevy::prelude::*;

use super::GearboxSim;

/// Physics rate — 60 Hz matches rapier's default recommendation and
/// keeps the suspension visibly bouncy (120 Hz over-damped it into
/// feeling like a go-kart). Still fixed so integration stays stable.
const PHYSICS_HZ: f32 = 60.0;
/// Cap on substeps per render frame so a stall / debugger pause can't
/// send the accumulator spiralling. Raised enough to accommodate the
/// 8× speed mode comfortably (8 steps/60 Hz frame in the normal case).
const MAX_SUBSTEPS: u32 = 24;

/// Sim-time controls: play/pause and speed multiplier. Resource is
/// read by `step_sim_system` each frame. Also carries the monotonic
/// sim-frame and accumulated sim-time counters so other systems
/// (the `SceneState` mirror, the tool API clock publisher) don't
/// have to re-derive them from `Time::elapsed_secs()` — that would
/// drift under variable render rate or pause.
#[derive(Resource, Debug, Copy, Clone, PartialEq)]
pub struct SimClock {
    pub paused: bool,
    pub speed: SimSpeed,
    /// Fixed-dt substeps run since startup. Incremented from
    /// `step_sim_system` only.
    pub sim_frame: u64,
    /// Accumulated sim time in seconds. Exactly
    /// `sim_frame * (1 / PHYSICS_HZ)` today; kept as an explicit
    /// field so the integrator can change its accumulation strategy
    /// without every consumer needing to follow.
    pub sim_time_s: f64,
}

impl Default for SimClock {
    fn default() -> Self {
        // Boot paused so the user can orient themselves (adjust the
        // camera, pick a machine, set up the scene) before physics
        // starts ticking. Hit the top-centre play button to run.
        Self {
            paused: true,
            speed: SimSpeed::X1,
            sim_frame: 0,
            sim_time_s: 0.0,
        }
    }
}

/// Discrete speed presets — click the transport bar's speed button to
/// cycle 1× → 2× → 4× → 8× → 1×.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SimSpeed {
    X1,
    X2,
    X4,
    X8,
}

impl SimSpeed {
    pub fn multiplier(self) -> f32 {
        match self {
            SimSpeed::X1 => 1.0,
            SimSpeed::X2 => 2.0,
            SimSpeed::X4 => 4.0,
            SimSpeed::X8 => 8.0,
        }
    }
    pub fn next(self) -> Self {
        match self {
            SimSpeed::X1 => SimSpeed::X2,
            SimSpeed::X2 => SimSpeed::X4,
            SimSpeed::X4 => SimSpeed::X8,
            SimSpeed::X8 => SimSpeed::X1,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            SimSpeed::X1 => "1x",
            SimSpeed::X2 => "2x",
            SimSpeed::X4 => "4x",
            SimSpeed::X8 => "8x",
        }
    }
}

pub fn step_sim_system(
    mut sim: ResMut<GearboxSim>,
    time: Res<Time>,
    mut clock: ResMut<SimClock>,
    mut accumulator: Local<f32>,
    mut warmed_up: Local<bool>,
) {
    // One-shot warm-up: run a real `pipeline.step(0.0)` the very
    // first time this system fires. Without it, if the sim boots
    // paused we go straight into `refresh_kinematics`, which does
    // wheel raycasts against a broad-phase BVH that `pipeline.step`
    // has never populated — parry 0.26 hits a `ray_aabb.rs:60`
    // underflow on degenerate BVH states. One zero-dt `pipeline.step`
    // builds the BVH cleanly and the bug doesn't fire.
    if !*warmed_up {
        sim.0.step(0.0);
        *warmed_up = true;
    }

    // Paused → no stepping at all, and drop any carried time so we
    // don't fast-forward when play resumes. Still refresh wheel
    // raycasts so wheels track chassis edits made via the inspector
    // fields or the 3-D drag gizmos.
    if clock.paused {
        *accumulator = 0.0;
        sim.0.refresh_kinematics();
        return;
    }

    let dt_fixed = 1.0 / PHYSICS_HZ;
    // Inflating the accumulator by the speed multiplier runs more
    // substeps per render frame while keeping each step at the same
    // fixed `dt`. Collider integration stays stable at any speed.
    *accumulator += time.delta_secs() * clock.speed.multiplier();

    let mut steps = 0;
    while *accumulator >= dt_fixed && steps < MAX_SUBSTEPS {
        sim.0.step(dt_fixed as f64);
        *accumulator -= dt_fixed;
        steps += 1;
        clock.sim_frame = clock.sim_frame.wrapping_add(1);
        clock.sim_time_s += dt_fixed as f64;
    }
    // Drop the carried-over fraction if we couldn't keep up — better
    // than letting the accumulator grow unboundedly.
    if *accumulator > dt_fixed * MAX_SUBSTEPS as f32 {
        *accumulator = 0.0;
    }
}
