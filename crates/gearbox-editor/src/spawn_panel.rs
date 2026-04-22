//! Spawn-panel content. Rendered by `left_dock` when the Spawn tab is active.
//!
//! Clicking a preset button doesn't drop the vehicle immediately — it
//! starts a **ghost placement**: a translucent preview follows the
//! cursor until the user clicks somewhere in the viewport to commit
//! (or Esc / RMB to cancel). The heavy lifting for that flow lives in
//! [`super::pending_spawn`]; this panel just queues the request.
//!
//! The list of available presets is driven by the [`PresetRegistry`]
//! resource, so adding a new robot is one entry in
//! `gearbox_physics::presets::all_presets` — no edits here.

use bevy::prelude::*;
use bevy_egui::egui;

use gearbox_viz::{GearboxSim, PlayerControlled, VehicleBody};

use super::pending_spawn::PendingSpawn;
use super::preset_registry::PresetRegistry;
use super::style::{caption, space};
use super::widgets::{card_button, section};

pub fn draw_content(
    ui: &mut egui::Ui,
    commands: &mut Commands,
    pending: &mut PendingSpawn,
    _existing_bodies: &Query<Entity, With<VehicleBody>>,
    registry: &PresetRegistry,
    _sim: &mut GearboxSim,
    _meshes: &mut Assets<Mesh>,
    _materials: &mut Assets<StandardMaterial>,
    _player_tagged: &Query<Entity, With<PlayerControlled>>,
    accent: egui::Color32,
) {
    section(ui, "library_vehicles", "Vehicles", accent, true, |ui| {
        for (i, entry) in registry.iter().enumerate() {
            if i > 0 {
                ui.add_space(space::TIGHT);
            }
            if card_button(ui, "+", entry.display_name, entry.subtitle, accent).clicked() {
                pending.request((entry.factory)(), commands);
            }
        }

        if pending.spec.is_some() {
            ui.add_space(space::BLOCK);
            ui.label(caption(
                "Click in the viewport to place · Esc / RMB to cancel",
            ));
        }
    });
}
