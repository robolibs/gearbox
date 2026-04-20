//! Spawn-panel content. Rendered by `left_dock` when the Spawn tab is active.
//!
//! Layout:
//!   - Collapsible "Vehicles" section with preset buttons
//!   - Collapsible "Stats" with live entity counts
//!   - Collapsible "Keybindings" cheat-sheet at the bottom

use bevy::prelude::*;
use bevy_egui::egui;

use gearbox::{
    datapod::{Point, Pose, Quaternion},
    presets, VehicleSpec,
};

use crate::viz::{GearboxSim, PlayerControlled, VehicleBody, spawn_vehicle_visuals};
use crate::BigSpaceRoot;

use super::selection::Selection;
use super::style::{accent_color, fg_dim, section_caps, TEXT_PRIMARY};

pub fn draw_content(
    ui: &mut egui::Ui,
    commands: &mut Commands,
    sim: &mut GearboxSim,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    existing_bodies: &Query<Entity, With<VehicleBody>>,
    player_tagged: &Query<Entity, With<PlayerControlled>>,
    selection: &mut Selection,
    big_space_root: Entity,
) {
    egui::CollapsingHeader::new(section_caps("Vehicles"))
        .id_salt("spawn_vehicles")
        .default_open(false)
        .show(ui, |ui| {
            if preset_button(ui, "+", "Tractor", "4-wheel RWD rig").clicked() {
                spawn_and_select(
                    presets::tractor(),
                    commands, sim, meshes, materials, player_tagged, selection, big_space_root,
                );
            }
            ui.add_space(2.0);
            if preset_button(ui, "+", "Car", "4-wheel sedan").clicked() {
                spawn_and_select(
                    presets::car(),
                    commands, sim, meshes, materials, player_tagged, selection, big_space_root,
                );
            }
        });

    egui::CollapsingHeader::new(section_caps("Stats"))
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

    egui::CollapsingHeader::new(section_caps("Keys"))
        .id_salt("spawn_keys")
        .default_open(false)
        .show(ui, |ui| {
            keybinding_row(ui, "W / S",    "throttle");
            keybinding_row(ui, "A / D",    "steer");
            keybinding_row(ui, "Space",    "brake");
            ui.add_space(4.0);
            keybinding_row(ui, "LMB",      "select / drag");
            keybinding_row(ui, "LMB+RMB",  "orbit");
            keybinding_row(ui, "MMB",      "pan");
            keybinding_row(ui, "Wheel",    "zoom");
        });
}

/// A full-width preset card — accent glyph on the left, primary name +
/// small subtitle on the right. Reads like UE5's "Create" entries.
fn preset_button(
    ui: &mut egui::Ui,
    glyph: &str,
    name: &str,
    subtitle: &str,
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
        accent_color(),
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

fn spawn_and_select(
    spec: VehicleSpec,
    commands: &mut Commands,
    sim: &mut GearboxSim,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    player_tagged: &Query<Entity, With<PlayerControlled>>,
    selection: &mut Selection,
    big_space_root: Entity,
) {
    let pose = Pose {
        point: Point::new(0.0, 1.4, 0.0),
        rotation: Quaternion::identity(),
    };
    let id = sim.0.spawn_vehicle(spec.clone(), pose);
    let root = spawn_vehicle_visuals(commands, meshes, materials, id, &spec, big_space_root);
    for e in player_tagged {
        commands.entity(e).remove::<PlayerControlled>();
    }
    commands.entity(root).insert(PlayerControlled);
    selection.vehicle = Some(id);
}
