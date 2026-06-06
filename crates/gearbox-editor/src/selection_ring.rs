//! Drives the [`bevy_glacial::SelectionRing`] resource from gearbox-
//! specific state — the selected vehicle's pose + footprint, the
//! sim clock (paused state hides the ring), and the editor accent.
//!
//! All rendering / shader / mesh-management lives in
//! [`bevy_glacial::SelectionRingPlugin`]; this file just writes the
//! per-frame target.

use bevy::prelude::*;

use bevy_glacial::SelectionRing;
use gearbox_physics::DriveMode;
use gearbox_viz::{GearboxSim, SimClock};

use super::selection::Selection;
use super::style::AccentColor;
use super::usd_load::UsdSelectable;

// Re-export so callers that still import from
// `super::selection_ring::{SelectionRingPlugin, SelectionRingSettings}`
// keep working unchanged.
pub use bevy_glacial::{SelectionRingPlugin, SelectionRingSettings};

/// Height above the ground for ground vehicles. Drones override this
/// with their actual world-Y so the marker floats with the machine.
const RING_GROUND_OFFSET: f32 = 0.05;

/// Ring sizing uses a *fixed padding* rather than a multiplier so
/// very long machines don't get a ring 10 m wider than their
/// silhouette.
const RING_PADDING_GROUND: f32 = 0.65;
const RING_PADDING_DRONE: f32 = 0.3;
const MIN_OUTER_RADIUS_GROUND: f32 = 1.3;
const MIN_OUTER_RADIUS_DRONE: f32 = 0.9;

/// Per-frame system: compute where the selection ring should sit
/// (or hide it) and write the target into the bevy_glacial
/// resource.
pub fn update_selection_ring(
    selection: Res<Selection>,
    sim: Res<GearboxSim>,
    clock: Res<SimClock>,
    accent: Res<AccentColor>,
    mut target: ResMut<SelectionRing>,
    usd_selected: Query<(&GlobalTransform, &UsdSelectable)>,
) {
    // Drive-mode marker only — edit-mode (paused) uses transform
    // gizmos instead.
    if clock.paused {
        target.anchor = None;
        return;
    }
    // USD-loaded asset selected: ring sits at its world position
    // sized to its `pick_radius`. Same colour / fade as vehicles.
    if let Some(usd_entity) = selection.usd_entity
        && let Ok((gt, sel)) = usd_selected.get(usd_entity)
    {
        let pos = gt.translation();
        target.anchor = Some(Vec3::new(pos.x, RING_GROUND_OFFSET, pos.z));
        target.outer_radius = sel.pick_radius.max(MIN_OUTER_RADIUS_GROUND);
        target.color = egui_to_color(accent.0);
        return;
    }
    let Some(id) = selection.vehicle else {
        target.anchor = None;
        return;
    };
    let Some(state) = sim.0.vehicle(id) else {
        target.anchor = None;
        return;
    };

    // Outer radius = farthest point of the whole machine from the
    // chassis origin (top-down), plus a per-mode padding floor.
    let mut max_reach = {
        let hx = state.spec.chassis.size.x * 0.5;
        let hz = state.spec.chassis.size.z * 0.5;
        hx.max(hz) as f32
    };
    for p in &state.spec.parts {
        let hx = p.size.x * 0.5;
        let hz = p.size.z * 0.5;
        let reach_x = (p.position.x.abs() + hx) as f32;
        let reach_z = (p.position.z.abs() + hz) as f32;
        max_reach = max_reach.max(reach_x).max(reach_z);
    }
    let (padding, min_outer) = match state.spec.drive_mode {
        DriveMode::Drone => (RING_PADDING_DRONE, MIN_OUTER_RADIUS_DRONE),
        _ => (RING_PADDING_GROUND, MIN_OUTER_RADIUS_GROUND),
    };
    let outer = (max_reach + padding).max(min_outer);

    let pose = sim.0.vehicle_pose(id);
    let ring_y = match state.spec.drive_mode {
        DriveMode::Drone => pose.point.y as f32,
        _ => RING_GROUND_OFFSET,
    };

    target.anchor = Some(Vec3::new(pose.point.x as f32, ring_y, pose.point.z as f32));
    target.outer_radius = outer;
    target.color = egui_to_color(accent.0);
}

fn egui_to_color(c: bevy_egui::egui::Color32) -> Color {
    Color::srgba(
        c.r() as f32 / 255.0,
        c.g() as f32 / 255.0,
        c.b() as f32 / 255.0,
        c.a() as f32 / 255.0,
    )
}
