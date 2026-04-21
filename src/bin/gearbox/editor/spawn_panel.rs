//! Spawn-panel content. Rendered by `left_dock` when the Spawn tab is active.
//!
//! Clicking a preset button doesn't drop the vehicle immediately — it
//! starts a **ghost placement**: a translucent preview follows the
//! cursor until the user clicks somewhere in the viewport to commit
//! (or Esc / RMB to cancel). The heavy lifting for that flow lives in
//! [`super::pending_spawn`]; this panel just queues the request.

use bevy::prelude::*;
use bevy_egui::egui;

use gearbox::presets;

use crate::viz::{GearboxSim, PlayerControlled, VehicleBody};

use super::pending_spawn::PendingSpawn;
use super::style::{fg_dim, section_caps, TEXT_PRIMARY};

pub fn draw_content(
    ui: &mut egui::Ui,
    commands: &mut Commands,
    pending: &mut PendingSpawn,
    existing_bodies: &Query<Entity, With<VehicleBody>>,
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
            if preset_button(ui, "+", "Tractor", "John Deere 8R · 4W RWD", accent).clicked() {
                pending.request(presets::tractor(), commands);
            }
            ui.add_space(2.0);
            if preset_button(ui, "+", "Car", "4-wheel sedan", accent).clicked() {
                pending.request(presets::car(), commands);
            }
            ui.add_space(2.0);
            if preset_button(ui, "+", "Oxbo 2475", "6W pea harvester · crab-steer", accent).clicked() {
                pending.request(presets::oxbo_harvester(), commands);
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

/// A full-width preset card — accent glyph on the left, primary name +
/// small subtitle on the right. Reads like UE5's "Create" entries.
fn preset_button(
    ui: &mut egui::Ui,
    glyph: &str,
    name: &str,
    subtitle: &str,
    accent: egui::Color32,
) -> egui::Response {
    let row_h = 32.0;
    let w = ui.available_width();
    let btn = egui::Button::new("")
        .corner_radius(egui::CornerRadius::same(6))
        .min_size(egui::vec2(w, row_h));
    let resp = ui.add_sized([w, row_h], btn);

    // Custom paint inside the button's rect.
    let rect = resp.rect;
    let painter = ui.painter_at(rect);
    let text_rect = rect.shrink2(egui::vec2(8.0, 0.0));

    painter.text(
        egui::pos2(text_rect.min.x, text_rect.center().y),
        egui::Align2::LEFT_CENTER,
        glyph,
        egui::FontId::proportional(14.0),
        accent,
    );
    painter.text(
        egui::pos2(text_rect.min.x + 22.0, text_rect.center().y - 6.0),
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::proportional(12.0),
        TEXT_PRIMARY,
    );
    painter.text(
        egui::pos2(text_rect.min.x + 22.0, text_rect.center().y + 7.0),
        egui::Align2::LEFT_CENTER,
        subtitle,
        egui::FontId::proportional(10.0),
        fg_dim(),
    );
    resp
}

fn keybinding_row(ui: &mut egui::Ui, keys: &str, action: &str) {
    ui.horizontal(|ui| {
        let chip = egui::RichText::new(keys)
            .monospace()
            .small()
            .color(ui.visuals().text_color());
        let frame = egui::Frame::new()
            .fill(ui.visuals().faint_bg_color)
            .inner_margin(egui::Margin::symmetric(5, 1))
            .corner_radius(egui::CornerRadius::same(3));
        frame.show(ui, |ui| ui.label(chip));
        ui.add_space(6.0);
        ui.label(egui::RichText::new(action).small().color(fg_dim()));
    });
}
