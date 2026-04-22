//! Scene-tree panel content. Driven by `left_dock`.
//!
//! Blender 4 outliner / UE5 world-outliner hybrid:
//!   - 20 px rows, 14 px glyph, label 12 px
//!   - default / hover / selected-inactive / selected-active states
//!   - 2 px accent left-border on the focused row
//!   - small dot prefix in `warning` for the player-controlled vehicle

use bevy::prelude::*;
use bevy_egui::egui;

use gearbox_physics::VehicleId;

use gearbox_viz::{GearboxSim, PlayerControlled, VehicleBody};

use super::selection::Selection;
use super::style::{
    space, BG_2_RAISED, BG_3_HOVER, TEXT_PRIMARY, TEXT_SECONDARY, WARNING,
};
use super::widgets::section;

const ROW_H: f32 = 20.0;

pub fn draw_content(
    ui: &mut egui::Ui,
    commands: &mut Commands,
    sim: &GearboxSim,
    bodies: &Query<(Entity, &VehicleBody, Option<&Name>, Has<PlayerControlled>)>,
    selection: &mut Selection,
    accent: egui::Color32,
    // Set by this function when the user double-clicks a vehicle
    // row; caller (`left_dock`) reads it after the panel closes and
    // reframes the chase camera on that vehicle.
    frame_to: &mut Option<VehicleId>,
) {
    let mut give_drive_to: Option<(VehicleId, Entity)> = None;

    // ─── Scene outliner (first — default-open) ────────────────────
    section(ui, "tree_scene", "Scene", accent, true, |ui| {
        if bodies.is_empty() {
            ui.add_space(space::BLOCK);
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new("Empty scene")
                        .strong()
                        .color(TEXT_SECONDARY),
                );
                ui.add_space(space::TIGHT);
                ui.label(
                    egui::RichText::new("Open the Spawn tab to add something.")
                        .small()
                        .color(TEXT_SECONDARY),
                );
            });
            return;
        }

        let mut rows: Vec<(Entity, VehicleId, String, bool)> = bodies
            .iter()
            .map(|(e, vb, name, is_player)| {
                let label = match name {
                    Some(n) => n.as_str().to_string(),
                    None => format!("Vehicle #{}", vb.id.0),
                };
                (e, vb.id, label, is_player)
            })
            .collect();
        rows.sort_by_key(|(_, id, _, _)| id.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            for (entity, id, label, is_player) in rows {
                let selected = selection.vehicle == Some(id);
                let resp = outliner_row(ui, &label, id, is_player, selected, accent);
                if resp.clicked() {
                    selection.vehicle = Some(id);
                }
                if resp.double_clicked() {
                    *frame_to = Some(id);
                    if !is_player {
                        give_drive_to = Some((id, entity));
                    }
                }
            }
        });
    });

    ui.add_space(space::SECTION);

    // ─── Stats (default-closed) ───────────────────────────────────
    section(ui, "tree_stats", "Stats", accent, false, |ui| {
        ui.label(
            egui::RichText::new(format!(
                "{} total · double-click to focus + drive",
                sim.0.vehicles().count()
            ))
            .small()
            .color(TEXT_SECONDARY),
        );
    });

    if let Some((_new_id, new_entity)) = give_drive_to {
        let currently_player: Vec<Entity> = bodies
            .iter()
            .filter(|(_, _, _, p)| *p)
            .map(|(e, _, _, _)| e)
            .collect();
        for e in currently_player {
            commands.entity(e).remove::<PlayerControlled>();
        }
        commands.entity(new_entity).insert(PlayerControlled);
    }
}

/// Single outliner row painted manually so we get proper state fills,
/// a hover glow, and the 2 px accent left-border on selection. egui's
/// built-in `SelectableLabel` can't match these cues.
fn outliner_row(
    ui: &mut egui::Ui,
    label: &str,
    id: VehicleId,
    is_player: bool,
    selected: bool,
    accent: egui::Color32,
) -> egui::Response {
    let w = ui.available_width();
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(w, ROW_H), egui::Sense::click());
    let painter = ui.painter_at(rect);

    // State fills.
    if selected {
        // 35 % accent blended over BG_2_RAISED — readable against the
        // panel without drowning the label in the vehicle's own tint.
        let blend = |a: u8, b: u8| {
            ((a as f32) * 0.65 + (b as f32) * 0.35).round() as u8
        };
        let selection_tint = egui::Color32::from_rgb(
            blend(BG_2_RAISED.r(), accent.r()),
            blend(BG_2_RAISED.g(), accent.g()),
            blend(BG_2_RAISED.b(), accent.b()),
        );
        painter.rect_filled(rect, egui::CornerRadius::same(3), selection_tint);
    } else if resp.hovered() {
        painter.rect_filled(rect, egui::CornerRadius::same(3), BG_3_HOVER);
    }
    // Accent left-border on selection.
    if selected {
        let bar = egui::Rect::from_min_size(
            egui::pos2(rect.min.x, rect.min.y + 2.0),
            egui::vec2(2.0, rect.height() - 4.0),
        );
        painter.rect_filled(bar, egui::CornerRadius::same(1), accent);
    }

    // Player dot (small bold accent glyph) or inactive ring.
    let dot_x = rect.min.x + 10.0;
    let mid_y = rect.center().y;
    if is_player {
        painter.circle_filled(egui::pos2(dot_x, mid_y), 3.0, WARNING);
    } else {
        painter.circle_stroke(
            egui::pos2(dot_x, mid_y),
            3.0,
            egui::Stroke::new(1.0, TEXT_SECONDARY),
        );
    }

    // Label + id on the right.
    let text_color = if selected { TEXT_PRIMARY } else { TEXT_PRIMARY };
    painter.text(
        egui::pos2(rect.min.x + 22.0, mid_y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.0),
        text_color,
    );
    painter.text(
        egui::pos2(rect.max.x - 6.0, mid_y),
        egui::Align2::RIGHT_CENTER,
        format!("#{}", id.0),
        egui::FontId::proportional(10.0),
        TEXT_SECONDARY,
    );

    resp
}
