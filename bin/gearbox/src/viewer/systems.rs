//! Bevy-side viewer systems: what the scene does on its own every frame
//! (selection ring, follow and fly cameras, stage info capture, loads,
//! despawns, the stage clock). The panes that drive them live in the mara
//! host and reach the world through `HostCommands` and direct resource
//! access; nothing here draws UI.

use bevy::ecs::hierarchy::Children;
use bevy::prelude::*;
use mara::ui::modules::bevy::{BevyViewportInput, ChaseCamera, apply_rig};
use usd_bevy::UsdPrimRef;
use usd_bevy::instance::UsdInstances;

use crate::controller::{ControllerInventory, ControllerKey, ControllerStates};
use crate::load::{LoadQueue, LoadedAsset};
use crate::viewer::commands::{HostCommands, apply_host_commands};
use crate::viewer::state::{
    ActiveStage, ActiveVariants, CameraBookmarks, CameraMount, ChaseCameraFly, FlyTarget, FlyTo,
    FollowTarget, LoadRequest, LoaderTuning, ReloadRequest, SelectedPrim, StageInfo,
    UsdStageTime, VariantEntry,
};

/// Top-level entity selection (a LoadedAsset root). Separate from
/// `SelectedPrim`, which targets prims inside the active stage.
#[derive(Resource, Default)]
pub struct Selection(pub Option<Entity>);

/// The pose gizmo holds the pointer: the camera does not orbit and a click
/// does not reselect.
#[derive(Resource, Default)]
pub struct GizmoGrab(pub bool);

#[derive(Resource, Debug, Clone)]
pub struct SelectionRing {
    pub anchor: Option<Vec3>,
    pub outer_radius: f32,
    pub color: Color,
}

impl Default for SelectionRing {
    fn default() -> Self {
        Self {
            anchor: None,
            outer_radius: 1.0,
            color: Color::srgb(0.9, 0.9, 0.95),
        }
    }
}

/// LoadedAsset roots to despawn with their rapier bodies; drained each frame.
#[derive(Resource, Default)]
pub struct PendingDespawn(pub Vec<Entity>);

pub struct ViewerSystemsPlugin;

impl Plugin for ViewerSystemsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Selection>()
            .init_resource::<SelectionRing>()
            .init_resource::<GizmoGrab>()
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
            .init_resource::<ActiveVariants>()
            .init_resource::<CameraBookmarks>()
            .init_resource::<FollowTarget>()
            .init_resource::<ChaseCameraFly>()
            .init_resource::<HostCommands>()
            .add_systems(
                PostUpdate,
                follow_target
                    .after(crate::physics::PhysicsWriteback)
                    .after(crate::world::chase_camera_floor)
                    .before(bevy::transform::TransformSystems::Propagate),
            )
            .add_systems(
                Update,
                (
                    apply_host_commands,
                    mirror_api_selection,
                    pick_on_click,
                    rebase_loaded_assets_on_pause,
                    drive_selection_ring,
                    draw_selection_ring,
                    drain_despawn,
                    auto_set_active_stage,
                    capture_active_stage_info,
                    apply_load_request,
                    apply_reload_request,
                    apply_fly_to,
                    chase_camera_fly,
                    draw_selected_prim_highlight,
                    tick_stage_time,
                )
                    .chain(),
            );
    }
}

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

/// Latch `StageInfo` and the variant sets from the active root's live stage
/// when it is projected or the active stage changes. Counts come from one
/// walk of the stage; what usd_bevy does not expose stays zero.
fn capture_active_stage_info(
    active: Res<ActiveStage>,
    loaded: Query<&LoadedAsset>,
    instances: Option<NonSend<UsdInstances>>,
    mut info: ResMut<StageInfo>,
    mut variants: ResMut<ActiveVariants>,
    mut last_active: Local<Option<Entity>>,
    mut last_was_loaded: Local<bool>,
) {
    let Some(entity) = active.0 else {
        if last_active.is_some() {
            *info = StageInfo::default();
            *variants = ActiveVariants::default();
            *last_active = None;
            *last_was_loaded = false;
        }
        return;
    };
    let Ok(la) = loaded.get(entity) else {
        return;
    };
    let active_changed = *last_active != Some(entity);
    let stage = instances.as_ref().and_then(|instances| instances.stage(entity));
    let now_loaded = stage.is_some();
    if !active_changed && *last_was_loaded == now_loaded {
        return;
    }
    *last_active = Some(entity);
    *last_was_loaded = now_loaded;

    *info = StageInfo {
        path: la.path.display().to_string(),
        ..Default::default()
    };
    *variants = ActiveVariants {
        root: Some(entity),
        entries: Vec::new(),
    };
    let Some(stage) = stage else {
        return;
    };
    use crate::usd_ext::StageExt;
    info.default_prim = stage.default_prim().map(|t| t.as_str().to_string());
    info.layer_count = stage.layer_stack().len();
    let _ = stage.traverse(Default::default(), |path: &openusd::sdf::Path| {
        if let Ok(Some(ty)) = stage.type_name(path) {
            if ty == "PhysicsScene" {
                info.physics_scene_count += 1;
            } else if ty.starts_with("Physics") && ty.ends_with("Joint") {
                info.joint_count += 1;
            }
        }
        if let Ok(schemas) = stage.api_schemas(path)
            && schemas.iter().any(|s| s == "PhysicsRigidBodyAPI")
        {
            info.rigid_body_count += 1;
        }
        if let Ok(prim) = stage.prim(path.clone())
            && let Ok(sets) = prim.variant_sets().get_all_variant_selections()
        {
            for (name, selection) in sets {
                let options = usd_bevy::read::variants::variant_options(stage, path, &name);
                variants.entries.push(VariantEntry {
                    prim: path.as_str().to_string(),
                    name,
                    selection: Some(selection),
                    options,
                });
            }
        }
    });
    info.variant_count = variants.entries.len();
}

/// The active root's live stage, once usd_bevy has projected it.
fn active_stage<'a>(
    active: &ActiveStage,
    instances: &'a UsdInstances,
) -> Option<&'a openusd::usd::Stage> {
    instances.stage(active.0?)
}

// ─── Click in viewport → set Selection (top-level) ─────────────────


/// Walk up `ChildOf` parents until we find an entity carrying
/// `LoadedAsset`, or run out of ancestors.
pub(crate) fn find_loaded_ancestor(
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

/// World-space bounding sphere of everything under a loaded asset.
pub(crate) fn asset_bounds(
    root: Entity,
    parents: &Query<&ChildOf>,
    loaded: &Query<Entity, With<LoadedAsset>>,
    aabbs: &Query<(Entity, &GlobalTransform, &bevy::camera::primitives::Aabb)>,
) -> Option<(Vec3, f32)> {
    let mut wmin = Vec3::splat(f32::INFINITY);
    let mut wmax = Vec3::splat(f32::NEG_INFINITY);
    for (e, gt, aabb) in aabbs.iter() {
        if find_loaded_ancestor(e, parents, loaded) != Some(root) {
            continue;
        }
        let m = gt.to_matrix();
        let c = Vec3::from(aabb.center);
        let h = Vec3::from(aabb.half_extents);
        for sx in [-1.0, 1.0] {
            for sy in [-1.0, 1.0] {
                for sz in [-1.0, 1.0] {
                    let p = m.transform_point3(c + Vec3::new(sx, sy, sz) * h);
                    wmin = wmin.min(p);
                    wmax = wmax.max(p);
                }
            }
        }
    }
    if !wmin.is_finite() || !wmax.is_finite() {
        return None;
    }
    Some(((wmin + wmax) * 0.5, (wmax - wmin).length() * 0.5))
}

/// Pin the camera focus to the current body position without overriding zoom.
fn follow_target(
    mut follow: ResMut<FollowTarget>,
    commands: Res<HostCommands>,
    fly: Res<FlyTo>,
    agent_fly: Res<ChaseCameraFly>,
    inventory: Res<ControllerInventory>,
    prims: Query<(Entity, &UsdPrimRef)>,
    parents: Query<&ChildOf>,
    mut camera: ParamSet<(
        bevy::transform::helper::TransformHelper,
        Query<(&mut ChaseCamera, &mut Transform)>,
    )>,
) {
    let Some(root) = follow.entity else {
        return;
    };
    if fly.remaining > 0.0
        || agent_fly.target.is_some()
        || commands.0.iter().any(|command| {
            matches!(command,
                crate::viewer::commands::HostCommand::FlyToMachine(target) if *target == root)
        })
    {
        return;
    }
    let body = machine_body_entity(root, &inventory, &prims, &parents);
    let Ok(gt) = camera.p0().compute_global_transform(body) else {
        follow.set(None);
        return;
    };
    let current = gt.translation();
    for (mut cam, mut tr) in &mut camera.p1() {
        cam.focus = current;
        apply_rig(&cam, &mut tr);
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
    physics: Res<gearbox_api::PhysicsActive>,
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
}

fn draw_selection_ring(ring: Res<SelectionRing>, mut gizmos: Gizmos) {
    let Some(anchor) = ring.anchor else {
        return;
    };
    let iso = Isometry3d::new(anchor, Quat::from_rotation_x(core::f32::consts::FRAC_PI_2));
    gizmos
        .circle(iso, ring.outer_radius.max(0.05), ring.color)
        .resolution(96);
}

/// On the ON→OFF edge of `PhysicsActive`, rebase each LoadedAsset's
/// root entity to its current world AABB centroid (XZ only; Y stays
/// at 0 / ground), and compensate the immediate children by `-Δ` so
/// the visuals don't jump. Without this, the gizmo (which targets
/// the root) sits at the asset's original mount point even though
/// the simulated robot drove away during play.
fn rebase_loaded_assets_on_pause(
    active: Res<gearbox_api::PhysicsActive>,
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

fn drain_despawn(
    mut commands: Commands,
    mut queue: ResMut<PendingDespawn>,
    world: ResMut<crate::physics::PhysicsWorld>,
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

fn apply_fly_to(
    time: Res<Time>,
    mut fly: ResMut<FlyTo>,
    mut cameras: Query<(&mut ChaseCamera, &mut Transform)>,
) {
    if fly.remaining <= 0.0 {
        return;
    }
    let Ok((mut cam, mut transform)) = cameras.single_mut() else {
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
    apply_rig(&cam, &mut transform);
}

/// The chassis prim entity of the machine rooted at `root`, or the root
/// itself when the machine names no body.
pub(crate) fn machine_body_entity(
    root: Entity,
    inventory: &ControllerInventory,
    prims: &Query<(Entity, &UsdPrimRef)>,
    parents: &Query<&ChildOf>,
) -> Entity {
    inventory
        .machines
        .iter()
        .find(|m| m.scene_root == Some(root))
        .and_then(|m| m.body.as_deref())
        .and_then(|body| crate::controller::find_prim_entity(root, body, prims, parents))
        .unwrap_or(root)
}

/// World heading of a machine body: the published controller state when
/// there is one, else the chassis prim's forward axis (USD -Y) projected on
/// the ground. 0 rad = +Z, +pi/2 = +X, like `heading_rad`.
pub(crate) fn machine_heading(
    root: Entity,
    gt: &GlobalTransform,
    inventory: &ControllerInventory,
    states: &ControllerStates,
) -> f32 {
    let published = inventory
        .machines
        .iter()
        .find(|m| m.scene_root == Some(root))
        .and_then(|m| {
            m.controllers.iter().find_map(|c| {
                states
                    .states
                    .get(&ControllerKey::new(root, &m.id, &c.instance))
                    .map(|s| s.heading_rad as f32)
            })
        });
    published.unwrap_or_else(|| {
        let fwd = gt.rotation() * Vec3::NEG_Y;
        fwd.x.atan2(fwd.z)
    })
}

// ─── Fly behind a machine (agent tree double-click) ────────────────

/// Two phases over `FlyTarget::duration`: A (0–30 %) keeps the camera where
/// it is and turns it toward the machine; B pulls back to the apex, arcs the
/// yaw to behind the machine and comes in to the final distance while the
/// elevation eases back to what the user had. Any camera input cancels it.


fn sub_progress(t: f32, a: f32, b: f32) -> f32 {
    ((t - a) / (b - a)).clamp(0.0, 1.0)
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
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
    instances: Option<NonSend<UsdInstances>>,
) {
    if !clock.initialized
        && let Some(stage) = instances.as_deref().and_then(|i| active_stage(&active, i))
    {
        clock.start_time_code = stage.start_time_code();
        clock.end_time_code = stage.end_time_code();
        clock.time_codes_per_second = stage.time_codes_per_second().max(1e-6);
        clock.seconds = 0.0;
        clock.playing = clock.end_time_code > clock.start_time_code;
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


pub(crate) fn fit_params_for_entity(
    root: Entity,
    gt_q: &Query<&GlobalTransform>,
    extent_q: &Query<&bevy::camera::primitives::Aabb>,
    children: &Query<&Children>,
    current_cam_dist: f32,
) -> (Vec3, f32) {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut found = false;

    let mut stack: Vec<Entity> = vec![root];
    while let Some(e) = stack.pop() {
        if let (Ok(gt), Ok(aabb)) = (gt_q.get(e), extent_q.get(e)) {
            let m = gt.to_matrix();
            let lo = Vec3::from(aabb.center) - Vec3::from(aabb.half_extents);
            let hi = Vec3::from(aabb.center) + Vec3::from(aabb.half_extents);
            for i in 0..8 {
                let c = Vec3::new(
                    if i & 1 == 0 { lo.x } else { hi.x },
                    if i & 2 == 0 { lo.y } else { hi.y },
                    if i & 4 == 0 { lo.z } else { hi.z },
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

// ─── Click in viewport → set Selection (top-level) ─────────────────

/// A primary click on the viewport (mara forwards it with the pointer in
/// render-target pixels) picks the closest loaded asset under the cursor.
pub(crate) fn pick_on_click(
    input: Res<BevyViewportInput>,
    grab: Res<GizmoGrab>,
    keys: Res<ButtonInput<KeyCode>>,
    cameras: Query<(&Camera, &GlobalTransform), With<ChaseCamera>>,
    loaded: Query<Entity, With<LoadedAsset>>,
    parents: Query<&ChildOf>,
    aabbs: Query<(Entity, &GlobalTransform, &bevy::camera::primitives::Aabb)>,
    mut selection: ResMut<Selection>,
    mut active: ResMut<ActiveStage>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        selection.0 = None;
    }
    if !input.primary_clicked || grab.0 {
        return;
    }
    let Some(cursor) = input.pointer_pos else {
        return;
    };
    let Ok((camera, cam_tr)) = cameras.single() else {
        return;
    };
    let Ok(ray) = camera.viewport_to_world(cam_tr, Vec2::new(cursor[0], cursor[1])) else {
        return;
    };
    let origin = ray.origin;
    let dir = *ray.direction;
    let mut best: Option<(Entity, f32)> = None;
    for (e, gt, aabb) in aabbs.iter() {
        let Some(root) = find_loaded_ancestor(e, &parents, &loaded) else {
            continue;
        };
        if let Some(t) = ray_aabb_world(origin, dir, gt, aabb)
            && best.is_none_or(|(_, bt)| t < bt)
        {
            best = Some((root, t));
        }
    }
    if let Some((root, t)) = best {
        info!("pick: hit LoadedAsset {root:?} at t={t:.3}");
        selection.0 = Some(root);
        active.0 = Some(root);
    }
}

// ─── Fly behind a machine (agent tree double-click) ────────────────

/// Two phases over `FlyTarget::duration`: A (0–30 %) keeps the camera where
/// it is and turns it toward the machine; B pulls back to the apex, arcs the
/// yaw to behind the machine and comes in to the final distance while the
/// elevation eases back to what the user had. Any camera input cancels it.
fn chase_camera_fly(
    time: Res<Time>,
    input: Res<BevyViewportInput>,
    mut fly: ResMut<ChaseCameraFly>,
    inventory: Res<ControllerInventory>,
    states: Res<ControllerStates>,
    transforms: Query<&GlobalTransform>,
    mut cameras: Query<(&mut ChaseCamera, &mut Transform)>,
) {
    let Some(mut target) = fly.target else {
        return;
    };
    let dragged = input.drag_delta != [0.0, 0.0] || input.pan_delta != [0.0, 0.0];
    if dragged || input.scroll_delta.abs() > f32::EPSILON {
        fly.target = None;
        return;
    }
    let Ok(gt) = transforms.get(target.body) else {
        fly.target = None;
        return;
    };
    let Ok((mut cam, mut tr)) = cameras.single_mut() else {
        return;
    };

    target.elapsed += time.delta_secs();
    let t = (target.elapsed / target.duration).clamp(0.0, 1.0);

    let target_focus = gt.translation();
    let target_cam_yaw =
        machine_heading(target.root, gt, &inventory, &states) + std::f32::consts::PI;

    // Slide the start references with the machine so a driving target does
    // not shift the geometry under the interpolation.
    if let Some(last) = target.last_target_pos {
        let delta = target_focus - last;
        target.start_focus += delta;
        target.start_cam_world += delta;
    }
    target.last_target_pos = Some(target_focus);

    let phase_a_end = FlyTarget::PHASE_A_END;
    if t < phase_a_end {
        let s = smoothstep((t / phase_a_end).clamp(0.0, 1.0));
        let focus = target.start_focus.lerp(target_focus, s);
        let off = target.start_cam_world - focus;
        let dist = off.length().max(0.1);
        cam.focus = focus;
        cam.distance = dist;
        cam.yaw = off.x.atan2(off.z);
        cam.elevation = (off.y / dist).clamp(-1.0, 1.0).asin();
    } else {
        let tb = ((t - phase_a_end) / (1.0 - phase_a_end)).clamp(0.0, 1.0);
        let off = target.start_cam_world - target_focus;
        let dist_b_start = off.length().max(0.1);
        let yaw_b_start = off.x.atan2(off.z);
        let elev_b_start = (off.y / dist_b_start).clamp(-1.0, 1.0).asin();

        let distance = if tb < 0.5 {
            let s = smoothstep(sub_progress(tb, 0.0, 0.5));
            dist_b_start + (target.apex_distance - dist_b_start) * s
        } else {
            let s = smoothstep(sub_progress(tb, 0.5, 1.0));
            target.apex_distance + (target.distance - target.apex_distance) * s
        };
        let yaw = lerp_angle(
            yaw_b_start,
            target_cam_yaw,
            smoothstep(sub_progress(tb, 0.0, 0.85)),
        );
        let elevation = elev_b_start + (target.start_elevation - elev_b_start) * smoothstep(tb);

        cam.focus = target_focus;
        cam.yaw = yaw;
        cam.distance = distance;
        cam.elevation = elevation;
    }

    if t >= 1.0 {
        cam.focus = target_focus;
        cam.yaw = target_cam_yaw;
        cam.distance = target.distance;
        cam.elevation = target.start_elevation;
        fly.target = None;
    } else {
        fly.target = Some(target);
    }
    apply_rig(&cam, &mut tr);
}

/// Start a short fly of the chase camera to `focus` at `distance`.
pub(crate) fn start_fly_to(fly: &mut FlyTo, cam: &ChaseCamera, focus: Vec3, distance: f32) {
    fly.start_focus = cam.focus;
    fly.start_distance = cam.distance;
    fly.target_focus = focus;
    fly.target_distance = distance;
    fly.start_yaw = None;
    fly.target_yaw = None;
    fly.start_elevation = None;
    fly.target_elevation = None;
    fly.duration = 0.4;
    fly.remaining = 0.4;
}

// ─── Bus selection → viewer selection ──────────────────────────────

/// `gearbox select machine NS` (and `object ID`) reaches the viewer through
/// `SelectionState`; apply it once and clear the flag.
fn mirror_api_selection(
    mut state: ResMut<gearbox_api::SelectionState>,
    inventory: Res<ControllerInventory>,
    loaded: Query<(Entity, &LoadedAsset)>,
    mut selection: ResMut<Selection>,
    mut active: ResMut<ActiveStage>,
) {
    if !state.from_api {
        return;
    }
    state.from_api = false;
    if state.id.is_empty() {
        selection.0 = None;
        return;
    }
    let root = inventory
        .machines
        .iter()
        .find(|m| m.id == state.id)
        .and_then(|m| m.scene_root)
        .or_else(|| {
            loaded
                .iter()
                .find(|(_, asset)| asset.label == state.id)
                .map(|(entity, _)| entity)
        });
    if let Some(root) = root {
        selection.0 = Some(root);
        active.0 = Some(root);
    }
}
