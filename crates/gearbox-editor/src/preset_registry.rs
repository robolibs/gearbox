//! Bevy-level wrapper around the preset catalogue from the gearbox
//! library. Held as a `Resource` so any UI system can iterate the
//! available presets without hard-coding a button per entry.
//!
//! Consumers add a preset by either:
//!   - appending to [`gearbox_physics::presets::all_presets`] (the built-in
//!     catalogue); or
//!   - pushing directly into the `PresetRegistry` resource at startup
//!     to register app-specific presets without changing the library.

use bevy::prelude::*;

use gearbox_physics::presets::{all_presets, PresetEntry};

/// Editor-visible list of spawnable presets.
#[derive(Resource, Default, Clone)]
pub struct PresetRegistry(pub Vec<PresetEntry>);

impl PresetRegistry {
    /// Seed the registry with the library's built-in preset catalogue.
    pub fn with_defaults() -> Self {
        Self(all_presets())
    }

    /// Register an additional preset at runtime (e.g. from a user plugin).
    #[allow(dead_code)]
    pub fn push(&mut self, entry: PresetEntry) {
        self.0.push(entry);
    }

    pub fn iter(&self) -> impl Iterator<Item = &PresetEntry> {
        self.0.iter()
    }
}
