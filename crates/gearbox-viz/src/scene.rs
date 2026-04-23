//! Renderer-facing state mirror.
//!
//! # The boundary this enforces
//!
//! The renderer MUST NOT touch rapier / `Sim` directly. Anything a
//! renderer system needs to draw — poses, clock, eventually USD
//! prim xforms — lives on [`SceneState`]. A mirror system copies
//! the authoritative server-side data into it each frame.
//!
//! Right now the mirror is deliberately thin (clock state only).
//! Vehicle-specific reads still hit `Sim` directly from `viz` and
//! `editor`; that legacy code path will migrate here when OpenUSD
//! replaces the vehicle model and poses become keyed by `sdf::Path`
//! instead of `VehicleId`. The abstraction is landed *now* so the
//! direction of travel is obvious in the source tree.
//!
//! ## How it splits later
//!
//! - **One binary (today).** `mirror_scene_state_system` copies
//!   `SimClock` + the step system's frame counter into `SceneState`
//!   every `PostUpdate`. In-memory, no transport.
//! - **Split across processes (future).** The server-side copier
//!   stays exactly as written. A `gearbox-link` publisher serialises
//!   `SceneState` onto the wire; a client-side consumer deserialises
//!   straight back into the renderer's own `SceneState`. The
//!   renderer code — which reads only from `SceneState` — never
//!   learns about the split.

use bevy::prelude::*;

use super::step::SimClock;

/// Everything the renderer is allowed to see. Populated by the
/// mirror system; renderer code is read-only on this resource.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct SceneState {
    pub clock: SceneClock,
    // Future: `pub prim_xforms: HashMap<SdfPath, Pose>` once the
    // OpenUSD scene layer lands. Vehicle-specific snapshot data
    // will NOT live here — it'll live as USD prims authored by
    // `bevy_openusd`, mirrored into this map.
}

/// Sim-clock snapshot. Duplicates `SimClock` on purpose: `SimClock`
/// is the *authoritative* server-owned state, this is the
/// *observable* mirror. In a split deployment the two may not be
/// the same process, and the renderer is supposed to read the
/// mirror, never the authoritative copy.
#[derive(Debug, Default, Clone, Copy)]
pub struct SceneClock {
    pub paused: bool,
    pub speed: f32,
    /// Monotonic count of fixed-dt sim substeps that have run since
    /// startup. Useful for a "did physics advance this render frame"
    /// check on the renderer side without a second resource.
    pub frame: u64,
    /// Accumulated sim time in seconds. `frame * (1/60)` today —
    /// kept as an explicit field so the accumulation strategy can
    /// change without renderer-side code caring.
    pub time_s: f64,
}

/// Copies server-side state into [`SceneState`]. Intended to run
/// once per frame, after the sim step so the mirror reflects the
/// state the renderer will draw.
pub fn mirror_scene_state_system(clock: Res<SimClock>, mut scene: ResMut<SceneState>) {
    // Treat SceneState as write-only on the server side (this
    // system) and read-only everywhere else in the renderer. The
    // compiler doesn't enforce it, but that's the contract.
    scene.clock.paused = clock.paused;
    scene.clock.speed = clock.speed.multiplier();
    scene.clock.frame = clock.sim_frame;
    scene.clock.time_s = clock.sim_time_s;
}
