//! Scene-tree panel content. Driven by `left_dock`.
//!
//! Blender 4 outliner / UE5 world-outliner hybrid:
//!   - 20 px rows, 14 px glyph, label 12 px
//!   - default / hover / selected-inactive / selected-active states
//!   - right-edge radio to toggle camera follow (independent of the row).

use bevy::prelude::*;
use bevy_egui::egui;

use gearbox_physics::VehicleId;

use gearbox_viz::{FollowTarget, GearboxSim, PlayerControlled, VehicleBody};

use super::selection::Selection;
use super::style::{space, BG_2_RAISED, BG_3_HOVER, TEXT_PRIMARY, TEXT_SECONDARY};
use super::widgets::section;

const ROW_H: f32 = 20.0;

pub fn draw_content(
    ui: &mut egui::Ui,
    commands: &mut Commands,
    sim: &GearboxSim,
    bodies: &Query<(Entity, &VehicleBody, Option<&Name>, Has<PlayerControlled>)>,
    selection: &mut Selection,
    follow: &mut FollowTarget,
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
                let following = follow.vehicle == Some(id);
                let (row_resp, radio_resp) =
                    outliner_row(ui, &label, id, selected, following, accent);
                if row_resp.clicked() {
                    selection.vehicle = Some(id);
                }
                if row_resp.double_clicked() {
                    *frame_to = Some(id);
                    if !is_player {
                        give_drive_to = Some((id, entity));
                    }
                }
                if radio_resp.clicked() {
                    follow.toggle(id);
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

/// Single outliner row. Painted manually so the selection fill, hover
/// glow and right-edge follow radio all sit flush. The row has two
/// independent click targets: the main body (selection / double-click
/// to focus) and a radio button at the right edge (camera-follow
/// toggle). The radio reserves its own rect so clicks on it never
/// propagate to the row body.
fn outliner_row(
    ui: &mut egui::Ui,
    label: &str,
    id: VehicleId,
    selected: bool,
    following: bool,
    accent: egui::Color32,
) -> (egui::Response, egui::Response) {
    let w = ui.available_width();

    // Radio geometry. Kept compact so the label still gets most of
    // the row width.
    let radio_outer_r: f32 = 4.5;
    let radio_slot_w: f32 = 14.0;
    let radio_pad_r: f32 = 5.0;

    // Claim the whole row with `hover()` so the sub-rects below own
    // click routing. This is the only way I know to keep the row's
    // layout accounting correct while splitting click targets.
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(w, ROW_H), egui::Sense::hover());

    let radio_rect = egui::Rect::from_min_size(
        egui::pos2(rect.max.x - radio_slot_w - radio_pad_r, rect.min.y),
        egui::vec2(radio_slot_w, rect.height()),
    );
    // Main body stops short of the radio area so the click targets
    // never overlap, even near the boundary.
    let body_rect = egui::Rect::from_min_max(
        rect.min,
        egui::pos2(radio_rect.min.x, rect.max.y),
    );

    let body_resp = ui.interact(
        body_rect,
        ui.id().with(("outliner_row", id.0)),
        egui::Sense::click(),
    );
    let radio_resp = ui.interact(
        radio_rect,
        ui.id().with(("outliner_radio", id.0)),
        egui::Sense::click(),
    );

    let painter = ui.painter_at(rect);
    let mid_y = rect.center().y;

    // Selection / hover fill — applied only to the body so the radio
    // slot reads as a separate control, not a continuation of the row.
    if selected {
        let blend = |a: u8, b: u8| {
            ((a as f32) * 0.65 + (b as f32) * 0.35).round() as u8
        };
        let tint = egui::Color32::from_rgb(
            blend(BG_2_RAISED.r(), accent.r()),
            blend(BG_2_RAISED.g(), accent.g()),
            blend(BG_2_RAISED.b(), accent.b()),
        );
        painter.rect_filled(body_rect, egui::CornerRadius::same(3), tint);
    } else if body_resp.hovered() {
        painter.rect_filled(body_rect, egui::CornerRadius::same(3), BG_3_HOVER);
    }

    // Label + id.
    painter.text(
        egui::pos2(body_rect.min.x + 10.0, mid_y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.0),
        TEXT_PRIMARY,
    );
    painter.text(
        egui::pos2(body_rect.max.x - 6.0, mid_y),
        egui::Align2::RIGHT_CENTER,
        format!("#{}", id.0),
        egui::FontId::proportional(10.0),
        TEXT_SECONDARY,
    );

    // Radio button — outline ring + filled dot when this vehicle is
    // the active follow target. Hover just brightens the ring to the
    // accent colour so the control reads as interactive.
    let radio_center = egui::pos2(radio_rect.center().x, mid_y);
    let ring_color = if following || radio_resp.hovered() {
        accent
    } else {
        TEXT_SECONDARY
    };
    painter.circle_stroke(
        radio_center,
        radio_outer_r,
        egui::Stroke::new(1.2, ring_color),
    );
    if following {
        painter.circle_filled(radio_center, radio_outer_r - 1.8, accent);
    }

    (body_resp, radio_resp)
}
