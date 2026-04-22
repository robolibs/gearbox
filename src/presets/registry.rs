//! Catalogue of built-in presets, exposed as a registry so the editor
//! UI can iterate it instead of hard-coding a button per preset.
//!
//! Adding a new preset is:
//!   1. drop a new file under `src/presets/` whose `pub fn foo() -> VehicleSpec`,
//!   2. register it in [`all_presets`].
//!
//! The `PresetRegistry` resource in the editor binary wraps the Vec
//! returned from [`all_presets`]; downstream UIs iterate it.

use crate::VehicleSpec;

/// One catalogue entry: identity, presentation strings, and the
/// factory that builds a fresh [`VehicleSpec`].
#[derive(Debug, Clone, Copy)]
pub struct PresetEntry {
    /// Stable identifier (used for keyboard shortcuts, serialisation,
    /// test IDs — never shown to the user).
    pub id: &'static str,
    /// Short human-facing label.
    pub display_name: &'static str,
    /// One-line description shown beneath the label in the UI.
    pub subtitle: &'static str,
    /// Pure builder — called every spawn so each spec is a fresh copy.
    pub factory: fn() -> VehicleSpec,
}

/// Built-in preset catalogue. Extend this list to add a new robot to
/// the editor's spawn panel.
pub fn all_presets() -> Vec<PresetEntry> {
    vec![
        PresetEntry {
            id: "tractor",
            display_name: "Tractor",
            subtitle: "John Deere 8R · 4W RWD",
            factory: super::tractor,
        },
        PresetEntry {
            id: "husky",
            display_name: "Husky",
            subtitle: "Clearpath robot · differential drive",
            factory: super::husky,
        },
        PresetEntry {
            id: "robotti",
            display_name: "Robotti",
            subtitle: "AGROINTELLI gantry · 4WIS omni",
            factory: super::robotti,
        },
        PresetEntry {
            id: "drone",
            display_name: "Drone",
            subtitle: "Quadcopter · WASD + QE yaw + ZX lift",
            factory: super::drone,
        },
        PresetEntry {
            id: "oxbo",
            display_name: "Oxbo",
            subtitle: "6W pea harvester · crab-steer",
            factory: super::oxbo_harvester,
        },
    ]
}
