//! Thin wrapper around [`bevy_frost::style`] that adds the one
//! gearbox-specific piece of theme wiring: pull the accent colour
//! from the selected vehicle's chassis colour each frame.
//!
//! The rest of the theme — palette, fonts, glass opacity, widgets —
//! lives in the reusable `bevy_frost` crate. Editor-internal code
//! keeps importing `super::style::*` as before; this module
//! re-exports the frost surface so nothing downstream had to change
//! its imports during the extraction.

pub use bevy_frost::style::*;

use bevy::prelude::*;

use gearbox_viz::GearboxSim;

use super::selection::Selection;

/// Update the accent colour from the currently selected vehicle's
/// chassis colour. Runs before panels so the new colour lands in the
/// same frame the selection changes. Defaults to
/// [`bevy_frost::style::ACCENT_NEUTRAL`] when nothing is selected.
pub fn update_accent_from_selection(
    selection: Res<Selection>,
    sim: Res<GearboxSim>,
    mut accent: ResMut<AccentColor>,
) {
    let new_color = selection
        .vehicle
        .and_then(|id| sim.0.vehicle(id))
        .map(|v| srgb_to_egui(v.spec.chassis.color))
        .unwrap_or(ACCENT_NEUTRAL);
    if accent.0 != new_color {
        accent.0 = new_color;
    }
}
