//! Scene-tree panel content. Driven by `left_dock`.
//!
//! Each row is a [`bevy_frost::widgets::hybrid_select_row`]: the body
//! handles transient selection (click) + focus-and-drive
//! (double-click), while the right-edge radio is a durable one-at-a-
//! time "camera follows this" pin. The two click targets are
//! independent — hovering / selecting a row never flips follow, and
//! clicking the radio never selects.

use bevy::prelude::*;
use bevy_egui::egui;
use bevy_frost::PaneBuilder;

use gearbox_physics::VehicleId;

use gearbox_viz::{FollowTarget, GearboxSim, PlayerControlled, VehicleBody};

use super::selection::Selection;
use super::style::{space, TEXT_SECONDARY};
use super::usd_load::UsdSelectable;
use super::widgets::hybrid_select_row;

pub fn draw_content(
    pane: &mut PaneBuilder,
    commands: &mut Commands,
    sim: &GearboxSim,
    bodies: &Query<(Entity, &VehicleBody, Option<&Name>, Has<PlayerControlled>)>,
    usd_assets: &Query<(Entity, Option<&Name>), With<UsdSelectable>>,
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
    pane.section("tree_scene", "Scene", true, |ui| {
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
                let trailing = format!("#{}", id.0);
                let resp = hybrid_select_row(
                    ui,
                    id.0,
                    &label,
                    Some(&trailing),
                    selected,
                    following,
                    accent,
                );
                if resp.body.clicked() {
                    selection.vehicle = Some(id);
                }
                if resp.body.double_clicked() {
                    *frame_to = Some(id);
                    if !is_player {
                        give_drive_to = Some((id, entity));
                    }
                }
                if resp.radio.clicked() {
                    follow.toggle(id);
                }
            }
        });
    });

    // ─── USDs (default-open when any are loaded) ──────────────────
    {
        let usd_count = usd_assets.iter().count();
        let mut usd_rows: Vec<(Entity, String)> = usd_assets
            .iter()
            .map(|(e, name)| {
                let label = name
                    .map(|n| n.as_str().to_string())
                    .unwrap_or_else(|| format!("USD {:?}", e));
                (e, label)
            })
            .collect();
        usd_rows.sort_by(|a, b| a.1.cmp(&b.1));
        pane.section("tree_usd", "USDs", usd_count > 0, |ui| {
            if usd_count == 0 {
                ui.add_space(space::TIGHT);
                ui.label(
                    egui::RichText::new("No USDs loaded yet — click 📂 in the left rail.")
                        .small()
                        .color(TEXT_SECONDARY),
                );
                return;
            }
            for (entity, label) in usd_rows {
                let selected = selection.usd_entity == Some(entity);
                // No "follow" semantics for USDs (yet) — pass false
                // for the radio so it renders inactive.
                let resp = hybrid_select_row(
                    ui,
                    entity.to_bits() as u32,
                    &label,
                    None,
                    selected,
                    false,
                    accent,
                );
                if resp.body.clicked() {
                    selection.usd_entity = Some(entity);
                    selection.vehicle = None;
                }
            }
        });
    }

    // ─── Stats (default-closed) ───────────────────────────────────
    pane.section("tree_stats", "Stats", false, |ui| {
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
