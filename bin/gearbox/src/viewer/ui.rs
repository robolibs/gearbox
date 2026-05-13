//! Viewer UI for the gearbox simulator: bevy_frost ribbons + floating
//! panels + widgets. Adapted from `bevy_openusd::ui` for multi-USD
//! loading: the "stage" is whichever LoadedAsset entity is currently
//! `ActiveStage`, and the prim tree is a two-level hierarchy (top:
//! loaded assets; expand → prim sub-tree).

use bevy::asset::{AssetServer, Assets};
use bevy::ecs::hierarchy::Children;
use bevy::mesh::Mesh3d;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use bevy_frost::prelude::*;
use bevy_frost::style;
use bevy_glacial::{ChaseCamera, EnumSet, GizmoMode, GizmoOptions, GizmoTarget, SelectionRing};
use std::collections::HashMap;
use std::path::PathBuf;
use usd_bevy::{UsdAsset, UsdDisplayName, UsdKind, UsdPrimRef, UsdProcedural, UsdSpatialAudio};

use crate::controller::{
    CmdVel, ControllerCommands, ControllerInventory, ControllerKey, ControllerStates,
    ExternalControllerPolicy, ExternalControllerProcesses,
};
use crate::load::{LoadQueue, LoadedAsset, UsdAssetHandle};
use crate::viewer::overlays::DisplayToggles;
use crate::viewer::state::{
    ActiveStage, CameraBookmark, CameraBookmarks, CameraMount, FlyTo, LoadRequest, LoaderTuning,
    ReloadRequest, SelectedPrim, StageInfo, UsdStageTime,
};

// ─── Ribbon declaration ─────────────────────────────────────────────

pub const RIBBON_LEFT: &str = "viewer_left";

pub const RIB_SELECTION: &str = "viewer_selection";
pub const RIB_TREE: &str = "viewer_tree";
pub const RIB_INFO: &str = "viewer_info";
pub const RIB_CONTROLLERS: &str = "viewer_controllers";
pub const RIB_CAMERAS: &str = "viewer_cameras";
pub const RIB_OVERLAYS: &str = "viewer_overlays";
pub const RIB_TIMELINE: &str = "viewer_timeline";
pub const RIB_KEYS: &str = "viewer_keys";
pub const RIB_LOG: &str = "viewer_log";
pub const RIB_PLAY: &str = "viewer_play";

const RIBBONS: &[RibbonDef] = &[RibbonDef {
    id: RIBBON_LEFT,
    edge: RibbonEdge::Left,
    role: RibbonRole::Panel,
    mode: RibbonMode::ThreeSided,
    draggable: false,
    accepts: &[],
}];

const RIBBON_ITEMS: &[RibbonItem] = &[
    RibbonItem {
        id: RIB_SELECTION,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::Start,
        slot: 0,
        glyph: bevy_frost::RibbonGlyph::Text("F"),
        tooltip: "File / selection",
        child_ribbon: None,
        role: None,
    },
    RibbonItem {
        id: RIB_TREE,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::Start,
        slot: 1,
        glyph: bevy_frost::RibbonGlyph::Text("T"),
        tooltip: "Prim tree (T)",
        child_ribbon: None,
        role: None,
    },
    RibbonItem {
        id: RIB_INFO,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::Start,
        slot: 2,
        glyph: bevy_frost::RibbonGlyph::Text("i"),
        tooltip: "Stage info (I)",
        child_ribbon: None,
        role: None,
    },
    RibbonItem {
        id: RIB_CAMERAS,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::Start,
        slot: 3,
        glyph: bevy_frost::RibbonGlyph::Text("C"),
        tooltip: "Cameras",
        child_ribbon: None,
        role: None,
    },
    RibbonItem {
        id: RIB_CONTROLLERS,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::Middle,
        slot: 0,
        glyph: bevy_frost::RibbonGlyph::Text("⚙"),
        tooltip: "Machine controllers",
        child_ribbon: None,
        role: None,
    },
    RibbonItem {
        id: RIB_PLAY,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::Middle,
        slot: 1,
        glyph: bevy_frost::RibbonGlyph::Text("▶"),
        tooltip: "Play / pause physics",
        child_ribbon: None,
        role: Some(bevy_frost::RibbonRole::Icon),
    },
    RibbonItem {
        id: RIB_OVERLAYS,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::End,
        slot: 0,
        glyph: bevy_frost::RibbonGlyph::Text("O"),
        tooltip: "Overlays (O)",
        child_ribbon: None,
        role: None,
    },
    RibbonItem {
        id: RIB_TIMELINE,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::End,
        slot: 1,
        glyph: bevy_frost::RibbonGlyph::Text("⏱"),
        tooltip: "Timeline",
        child_ribbon: None,
        role: None,
    },
    RibbonItem {
        id: RIB_KEYS,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::End,
        slot: 2,
        glyph: bevy_frost::RibbonGlyph::Text("?"),
        tooltip: "Controls (?)",
        child_ribbon: None,
        role: None,
    },
    RibbonItem {
        id: RIB_LOG,
        ribbon: RIBBON_LEFT,
        cluster: RibbonCluster::End,
        slot: 3,
        glyph: bevy_frost::RibbonGlyph::Text("📜"),
        tooltip: "Log",
        child_ribbon: None,
        role: None,
    },
];

/// Tree expansion state for both top-level (LoadedAsset entity bits)
/// and prim paths. Keyed by `String` so prim paths and asset labels
/// share the same map without collisions (prim paths start with `/`,
/// asset keys with `@`).
#[derive(Resource, Default)]
pub struct TreeExpanded(pub HashMap<String, bool>);

#[derive(Resource, Default)]
pub struct TreeFilter(pub String);

#[derive(Resource, Default)]
pub struct ViewerCommandPalette(pub CommandPaletteState);

/// Top-level entity selection (a LoadedAsset row). Drives the
/// transform gizmo. Separate from `SelectedPrim` which targets prims
/// inside the active stage.
#[derive(Resource, Default)]
pub struct Selection(pub Option<Entity>);

/// Queue of LoadedAsset entities to despawn (with rapier-body
/// cleanup). Drained each frame.
#[derive(Resource, Default)]
pub struct PendingDespawn(pub Vec<Entity>);

const PALETTE_ITEMS: &[PaletteItem] = &[
    PaletteItem {
        id: "open_selection",
        label: "Open: Selection panel",
        hint: Some("F"),
    },
    PaletteItem {
        id: "open_tree",
        label: "Open: Prim tree",
        hint: Some("T"),
    },
    PaletteItem {
        id: "open_info",
        label: "Open: Stage info",
        hint: Some("I"),
    },
    PaletteItem {
        id: "open_cameras",
        label: "Open: Cameras",
        hint: None,
    },
    PaletteItem {
        id: "open_controllers",
        label: "Open: Machine controllers",
        hint: None,
    },
    PaletteItem {
        id: "open_overlays",
        label: "Open: Overlays",
        hint: Some("O"),
    },
    PaletteItem {
        id: "open_timeline",
        label: "Open: Timeline",
        hint: None,
    },
    PaletteItem {
        id: "open_keys",
        label: "Open: Controls",
        hint: Some("?"),
    },
    PaletteItem {
        id: "open_log",
        label: "Open: Log",
        hint: None,
    },
    PaletteItem {
        id: "toggle_grid",
        label: "Toggle: Ground grid",
        hint: Some("G"),
    },
    PaletteItem {
        id: "toggle_axes",
        label: "Toggle: World axes",
        hint: Some("X"),
    },
    PaletteItem {
        id: "toggle_markers",
        label: "Toggle: Prim markers",
        hint: Some("P"),
    },
    PaletteItem {
        id: "toggle_wireframe",
        label: "Toggle: Wireframe",
        hint: None,
    },
    PaletteItem {
        id: "reload_stage",
        label: "Stage: Reload",
        hint: Some("R"),
    },
    PaletteItem {
        id: "browse_usd",
        label: "Stage: Add USD…",
        hint: None,
    },
];

// ─── Plugin ─────────────────────────────────────────────────────────

pub struct ViewerUiPlugin;

impl Plugin for ViewerUiPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<bevy_frost::FrostPlugin>() {
            app.add_plugins(bevy_frost::FrostPlugin);
        }
        app.init_resource::<TreeExpanded>()
            .init_resource::<TreeFilter>()
            .init_resource::<ViewerCommandPalette>()
            .init_resource::<Selection>()
            .init_resource::<PendingDespawn>()
            .init_resource::<ActiveStage>()
            .init_resource::<StageInfo>()
            .init_resource::<ReloadRequest>()
            .init_resource::<LoadRequest>()
            .init_resource::<SelectedPrim>()
            .init_resource::<FlyTo>()
            .init_resource::<CameraMount>()
            .init_resource::<LoaderTuning>()
            .init_resource::<UsdStageTime>()
            .init_resource::<CameraBookmarks>()
            .add_systems(Startup, open_default_panel)
            .add_systems(
                Update,
                (
                    pick_on_click,
                    rebase_loaded_assets_on_pause,
                    gate_gizmo_on_play,
                    sync_gizmo_target,
                    drive_selection_ring,
                    drain_despawn,
                    auto_set_active_stage,
                    capture_active_stage_info,
                    apply_load_request,
                    apply_reload_request,
                    apply_fly_to,
                    draw_selected_prim_highlight,
                    tick_stage_time,
                )
                    .chain(),
            )
            .add_systems(
                EguiPrimaryContextPass,
                (
                    draw_ribbons,
                    draw_selection_panel,
                    draw_tree_panel,
                    draw_info_panel,
                    draw_controllers_panel,
                    draw_cameras_panel,
                    draw_overlays_panel,
                    draw_timeline_panel,
                    draw_keys_panel,
                    draw_log_panel,
                    draw_palette_panel,
                )
                    .chain(),
            );
    }
}

const PANEL_W: f32 = 340.0;
const PANEL_H: f32 = 560.0;

fn open_default_panel(mut ribbon: ResMut<bevy_frost::RibbonOpen>) {
    ribbon.toggle(RIBBON_LEFT, RIB_TREE);
}

// ─── Selection ↔ ActiveStage helpers ──────────────────────────────

/// Default the active stage to the most-recently spawned LoadedAsset
/// when nothing is selected yet. Runs every frame; cheap because it
/// no-ops once `ActiveStage(Some(_))` is set.
fn auto_set_active_stage(
    mut active: ResMut<ActiveStage>,
    loaded: Query<Entity, With<LoadedAsset>>,
) {
    if active.0.is_some() {
        // Validate: clear if the entity was despawned.
        if let Some(e) = active.0
            && loaded.get(e).is_err()
        {
            active.0 = None;
        }
        return;
    }
    if let Some(e) = loaded.iter().last() {
        active.0 = Some(e);
    }
}

/// Latch `StageInfo` from the active stage's `UsdAsset` when the
/// asset finishes loading or when the active stage changes.
fn capture_active_stage_info(
    active: Res<ActiveStage>,
    handles: Query<(&UsdAssetHandle, &LoadedAsset)>,
    usd_assets: Res<Assets<UsdAsset>>,
    mut info: ResMut<StageInfo>,
    mut last_active: Local<Option<Entity>>,
    mut last_was_loaded: Local<bool>,
) {
    let Some(entity) = active.0 else {
        if last_active.is_some() {
            *info = StageInfo::default();
            *last_active = None;
            *last_was_loaded = false;
        }
        return;
    };
    let Ok((handle, la)) = handles.get(entity) else {
        return;
    };
    let active_changed = *last_active != Some(entity);
    let asset = usd_assets.get(&handle.0);
    let now_loaded = asset.is_some();
    if !active_changed && *last_was_loaded == now_loaded {
        return;
    }
    *last_active = Some(entity);
    *last_was_loaded = now_loaded;

    info.path = la.path.display().to_string();
    if let Some(asset) = asset {
        info.default_prim = asset.default_prim.clone();
        info.layer_count = asset.layer_count;
        info.variant_count = asset.variants.values().map(|sets| sets.len()).sum();
        info.lights_directional = asset.light_tally.directional;
        info.lights_point = asset.light_tally.point;
        info.lights_spot = asset.light_tally.spot;
        info.lights_dome = asset.light_tally.dome;
        info.instance_prim_count = asset.instance_prim_count;
        info.instance_prototype_reuses = asset.instance_prototype_reuses;
        info.animated_prim_count = asset.animated_prims.len();
        info.skeleton_count = asset.skeletons.len();
        info.skel_root_count = asset.skel_roots.len();
        info.skel_binding_count = asset.skel_bindings.len();
        info.render_settings_count = asset.render_settings.len();
        info.render_product_count = asset.render_products.len();
        info.render_var_count = asset.render_vars.len();
        let primary = asset.render_settings.first();
        info.render_primary_resolution = primary.and_then(|s| s.resolution);
        info.render_primary_path = primary.map(|s| s.path.clone());
        info.rigid_body_count = asset.rigid_body_prims.len();
        info.physics_scene_count = asset.physics_scene_prims.len();
        info.joint_count = asset.joints.len();
        info.custom_attr_prim_count = asset.custom_attrs.len();
        info.custom_layer_data_entries = asset.custom_layer_data.len();
        info.subdivision_prim_count = asset.subdivision_prims.len();
        info.light_linked_count = asset.light_linking_prims.len();
        info.clip_prim_count = asset.clip_sets.len();
    }
}

/// Resolve the active stage's `UsdAsset` from its `Handle<UsdAsset>`
/// component. Returns the LoadedAsset's path label too (handy for
/// panel headers).
fn active_asset<'a>(
    active: &ActiveStage,
    handles: &Query<(&UsdAssetHandle, &LoadedAsset)>,
    usd_assets: &'a Assets<UsdAsset>,
) -> Option<&'a UsdAsset> {
    let entity = active.0?;
    let (handle, _) = handles.get(entity).ok()?;
    usd_assets.get(&handle.0)
}

// ─── Click in viewport → set Selection (top-level) ─────────────────

fn pick_on_click(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    loaded: Query<Entity, With<LoadedAsset>>,
    parents: Query<&ChildOf>,
    aabbs: Query<(Entity, &GlobalTransform, &bevy::camera::primitives::Aabb)>,
    mut contexts: EguiContexts,
    mut selection: ResMut<Selection>,
    mut active: ResMut<ActiveStage>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        selection.0 = None;
    }
    if !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    if buttons.pressed(MouseButton::Right) {
        return;
    }
    let over_ui = contexts
        .ctx_mut()
        .map(|c| c.wants_pointer_input())
        .unwrap_or(false);
    if over_ui {
        return;
    }
    let Ok(window) = windows.single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let Ok((camera, cam_tr)) = cameras.single() else {
        return;
    };
    let Ok(ray) = camera.viewport_to_world(cam_tr, cursor) else {
        return;
    };
    let origin = ray.origin;
    let dir = *ray.direction;
    // Ray-vs-world-AABB across every mesh entity in the scene; the
    // closest hit's owning LoadedAsset wins. Catches geometry that
    // sits offset from the LoadedAsset root entity, which a fixed
    // sphere around the root translation would miss.
    let mut best: Option<(Entity, f32)> = None;
    for (e, gt, aabb) in aabbs.iter() {
        let Some(root) = find_loaded_ancestor(e, &parents, &loaded) else {
            continue;
        };
        if let Some(t) = ray_aabb_world(origin, dir, gt, aabb)
            && best.map_or(true, |(_, bt)| t < bt)
        {
            best = Some((root, t));
        }
    }
    if let Some((root, t)) = best {
        info!("pick: hit LoadedAsset {root:?} at t={t:.3}");
        selection.0 = Some(root);
        active.0 = Some(root);
    } else {
        info!(
            "pick: no LoadedAsset hit (loaded_count={}, aabb_count={})",
            loaded.iter().count(),
            aabbs.iter().count()
        );
    }
}

/// Walk up `ChildOf` parents until we find an entity carrying
/// `LoadedAsset`, or run out of ancestors.
fn find_loaded_ancestor(
    mut e: Entity,
    parents: &Query<&ChildOf>,
    loaded: &Query<Entity, With<LoadedAsset>>,
) -> Option<Entity> {
    loop {
        if loaded.get(e).is_ok() {
            return Some(e);
        }
        match parents.get(e) {
            Ok(c) => e = c.parent(),
            Err(_) => return None,
        }
    }
}

/// Project the local AABB to a world-space AABB by transforming all
/// 8 corners through `gt`, then slab-test against the ray. Loose for
/// rotated transforms but plenty accurate for click-to-select.
fn ray_aabb_world(
    origin: Vec3,
    dir: Vec3,
    gt: &GlobalTransform,
    aabb: &bevy::camera::primitives::Aabb,
) -> Option<f32> {
    let m = gt.to_matrix();
    let centre = Vec3::from(aabb.center);
    let half = Vec3::from(aabb.half_extents);
    let mut wmin = Vec3::splat(f32::INFINITY);
    let mut wmax = Vec3::splat(f32::NEG_INFINITY);
    for i in 0..8 {
        let local = Vec3::new(
            if i & 1 == 0 {
                centre.x - half.x
            } else {
                centre.x + half.x
            },
            if i & 2 == 0 {
                centre.y - half.y
            } else {
                centre.y + half.y
            },
            if i & 4 == 0 {
                centre.z - half.z
            } else {
                centre.z + half.z
            },
        );
        let w = m.transform_point3(local);
        wmin = wmin.min(w);
        wmax = wmax.max(w);
    }
    // Slab test. Inverse direction guarded against zero components.
    let inv = Vec3::new(
        if dir.x.abs() > 1e-8 {
            1.0 / dir.x
        } else {
            f32::INFINITY
        },
        if dir.y.abs() > 1e-8 {
            1.0 / dir.y
        } else {
            f32::INFINITY
        },
        if dir.z.abs() > 1e-8 {
            1.0 / dir.z
        } else {
            f32::INFINITY
        },
    );
    let t1 = (wmin - origin) * inv;
    let t2 = (wmax - origin) * inv;
    let tmin = t1.min(t2).max_element();
    let tmax = t1.max(t2).min_element();
    if tmax < 0.0 || tmin > tmax {
        None
    } else if tmin >= 0.0 {
        Some(tmin)
    } else {
        Some(tmax)
    }
}

fn sync_gizmo_target(
    selection: Res<Selection>,
    physics: Res<usd_bevy::physics::PhysicsActive>,
    mut commands: Commands,
    targets: Query<Entity, With<GizmoTarget>>,
    loaded: Query<(), With<LoadedAsset>>,
) {
    // While physics is playing, the gizmo is hidden and the user
    // shouldn't be moving things by hand. Detach any live targets and
    // bail before re-attaching.
    if physics.0 {
        for e in targets.iter() {
            commands.entity(e).remove::<GizmoTarget>();
        }
        return;
    }
    if !selection.is_changed() && targets.iter().count() == (selection.0.is_some() as usize) {
        return;
    }
    let want = selection.0;
    let mut already_correct = false;
    for e in targets.iter() {
        if Some(e) == want {
            already_correct = true;
        } else {
            commands.entity(e).remove::<GizmoTarget>();
        }
    }
    if let Some(e) = want
        && !already_correct
        && loaded.get(e).is_ok()
    {
        commands.entity(e).insert(GizmoTarget::default());
    }
}

/// During play, the gizmo is hidden — so the selection ring is the
/// only visual cue for "this asset is selected". Position the ring at
/// the asset's footprint on the ground tangent plane (`y = 0`) and
/// size it from the asset's world-AABB radius. Mirrors the old
/// `gearbox-editor::selection_ring::update_selection_ring` rule:
/// edit-mode hides the ring (gizmo handles take over).
fn drive_selection_ring(
    selection: Res<Selection>,
    physics: Res<usd_bevy::physics::PhysicsActive>,
    accent: Res<bevy_frost::prelude::AccentColor>,
    parents: Query<&ChildOf>,
    loaded: Query<Entity, With<LoadedAsset>>,
    aabbs: Query<(Entity, &GlobalTransform, &bevy::camera::primitives::Aabb)>,
    mut ring: ResMut<SelectionRing>,
) {
    if !physics.0 {
        ring.anchor = None;
        return;
    }
    let Some(root) = selection.0 else {
        ring.anchor = None;
        return;
    };
    // Anchor the ring at the **world AABB centroid** of the asset's
    // mesh subtree, not the root entity's translation — rapier writes
    // poses onto the descendant prims, not the root, so during play
    // the root stays at its mount point even though the visual robot
    // has driven away. The centroid follows wherever the meshes are.
    let mut wmin = Vec3::splat(f32::INFINITY);
    let mut wmax = Vec3::splat(f32::NEG_INFINITY);
    for (e, gt, aabb) in aabbs.iter() {
        if find_loaded_ancestor(e, &parents, &loaded) != Some(root) {
            continue;
        }
        let m = gt.to_matrix();
        let c = Vec3::from(aabb.center);
        let h = Vec3::from(aabb.half_extents);
        for i in 0..8 {
            let local = Vec3::new(
                if i & 1 == 0 { c.x - h.x } else { c.x + h.x },
                if i & 2 == 0 { c.y - h.y } else { c.y + h.y },
                if i & 4 == 0 { c.z - h.z } else { c.z + h.z },
            );
            let w = m.transform_point3(local);
            wmin = wmin.min(w);
            wmax = wmax.max(w);
        }
    }
    if wmin.x.is_infinite() {
        ring.anchor = None;
        return;
    }
    let centroid = (wmin + wmax) * 0.5;
    let half = (wmax - wmin) * 0.5;
    let outer = (half.x.max(half.z) + 0.3).max(1.0);
    ring.anchor = Some(Vec3::new(centroid.x, 0.05, centroid.z));
    ring.outer_radius = outer;
    let c = accent.0;
    ring.color = Color::srgba(
        c.r() as f32 / 255.0,
        c.g() as f32 / 255.0,
        c.b() as f32 / 255.0,
        c.a() as f32 / 255.0,
    );
}

/// On the ON→OFF edge of `PhysicsActive`, rebase each LoadedAsset's
/// root entity to its current world AABB centroid (XZ only; Y stays
/// at 0 / ground), and compensate the immediate children by `-Δ` so
/// the visuals don't jump. Without this, the gizmo (which targets
/// the root) sits at the asset's original mount point even though
/// the simulated robot drove away during play.
fn rebase_loaded_assets_on_pause(
    active: Res<usd_bevy::physics::PhysicsActive>,
    mut prev: Local<bool>,
    parents: Query<&ChildOf>,
    loaded: Query<Entity, With<LoadedAsset>>,
    children_q: Query<&Children>,
    aabbs: Query<(Entity, &GlobalTransform, &bevy::camera::primitives::Aabb)>,
    mut transforms: Query<&mut Transform>,
    gt_q: Query<&GlobalTransform>,
) {
    let was = *prev;
    *prev = active.0;
    if active.0 || !was {
        return;
    }
    let roots: Vec<Entity> = loaded.iter().collect();
    for root in roots {
        let mut wmin = Vec3::splat(f32::INFINITY);
        let mut wmax = Vec3::splat(f32::NEG_INFINITY);
        for (e, gt, aabb) in aabbs.iter() {
            if find_loaded_ancestor(e, &parents, &loaded) != Some(root) {
                continue;
            }
            let m = gt.to_matrix();
            let c = Vec3::from(aabb.center);
            let h = Vec3::from(aabb.half_extents);
            for i in 0..8 {
                let local = Vec3::new(
                    if i & 1 == 0 { c.x - h.x } else { c.x + h.x },
                    if i & 2 == 0 { c.y - h.y } else { c.y + h.y },
                    if i & 4 == 0 { c.z - h.z } else { c.z + h.z },
                );
                let w = m.transform_point3(local);
                wmin = wmin.min(w);
                wmax = wmax.max(w);
            }
        }
        if wmin.x.is_infinite() {
            continue;
        }
        let centroid = (wmin + wmax) * 0.5;
        let Ok(root_gt) = gt_q.get(root) else {
            continue;
        };
        let old_root_world = root_gt.translation();
        // Keep root Y at 0 — we want the gizmo on the ground plane,
        // not floating up at the AABB midpoint.
        let new_root_world = Vec3::new(centroid.x, 0.0, centroid.z);
        let delta = new_root_world - old_root_world;
        if delta.length_squared() < 1e-8 {
            continue;
        }
        let immediate: Vec<Entity> = children_q
            .get(root)
            .map(|c| c.iter().collect())
            .unwrap_or_default();
        if let Ok(mut tr) = transforms.get_mut(root) {
            tr.translation += delta;
        }
        // Compensate so descendants stay at their current world poses.
        // Assumes the root has identity rotation, which our load.rs
        // pipeline guarantees on initial spawn.
        for child in immediate {
            if let Ok(mut tr) = transforms.get_mut(child) {
                tr.translation -= delta;
            }
        }
    }
}

/// While physics is active, clear `GizmoOptions.gizmo_modes` so the
/// transform-gizmo-bevy crate draws nothing and accepts no drags. On
/// pause, restore the full mode set so handles reappear. Mirrors the
/// old `gearbox-editor::transform_gizmos::apply_gizmo_options` rule.
fn gate_gizmo_on_play(
    physics: Res<usd_bevy::physics::PhysicsActive>,
    mut options: ResMut<GizmoOptions>,
) {
    let want: EnumSet<GizmoMode> = if physics.0 {
        EnumSet::empty()
    } else {
        GizmoMode::all()
    };
    if options.gizmo_modes != want {
        options.gizmo_modes = want;
    }
}

fn drain_despawn(
    mut commands: Commands,
    mut queue: ResMut<PendingDespawn>,
    world: ResMut<usd_bevy::physics::PhysicsWorld>,
    mut selection: ResMut<Selection>,
    mut active: ResMut<ActiveStage>,
    children_q: Query<&Children>,
) {
    if queue.0.is_empty() {
        return;
    }
    let world = world.into_inner();
    for root in queue.0.drain(..) {
        if selection.0 == Some(root) {
            selection.0 = None;
        }
        if active.0 == Some(root) {
            active.0 = None;
        }
        let mut stack = vec![root];
        while let Some(e) = stack.pop() {
            if let Some(handle) = world.entity_to_body.remove(&e) {
                let _ = world.bodies.remove(
                    handle,
                    &mut world.islands,
                    &mut world.colliders,
                    &mut world.impulse_joints,
                    &mut world.multibody_joints,
                    true,
                );
            }
            if let Some(coll) = world.entity_to_collider.remove(&e) {
                world
                    .colliders
                    .remove(coll, &mut world.islands, &mut world.bodies, false);
            }
            if let Ok(cs) = children_q.get(e) {
                stack.extend(cs.iter());
            }
        }
        commands.entity(root).despawn();
    }
}

// ─── Reload + Browse application ───────────────────────────────────

fn apply_load_request(mut req: ResMut<LoadRequest>, mut queue: ResMut<LoadQueue>) {
    let Some(path) = req.path.take() else {
        return;
    };
    queue.0.push(path);
}

/// Hot-reload: despawn the active LoadedAsset (with rapier cleanup)
/// and re-push its path through `LoadQueue` so the loader pipeline
/// re-runs and remounts a fresh entity.
fn apply_reload_request(
    mut reload: ResMut<ReloadRequest>,
    active: Res<ActiveStage>,
    loaded: Query<&LoadedAsset>,
    mut despawn: ResMut<PendingDespawn>,
    mut queue: ResMut<LoadQueue>,
) {
    if !reload.requested {
        return;
    }
    reload.requested = false;
    let Some(entity) = active.0 else {
        return;
    };
    let Ok(la) = loaded.get(entity) else {
        return;
    };
    let path = la.path.clone();
    despawn.0.push(entity);
    queue.0.push(path);
}

// ─── FlyTo tween (against ChaseCamera) ─────────────────────────────

fn apply_fly_to(time: Res<Time>, mut fly: ResMut<FlyTo>, mut cameras: Query<&mut ChaseCamera>) {
    if fly.remaining <= 0.0 {
        return;
    }
    let Ok(mut cam) = cameras.single_mut() else {
        return;
    };
    let dt = time.delta_secs().min(1.0 / 30.0);
    fly.remaining = (fly.remaining - dt).max(0.0);
    let progress = if fly.duration > 0.0 {
        1.0 - (fly.remaining / fly.duration).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let eased = 1.0 - ((1.0 - progress) * core::f32::consts::FRAC_PI_2).cos();

    cam.focus = fly.start_focus.lerp(fly.target_focus, eased);
    cam.distance = fly
        .start_distance
        .lerp(fly.target_distance, eased)
        .max(cam.min_distance);

    if let (Some(sy), Some(ty)) = (fly.start_yaw, fly.target_yaw) {
        cam.yaw = lerp_angle(sy, ty, eased);
    }
    if let (Some(se), Some(te)) = (fly.start_elevation, fly.target_elevation) {
        cam.elevation = se + (te - se) * eased;
    }
}

fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let two_pi = core::f32::consts::TAU;
    let mut delta = (b - a) % two_pi;
    if delta > core::f32::consts::PI {
        delta -= two_pi;
    } else if delta < -core::f32::consts::PI {
        delta += two_pi;
    }
    a + delta * t
}

// ─── Selected-prim AABB highlight ──────────────────────────────────

fn draw_selected_prim_highlight(
    selected: Res<SelectedPrim>,
    xforms: Query<&GlobalTransform>,
    aabbs: Query<&bevy::camera::primitives::Aabb>,
    mut gizmos: Gizmos,
) {
    let Some(entity) = selected.0 else {
        return;
    };
    let Ok(gt) = xforms.get(entity) else {
        return;
    };
    let origin = gt.translation();
    let color = Color::srgb(1.0, 0.9, 0.2);

    if let Ok(aabb) = aabbs.get(entity) {
        let half = Vec3::new(
            aabb.half_extents.x,
            aabb.half_extents.y,
            aabb.half_extents.z,
        );
        let centre_local = Vec3::new(aabb.center.x, aabb.center.y, aabb.center.z);
        let iso = gt.compute_transform();
        let corners = [
            Vec3::new(-half.x, -half.y, -half.z),
            Vec3::new(half.x, -half.y, -half.z),
            Vec3::new(half.x, half.y, -half.z),
            Vec3::new(-half.x, half.y, -half.z),
            Vec3::new(-half.x, -half.y, half.z),
            Vec3::new(half.x, -half.y, half.z),
            Vec3::new(half.x, half.y, half.z),
            Vec3::new(-half.x, half.y, half.z),
        ];
        let worldify = |v: Vec3| iso.translation + iso.rotation * ((v + centre_local) * iso.scale);
        let c: [Vec3; 8] = std::array::from_fn(|i| worldify(corners[i]));
        let edges = [
            (0, 1),
            (1, 2),
            (2, 3),
            (3, 0),
            (4, 5),
            (5, 6),
            (6, 7),
            (7, 4),
            (0, 4),
            (1, 5),
            (2, 6),
            (3, 7),
        ];
        for (a, b) in edges {
            gizmos.line(c[a], c[b], color);
        }
    } else {
        let l = 0.2;
        gizmos.line(origin - Vec3::X * l, origin + Vec3::X * l, color);
        gizmos.line(origin - Vec3::Y * l, origin + Vec3::Y * l, color);
        gizmos.line(origin - Vec3::Z * l, origin + Vec3::Z * l, color);
    }
}

// ─── Stage time clock ──────────────────────────────────────────────

fn tick_stage_time(
    time: Res<Time>,
    mut clock: ResMut<UsdStageTime>,
    active: Res<ActiveStage>,
    handles: Query<(&UsdAssetHandle, &LoadedAsset)>,
    usd_assets: Res<Assets<UsdAsset>>,
) {
    if !clock.initialized
        && let Some(asset) = active_asset(&active, &handles, &usd_assets)
    {
        clock.start_time_code = asset.start_time_code;
        clock.end_time_code = asset.end_time_code;
        clock.time_codes_per_second = asset.time_codes_per_second;
        clock.seconds = 0.0;
        clock.playing = asset.animated_prims.iter().next().is_some()
            || !asset.skel_animations.is_empty()
            || asset.end_time_code > asset.start_time_code;
        clock.initialized = true;
    }
    if clock.playing {
        clock.seconds += time.delta_secs_f64();
        let dur = clock.duration_seconds();
        if dur > 0.0 && clock.seconds >= dur {
            clock.seconds = clock.seconds.rem_euclid(dur);
        }
    }
}

// ─── Ribbon rail ────────────────────────────────────────────────────

fn draw_ribbons(
    mut contexts: EguiContexts,
    accent: Res<AccentColor>,
    mut open: ResMut<RibbonOpen>,
    mut placement: ResMut<RibbonPlacement>,
    mut drag: ResMut<RibbonDrag>,
    mut physics: ResMut<usd_bevy::physics::PhysicsActive>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let physics_on = physics.0;
    let clicks = draw_assembly(
        ctx,
        accent.0,
        RIBBONS,
        RIBBON_ITEMS,
        &mut open,
        &mut placement,
        &mut drag,
        |id| id == RIB_PLAY && physics_on,
    );
    for click in clicks {
        if click.item == RIB_PLAY {
            physics.0 = !physics.0;
        }
    }
}

fn is_panel_open(open: &RibbonOpen, item: &'static str) -> bool {
    open.is_open(RIBBON_LEFT, item)
}

// ─── Selection panel ────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn draw_selection_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    info: Res<StageInfo>,
    mut load_req: ResMut<LoadRequest>,
    mut selected: ResMut<SelectedPrim>,
    selection: Res<Selection>,
    loaded: Query<(&LoadedAsset, &GlobalTransform)>,
    prims: Query<(Entity, &Name, &UsdPrimRef)>,
    mesh_q: Query<(), With<Mesh3d>>,
    kind_q: Query<&UsdKind>,
    audio_q: Query<&UsdSpatialAudio>,
    proc_q: Query<&UsdProcedural>,
    vis_q: Query<&Visibility>,
    children: Query<&Children>,
) {
    if !is_panel_open(&open, RIB_SELECTION) {
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
        RIB_SELECTION,
        "Selection",
        egui::vec2(PANEL_W, PANEL_H + 80.0),
        &mut keep,
        accent_col,
        |pane| {
            pane.section("sel_stage", "Active stage", true, |ui| {
                readout_row(ui, "file", info.path.as_str());
                if wide_button(ui, "➕  Add USD…", accent_col).clicked()
                    && let Some(picked) = rfd::FileDialog::new()
                        .add_filter("USD stages", &["usda", "usdc", "usd", "usdz"])
                        .pick_file()
                {
                    load_req.path = Some(PathBuf::from(picked));
                }
                if !info.path.is_empty()
                    && wide_button(ui, "🗂  Reveal in filesystem", accent_col).clicked()
                {
                    let p = std::path::Path::new(&info.path);
                    let target = p.parent().unwrap_or(p);
                    let _ = std::process::Command::new("xdg-open").arg(target).spawn();
                }
            });

            pane.section("sel_entity", "Selected asset", true, |ui| {
                match selection.0.and_then(|e| loaded.get(e).ok()) {
                    Some((la, gt)) => {
                        let t = gt.compute_transform();
                        readout_row(ui, "label", &la.label);
                        readout_row(ui, "path", &la.path.display().to_string());
                        ui.add_space(style::space::TIGHT);
                        readout_row(ui, "X", &format!("{:+.3} m", t.translation.x));
                        readout_row(ui, "Y", &format!("{:+.3} m", t.translation.y));
                        readout_row(ui, "Z", &format!("{:+.3} m", t.translation.z));
                        sub_caption(ui, "Drag the gizmo handles to translate / rotate / scale.");
                    }
                    None => sub_caption(ui, "Click a loaded asset in the viewport."),
                }
            });

            pane.section("sel_prim", "Selected prim", true, |ui| match selected.0 {
                Some(entity) => {
                    if let Ok((_, n, pr)) = prims.get(entity) {
                        readout_row(ui, "name", n.as_str());
                        readout_row(ui, "path", pr.path.as_str());
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 3.0;
                            if mesh_q.get(entity).is_ok() {
                                chip(ui, "mesh", accent_col);
                            }
                            if let Ok(k) = kind_q.get(entity) {
                                chip(ui, &format!("kind:{}", k.kind), accent_col);
                            }
                            if children.get(entity).map(|c| !c.is_empty()).unwrap_or(false) {
                                chip(ui, "parent", accent_col);
                            }
                            if audio_q.get(entity).is_ok() {
                                chip(ui, "audio", accent_col);
                            }
                            if proc_q.get(entity).is_ok() {
                                chip(ui, "procedural", accent_col);
                            }
                            if matches!(vis_q.get(entity), Ok(Visibility::Hidden)) {
                                chip_colored(ui, "hidden", style::WARNING, accent_col);
                            }
                        });
                        if wide_button(ui, "Clear selection", accent_col).clicked() {
                            selected.0 = None;
                        }
                    } else {
                        sub_caption(ui, "(selection stale)");
                        selected.0 = None;
                    }
                }
                None => sub_caption(ui, "Click a prim in the Tree panel"),
            });
        },
    );
}

// ─── Prim-tree panel — TWO-LEVEL ────────────────────────────────────

#[derive(bevy::ecs::system::SystemParam)]
pub struct TreeParams<'w, 's> {
    pub loaded: Query<'w, 's, (Entity, &'static LoadedAsset)>,
    pub prims: Query<
        'w,
        's,
        (
            Entity,
            &'static Name,
            &'static UsdPrimRef,
            Option<&'static UsdDisplayName>,
        ),
    >,
    pub mat_q: Query<'w, 's, &'static MeshMaterial3d<StandardMaterial>>,
    pub mesh_mats:
        Query<'w, 's, (Entity, &'static MeshMaterial3d<StandardMaterial>), With<UsdPrimRef>>,
    pub handles: Query<'w, 's, (&'static UsdAssetHandle, &'static LoadedAsset)>,
    pub visibility_q: Query<'w, 's, (Entity, &'static mut Visibility)>,
    pub children: Query<'w, 's, &'static Children>,
    pub gt_query: Query<'w, 's, &'static GlobalTransform>,
    pub extent_q: Query<'w, 's, &'static usd_bevy::UsdLocalExtent>,
    pub parents: Query<'w, 's, &'static ChildOf>,
    pub loaded_only: Query<'w, 's, Entity, With<LoadedAsset>>,
    pub materials_assets: ResMut<'w, Assets<StandardMaterial>>,
    pub cameras: Query<'w, 's, &'static ChaseCamera>,
}

#[allow(clippy::too_many_arguments)]
fn draw_tree_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    mut selection: ResMut<Selection>,
    mut active: ResMut<ActiveStage>,
    mut selected: ResMut<SelectedPrim>,
    mut fly: ResMut<FlyTo>,
    mut expanded: ResMut<TreeExpanded>,
    mut filter: ResMut<TreeFilter>,
    mut despawn: ResMut<PendingDespawn>,
    usd_assets: Res<Assets<UsdAsset>>,
    asset_server: Res<AssetServer>,
    mut loader_tuning: ResMut<LoaderTuning>,
    mut reload: ResMut<ReloadRequest>,
    mut params: TreeParams,
) {
    if !is_panel_open(&open, RIB_TREE) {
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
        RIB_TREE,
        "Prim tree",
        egui::vec2(PANEL_W, 720.0),
        &mut keep,
        accent_col,
        |pane| {
            pane.section("tree_hierarchy", "Hierarchy", true, |ui| {
                let mut roots: Vec<(Entity, String)> = params
                    .loaded
                    .iter()
                    .map(|(e, la)| (e, la.label.clone()))
                    .collect();
                roots.sort_by(|a, b| a.1.cmp(&b.1));
                sub_caption(
                    ui,
                    &format!(
                        "{} loaded asset(s) · {} prim(s)",
                        roots.len(),
                        params.prims.iter().count()
                    ),
                );
                ui.add_space(style::space::TIGHT);
                search_field(ui, &mut filter.0, "Search prims…", accent_col);
                ui.add_space(style::space::BLOCK);

                let mut vis_cache: HashMap<Entity, bool> = HashMap::new();
                for (e, v) in params.visibility_q.iter() {
                    vis_cache.insert(e, !matches!(*v, Visibility::Hidden));
                }
                let vis_before = vis_cache.clone();

                let filter_lc = filter.0.to_lowercase();
                let flat = !filter_lc.is_empty();

                let mut outcome = RowOutcome::default();
                let mut clicked_root: Option<Entity> = None;
                let mut despawn_root: Option<Entity> = None;
                // Group materials per LoadedAsset root by walking each
                // mesh-material entity's parents until we reach a
                // LoadedAsset. Used by the synthetic "Materials"
                // subtree under each top-level row.
                let mut mats_by_root: HashMap<Entity, Vec<bevy::asset::AssetId<StandardMaterial>>> =
                    HashMap::new();
                for (e, mm) in params.mesh_mats.iter() {
                    if let Some(root) =
                        find_loaded_ancestor(e, &params.parents, &params.loaded_only)
                    {
                        let entry = mats_by_root.entry(root).or_default();
                        let id = mm.0.id();
                        if !entry.contains(&id) {
                            entry.push(id);
                        }
                    }
                }

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .min_scrolled_height(600.0)
                    .max_height(600.0)
                    .show(ui, |ui| {
                        if roots.is_empty() {
                            sub_caption(
                                ui,
                                "Nothing loaded — open Selection (F) and click ➕ Add USD.",
                            );
                            return;
                        }
                        if flat {
                            // Flat search across ALL prims regardless of root.
                            let mut matches: Vec<(
                                Entity,
                                &Name,
                                &UsdPrimRef,
                                Option<&UsdDisplayName>,
                            )> = params
                                .prims
                                .iter()
                                .filter(|(_, _, pref, _)| {
                                    pref.path.to_lowercase().contains(&filter_lc)
                                })
                                .collect();
                            matches.sort_by(|a, b| a.2.path.cmp(&b.2.path));
                            if matches.is_empty() {
                                sub_caption(ui, "(no matches)");
                            }
                            for (entity, name, pref, dn) in &matches {
                                let sub = draw_tree_row(
                                    ui,
                                    *entity,
                                    name,
                                    pref,
                                    *dn,
                                    &params.prims,
                                    &params.mat_q,
                                    &params.materials_assets,
                                    &mut vis_cache,
                                    &params.children,
                                    &selected,
                                    &mut expanded,
                                    accent_col,
                                    0,
                                    true,
                                );
                                outcome.merge(sub);
                            }
                            return;
                        }

                        // Two-level: each LoadedAsset is a top-level row
                        // styled with the same `tree_row` widget as the
                        // prim children below it. Eye toggles root-level
                        // visibility (cascades via inherited visibility);
                        // the trash slot pushes the entity into
                        // PendingDespawn.
                        for (root_entity, label) in &roots {
                            let key = format!("@root:{}", root_entity.to_bits());
                            let is_active = active.0 == Some(*root_entity);
                            let is_open_before = *expanded.0.entry(key.clone()).or_insert(true);
                            let mut is_open_local = is_open_before;

                            let mut visible_flag = *vis_cache.get(root_entity).unwrap_or(&true);
                            let mut delete_sentinel = false;
                            let mut slot_buf: Vec<TreeIconSlot<'_>> = vec![
                                TreeIconSlot::new(TreeIconKind::Eye, &mut visible_flag)
                                    .with_tooltip("Toggle visibility"),
                                TreeIconSlot::new(
                                    TreeIconKind::Glyph {
                                        on: "🗑", off: "🗑"
                                    },
                                    &mut delete_sentinel,
                                )
                                .with_tooltip("Remove from scene"),
                            ];
                            let resp = tree_row(
                                ui,
                                root_entity.to_bits(),
                                0,
                                Some(&mut is_open_local),
                                None,
                                label,
                                is_active,
                                accent_col,
                                &mut slot_buf,
                            );
                            vis_cache.insert(*root_entity, visible_flag);
                            if is_open_local != is_open_before {
                                expanded.0.insert(key.clone(), is_open_local);
                            }
                            if resp.body.clicked() {
                                clicked_root = Some(*root_entity);
                            }
                            if let Some(trash) = resp.icons.get(1)
                                && trash.clicked()
                            {
                                despawn_root = Some(*root_entity);
                            }

                            if !is_open_local {
                                continue;
                            }

                            // ── Synthetic "Materials" branch (depth 1) ──
                            let mats_key = format!("@mats:{}", root_entity.to_bits());
                            let mats_open_before =
                                *expanded.0.entry(mats_key.clone()).or_insert(false);
                            let mut mats_open = mats_open_before;
                            let empty_ids: Vec<bevy::asset::AssetId<StandardMaterial>> = Vec::new();
                            let mat_ids = mats_by_root.get(root_entity).unwrap_or(&empty_ids);
                            let mats_label = format!("Materials ({})", mat_ids.len());
                            let mats_resp = tree_row(
                                ui,
                                mats_key.as_str(),
                                1,
                                Some(&mut mats_open),
                                None,
                                &mats_label,
                                false,
                                accent_col,
                                &mut [],
                            );
                            if mats_open != mats_open_before {
                                expanded.0.insert(mats_key.clone(), mats_open);
                            }
                            // Body click on the branch toggles too —
                            // mirrors common outliner UX.
                            if mats_resp.body.clicked() {
                                let new_state = !mats_open;
                                expanded.0.insert(mats_key.clone(), new_state);
                            }
                            if mats_open {
                                if mat_ids.is_empty() {
                                    let mut placeholder = false;
                                    let mut placeholder_slot = [TreeIconSlot::new(
                                        TreeIconKind::Glyph {
                                            on: "·", off: "·"
                                        },
                                        &mut placeholder,
                                    )];
                                    let empty_id = format!("{mats_key}:empty");
                                    tree_row(
                                        ui,
                                        empty_id.as_str(),
                                        2,
                                        None,
                                        None,
                                        "(none)",
                                        false,
                                        accent_col,
                                        &mut placeholder_slot,
                                    );
                                } else {
                                    for id in mat_ids {
                                        let label = asset_server
                                            .get_path(*id)
                                            .map(|p| {
                                                let s = p.to_string();
                                                s.rsplit('/').next().unwrap_or(&s).to_string()
                                            })
                                            .unwrap_or_else(|| format!("{id:?}"));
                                        let depth: u32 = 2;
                                        let leaf_id = format!("{mats_key}:{id:?}");
                                        ui.push_id(leaf_id.as_str(), |ui| {
                                            ui.allocate_ui_with_layout(
                                                egui::vec2(ui.available_width(), TREE_ROW_H),
                                                egui::Layout::left_to_right(egui::Align::Center),
                                                |ui| {
                                                    ui.add_space(depth as f32 * TREE_INDENT);
                                                    let Some(mat) =
                                                        params.materials_assets.get_mut(*id)
                                                    else {
                                                        ui.label(
                                                            egui::RichText::new(&label)
                                                                .color(accent_col),
                                                        );
                                                        return;
                                                    };
                                                    let linear = mat.base_color.to_linear();
                                                    let mut rgb =
                                                        [linear.red, linear.green, linear.blue];
                                                    if ui.color_edit_button_rgb(&mut rgb).changed()
                                                    {
                                                        mat.base_color =
                                                            Color::LinearRgba(LinearRgba {
                                                                red: rgb[0],
                                                                green: rgb[1],
                                                                blue: rgb[2],
                                                                alpha: linear.alpha,
                                                            });
                                                    }
                                                    ui.add(
                                                        egui::Label::new(
                                                            egui::RichText::new(&label)
                                                                .color(accent_col),
                                                        )
                                                        .truncate(),
                                                    );
                                                },
                                            );
                                        });
                                    }
                                }
                            }

                            // ── Synthetic "Variants" branch (depth 1) ──
                            let vars_key = format!("@vars:{}", root_entity.to_bits());
                            let vars_open_before =
                                *expanded.0.entry(vars_key.clone()).or_insert(false);
                            let mut vars_open = vars_open_before;
                            let asset = params
                                .handles
                                .get(*root_entity)
                                .ok()
                                .and_then(|(h, _)| usd_assets.get(&h.0));
                            let var_count: usize = asset
                                .map(|a| a.variants.values().map(|v| v.len()).sum())
                                .unwrap_or(0);
                            let vars_label = format!("Variants ({var_count})");
                            let vars_resp = tree_row(
                                ui,
                                vars_key.as_str(),
                                1,
                                Some(&mut vars_open),
                                None,
                                &vars_label,
                                false,
                                accent_col,
                                &mut [],
                            );
                            if vars_open != vars_open_before {
                                expanded.0.insert(vars_key.clone(), vars_open);
                            }
                            if vars_resp.body.clicked() {
                                let new_state = !vars_open;
                                expanded.0.insert(vars_key.clone(), new_state);
                            }
                            if vars_open {
                                if var_count == 0 {
                                    let mut placeholder = false;
                                    let mut placeholder_slot = [TreeIconSlot::new(
                                        TreeIconKind::Glyph {
                                            on: "·", off: "·"
                                        },
                                        &mut placeholder,
                                    )];
                                    let empty_id = format!("{vars_key}:empty");
                                    tree_row(
                                        ui,
                                        empty_id.as_str(),
                                        2,
                                        None,
                                        None,
                                        "(none)",
                                        false,
                                        accent_col,
                                        &mut placeholder_slot,
                                    );
                                } else if let Some(asset) = asset {
                                    let mut entries: Vec<(&String, &Vec<usd_bevy::VariantSet>)> =
                                        asset.variants.iter().collect();
                                    entries.sort_by(|a, b| a.0.cmp(b.0));
                                    for (prim_path, sets) in entries {
                                        for set in sets {
                                            let key = (prim_path.clone(), set.name.clone());
                                            let authored = set.selection.as_deref().unwrap_or("");
                                            let current = loader_tuning
                                                .variants
                                                .get(&key)
                                                .cloned()
                                                .unwrap_or_else(|| authored.to_string());
                                            let label = format!("{prim_path} • {}", set.name);
                                            let leaf_id =
                                                format!("{vars_key}:{prim_path}:{}", set.name);
                                            let depth: u32 = 2;
                                            ui.push_id(leaf_id.as_str(), |ui| {
                                                ui.allocate_ui_with_layout(
                                                    egui::vec2(ui.available_width(), TREE_ROW_H),
                                                    egui::Layout::left_to_right(
                                                        egui::Align::Center,
                                                    ),
                                                    |ui| {
                                                        ui.add_space(depth as f32 * TREE_INDENT);
                                                        ui.add(
                                                            egui::Label::new(
                                                                egui::RichText::new(&label)
                                                                    .color(accent_col),
                                                            )
                                                            .truncate(),
                                                        );
                                                        ui.with_layout(
                                                            egui::Layout::right_to_left(
                                                                egui::Align::Center,
                                                            ),
                                                            |ui| {
                                                                if set.options.is_empty() {
                                                                    ui.add_enabled(
                                                                        false,
                                                                        egui::Label::new(
                                                                            "(no options)",
                                                                        ),
                                                                    );
                                                                    return;
                                                                }
                                                                let display = if current.is_empty()
                                                                {
                                                                    "(none)"
                                                                } else {
                                                                    current.as_str()
                                                                };
                                                                let mut picked: Option<String> =
                                                                    None;
                                                                egui::ComboBox::from_id_salt(
                                                                    leaf_id.as_str(),
                                                                )
                                                                .selected_text(display)
                                                                .show_ui(ui, |ui| {
                                                                    for opt in &set.options {
                                                                        let selected =
                                                                            opt == &current;
                                                                        if ui
                                                                            .selectable_label(
                                                                                selected, opt,
                                                                            )
                                                                            .clicked()
                                                                            && !selected
                                                                        {
                                                                            picked =
                                                                                Some(opt.clone());
                                                                        }
                                                                    }
                                                                });
                                                                if let Some(p) = picked {
                                                                    loader_tuning
                                                                        .variants
                                                                        .insert(key.clone(), p);
                                                                    reload.requested = true;
                                                                }
                                                            },
                                                        );
                                                    },
                                                );
                                            });
                                        }
                                    }
                                }
                            }

                            // Find the prim sub-roots under this LoadedAsset
                            // entity by walking its `Children`.
                            let mut child_prims: Vec<(
                                Entity,
                                &Name,
                                &UsdPrimRef,
                                Option<&UsdDisplayName>,
                            )> = vec![];
                            if let Ok(cs) = params.children.get(*root_entity) {
                                for c in cs.iter() {
                                    if let Ok(row) = params.prims.get(c) {
                                        child_prims.push(row);
                                    } else if let Ok(grand) = params.children.get(c) {
                                        // Bevy's `SceneRoot` inserts an
                                        // intermediate entity above the
                                        // USD prim subtree; descend one
                                        // more level.
                                        for gc in grand.iter() {
                                            if let Ok(row) = params.prims.get(gc) {
                                                child_prims.push(row);
                                            }
                                        }
                                    }
                                }
                            }
                            child_prims.sort_by(|a, b| a.2.path.cmp(&b.2.path));
                            if child_prims.is_empty() {
                                ui.indent(format!("indent_{}", root_entity.to_bits()), |ui| {
                                    sub_caption(ui, "(stage still loading…)");
                                });
                                continue;
                            }
                            for (entity, name, pref, dn) in &child_prims {
                                let sub = draw_tree_row(
                                    ui,
                                    *entity,
                                    name,
                                    pref,
                                    *dn,
                                    &params.prims,
                                    &params.mat_q,
                                    &params.materials_assets,
                                    &mut vis_cache,
                                    &params.children,
                                    &selected,
                                    &mut expanded,
                                    accent_col,
                                    1,
                                    false,
                                );
                                outcome.merge(sub);
                            }
                        }
                    });

                // Commit eye-icon toggles back to the ECS.
                for (entity, visible) in &vis_cache {
                    if vis_before.get(entity) != Some(visible)
                        && let Ok((_, mut v)) = params.visibility_q.get_mut(*entity)
                    {
                        *v = if *visible {
                            Visibility::Inherited
                        } else {
                            Visibility::Hidden
                        };
                    }
                }

                if let Some(root) = clicked_root {
                    selection.0 = Some(root);
                    active.0 = Some(root);
                }
                if let Some(root) = despawn_root {
                    despawn.0.push(root);
                }

                if let Some(action) = outcome.ctx_action {
                    match action {
                        CtxAction::FlyTo(entity) => {
                            selected.0 = Some(entity);
                            if let (Ok(target_gt), Ok(cam)) =
                                (params.gt_query.get(entity), params.cameras.single())
                            {
                                let target = target_gt.translation();
                                let target_dist = (cam.distance * 0.25).clamp(0.2, 40.0);
                                fly.start_focus = cam.focus;
                                fly.start_distance = cam.distance;
                                fly.target_focus = target;
                                fly.target_distance = target_dist;
                                fly.duration = 0.4;
                                fly.remaining = 0.4;
                            }
                        }
                        CtxAction::Fit(entity) => {
                            selected.0 = Some(entity);
                            if let Ok(cam) = params.cameras.single() {
                                let (target, target_dist) = fit_params_for_entity(
                                    entity,
                                    &params.gt_query,
                                    &params.extent_q,
                                    &params.children,
                                    cam.distance,
                                );
                                fly.start_focus = cam.focus;
                                fly.start_distance = cam.distance;
                                fly.target_focus = target;
                                fly.target_distance = target_dist;
                                fly.duration = 0.4;
                                fly.remaining = 0.4;
                            }
                        }
                        CtxAction::ExpandDesc(entity) => {
                            set_subtree_expanded(
                                entity,
                                &params.prims,
                                &params.children,
                                &mut expanded,
                                true,
                            );
                        }
                        CtxAction::CollapseDesc(entity) => {
                            set_subtree_expanded(
                                entity,
                                &params.prims,
                                &params.children,
                                &mut expanded,
                                false,
                            );
                        }
                    }
                }

                if let Some(entity) = outcome.double_clicked {
                    selected.0 = Some(entity);
                    if let Ok(cam) = params.cameras.single() {
                        let (target, target_dist) = fit_params_for_entity(
                            entity,
                            &params.gt_query,
                            &params.extent_q,
                            &params.children,
                            cam.distance,
                        );
                        fly.start_focus = cam.focus;
                        fly.start_distance = cam.distance;
                        fly.target_focus = target;
                        fly.target_distance = target_dist;
                        fly.duration = 0.4;
                        fly.remaining = 0.4;
                    }
                } else if let Some(entity) = outcome.clicked {
                    selected.0 = Some(entity);
                    // Pin the gizmo on the LoadedAsset that owns this
                    // prim so the user can drag handles to move the
                    // whole USD — moving an internal prim isn't well
                    // defined (transform belongs to the SceneRoot tree).
                    if let Some(root) =
                        find_loaded_ancestor(entity, &params.parents, &params.loaded_only)
                    {
                        selection.0 = Some(root);
                        active.0 = Some(root);
                    }
                    if let (Ok(target_gt), Ok(cam)) =
                        (params.gt_query.get(entity), params.cameras.single())
                    {
                        let target = target_gt.translation();
                        let target_dist = (cam.distance * 0.25).clamp(0.2, 40.0);
                        fly.start_focus = cam.focus;
                        fly.start_distance = cam.distance;
                        fly.target_focus = target;
                        fly.target_distance = target_dist;
                        fly.duration = 0.4;
                        fly.remaining = 0.4;
                    }
                }
            });
        },
    );
}

#[derive(Default, Clone, Copy)]
struct RowOutcome {
    clicked: Option<Entity>,
    double_clicked: Option<Entity>,
    ctx_action: Option<CtxAction>,
}

#[derive(Clone, Copy, Debug)]
enum CtxAction {
    FlyTo(Entity),
    Fit(Entity),
    ExpandDesc(Entity),
    CollapseDesc(Entity),
}

impl RowOutcome {
    fn merge(&mut self, other: RowOutcome) {
        if other.double_clicked.is_some() {
            self.double_clicked = other.double_clicked;
        }
        if other.clicked.is_some() {
            self.clicked = other.clicked;
        }
        if other.ctx_action.is_some() {
            self.ctx_action = other.ctx_action;
        }
    }
}

fn set_subtree_expanded(
    root: Entity,
    prims: &Query<(Entity, &Name, &UsdPrimRef, Option<&UsdDisplayName>)>,
    children: &Query<&Children>,
    expanded: &mut TreeExpanded,
    open: bool,
) {
    let mut stack = vec![root];
    while let Some(e) = stack.pop() {
        if let Ok((_, _, pref, _)) = prims.get(e) {
            expanded.0.insert(pref.path.clone(), open);
        }
        if let Ok(cs) = children.get(e) {
            for c in cs.iter() {
                stack.push(c);
            }
        }
    }
}

fn swatch_color_for(
    entity: Entity,
    mat_q: &Query<&MeshMaterial3d<StandardMaterial>>,
    children: &Query<&Children>,
    materials: &Assets<StandardMaterial>,
) -> Option<egui::Color32> {
    let pick = |e: Entity| -> Option<egui::Color32> {
        let mm = mat_q.get(e).ok()?;
        let mat = materials.get(&mm.0)?;
        let c = mat.base_color.to_linear();
        Some(style::srgb_to_egui([c.red, c.green, c.blue]))
    };
    if let Some(c) = pick(entity) {
        return Some(c);
    }
    if let Ok(cs) = children.get(entity) {
        for c in cs.iter() {
            if let Some(col) = pick(c) {
                return Some(col);
            }
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn draw_tree_row(
    ui: &mut egui::Ui,
    entity: Entity,
    name: &Name,
    prim_ref: &UsdPrimRef,
    display_name: Option<&UsdDisplayName>,
    prims: &Query<(Entity, &Name, &UsdPrimRef, Option<&UsdDisplayName>)>,
    mat_q: &Query<&MeshMaterial3d<StandardMaterial>>,
    materials: &Assets<StandardMaterial>,
    vis_cache: &mut HashMap<Entity, bool>,
    children: &Query<&Children>,
    selected: &SelectedPrim,
    expanded: &mut TreeExpanded,
    accent: egui::Color32,
    depth: u32,
    leaf_override: bool,
) -> RowOutcome {
    let child_ids: Vec<Entity> = children
        .get(entity)
        .map(|c| c.iter().collect())
        .unwrap_or_default();
    let mut prim_children: Vec<(Entity, &Name, &UsdPrimRef, Option<&UsdDisplayName>)> = child_ids
        .iter()
        .filter_map(|c| prims.get(*c).ok())
        .collect();
    prim_children.sort_by(|a, b| a.2.path.cmp(&b.2.path));
    let has_children = !leaf_override && !prim_children.is_empty();

    let is_selected = selected.0 == Some(entity);
    let path_key = prim_ref.path.clone();
    let row_id_salt = entity.to_bits();
    let mut outcome = RowOutcome::default();

    let mut visible_flag = *vis_cache.get(&entity).unwrap_or(&true);
    let swatch = swatch_color_for(entity, mat_q, children, materials);
    let mut color_sentinel = false;

    let label_owned: String = display_name
        .map(|d| d.0.clone())
        .unwrap_or_else(|| name.as_str().to_string());

    let resp = {
        let mut slot_buf: Vec<TreeIconSlot<'_>> = Vec::with_capacity(2);
        slot_buf.push(
            TreeIconSlot::new(TreeIconKind::Eye, &mut visible_flag)
                .with_tooltip("Toggle visibility"),
        );
        if let Some(c) = swatch {
            slot_buf.push(TreeIconSlot::new(
                TreeIconKind::Color(c),
                &mut color_sentinel,
            ));
        }

        if has_children {
            let is_open = *expanded.0.entry(path_key.clone()).or_insert(true);
            let mut open_ref = is_open;
            let r = tree_row(
                ui,
                row_id_salt,
                depth,
                Some(&mut open_ref),
                None,
                &label_owned,
                is_selected,
                accent,
                &mut slot_buf,
            );
            if open_ref != is_open {
                expanded.0.insert(path_key.clone(), open_ref);
            }
            r
        } else {
            tree_row(
                ui,
                row_id_salt,
                depth,
                None,
                None,
                &label_owned,
                is_selected,
                accent,
                &mut slot_buf,
            )
        }
    };

    vis_cache.insert(entity, visible_flag);

    if resp.body.hovered() {
        resp.body.clone().on_hover_text(&prim_ref.path);
    }
    if resp.body.double_clicked() {
        outcome.double_clicked = Some(entity);
    } else if resp.body.clicked() {
        outcome.clicked = Some(entity);
    }

    context_menu_frost(&resp.body, accent, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        if wide_button(ui, "Fly to", accent).clicked() {
            outcome.ctx_action = Some(CtxAction::FlyTo(entity));
            ui.close();
        }
        if wide_button(ui, "Fit to bounds", accent).clicked() {
            outcome.ctx_action = Some(CtxAction::Fit(entity));
            ui.close();
        }
        if wide_button(ui, "Copy path", accent).clicked() {
            ui.ctx().copy_text(prim_ref.path.clone());
            ui.close();
        }
        if wide_button(ui, "Expand descendants", accent).clicked() {
            outcome.ctx_action = Some(CtxAction::ExpandDesc(entity));
            ui.close();
        }
        if wide_button(ui, "Collapse descendants", accent).clicked() {
            outcome.ctx_action = Some(CtxAction::CollapseDesc(entity));
            ui.close();
        }
    });

    let show_children = if has_children {
        *expanded.0.get(&path_key).unwrap_or(&true)
    } else {
        false
    };
    if show_children {
        for (child_entity, child_name, child_ref, child_dn) in prim_children {
            let sub = draw_tree_row(
                ui,
                child_entity,
                child_name,
                child_ref,
                child_dn,
                prims,
                mat_q,
                materials,
                vis_cache,
                children,
                selected,
                expanded,
                accent,
                depth + 1,
                false,
            );
            outcome.merge(sub);
        }
    }

    outcome
}

fn fit_params_for_entity(
    root: Entity,
    gt_q: &Query<&GlobalTransform>,
    extent_q: &Query<&usd_bevy::UsdLocalExtent>,
    children: &Query<&Children>,
    current_cam_dist: f32,
) -> (Vec3, f32) {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut found = false;

    let mut stack: Vec<Entity> = vec![root];
    while let Some(e) = stack.pop() {
        if let (Ok(gt), Ok(le)) = (gt_q.get(e), extent_q.get(e)) {
            let m = gt.to_matrix();
            for i in 0..8 {
                let c = Vec3::new(
                    if i & 1 == 0 { le.min[0] } else { le.max[0] },
                    if i & 2 == 0 { le.min[1] } else { le.max[1] },
                    if i & 4 == 0 { le.min[2] } else { le.max[2] },
                );
                let w = m.transform_point3(c);
                min = min.min(w);
                max = max.max(w);
            }
            found = true;
        }
        if let Ok(cs) = children.get(e) {
            for c in cs.iter() {
                stack.push(c);
            }
        }
    }

    if found {
        let center = (min + max) * 0.5;
        let size = (max - min).abs();
        let max_dim = size.x.max(size.y).max(size.z).max(0.05);
        let dist = (max_dim * 1.6).clamp(0.2, 200.0);
        (center, dist)
    } else if let Ok(gt) = gt_q.get(root) {
        (gt.translation(), (current_cam_dist * 0.25).clamp(0.2, 40.0))
    } else {
        (Vec3::ZERO, current_cam_dist)
    }
}

// ─── Stage-info panel ───────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn draw_info_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    info: Res<StageInfo>,
    mut reload: ResMut<ReloadRequest>,
    prims: Query<&UsdPrimRef>,
    meshes_q: Query<&Mesh3d, With<UsdPrimRef>>,
    spatial_audio_q: Query<&UsdSpatialAudio>,
    procedural_q: Query<&UsdProcedural>,
) {
    if !is_panel_open(&open, RIB_INFO) {
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
        RIB_INFO,
        "Stage info",
        egui::vec2(PANEL_W, PANEL_H + 40.0),
        &mut keep,
        accent_col,
        |pane| {
            pane.section("info_stage", "Stage", true, |ui| {
                readout_row(ui, "file", &info.path);
                readout_row(
                    ui,
                    "defaultPrim",
                    info.default_prim.as_deref().unwrap_or("—"),
                );
                readout_row(ui, "layers", &info.layer_count.to_string());
                readout_row(ui, "prims", &prims.iter().count().to_string());
                readout_row(ui, "meshes", &meshes_q.iter().count().to_string());
                readout_row(ui, "variants", &info.variant_count.to_string());
            });
            pane.section("info_lights", "Lights & instances", true, |ui| {
                let light_labels = [
                    format!("{} dir", info.lights_directional),
                    format!("{} pt", info.lights_point),
                    format!("{} spot", info.lights_spot),
                    format!("{} dome", info.lights_dome),
                ];
                let refs: Vec<&str> = light_labels.iter().map(String::as_str).collect();
                badge_row(ui, "lights", &refs, accent_col);

                let inst_labels = [
                    format!("{} prim", info.instance_prim_count),
                    format!("{} reuse", info.instance_prototype_reuses),
                ];
                let refs: Vec<&str> = inst_labels.iter().map(String::as_str).collect();
                badge_row(ui, "instances", &refs, accent_col);

                readout_row(
                    ui,
                    "animated",
                    &format!("{} prim(s)", info.animated_prim_count),
                );
            });
            pane.section("info_skel_render", "Skel & render", true, |ui| {
                let skel_labels = [
                    format!("{} skel", info.skeleton_count),
                    format!("{} root", info.skel_root_count),
                    format!("{} bind", info.skel_binding_count),
                ];
                let refs: Vec<&str> = skel_labels.iter().map(String::as_str).collect();
                badge_row(ui, "skel", &refs, accent_col);

                let render_labels = [
                    format!("{} settings", info.render_settings_count),
                    format!("{} product", info.render_product_count),
                    format!("{} var", info.render_var_count),
                ];
                let refs: Vec<&str> = render_labels.iter().map(String::as_str).collect();
                badge_row(ui, "render", &refs, accent_col);

                if let Some([w, h]) = info.render_primary_resolution {
                    readout_row(ui, "resolution", &format!("{w} × {h}"));
                }

                let phys_labels = [
                    format!("{} scene", info.physics_scene_count),
                    format!("{} rigid", info.rigid_body_count),
                    format!("{} joint", info.joint_count),
                ];
                let refs: Vec<&str> = phys_labels.iter().map(String::as_str).collect();
                badge_row(ui, "physics", &refs, accent_col);
            });
            pane.section("info_authoring", "Authoring detail", true, |ui| {
                readout_row(
                    ui,
                    "custom",
                    &format!(
                        "{} prim · {} layer entries",
                        info.custom_attr_prim_count, info.custom_layer_data_entries
                    ),
                );
                readout_row(
                    ui,
                    "subdiv",
                    &format!("{} mesh(es) subdivision", info.subdivision_prim_count),
                );
                readout_row(
                    ui,
                    "light-link",
                    &format!("{} light(s) linked", info.light_linked_count),
                );
                readout_row(
                    ui,
                    "clips",
                    &format!("{} prim(s) UsdClipsAPI", info.clip_prim_count),
                );
                readout_row(
                    ui,
                    "spatial-audio",
                    &format!("{} source(s)", spatial_audio_q.iter().count()),
                );
                readout_row(
                    ui,
                    "procedural",
                    &format!("{} prim(s)", procedural_q.iter().count()),
                );
            });
            pane.section("info_actions", "Actions", true, |ui| {
                if wide_button(ui, "⟳  Reload stage (R)", accent_col).clicked() {
                    reload.requested = true;
                }
            });
        },
    );
}

// ─── Machine controllers panel ──────────────────────────────────────

fn draw_controllers_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    active: Res<ActiveStage>,
    inventory: Res<ControllerInventory>,
    states: Res<ControllerStates>,
    external_policy: Res<ExternalControllerPolicy>,
    external_processes: Res<ExternalControllerProcesses>,
    mut commands: ResMut<ControllerCommands>,
) {
    if !is_panel_open(&open, RIB_CONTROLLERS) {
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
        RIB_CONTROLLERS,
        "Machine controllers",
        egui::vec2(PANEL_W + 60.0, PANEL_H),
        &mut keep,
        accent_col,
        |pane| {
            pane.section("controllers_summary", "Discovered", true, |ui| {
                readout_row(ui, "machines", &inventory.machines.len().to_string());
                let controller_count: usize =
                    inventory.machines.iter().map(|m| m.controllers.len()).sum();
                readout_row(ui, "controllers", &controller_count.to_string());
                readout_row(
                    ui,
                    "external proc",
                    if external_policy.allow_processes {
                        "enabled"
                    } else {
                        "disabled"
                    },
                );
                match active.0 {
                    Some(entity) => {
                        let active_count = inventory
                            .machines
                            .iter()
                            .filter(|m| m.scene_root == Some(entity))
                            .count();
                        readout_row(ui, "active asset", &format!("{active_count} machine(s)"));
                    }
                    None => sub_caption(ui, "No active asset; showing all discoveries below."),
                }
            });

            pane.section("controllers_list", "Machines", true, |ui| {
                if inventory.machines.is_empty() {
                    sub_caption(ui, "No GearboxMachineAPI metadata has been discovered yet.");
                    return;
                }

                let mut shown = 0usize;
                for machine in &inventory.machines {
                    if let Some(active_entity) = active.0
                        && machine.scene_root != Some(active_entity)
                    {
                        continue;
                    }
                    shown += 1;
                    ui.add_space(style::space::TIGHT);
                    ui.label(
                        egui::RichText::new(format!("{}  {}", machine.id, machine.prim_path))
                            .strong()
                            .color(accent_col),
                    );
                    readout_row(ui, "kind", machine.kind.as_deref().unwrap_or("—"));
                    readout_row(ui, "source", &machine.asset_label);
                    readout_row(ui, "id policy", &machine.id_policy);
                    readout_row(ui, "body", machine.body.as_deref().unwrap_or("—"));
                    readout_row(
                        ui,
                        "control",
                        &format!(
                            "{} drive wheel(s), {} steer joint(s), {} wheel joint(s)",
                            machine.drive_wheels.len(),
                            machine.steer_joints.len(),
                            machine.wheel_joints.len()
                        ),
                    );

                    if machine.controllers.is_empty() {
                        sub_caption(ui, "No GearboxControllerAPI instances on this machine.");
                    }
                    for controller in &machine.controllers {
                        let command_key = machine.scene_root.map(|scene_root| {
                            ControllerKey::new(scene_root, &machine.id, &controller.instance)
                        });
                        ui.add_space(style::space::TIGHT);
                        ui.group(|ui| {
                            ui.label(
                                egui::RichText::new(format!("controller:{}", controller.instance))
                                    .strong(),
                            );
                            readout_row(ui, "type", &controller.controller_type);
                            readout_row(
                                ui,
                                "enabled",
                                if controller.enabled { "true" } else { "false" },
                            );
                            readout_row(ui, "namespace", &controller.namespace);
                            readout_row(
                                ui,
                                "update",
                                &format!("{:.1} Hz", controller.update_rate_hz),
                            );
                            readout_row(
                                ui,
                                "command",
                                controller.command_interface.as_deref().unwrap_or("—"),
                            );
                            let state_refs: Vec<&str> = controller
                                .state_interfaces
                                .iter()
                                .map(String::as_str)
                                .collect();
                            if state_refs.is_empty() {
                                readout_row(ui, "state", "—");
                            } else {
                                badge_row(ui, "state", &state_refs, accent_col);
                            }
                            readout_row(
                                ui,
                                "limits",
                                &format!(
                                    "wheelbase={} m · track={} m · steer={}°",
                                    opt_f32(controller.wheel_base),
                                    opt_f32(controller.track_width),
                                    opt_f32(controller.max_steer_deg)
                                ),
                            );
                            if controller.controller_type == "builtin:ackermann_cmd_vel"
                                && let Some(key) = command_key.clone()
                            {
                                if let Some(state) = states.states.get(&key) {
                                    readout_row(
                                        ui,
                                        "pose",
                                        &format!(
                                            "{:+.2}, {:+.2}, {:+.2}",
                                            state.position_m[0],
                                            state.position_m[1],
                                            state.position_m[2]
                                        ),
                                    );
                                    readout_row(
                                        ui,
                                        "velocity",
                                        &format!(
                                            "{:.2} m/s · yaw {:.2} rad/s",
                                            state.linear_speed_mps, state.yaw_rate_rps
                                        ),
                                    );
                                }
                                let cmd = commands.cmd_vel.entry(key).or_insert(CmdVel {
                                    linear_mps: 0.0,
                                    angular_rps: 0.0,
                                });
                                ui.add_space(style::space::TIGHT);
                                ui.label(egui::RichText::new("internal cmd_vel").strong());
                                ui.add(
                                    egui::Slider::new(&mut cmd.linear_mps, -5.0..=5.0)
                                        .text("linear m/s"),
                                );
                                ui.add(
                                    egui::Slider::new(&mut cmd.angular_rps, -1.5..=1.5)
                                        .text("angular rad/s"),
                                );
                                if wide_button(ui, "Stop", accent_col).clicked() {
                                    cmd.linear_mps = 0.0;
                                    cmd.angular_rps = 0.0;
                                }
                                sub_caption(
                                    ui,
                                    "Prototype runtime: applies body force/yaw torque while physics is playing.",
                                );
                            }
                            if controller.controller_type == "external:process" {
                                readout_row(
                                    ui,
                                    "executable",
                                    controller.executable.as_deref().unwrap_or("—"),
                                );
                                if !controller.args.is_empty() {
                                    readout_row(ui, "args", &controller.args.join(" "));
                                }
                                if let Some(key) = command_key.as_ref() {
                                    readout_row(
                                        ui,
                                        "process",
                                        external_processes
                                            .status
                                            .get(key)
                                            .map(String::as_str)
                                            .unwrap_or("not started"),
                                    );
                                }
                                sub_caption(
                                    ui,
                                    "External process controllers are denied unless the env policy allows them.",
                                );
                            }
                        });
                    }
                    ui.separator();
                }

                if shown == 0 {
                    sub_caption(ui, "The active asset has no discovered Gearbox machines.");
                }
            });
        },
    );
}

fn opt_f32(v: Option<f32>) -> String {
    v.map(|v| format!("{v:.2}"))
        .unwrap_or_else(|| "—".to_string())
}

// ─── Cameras panel ──────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn draw_cameras_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    active: Res<ActiveStage>,
    handles: Query<(&UsdAssetHandle, &LoadedAsset)>,
    usd_assets: Res<Assets<UsdAsset>>,
    mut camera_mount: ResMut<CameraMount>,
    mut bookmarks: ResMut<CameraBookmarks>,
    mut fly: ResMut<FlyTo>,
    cameras: Query<&ChaseCamera>,
) {
    if !is_panel_open(&open, RIB_CAMERAS) {
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
        RIB_CAMERAS,
        "Cameras",
        egui::vec2(PANEL_W, PANEL_H),
        &mut keep,
        accent_col,
        |pane| {
            pane.section("cameras_bookmarks", "Bookmarks", true, |ui| {
                if wide_button(ui, "💾  Save current view", accent_col).clicked()
                    && let Ok(cam) = cameras.single()
                {
                    let seq = bookmarks.next_seq + 1;
                    bookmarks.next_seq = seq;
                    bookmarks.items.push(CameraBookmark {
                        name: format!("View {seq}"),
                        focus: cam.focus,
                        distance: cam.distance,
                        yaw: cam.yaw,
                        elevation: cam.elevation,
                    });
                }
                if bookmarks.items.is_empty() {
                    sub_caption(ui, "(no bookmarks yet)");
                } else {
                    let mut to_delete: Option<usize> = None;
                    let mut to_jump: Option<usize> = None;
                    for (idx, bm) in bookmarks.items.iter().enumerate() {
                        let r = hybrid_select_row(
                            ui,
                            ("bookmark", idx),
                            &bm.name,
                            Some(&format!("d {:.1}", bm.distance)),
                            false,
                            false,
                            accent_col,
                        );
                        if r.body.clicked() {
                            to_jump = Some(idx);
                        }
                        if r.radio.clicked() {
                            to_delete = Some(idx);
                        }
                    }
                    if let Some(idx) = to_jump
                        && let (Ok(cam), Some(bm)) = (cameras.single(), bookmarks.items.get(idx))
                    {
                        *camera_mount = CameraMount::Arcball;
                        fly.start_focus = cam.focus;
                        fly.start_distance = cam.distance;
                        fly.start_yaw = Some(cam.yaw);
                        fly.start_elevation = Some(cam.elevation);
                        fly.target_focus = bm.focus;
                        fly.target_distance = bm.distance;
                        fly.target_yaw = Some(bm.yaw);
                        fly.target_elevation = Some(bm.elevation);
                        fly.duration = 0.5;
                        fly.remaining = 0.5;
                    }
                    if let Some(idx) = to_delete {
                        bookmarks.items.remove(idx);
                    }
                    sub_caption(ui, "Click row to jump · click radio to delete");
                }
            });

            pane.section("cameras_all", "Cameras", true, |ui| {
                let asset = active_asset(&active, &handles, &usd_assets);
                let Some(asset) = asset else {
                    sub_caption(ui, "(no stage loaded yet)");
                    return;
                };
                sub_caption(ui, &format!("{} authored cameras", asset.cameras.len()));
                ui.add_space(style::space::BLOCK);

                let arcball_active = matches!(*camera_mount, CameraMount::Arcball);
                let r = hybrid_select_row(
                    ui,
                    "arcball_mount",
                    "🎮  Arcball (free)",
                    None,
                    arcball_active,
                    arcball_active,
                    accent_col,
                );
                if r.body.clicked() || r.radio.clicked() {
                    *camera_mount = CameraMount::Arcball;
                }
                row_separator(ui);

                egui::ScrollArea::vertical().show(ui, |ui| {
                    for cam in &asset.cameras {
                        let mounted = matches!(
                            &*camera_mount,
                            CameraMount::Mounted { prim_path } if prim_path == &cam.path
                        );
                        let name = cam.path.rsplit('/').next().unwrap_or(&cam.path);
                        let focal = cam.data.focal_length_mm.unwrap_or(50.0);
                        let proj = match cam.data.projection {
                            Some(usd_schema::camera::Projection::Orthographic) => "ortho",
                            _ => "persp",
                        };
                        let label = format!("📷  {name}");
                        let trailing = format!("{focal:.0}mm · {proj}");
                        let r = hybrid_select_row(
                            ui,
                            cam.path.as_str(),
                            &label,
                            Some(&trailing),
                            mounted,
                            mounted,
                            accent_col,
                        );
                        if r.body.clicked() || r.radio.clicked() {
                            *camera_mount = CameraMount::Mounted {
                                prim_path: cam.path.clone(),
                            };
                        }
                    }
                });
                ui.add_space(style::space::TIGHT);
                sub_caption(
                    ui,
                    "Camera mounting (follow USD camera) is not yet wired in simulator.",
                );
            });
        },
    );
}

// ─── Overlays panel ─────────────────────────────────────────────────

fn draw_overlays_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    mut toggles: ResMut<DisplayToggles>,
    mut loader_tuning: ResMut<LoaderTuning>,
) {
    if !is_panel_open(&open, RIB_OVERLAYS) {
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
        RIB_OVERLAYS,
        "Overlays",
        egui::vec2(PANEL_W, PANEL_H),
        &mut keep,
        accent_col,
        |pane| {
            pane.section("overlay_toggles", "World overlays", true, |ui| {
                toggle(
                    ui,
                    "Ground grid (G)",
                    &mut toggles.show_world_grid,
                    accent_col,
                );
                toggle(
                    ui,
                    "World axes (X)",
                    &mut toggles.show_world_axes,
                    accent_col,
                );
                toggle(
                    ui,
                    "Prim markers (P)",
                    &mut toggles.show_prim_markers,
                    accent_col,
                );
                let mut v = toggles.prim_marker_bias as f64;
                if pretty_slider(
                    ui,
                    "Prim marker bias",
                    &mut v,
                    0.0..=5.0,
                    2,
                    "×",
                    accent_col,
                )
                .changed()
                {
                    toggles.prim_marker_bias = v as f32;
                }
                toggle(
                    ui,
                    "Skeleton bones (B)",
                    &mut toggles.show_skeleton,
                    accent_col,
                );
                toggle(
                    ui,
                    "Physics gizmos (Y)",
                    &mut toggles.show_physics,
                    accent_col,
                );
                toggle(
                    ui,
                    "Collider wireframes (C)",
                    &mut toggles.show_colliders,
                    accent_col,
                );
            });

            pane.section("overlay_render", "Render", true, |ui| {
                toggle(ui, "Wireframe", &mut toggles.wireframe, accent_col);
                let mut s = toggles.light_intensity_scale as f64;
                if pretty_slider(ui, "Light intensity", &mut s, 0.0..=5.0, 2, "×", accent_col)
                    .changed()
                {
                    toggles.light_intensity_scale = s as f32;
                }
                sub_caption(ui, "Scales every authored light from its original value.");
            });

            pane.section("overlay_curves", "Curves (tubes)", true, |ui| {
                sub_caption(ui, "Default radius used when widths aren't authored");
                let mut r = loader_tuning.curves.default_radius as f64;
                if pretty_slider(ui, "Radius", &mut r, 0.001..=0.2, 3, " m", accent_col).changed() {
                    loader_tuning.curves.default_radius = r as f32;
                }
                let mut seg = loader_tuning.curves.ring_segments as f64;
                if pretty_slider(ui, "Ring segments", &mut seg, 3.0..=24.0, 0, "", accent_col)
                    .changed()
                {
                    loader_tuning.curves.ring_segments = seg.round() as u32;
                }
                let mut ps = loader_tuning.curves.point_scale as f64;
                if pretty_slider(ui, "Point scale", &mut ps, 0.05..=4.0, 2, "×", accent_col)
                    .changed()
                {
                    loader_tuning.curves.point_scale = ps as f32;
                }
                sub_caption(ui, "Sliders apply on next reload (R).");
            });
        },
    );
}

// ─── Timeline panel ─────────────────────────────────────────────────

fn draw_timeline_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    mut clock: ResMut<UsdStageTime>,
    active: Res<ActiveStage>,
    handles: Query<(&UsdAssetHandle, &LoadedAsset)>,
    usd_assets: Res<Assets<UsdAsset>>,
) {
    if !is_panel_open(&open, RIB_TIMELINE) {
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
        RIB_TIMELINE,
        "Timeline",
        egui::vec2(PANEL_W, 320.0),
        &mut keep,
        accent_col,
        |pane| {
            pane.section("timeline_playback", "Playback", true, |ui| {
                let asset = active_asset(&active, &handles, &usd_assets);
                let animated_count = asset.map(|a| a.animated_prims.len()).unwrap_or(0);
                sub_caption(
                    ui,
                    &format!(
                        "{animated_count} animated prim(s) · {:.1} fps · {:.1}s total",
                        clock.time_codes_per_second,
                        clock.duration_seconds()
                    ),
                );
                ui.add_space(style::space::BLOCK);

                let play_label = if clock.playing {
                    "⏸  Pause"
                } else {
                    "▶  Play"
                };
                if wide_button(ui, play_label, accent_col).clicked() {
                    clock.playing = !clock.playing;
                }
                if wide_button(ui, "⏮  Rewind", accent_col).clicked() {
                    clock.seconds = 0.0;
                }

                ui.add_space(style::space::BLOCK);
                let dur = clock.duration_seconds().max(1e-3);
                let _ = pretty_slider(
                    ui,
                    "Seconds",
                    &mut clock.seconds,
                    0.0..=dur,
                    3,
                    " s",
                    accent_col,
                );

                readout_row(ui, "timeCode", &format!("{:.3}", clock.current_time_code()));
                readout_row(
                    ui,
                    "range",
                    &format!("{:.2} … {:.2}", clock.start_time_code, clock.end_time_code),
                );
                readout_row(ui, "fps", &format!("{:.2}", clock.time_codes_per_second));
            });
        },
    );
}

// ─── Keys panel ─────────────────────────────────────────────────────

fn draw_keys_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
) {
    if !is_panel_open(&open, RIB_KEYS) {
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
        RIB_KEYS,
        "Controls",
        egui::vec2(PANEL_W, PANEL_H),
        &mut keep,
        accent_col,
        |pane| {
            pane.section("keys_camera", "Camera", true, |ui| {
                keybinding_row(ui, "L+R drag", "Orbit");
                keybinding_row(ui, "Middle", "Pan");
                keybinding_row(ui, "Scroll", "Zoom");
            });
            pane.section("keys_panels", "Panels", true, |ui| {
                keybinding_row(ui, "T", "Toggle prim tree");
                keybinding_row(ui, "I", "Toggle stage info");
                keybinding_row(ui, "O", "Toggle overlays");
                keybinding_row(ui, "?", "Toggle this panel");
                keybinding_row(ui, "Ctrl+K", "Command palette");
            });
            pane.section("keys_overlays", "Overlays", true, |ui| {
                keybinding_row(ui, "G", "Ground grid");
                keybinding_row(ui, "X", "World axes");
                keybinding_row(ui, "P", "Prim markers");
                keybinding_row(ui, "B", "Skeleton bones");
                keybinding_row(ui, "Y", "Physics gizmos");
                keybinding_row(ui, "C", "Collider wireframes");
            });
            pane.section("keys_stage", "Stage", true, |ui| {
                keybinding_row(ui, "R", "Reload active stage");
                keybinding_row(ui, "Esc", "Clear selection");
            });
        },
    );
    let _ = accent_col;
}

// ─── Log panel ──────────────────────────────────────────────────────

fn draw_log_panel(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    accent: Res<AccentColor>,
    log: Option<Res<crate::viewer::log_panel::LoaderLog>>,
) {
    if !is_panel_open(&open, RIB_LOG) {
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
        RIB_LOG,
        "Log",
        egui::vec2(PANEL_W + 80.0, PANEL_H),
        &mut keep,
        accent_col,
        |pane| {
            pane.section("log_lines", "Loader log", true, |ui| {
                let Some(log) = log.as_deref() else {
                    sub_caption(ui, "(LoaderLog plugin not active)");
                    return;
                };
                let count = log.buffer.lock().map(|b| b.len()).unwrap_or(0);
                sub_caption(ui, &format!("{count} entries · capped at 500"));
                ui.horizontal(|ui| {
                    if ui.small_button("Clear").clicked()
                        && let Ok(mut buf) = log.buffer.lock()
                    {
                        buf.clear();
                    }
                });
                ui.add_space(style::space::TIGHT);

                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        let snapshot: Vec<crate::viewer::log_panel::LogLine> = log
                            .buffer
                            .lock()
                            .map(|b| b.iter().cloned().collect())
                            .unwrap_or_default();
                        if snapshot.is_empty() {
                            sub_caption(ui, "(no events yet — load a stage)");
                            return;
                        }
                        for line in &snapshot {
                            let level_color = level_to_color(line.level);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                ui.painter().rect_filled(
                                    egui::Rect::from_center_size(
                                        ui.cursor().min + egui::vec2(4.0, 8.0),
                                        egui::vec2(6.0, 6.0),
                                    ),
                                    egui::CornerRadius::same(1),
                                    level_color,
                                );
                                ui.add_space(10.0);
                                ui.label(
                                    egui::RichText::new(short_target(&line.target))
                                        .small()
                                        .monospace()
                                        .color(style::TEXT_SECONDARY),
                                );
                                ui.label(
                                    egui::RichText::new(&line.message)
                                        .small()
                                        .color(style::TEXT_PRIMARY),
                                );
                            });
                        }
                    });
            });
        },
    );
}

fn level_to_color(level: bevy::log::Level) -> egui::Color32 {
    match level {
        bevy::log::Level::ERROR => style::DANGER,
        bevy::log::Level::WARN => style::WARNING,
        bevy::log::Level::INFO => style::SUCCESS,
        _ => style::TEXT_SECONDARY,
    }
}

fn short_target(target: &str) -> String {
    target.rsplit("::").next().unwrap_or(target).to_string()
}

// ─── Command palette ───────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn draw_palette_panel(
    mut contexts: EguiContexts,
    accent: Res<AccentColor>,
    mut palette: ResMut<ViewerCommandPalette>,
    mut ribbon: ResMut<RibbonOpen>,
    mut toggles: ResMut<DisplayToggles>,
    mut reload: ResMut<ReloadRequest>,
    mut load_req: ResMut<LoadRequest>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let Some(id) = command_palette(ctx, &mut palette.0, PALETTE_ITEMS, accent.0) else {
        return;
    };
    match id {
        "open_selection" => {
            ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_SELECTION);
        }
        "open_tree" => {
            ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_TREE);
        }
        "open_info" => {
            ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_INFO);
        }
        "open_cameras" => {
            ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_CAMERAS);
        }
        "open_controllers" => {
            ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_CONTROLLERS);
        }
        "open_overlays" => {
            ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_OVERLAYS);
        }
        "open_timeline" => {
            ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_TIMELINE);
        }
        "open_keys" => {
            ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_KEYS);
        }
        "open_log" => {
            ribbon.per_ribbon.insert(RIBBON_LEFT, RIB_LOG);
        }
        "toggle_grid" => {
            toggles.show_world_grid = !toggles.show_world_grid;
        }
        "toggle_axes" => {
            toggles.show_world_axes = !toggles.show_world_axes;
        }
        "toggle_markers" => {
            toggles.show_prim_markers = !toggles.show_prim_markers;
        }
        "toggle_wireframe" => {
            toggles.wireframe = !toggles.wireframe;
        }
        "reload_stage" => {
            reload.requested = true;
        }
        "browse_usd" => {
            if let Some(picked) = rfd::FileDialog::new()
                .add_filter("USD stages", &["usda", "usdc", "usd", "usdz"])
                .pick_file()
            {
                load_req.path = Some(PathBuf::from(picked));
            }
        }
        _ => {}
    }
    palette.0.open = false;
}
