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
//! `gearbox::presets::all_presets` — no edits here.

use bevy::prelude::*;
use bevy_egui::egui;

use crate::viz::{GearboxSim, PlayerControlled, VehicleBody};

use super::pending_spawn::PendingSpawn;
use super::preset_registry::PresetRegistry;
use super::style::{fg_dim, section_caps};
use super::widgets::{card_button, keybinding_row};

pub fn draw_content(
    ui: &mut egui::Ui,
    commands: &mut Commands,
    pending: &mut PendingSpawn,
    existing_bodies: &Query<Entity, With<VehicleBody>>,
    registry: &PresetRegistry,
    _sim: &mut GearboxSim,
    _meshes: &mut Assets<Mesh>,
    _materials: &mut Assets<StandardMaterial>,
    _player_tagged: &Query<Entity, With<PlayerControlled>>,
    accent: egui::Color32,
) {
    egui::CollapsingHeader::new(section_caps("Vehicles", accent))
        .id_salt("spawn_vehicles")
        .default_open(true)
        .show(ui, |ui| {
            let mut first = true;
            for entry in registry.iter() {
                if !first {
                    ui.add_space(2.0);
                }
                first = false;
                if card_button(ui, "+", entry.display_name, entry.subtitle, accent).clicked() {
                    pending.request((entry.factory)(), commands);
                }
            }

            if pending.spec.is_some() {
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("Click in the viewport to place · Esc / RMB to cancel")
                        .small()
                        .italics()
                        .color(fg_dim()),
                );
            }
        });

    egui::CollapsingHeader::new(section_caps("Stats", accent))
        .id_salt("spawn_stats")
        .default_open(false)
        .show(ui, |ui| {
            egui::Grid::new("spawn_stats_grid")
                .num_columns(2)
                .spacing([8.0, 3.0])
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new("vehicles").small().color(fg_dim()),
                    );
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.label(
                                egui::RichText::new(
                                    existing_bodies.iter().count().to_string(),
                                )
                                .strong(),
                            );
                        },
                    );
                    ui.end_row();
                });
        });

    egui::CollapsingHeader::new(section_caps("Keys", accent))
        .id_salt("spawn_keys")
        .default_open(false)
        .show(ui, |ui| {
            keybinding_row(ui, "W / S",    "throttle");
            keybinding_row(ui, "A / D",    "steer");
            keybinding_row(ui, "Space",    "brake");
            ui.add_space(4.0);
            keybinding_row(ui, "LMB",      "select / drag / place");
            keybinding_row(ui, "LMB+RMB",  "orbit");
            keybinding_row(ui, "MMB",      "pan");
            keybinding_row(ui, "Wheel",    "zoom");
            keybinding_row(ui, "Esc",      "cancel placement");
        });
}
