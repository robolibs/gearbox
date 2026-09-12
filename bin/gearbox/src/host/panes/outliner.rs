//! The Outliner: every loaded asset as a tree root with its prims beneath,
//! eye toggles for visibility, a trash slot on roots, a search filter; then
//! the active asset's materials (colour pods) and variant sets (dropdowns).

use bevy::asset::{AssetId, AssetServer};
use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::container::SeparatorStyle;
use mara_core::pane::PaneBody;
use mara_core::pod::Pod;
use mara_core::vocab::{Color32, Id as MaraId};
use mara_core::widget::{TreeBody, TreeIconKind, TreeIconSlot};
use usd_bevy::UsdPrimRef;
use usd_bevy::route::meta::UsdDisplayName;

use super::{Outbox, PaneCtx, cid, pid, pod_response};
use crate::host::PANE_OUTLINER as P;
use crate::load::LoadedAsset;
use crate::viewer::commands::HostCommand;
use crate::viewer::state::{ActiveStage, ActiveVariants, LoaderTuning, SelectedPrim};
use crate::viewer::systems::Selection;

const MAX_MATERIAL_PODS: usize = 32;

struct PrimNode {
    entity: Entity,
    label: String,
    path: String,
    visible: bool,
    swatch: Option<Color32>,
    children: Vec<PrimNode>,
}

struct RootNode {
    entity: Entity,
    label: String,
    visible: bool,
    active: bool,
    prims: Vec<PrimNode>,
}

fn swatch_for(world: &World, entity: Entity, materials: &Assets<StandardMaterial>) -> Option<Color32> {
    let pick = |e: Entity| -> Option<Color32> {
        let handle = world.get::<MeshMaterial3d<StandardMaterial>>(e)?;
        let c = materials.get(&handle.0)?.base_color.to_srgba();
        Some(Color32::from_rgb(
            (c.red * 255.0) as u8,
            (c.green * 255.0) as u8,
            (c.blue * 255.0) as u8,
        ))
    };
    pick(entity).or_else(|| {
        world
            .get::<Children>(entity)?
            .iter()
            .find_map(pick)
    })
}

fn build_prim(world: &World, entity: Entity, materials: &Assets<StandardMaterial>) -> Option<PrimNode> {
    let e = world.get_entity(entity).ok()?;
    let prim = e.get::<UsdPrimRef>()?;
    let label = e
        .get::<UsdDisplayName>()
        .map(|d| d.0.clone())
        .or_else(|| e.get::<Name>().map(|n| n.as_str().to_string()))
        .unwrap_or_else(|| prim.path.rsplit('/').next().unwrap_or(&prim.path).to_string());
    let visible = !matches!(e.get::<Visibility>(), Some(Visibility::Hidden));
    let mut children: Vec<PrimNode> = e
        .get::<Children>()
        .map(|c| c.iter().filter_map(|c| build_prim(world, c, materials)).collect())
        .unwrap_or_default();
    children.sort_by(|a, b| a.path.cmp(&b.path));
    Some(PrimNode {
        entity,
        label,
        path: prim.path.clone(),
        visible,
        swatch: swatch_for(world, entity, materials),
        children,
    })
}

/// The prim sub-roots under a loaded root; a scene root may sit one level
/// above the first prim, so entities without a prim are descended once.
fn root_prims(world: &World, root: Entity, materials: &Assets<StandardMaterial>) -> Vec<PrimNode> {
    let mut prims = Vec::new();
    let Some(children) = world.get::<Children>(root) else {
        return prims;
    };
    for child in children.iter() {
        if world.get::<UsdPrimRef>(child).is_some() {
            prims.extend(build_prim(world, child, materials));
        } else if let Some(grand) = world.get::<Children>(child) {
            for g in grand.iter() {
                prims.extend(build_prim(world, g, materials));
            }
        }
    }
    prims.sort_by(|a, b| a.path.cmp(&b.path));
    prims
}

fn loaded_ancestor(world: &World, mut e: Entity) -> Option<Entity> {
    loop {
        if world.get::<LoadedAsset>(e).is_some() {
            return Some(e);
        }
        e = world.get::<ChildOf>(e)?.parent();
    }
}

fn passes(node: &PrimNode, filter: &str) -> bool {
    filter.is_empty()
        || node.label.to_lowercase().contains(filter)
        || node.path.to_lowercase().contains(filter)
        || node.children.iter().any(|c| passes(c, filter))
}

pub fn show(body: &mut PaneBody<'_, '_>, world: &mut World, ctx: &PaneCtx) {
    let accent = ctx.accent;
    let active = world.resource::<ActiveStage>().0;
    let selected_prim = world.resource::<SelectedPrim>().0;
    let selected_root = world.resource::<Selection>().0;

    let loaded: Vec<(Entity, String, bool)> = {
        let mut q = world.query::<(Entity, &LoadedAsset, Option<&Visibility>)>();
        q.iter(world)
            .map(|(entity, asset, vis)| {
                (
                    entity,
                    asset.label.clone(),
                    !matches!(vis, Some(Visibility::Hidden)),
                )
            })
            .collect()
    };
    let mut roots: Vec<RootNode> = {
        let materials = world.resource::<Assets<StandardMaterial>>();
        loaded
            .into_iter()
            .map(|(entity, label, visible)| RootNode {
                entity,
                label,
                visible,
                active: active == Some(entity),
                prims: root_prims(world, entity, materials),
            })
            .collect()
    };
    roots.sort_by(|a, b| a.label.cmp(&b.label));
    let prim_count = world.query::<&UsdPrimRef>().iter(world).count();

    body.add_normal(
        cid(P, "overview"),
        "Scene",
        "list",
        vec![
            Pod::new(pid(P, "overview", 0))
                .with_readout("loaded assets", roots.len().to_string())
                .with_readout("projected prims", prim_count.to_string())
                .with_readout(
                    "active",
                    roots
                        .iter()
                        .find(|r| r.active)
                        .map(|r| r.label.as_str())
                        .unwrap_or("None"),
                ),
        ],
    );

    let search_id = pid(P, "scene", 0);
    let filter = body.search_query(search_id, 0).to_lowercase();
    let tree_root = MaraId::new((P, "tree_root"));
    let outbox = ctx.outbox.clone();
    let selected_path = selected_prim
        .and_then(|e| world.get::<UsdPrimRef>(e))
        .map(|p| p.path.clone())
        .unwrap_or_default();
    body.add_normal(
        cid(P, "scene"),
        "Hierarchy",
        "cube-tree",
        vec![
            Pod::new(search_id)
                .with_separator(SeparatorStyle::Line)
                .with_search("filter by name / path…", accent),
            Pod::new(pid(P, "scene", 1))
                .with_separator(SeparatorStyle::Line)
                .fill()
                .with_tree(7, move |tree| {
                    if roots.is_empty() {
                        let mut none = false;
                        let mut slot = [TreeIconSlot::new(
                            TreeIconKind::Glyph { on: "·", off: "·" },
                            &mut none,
                        )];
                        tree.row("empty", 0, None, None, "Nothing loaded — add a USD in Selection", false, accent, &mut slot);
                        return;
                    }
                    for root in &roots {
                        walk_root(tree, tree_root, root, selected_root, selected_prim, accent, &filter, &outbox);
                    }
                }),
            Pod::new(pid(P, "scene", 2)).with_readout(
                "selected",
                if selected_path.is_empty() {
                    "—".to_string()
                } else {
                    selected_path
                },
            ),
        ],
    );

    // Materials of the active asset, one colour pod each.
    let mut material_ids: Vec<(AssetId<StandardMaterial>, String, [f32; 3])> = Vec::new();
    if let Some(root) = active {
        let handles: Vec<(Entity, AssetId<StandardMaterial>)> = {
            let mut q = world
                .query_filtered::<(Entity, &MeshMaterial3d<StandardMaterial>), With<UsdPrimRef>>();
            q.iter(world).map(|(e, h)| (e, h.0.id())).collect()
        };
        let asset_server = world.resource::<AssetServer>();
        let materials = world.resource::<Assets<StandardMaterial>>();
        for (e, id) in handles {
            if loaded_ancestor(world, e) != Some(root) {
                continue;
            }
            if material_ids.iter().any(|(m, _, _)| *m == id) {
                continue;
            }
            let Some(mat) = materials.get(id) else {
                continue;
            };
            let c = mat.base_color.to_linear();
            let label = asset_server
                .get_path(id)
                .map(|p| {
                    let s = p.to_string();
                    s.rsplit('/').next().unwrap_or(&s).to_string()
                })
                .unwrap_or_else(|| format!("material {}", material_ids.len() + 1));
            material_ids.push((id, label, [c.red, c.green, c.blue]));
            if material_ids.len() >= MAX_MATERIAL_PODS {
                break;
            }
        }
    }
    let materials_id = cid(P, "materials");
    if material_ids.is_empty() {
        body.add_normal(
            materials_id,
            "Materials",
            "color",
            vec![Pod::new(pid(P, "materials", 0)).with_readout("materials", "none on the active asset")],
        );
    } else {
        let mut pod = Pod::new(pid(P, "materials", 0));
        for (_, label, rgb) in &material_ids {
            pod = pod.with_color_rgb(label.clone(), rgb.map(|v| v.clamp(0.0, 1.0)), accent);
        }
        body.add_normal(materials_id, format!("Materials ({})", material_ids.len()), "color", vec![pod]);
    }

    // Variant sets of the active asset: one dropdown each.
    let variants = world.resource::<ActiveVariants>().clone();
    let tuning = world.resource::<LoaderTuning>();
    let variants_id = cid(P, "variants");
    let mut dropdown_entries: Vec<(String, String, Vec<String>)> = Vec::new();
    if variants.entries.is_empty() {
        body.add_normal(
            variants_id,
            "Variants",
            "branch",
            vec![Pod::new(pid(P, "variants", 0)).with_readout("variant sets", "none on the active asset")],
        );
    } else {
        let mut pod = Pod::new(pid(P, "variants", 0));
        for set in &variants.entries {
            let key = (set.prim.clone(), set.name.clone());
            let current = tuning
                .variants
                .get(&key)
                .cloned()
                .or_else(|| set.selection.clone())
                .unwrap_or_default();
            pod = pod.with_readout(format!("{} • {}", set.prim, set.name), if current.is_empty() { "(none)".to_string() } else { current.clone() });
            if !set.options.is_empty() {
                let initial = set.options.iter().position(|o| *o == current).unwrap_or(0);
                pod = pod.with_dropdown(set.options.clone(), initial, accent);
                dropdown_entries.push((set.prim.clone(), set.name.clone(), set.options.clone()));
            }
        }
        body.add_normal(variants_id, format!("Variants ({})", variants.entries.len()), "branch", vec![pod]);
    }

    let responses = body.render();
    if let Some(resp) = pod_response(&responses, materials_id, 0) {
        for (i, color) in resp.colors.iter().enumerate() {
            if color.changed
                && let Some((id, _, _)) = material_ids.get(i)
            {
                ctx.send(HostCommand::SetMaterialColor(
                    *id,
                    [color.rgba[0], color.rgba[1], color.rgba[2]],
                ));
            }
        }
    }
    if let (Some(root), Some(resp)) = (active, pod_response(&responses, variants_id, 0)) {
        for (i, dropdown) in resp.dropdowns.iter().enumerate() {
            if dropdown.changed
                && let Some((prim, set, options)) = dropdown_entries.get(i)
                && let Some(selection) = options.get(dropdown.selected)
            {
                ctx.send(HostCommand::SetVariant {
                    root,
                    prim: prim.clone(),
                    set: set.clone(),
                    selection: selection.clone(),
                });
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_root(
    tree: &mut TreeBody,
    tree_root: MaraId,
    root: &RootNode,
    selected_root: Option<Entity>,
    selected_prim: Option<Entity>,
    accent: Color32,
    filter: &str,
    outbox: &Outbox,
) {
    if !filter.is_empty() && !root.prims.iter().any(|p| passes(p, filter)) && !root.label.to_lowercase().contains(filter) {
        return;
    }
    let exp_key = tree_root.with(("root", root.entity.to_bits()));
    let mut expanded = tree.persisted_bool(exp_key).unwrap_or(true);
    let mut visible = root.visible;
    let mut delete = false;
    let mut slots = [
        TreeIconSlot::new(TreeIconKind::Eye, &mut visible).with_tooltip("Toggle visibility"),
        TreeIconSlot::new(TreeIconKind::Glyph { on: "🗑", off: "🗑" }, &mut delete)
            .with_tooltip("Remove from scene"),
    ];
    let resp = tree.row(
        root.entity.to_bits(),
        0,
        Some(&mut expanded),
        Some("cube-multiple"),
        &root.label,
        root.active || selected_root == Some(root.entity),
        accent,
        &mut slots,
    );
    tree.set_persisted_bool(exp_key, expanded);
    if resp.body.clicked() {
        outbox.push(HostCommand::SelectRoot(Some(root.entity)));
    }
    if resp.icons.get(1).is_some_and(|i| i.clicked()) {
        outbox.push(HostCommand::Despawn(root.entity));
    }
    if visible != root.visible {
        outbox.push(HostCommand::SetVisibility(root.entity, visible));
    }
    if !expanded {
        return;
    }
    if root.prims.is_empty() {
        let mut none = false;
        let mut slot = [TreeIconSlot::new(TreeIconKind::Glyph { on: "·", off: "·" }, &mut none)];
        tree.row(("loading", root.entity.to_bits()), 1, None, None, "(stage still loading…)", false, accent, &mut slot);
    }
    for prim in &root.prims {
        walk_prim(tree, tree_root, prim, 1, selected_prim, accent, filter, outbox);
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_prim(
    tree: &mut TreeBody,
    tree_root: MaraId,
    node: &PrimNode,
    depth: u32,
    selected_prim: Option<Entity>,
    accent: Color32,
    filter: &str,
    outbox: &Outbox,
) {
    if !passes(node, filter) {
        return;
    }
    let is_branch = !node.children.is_empty();
    let exp_key = tree_root.with(("exp", node.path.as_str()));
    let mut expanded = tree.persisted_bool(exp_key).unwrap_or(true) || !filter.is_empty();
    let mut visible = node.visible;
    let mut ignored = false;
    let mut slots: Vec<TreeIconSlot<'_>> = vec![
        TreeIconSlot::new(TreeIconKind::Eye, &mut visible).with_tooltip("Toggle visibility"),
    ];
    if let Some(swatch) = node.swatch {
        slots.push(TreeIconSlot::new(TreeIconKind::Color(swatch), &mut ignored));
    }
    let resp = tree.row(
        node.entity.to_bits(),
        depth,
        if is_branch { Some(&mut expanded) } else { None },
        Some("cube"),
        &node.label,
        selected_prim == Some(node.entity),
        accent,
        &mut slots,
    );
    if filter.is_empty() {
        tree.set_persisted_bool(exp_key, expanded);
    }
    if resp.body.double_clicked() {
        outbox.push(HostCommand::FitPrim(node.entity));
    } else if resp.body.clicked() {
        outbox.push(HostCommand::SelectPrim(Some(node.entity)));
        outbox.push(HostCommand::FlyToPrim(node.entity));
    }
    if visible != node.visible {
        outbox.push(HostCommand::SetVisibility(node.entity, visible));
    }
    if is_branch && expanded {
        for child in &node.children {
            walk_prim(tree, tree_root, child, depth + 1, selected_prim, accent, filter, outbox);
        }
    }
}
