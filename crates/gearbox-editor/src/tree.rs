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
use super::style::{TEXT_SECONDARY, space};
use super::usd_load::{PendingUsdRemoval, UsdSelectable, UsdTreeExpanded};
use super::widgets::hybrid_select_row;

/// Per-row depth indent in pixels — matches the viewer's tree.
const TREE_INDENT_PX: f32 = 14.0;

pub fn draw_content(
    pane: &mut PaneBuilder,
    commands: &mut Commands,
    sim: &GearboxSim,
    bodies: &Query<(Entity, &VehicleBody, Option<&Name>, Has<PlayerControlled>)>,
    usd_assets: &Query<(Entity, Option<&Name>), With<UsdSelectable>>,
    prim_q: &Query<(Option<&Name>, &usd_bevy::UsdPrimRef)>,
    children_q: &Query<&Children>,
    selection: &mut Selection,
    follow: &mut FollowTarget,
    pending_usd_removal: &mut PendingUsdRemoval,
    expanded: &mut UsdTreeExpanded,
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

        // Cap height so the USDs section below stays in view; an
        // unbounded `ScrollArea` here ate all the panel's vertical
        // room and the USDs section never appeared.
        egui::ScrollArea::vertical()
            .id_salt("tree_scene_scroll")
            .max_height(200.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
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

    // ─── USDs section moved to its own dedicated 🌳 USD Tree panel.
    //   The unused params here keep the function signature stable
    //   for the caller; remove them in a follow-up cleanup.
    let _ = (
        usd_assets,
        prim_q,
        children_q,
        pending_usd_removal,
        expanded,
    );

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

/// Render one row of the USD tree, then recurse into its children.
///
/// `is_root_asset` controls whether the trailing 🗑 trash button
/// appears (only on the top-level Asset rows — removing an inner
/// prim doesn't mean anything in our data model yet).
#[allow(clippy::too_many_arguments)]
fn draw_usd_row(
    ui: &mut egui::Ui,
    entity: Entity,
    label: &str,
    is_root_asset: bool,
    depth: u32,
    prim_q: &Query<(Option<&Name>, &usd_bevy::UsdPrimRef)>,
    children_q: &Query<&Children>,
    selection: &mut Selection,
    pending_removal: &mut PendingUsdRemoval,
    expanded: &mut UsdTreeExpanded,
    accent: egui::Color32,
) {
    // Collect this entity's USD-prim children (skipping non-USD
    // descendants that may have been spawned alongside the scene —
    // helper meshes, gizmos, etc.).
    let prim_children: Vec<(Entity, String)> = children_q
        .get(entity)
        .map(|cs| {
            cs.iter()
                .filter_map(|c| {
                    prim_q.get(c).ok().map(|(name, pref)| {
                        let lbl = leaf_name(pref, name);
                        (c, lbl)
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let has_children = !prim_children.is_empty();
    let is_selected = selection.usd_entity == Some(entity);
    // Default: roots open, inner nodes collapsed. Stored only when
    // the user explicitly toggles.
    let default_open = is_root_asset;
    let is_open = *expanded.0.get(&entity).unwrap_or(&default_open);

    ui.horizontal(|ui| {
        ui.add_space(depth as f32 * TREE_INDENT_PX);
        // Chevron / spacer.
        if has_children {
            let glyph = if is_open { "▾" } else { "▸" };
            if ui.small_button(glyph).clicked() {
                expanded.0.insert(entity, !is_open);
            }
        } else {
            ui.add_space(18.0);
        }
        // Selectable label (fills remaining minus trash slot).
        let trash_w = if is_root_asset { 28.0 } else { 0.0 };
        let label_w = (ui.available_width() - trash_w).max(40.0);
        let resp = ui.add_sized(
            [label_w, 0.0],
            egui::SelectableLabel::new(is_selected, label),
        );
        if resp.clicked() {
            selection.usd_entity = Some(entity);
            selection.vehicle = None;
        }
        if is_root_asset {
            if ui
                .small_button("🗑")
                .on_hover_text("Remove from scene")
                .clicked()
            {
                pending_removal.0.push(entity);
                if selection.usd_entity == Some(entity) {
                    selection.usd_entity = None;
                }
            }
        }
        let _ = accent;
    });

    if is_open && has_children {
        let mut sorted = prim_children;
        sorted.sort_by(|a, b| a.1.cmp(&b.1));
        for (child, child_label) in sorted {
            draw_usd_row(
                ui,
                child,
                &child_label,
                false,
                depth + 1,
                prim_q,
                children_q,
                selection,
                pending_removal,
                expanded,
                accent,
            );
        }
    }
}

/// Tree-row label preference: authored prim leaf name (last segment
/// of the USD path) > the Bevy entity `Name`. Falls back to "(prim)"
/// if neither has anything readable.
fn leaf_name(pref: &usd_bevy::UsdPrimRef, name: Option<&Name>) -> String {
    pref.path
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            name.map(|n| n.as_str().to_string())
                .unwrap_or_else(|| "(prim)".to_string())
        })
}
