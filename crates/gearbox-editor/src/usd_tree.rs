//! Dedicated **USD prim tree** floating panel — mirrors the layout
//! of the standalone usdview's `draw_tree_panel` (search box, indent
//! guides, expand/collapse chevrons, eye toggle, swatch slot,
//! selection highlight) but driven by gearbox-editor's `Selection`.
//!
//! Each loaded USD asset (a `SceneRoot` carrying `UsdSelectable`)
//! renders as a top-level row with a 🗑 trash button; descending
//! into it walks the Bevy parent/child hierarchy of `UsdPrimRef`-
//! tagged entities — the same composed prim tree the projected
//! scene exposes.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use bevy_frost::style;
use bevy_frost::widgets::{TreeIconKind, TreeIconSlot, sub_caption, tree_row};
use bevy_frost::{PaneBuilder, RibbonOpen, RibbonPlacement, floating_window_for_item};

use super::dock_ribbons::{ID_USD_TREE, RIBBON_ITEMS, RIBBONS, is_menu_open};
use super::selection::Selection;
use super::style::AccentColor;
use super::usd_load::{PendingUsdRemoval, UsdSelectable, UsdTreeExpanded};

const PANEL_W: f32 = 360.0;
const PANEL_H: f32 = 720.0;

/// Free-text filter used when the user types in the panel's search
/// box. Matches against any `UsdPrimRef.path` (substring, case
/// insensitive). Persists across frames so the box doesn't lose
/// focus / value on every redraw.
#[derive(Resource, Default)]
pub struct UsdTreeFilter(pub String);

#[allow(clippy::too_many_arguments)]
pub fn draw_usd_tree_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    mut selection: ResMut<Selection>,
    mut expanded: ResMut<UsdTreeExpanded>,
    mut pending_removal: ResMut<PendingUsdRemoval>,
    mut filter: ResMut<UsdTreeFilter>,
    roots: Query<(Entity, Option<&Name>), With<UsdSelectable>>,
    prims: Query<(Option<&Name>, &usd_bevy::UsdPrimRef)>,
    children: Query<&Children>,
) {
    if !is_menu_open(&open, &placement, ID_USD_TREE) {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let accent_col = accent.0;
    let mut keep = true;
    floating_window_for_item(
        ctx,
        RIBBONS,
        RIBBON_ITEMS,
        &placement,
        ID_USD_TREE,
        "USD Tree",
        egui::vec2(PANEL_W, PANEL_H),
        &mut keep,
        accent_col,
        |pane: &mut PaneBuilder| {
            pane.section("usd_tree_root", "Hierarchy", true, |ui| {
                let count = roots.iter().count();
                sub_caption(ui, &format!("{count} loaded asset(s)"));
                ui.add_space(style::space::TIGHT);
                ui.text_edit_singleline(&mut filter.0);
                ui.add_space(style::space::BLOCK);

                if count == 0 {
                    sub_caption(ui, "Nothing loaded yet — click 📂 in the left rail.");
                    return;
                }

                let filter_lc = filter.0.to_lowercase();
                let flat = !filter_lc.is_empty();

                let mut root_rows: Vec<(Entity, String)> = roots
                    .iter()
                    .map(|(e, n)| {
                        let label = n
                            .map(|n| n.as_str().to_string())
                            .unwrap_or_else(|| format!("USD {e:?}"));
                        (e, label)
                    })
                    .collect();
                root_rows.sort_by(|a, b| a.1.cmp(&b.1));

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .min_scrolled_height(560.0)
                    .max_height(560.0)
                    .show(ui, |ui| {
                        if flat {
                            // Flat search across every descendant of every loaded
                            // asset. `collect_descendants` walks the Bevy
                            // children tree and yields each USD-prim entity with
                            // its leaf name; we then filter by the typed query.
                            let mut all: HashMap<Entity, String> = HashMap::new();
                            for root in &root_rows {
                                collect_descendants(root.0, &prims, &children, &mut all);
                            }
                            let mut hits: Vec<(Entity, String)> = all
                                .into_iter()
                                .filter(|(_, label)| label.to_lowercase().contains(&filter_lc))
                                .collect();
                            hits.sort_by(|a, b| a.1.cmp(&b.1));
                            if hits.is_empty() {
                                sub_caption(ui, "(no matches)");
                            }
                            for (entity, label) in hits {
                                draw_row(
                                    ui,
                                    entity,
                                    &label,
                                    false,
                                    0,
                                    /*has_children*/ false,
                                    &mut selection,
                                    &mut expanded,
                                    &mut pending_removal,
                                    accent_col,
                                    /*force_leaf*/ true,
                                );
                            }
                        } else {
                            for (root, label) in &root_rows {
                                walk(
                                    ui,
                                    *root,
                                    label,
                                    true,
                                    0,
                                    &prims,
                                    &children,
                                    &mut selection,
                                    &mut expanded,
                                    &mut pending_removal,
                                    accent_col,
                                );
                            }
                        }
                    });
            });
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn walk(
    ui: &mut egui::Ui,
    entity: Entity,
    label: &str,
    is_root_asset: bool,
    depth: u32,
    prims: &Query<(Option<&Name>, &usd_bevy::UsdPrimRef)>,
    children: &Query<&Children>,
    selection: &mut Selection,
    expanded: &mut UsdTreeExpanded,
    pending_removal: &mut PendingUsdRemoval,
    accent: egui::Color32,
) {
    // Children that are themselves USD prims.
    let prim_children: Vec<(Entity, String)> = children
        .get(entity)
        .map(|cs| {
            cs.iter()
                .filter_map(|c| prims.get(c).ok().map(|(n, p)| (c, leaf_name(p, n))))
                .collect()
        })
        .unwrap_or_default();
    let has_children = !prim_children.is_empty();

    draw_row(
        ui,
        entity,
        label,
        is_root_asset,
        depth,
        has_children,
        selection,
        expanded,
        pending_removal,
        accent,
        /*force_leaf*/ false,
    );

    let is_open = *expanded.0.entry(entity).or_insert(is_root_asset);
    if is_open && has_children {
        let mut sorted = prim_children;
        sorted.sort_by(|a, b| a.1.cmp(&b.1));
        for (child, child_label) in sorted {
            walk(
                ui,
                child,
                &child_label,
                false,
                depth + 1,
                prims,
                children,
                selection,
                expanded,
                pending_removal,
                accent,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_row(
    ui: &mut egui::Ui,
    entity: Entity,
    label: &str,
    is_root_asset: bool,
    depth: u32,
    has_children: bool,
    selection: &mut Selection,
    expanded: &mut UsdTreeExpanded,
    pending_removal: &mut PendingUsdRemoval,
    accent: egui::Color32,
    force_leaf: bool,
) {
    let is_selected = selection.usd_entity == Some(entity);
    // Default-expanded for ALL nodes — the user explicitly asked
    // for the full tree visible by default. Collapse with the
    // chevron once it's shown.
    let mut is_open_local = *expanded.0.get(&entity).unwrap_or(&true);
    let mut visible_sentinel = true;
    // Trash slot is a stable gutter column for every row (frost
    // requires the same slot shape on every row in a tree).
    // Non-root clicks are silently ignored — only loaded assets
    // can actually be removed.
    let mut trash_sentinel = false;

    let mut slot_buf: Vec<TreeIconSlot<'_>> = vec![
        TreeIconSlot::new(TreeIconKind::Eye, &mut visible_sentinel),
        TreeIconSlot::new(
            TreeIconKind::Glyph {
                on: "🗑", off: "🗑"
            },
            &mut trash_sentinel,
        )
        .with_tooltip("Remove from scene (root only)"),
    ];

    let resp = if has_children && !force_leaf {
        tree_row(
            ui,
            entity.to_bits(),
            depth,
            Some(&mut is_open_local),
            None,
            label,
            is_selected,
            accent,
            &mut slot_buf,
        )
    } else {
        tree_row(
            ui,
            entity.to_bits(),
            depth,
            None,
            None,
            label,
            is_selected,
            accent,
            &mut slot_buf,
        )
    };

    if resp.body.clicked() {
        selection.usd_entity = Some(entity);
        selection.vehicle = None;
    }

    // Trash icon clicked — remove only when this row IS a loaded
    // asset root; in-tree prims aren't independently removable.
    if let Some(trash_resp) = resp.icons.get(1) {
        if trash_resp.clicked() && is_root_asset {
            pending_removal.0.push(entity);
            if selection.usd_entity == Some(entity) {
                selection.usd_entity = None;
            }
        }
    }

    if has_children && !force_leaf {
        expanded.0.insert(entity, is_open_local);
    }
}

fn collect_descendants(
    root: Entity,
    prims: &Query<(Option<&Name>, &usd_bevy::UsdPrimRef)>,
    children: &Query<&Children>,
    out: &mut HashMap<Entity, String>,
) {
    let mut stack = vec![root];
    while let Some(e) = stack.pop() {
        if let Ok((name, pref)) = prims.get(e) {
            out.insert(e, leaf_name(pref, name));
        }
        if let Ok(cs) = children.get(e) {
            for c in cs.iter() {
                stack.push(c);
            }
        }
    }
}

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
