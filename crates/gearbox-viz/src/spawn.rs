//! Spawn helpers for turning a `VehicleSpec` into Bevy entities.
//!
//! - Chassis is the root; each wheel is a top-level sibling (rapier's
//!   vehicle controller computes wheel poses in world space directly).
//! - Body **parts** (hitches, karosseries, tanks) are children of the
//!   chassis — they have a fixed local offset, so Bevy's transform
//!   propagation keeps them glued to the chassis for free.

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::math::Affine2;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use datapod::Size;
use gearbox_core::{MeshSource, PartKind, VehicleId, VehicleSpec};

use super::{ChassisTinted, VehicleBody, VehicleWheel};

/// Target physical arc-length of one "^" stripe on the tyre (metres).
/// Using a fixed arc length rather than a fixed count means every
/// wheel, big or small, shows roughly the same-sized chevron blocks.
const TYRE_STRIPE_ARC_M: f32 = 0.40;

/// Marker for all meshes that make up the currently-dragging ghost
/// spawn preview. Despawning the tagged root with `despawn_recursive`
/// removes every child mesh as well.
#[derive(Component)]
pub struct GhostTag;

/// Filesystem root that the Bevy `AssetServer` resolves asset paths
/// against — i.e. the `AssetPlugin.file_path`. Set from the binary's
/// `main.rs` so `gearbox-viz` doesn't have to guess.
///
/// Used by [`instantiate_pending_usd_scenes`] to compute the asset's
/// real filesystem parent for `bevy_openusd`'s `search_paths` so that
/// sibling references (`./tractor.usdc`) resolve.
#[derive(bevy::ecs::resource::Resource, Clone)]
pub struct UsdAssetRoot(pub std::path::PathBuf);

/// Tag carried by an entity that wants to load + instantiate a USD
/// asset. The deferred system [`instantiate_pending_usd_scenes`]
/// drives it through three states using a single `Option` field:
///
///   1. `handle: None` — load not yet started; on next frame the
///      system calls `asset_server.load_with_settings(...)` with
///      `search_paths` derived from [`UsdAssetRoot`] + the asset's
///      filesystem parent so sibling refs resolve.
///   2. `handle: Some(h)` and `LoadState::Loading` — asset still
///      streaming; system polls.
///   3. `handle: Some(h)` and `LoadState::Loaded` — system reads
///      `UsdAsset.scene`, attaches `SceneRoot` to the entity, and
///      removes the marker.
#[derive(Component)]
pub struct PendingUsdScene {
    pub asset_path: String,
    pub name: String,
    pub handle: Option<Handle<usd_bevy::UsdAsset>>,
}

/// One pending USD-wheel installation. We can't act on it until the
/// scene is projected (the prim entity exists with a `UsdPrimRef`).
/// Once we find it, we *don't* detach the wheel from the chassis —
/// instead we leave the USD scene's hierarchy intact and just drive
/// the wheel-prim's *local* rotation each frame, mirroring the
/// `urdf2usd` viewer's pattern. That way:
///   * The chassis pose propagates to the wheels via Bevy's transform
///     hierarchy automatically — no scrambling of authored offsets.
///   * The wheel's authored translate stays correct (the wheel hub
///     position is whatever the USDA placed it at).
///   * Spin and steer are added as small local rotations around the
///     wheel-prim's *local* axle / vertical axes — and those local
///     axes vary per preset depending on the asset's authored frame.
#[derive(Clone, Debug)]
pub struct PendingUsdWheelTag {
    pub vehicle_id: gearbox_core::VehicleId,
    pub wheel_index: usize,
    pub prim_path: String,
    /// Wheel-prim local axis that becomes the *axle* (gearbox-X
    /// lateral) once the chassis identity-rotates. Computed from
    /// `ChassisSpec.usd_scene_rotation` × `bevy_openusd`'s internal
    /// Z↔Y flip.
    pub spin_axis: Vec3,
    /// Wheel-prim local axis that becomes the *vertical* (gearbox-Y
    /// up) once the chassis identity-rotates. Used for steering.
    pub steer_axis: Vec3,
    /// Apply rapier spin angle to this prim's rotation each frame.
    pub apply_spin: bool,
    /// Apply rapier steer angle to this prim's rotation each frame.
    pub apply_steer: bool,
}

/// Component on the chassis entity that lists which USD-projected
/// prims still need to be promoted into driven wheels. Entries are
/// removed as they're satisfied; the component is removed when the
/// list empties.
#[derive(Component)]
pub struct PendingUsdWheelTags(pub Vec<PendingUsdWheelTag>);

/// Marker + state on a USD-projected wheel prim entity. Driven each
/// frame by [`drive_usd_wheels`].
#[derive(Component)]
pub struct UsdWheelDriver {
    pub vehicle_id: gearbox_core::VehicleId,
    pub wheel_index: usize,
    /// Snapshotted local Transform at install time (the authored rest
    /// pose: chassis-local hub position + identity rotation, scaled if
    /// applicable). We re-derive `Transform` each frame as
    /// `rest * R_steer * R_spin` so the rest pose is exactly
    /// preserved when both angles are zero.
    pub rest_local: Transform,
    /// Per-wheel local-frame axle direction — depends on the asset's
    /// authored conventions plus `ChassisSpec.usd_scene_rotation`.
    pub spin_axis: Vec3,
    /// Per-wheel local-frame vertical direction (steering pivot).
    pub steer_axis: Vec3,
    /// Whether to apply the wheel's *spin* angle (rolling) on this
    /// entity. False on the steering knuckle of a split layout (the
    /// knuckle only steers; the spin lives on the inner wheel prim).
    pub apply_spin: bool,
    /// Whether to apply the wheel's *steer* angle (kingpin rotation)
    /// on this entity. False on the inner wheel of a split layout
    /// (the wheel only spins).
    pub apply_steer: bool,
}

/// One-shot debug system: when *new* `UsdPrimRef` entities appear,
/// log their world position + world rotation. Helps diagnose
/// orientation bugs (e.g. "is this husky actually upside down?")
/// by giving the actual rendered axes for the asset's named parts.
///
/// Tracks which paths have already been logged in a `Local<HashSet>`
/// so each entity only fires once.
pub fn debug_log_new_usd_prims(
    prims: Query<(&usd_bevy::UsdPrimRef, &GlobalTransform), Added<usd_bevy::UsdPrimRef>>,
    mut seen: Local<std::collections::HashSet<String>>,
) {
    // Names that signal "vertical" or "front" so we can sanity-check
    // axis orientation at a glance. Limit logging to those plus the
    // root prims to keep the noise manageable.
    let interesting = [
        "base_link",
        "wheel_link",
        "top_chassis",
        "top_plate",
        "front_bumper",
        "rear_bumper",
        "imu_link",
        "user_rail",
        "robotti",
        "husky",
        "tractor",
    ];
    for (pr, gt) in prims.iter() {
        if seen.contains(&pr.path) {
            continue;
        }
        let interesting_match = interesting.iter().any(|kw| pr.path.contains(kw));
        if !interesting_match {
            continue;
        }
        seen.insert(pr.path.clone());
        let (s, r, t) = gt.to_scale_rotation_translation();
        let (axis, angle) = r.to_axis_angle();
        bevy::log::info!(
            "USD-PRIM `{}` world: t=({:+.3}, {:+.3}, {:+.3}) rot=axis({:.3},{:.3},{:.3})·angle={:+.1}° scale=({:.3},{:.3},{:.3})",
            pr.path,
            t.x, t.y, t.z,
            axis.x, axis.y, axis.z, angle.to_degrees(),
            s.x, s.y, s.z,
        );
    }
}

/// Per-frame system: walks every `UsdPrimRef` entity, resolves any
/// outstanding wheel-tagging requests, snapshots each matched entity's
/// current local Transform as its rest pose, and inserts
/// [`UsdWheelDriver`]. After this runs, [`drive_usd_wheels`] takes
/// over for that entity.
/// True if `entity` is a descendant of `target` (inclusive — `entity
/// == target` returns true). Walks the `ChildOf` chain up to a small
/// depth limit so a malformed cycle can't hang the system.
fn is_descendant_of(
    mut entity: Entity,
    target: Entity,
    parents: &Query<&ChildOf>,
) -> bool {
    for _ in 0..64 {
        if entity == target {
            return true;
        }
        let Ok(parent) = parents.get(entity) else {
            return false;
        };
        entity = parent.parent();
    }
    false
}

pub fn tag_usd_wheels_when_ready(
    mut commands: Commands,
    mut chassis_q: Query<(Entity, &mut PendingUsdWheelTags)>,
    prims: Query<(Entity, &usd_bevy::UsdPrimRef, &Transform)>,
    parents: Query<&ChildOf>,
) {
    for (chassis_entity, mut pending) in chassis_q.iter_mut() {
        pending.0.retain(|tag| {
            // `UsdPrimRef.path` is the asset's authored prim path
            // (e.g. `/robot/wheel_back_left`), which is identical for
            // every instance of the same preset. Filtering by path
            // alone — as we used to — picks an arbitrary prim from
            // some OTHER vehicle in the scene, so the wheel-driver
            // installed under chassis B ends up reading rapier wheel
            // state for chassis A. Scoping by descendants of THIS
            // chassis entity is what guarantees we tag the wheel
            // belonging to the vehicle we're currently processing.
            let Some((wheel_entity, _, current)) =
                prims.iter().find(|(prim_entity, pr, _)| {
                    pr.path == tag.prim_path
                        && is_descendant_of(*prim_entity, chassis_entity, &parents)
                })
            else {
                return true; // not yet projected — try again next frame
            };
            bevy::log::info!(
                "gearbox-viz: USD wheel `{}` → UsdWheelDriver {{ id: {:?}, index: {} }} (rest={:?})",
                tag.prim_path, tag.vehicle_id, tag.wheel_index, current.translation,
            );
            commands.entity(wheel_entity).insert(UsdWheelDriver {
                vehicle_id: tag.vehicle_id,
                wheel_index: tag.wheel_index,
                rest_local: *current,
                spin_axis: tag.spin_axis,
                steer_axis: tag.steer_axis,
                apply_spin: tag.apply_spin,
                apply_steer: tag.apply_steer,
            });
            false // drop — done
        });
        if pending.0.is_empty() {
            commands.entity(chassis_entity).remove::<PendingUsdWheelTags>();
        }
    }
}

/// Per-frame system: for every `UsdWheelDriver`, build the wheel
/// prim's local Transform as `rest * R_steer(Z) * R_spin(X)`.
///
/// Axis mapping notes — the wheel-prim is a child of the SceneRoot,
/// which `bevy_openusd` rotated by `rot_x(-π/2)` to flip USD's Z-up
/// to bevy's Y-up. So the wheel-prim's *local* axes (as seen by
/// `Transform`) map to world directions like this when the chassis
/// is unrotated:
///   * local-X → world-X (lateral / axle)
///   * local-Y → world-Z (USD's "back" axis)
///   * local-Z → world-Y (USD's up axis = vertical)
///
/// Therefore:
///   * **Steer** (around the suspension/kingpin axis = vertical)
///     ⇒ rotation around **local-Z**.
///   * **Spin** (around the axle = lateral)
///     ⇒ rotation around **local-X**.
///
/// `R_spin` is negated to match the procedural path's convention
/// (`-wh.rotation` in `Sim::wheel_pose`) so the tread rolls with
/// motion rather than against it.
pub fn drive_usd_wheels(
    sim: Res<crate::GearboxSim>,
    mut q: Query<(&UsdWheelDriver, &mut Transform)>,
    mut frame: Local<u32>,
) {
    *frame = frame.wrapping_add(1);
    let log_now = *frame % 60 == 0; // ~once per second at 60Hz
    for (drv, mut tr) in q.iter_mut() {
        let spin = if drv.apply_spin {
            sim.0.wheel_spin_angle(drv.vehicle_id, drv.wheel_index) as f32
        } else {
            0.0
        };
        let steer = if drv.apply_steer {
            sim.0.wheel_steering_angle(drv.vehicle_id, drv.wheel_index) as f32
        } else {
            0.0
        };
        if log_now && drv.wheel_index == 2 {
            // Once per second, log the rear-left wheel of every vehicle
            // so we can confirm rapier's wheel.rotation accumulates.
            bevy::log::info!(
                "DRIVE veh={:?} wheel={} apply_spin={} apply_steer={} spin={:.3} steer={:.3} spin_axis={:?}",
                drv.vehicle_id, drv.wheel_index, drv.apply_spin, drv.apply_steer,
                spin, steer, drv.spin_axis,
            );
        }
        let r_steer = Quat::from_axis_angle(drv.steer_axis, steer);
        let r_spin = Quat::from_axis_angle(drv.spin_axis, spin);
        tr.translation = drv.rest_local.translation;
        tr.rotation = drv.rest_local.rotation * r_steer * r_spin;
        tr.scale = drv.rest_local.scale;
    }
}

/// Per-frame state machine for `PendingUsdScene` markers. See the
/// component doc-comment for the three states.
pub fn instantiate_pending_usd_scenes(
    mut commands: Commands,
    asset_server: Res<bevy::asset::AssetServer>,
    usd_assets: Res<bevy::asset::Assets<usd_bevy::UsdAsset>>,
    asset_root: Option<Res<UsdAssetRoot>>,
    mut pending: Query<(Entity, &mut PendingUsdScene)>,
) {
    use bevy::asset::LoadState;
    for (entity, mut pend) in pending.iter_mut() {
        if pend.handle.is_none() {
            // Kick off the load. Compute the real filesystem parent of
            // the asset (asset_root + asset's relative subdir) and pass
            // it as the loader's first search path so bevy_openusd's
            // tempfile lands next to the original — sibling references
            // resolve correctly.
            let Some(root) = asset_root.as_deref() else {
                bevy::log::warn!(
                    "gearbox-viz: UsdAssetRoot resource missing — cannot load `{}`",
                    pend.asset_path
                );
                continue;
            };
            let asset_parent = std::path::Path::new(&pend.asset_path)
                .parent()
                .map(|p| root.0.join(p))
                .unwrap_or_else(|| root.0.clone());
            bevy::log::info!(
                "gearbox-viz: kicking off USD load `{}` for `{}` (search_paths=[{}])",
                pend.asset_path, pend.name, asset_parent.display()
            );
            let parent_clone = asset_parent.clone();
            let handle: Handle<usd_bevy::UsdAsset> = asset_server
                .load_with_settings(
                    pend.asset_path.clone(),
                    move |s: &mut usd_bevy::UsdLoaderSettings| {
                        s.search_paths = vec![parent_clone.clone()];
                    },
                );
            pend.handle = Some(handle);
            continue;
        }
        let handle = pend.handle.as_ref().unwrap();
        match asset_server.get_load_state(handle) {
            Some(LoadState::Loaded) => {
                let Some(asset) = usd_assets.get(handle) else {
                    continue; // race: try again next frame
                };
                let scene_handle = asset.scene.clone();
                bevy::log::info!(
                    "gearbox-viz: USD scene for `{}` loaded — attaching SceneRoot (default_prim={:?}, layers={})",
                    pend.name, asset.default_prim, asset.layer_count,
                );
                commands
                    .entity(entity)
                    .insert(bevy::scene::SceneRoot(scene_handle))
                    .remove::<PendingUsdScene>();
            }
            Some(LoadState::Failed(err)) => {
                bevy::log::error!(
                    "gearbox-viz: USD asset load FAILED for `{}`: {}",
                    pend.name, err
                );
                commands.entity(entity).remove::<PendingUsdScene>();
            }
            _ => {}
        }
    }
}

/// Build a [`Mesh`] handle for a sized volume with a given
/// [`MeshSource`]. This is the sole dispatch point — adding USD /
/// glTF support later means adding a variant here, not touching
/// anything that spawns parts or chassis.
fn mesh_for(source: MeshSource, size: Size, meshes: &mut Assets<Mesh>) -> Handle<Mesh> {
    match source {
        MeshSource::Box => meshes.add(Cuboid::new(
            size.x as f32,
            size.y as f32,
            size.z as f32,
        )),
        MeshSource::Cylinder => meshes.add(
            Cylinder::new((size.x as f32) * 0.5, size.y as f32)
                .mesh()
                .resolution(24)
                .build(),
        ),
    }
}

pub fn spawn_vehicle_visuals(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    asset_server: &bevy::asset::AssetServer,
    id: VehicleId,
    spec: &VehicleSpec,
) -> Entity {
    let [r, g, b] = spec.chassis.color;
    let chassis_color = Color::srgb(r, g, b);

    // Root entity carries the physics body's pose. Only get a mesh
    // when the preset wants the chassis box drawn — gantry-style
    // vehicles (Robotti) suppress it since their silhouette is
    // entirely carried by `parts`.
    let mut root_cmd = commands.spawn((
        Name::new(spec.name.clone()),
        Transform::default(),
        VehicleBody { id },
    ));
    if spec.chassis.render_chassis {
        let chassis_mesh = mesh_for(spec.chassis.mesh, spec.chassis.size, meshes);
        let chassis_mat = materials.add(StandardMaterial {
            base_color: chassis_color,
            perceptual_roughness: 0.6,
            metallic: 0.1,
            ..default()
        });
        root_cmd.insert((
            Mesh3d(chassis_mesh),
            MeshMaterial3d(chassis_mat),
            ChassisTinted { id },
        ));
    }
    let root = root_cmd.id();

    // Optional USD scene. We follow the proven pattern from
    // bevy_openusd's own integration tests:
    //
    //   1. `asset_server.load::<UsdAsset>(path)` — the loader's
    //      *primary* asset type. (Loading `Handle<Scene>` directly
    //      via the `#Scene` label silently fails because the asset
    //      server resolves loaders by primary type, not label.)
    //   2. Stash the handle in a `PendingUsdScene` marker on a
    //      child entity.
    //   3. `instantiate_pending_usd_scenes` polls each frame and,
    //      once the `UsdAsset` is `Loaded`, copies its `.scene`
    //      handle into a `SceneRoot` and clears the marker.
    if let Some(path) = spec.chassis.usd_asset {
        // Strip a `#Scene` suffix if a preset still has one — the
        // bevy_openusd loader takes the bare path; the suffix is
        // only used at the labeled-asset layer (which we skip).
        let bare = path.split('#').next().unwrap_or(path).to_string();
        let offset = spec.chassis.usd_scene_offset;
        let rot = spec.chassis.usd_scene_rotation;
        let scene_transform = Transform {
            translation: Vec3::new(offset.x as f32, offset.y as f32, offset.z as f32),
            rotation: Quat::from_xyzw(rot.x as f32, rot.y as f32, rot.z as f32, rot.w as f32),
            scale: Vec3::ONE,
        };
        bevy::log::info!(
            "gearbox-viz: queuing USD asset `{}` for `{}` (scene_offset=({:.3}, {:.3}, {:.3}), scene_rotation=(w={:.3}, x={:.3}, y={:.3}, z={:.3}))",
            bare, spec.name, offset.x, offset.y, offset.z, rot.w, rot.x, rot.y, rot.z,
        );
        commands
            .spawn((
                Name::new(format!("{}::usd_scene", spec.name)),
                scene_transform,
                Visibility::default(),
                PendingUsdScene {
                    asset_path: bare,
                    name: spec.name.clone(),
                    handle: None,
                },
            ))
            .insert(ChildOf(root));

        // Queue any per-wheel USD-prim → VehicleWheel tagging. The
        // tagging system polls each frame for the projected prim
        // entities and finishes the job once the SceneRoot has had a
        // chance to instantiate.
        //
        // Pre-compute the wheel-local axle / vertical axes from the
        // composite scene rotation: `composite = scene_rot *
        // bevy_openusd_internal_flip`. The wheel-prim's local axes
        // for "lateral" and "up" are `composite.inverse() * world_X`
        // and `composite.inverse() * world_Y` respectively. These
        // become `spin_axis` (axle) and `steer_axis` (kingpin) in
        // [`UsdWheelDriver`].
        let bevy_flip = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
        let scene_rot_q = Quat::from_xyzw(
            rot.x as f32, rot.y as f32, rot.z as f32, rot.w as f32,
        );
        let composite_inv = (scene_rot_q * bevy_flip).inverse();
        let spin_axis = (composite_inv * Vec3::X).normalize();
        let steer_axis = (composite_inv * Vec3::Y).normalize();
        bevy::log::info!(
            "gearbox-viz: USD wheel axes for `{}`: spin={:?} steer={:?}",
            spec.name, spin_axis, steer_axis,
        );
        let mut wheel_tags: Vec<PendingUsdWheelTag> = Vec::new();
        for (idx, w) in spec.wheels.iter().enumerate() {
            let Some(spin_path) = w.usd_prim_path else {
                continue;
            };
            // When the preset declares a separate steering knuckle
            // (`usd_steer_prim_path`), the inner wheel-prim only
            // spins and the knuckle gets the steer rotation. When
            // it's not set (tractor layout) both rotations land on
            // the wheel-prim itself.
            let split_steer = w.usd_steer_prim_path.is_some();
            wheel_tags.push(PendingUsdWheelTag {
                vehicle_id: id,
                wheel_index: idx,
                prim_path: spin_path.to_string(),
                spin_axis,
                steer_axis,
                apply_spin: true,
                apply_steer: !split_steer,
            });
            if let Some(steer_path) = w.usd_steer_prim_path {
                wheel_tags.push(PendingUsdWheelTag {
                    vehicle_id: id,
                    wheel_index: idx,
                    prim_path: steer_path.to_string(),
                    spin_axis,
                    steer_axis,
                    apply_spin: false,
                    apply_steer: true,
                });
            }
        }
        if !wheel_tags.is_empty() {
            commands.entity(root).insert(PendingUsdWheelTags(wheel_tags));
        }
    } else {
        bevy::log::info!("gearbox-viz: spawning `{}` without USD asset", spec.name);
    }

    // Shared tread image — one repeat of the chevron block.  Each
    // wheel gets its own material below with a `uv_transform` that
    // tiles this image based on circumference, so the stripe size on
    // the tyre stays physically consistent regardless of wheel radius.
    let tread_tex = images.add(make_tyre_tread_texture());
    // Flat dark material for the circular tyre caps (shared across
    // every wheel) — the tread texture doesn't land on them.
    let cap_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.06, 0.06, 0.07),
        perceptual_roughness: 0.95,
        metallic: 0.0,
        ..default()
    });

    // Wheels — tracked separately, not parented (pose from controller).
    // For wheels with `usd_prim_path`, skip the procedural cylinder
    // entirely; the USD-tagging system below detaches the asset's
    // wheel prim and tags it as `VehicleWheel` instead, so the asset
    // mesh becomes the live raycast wheel.
    for (idx, wheel) in spec.wheels.iter().enumerate() {
        if wheel.usd_prim_path.is_some() {
            continue;
        }
        let circumference = std::f32::consts::TAU * wheel.radius as f32;
        // Tiles-per-revolution = circumference / desired-stripe-arc.
        let uv_tile = (circumference / TYRE_STRIPE_ARC_M).max(1.0);
        let tread_mat = materials.add(StandardMaterial {
            // Dark multiplier: texture samples are multiplied by this,
            // so the overall tyre is always darker than the raw chevron
            // texture, regardless of scene lighting.
            base_color: Color::srgb(0.45, 0.45, 0.45),
            base_color_texture: Some(tread_tex.clone()),
            uv_transform: Affine2::from_scale(Vec2::new(uv_tile, 1.0)),
            perceptual_roughness: 1.0,
            metallic: 0.0,
            ..default()
        });

        // Side (tread) mesh — cylinder without caps.
        let side_mesh = meshes.add(
            Cylinder::new(wheel.radius as f32, wheel.width as f32)
                .mesh()
                .resolution(32)
                .without_caps()
                .build(),
        );

        let wheel_entity = commands
            .spawn((
                Name::new(format!("{}::wheel[{}]", spec.name, idx)),
                Transform::default(),
                Mesh3d(side_mesh),
                MeshMaterial3d(tread_mat),
                VehicleWheel { id, index: idx },
            ))
            .id();

        // Two cap discs as children of the wheel entity. Circle is a
        // 2-D primitive in the XY plane (normal +Z); rotate ±90° around
        // X so the normal faces the axle direction (local +Y / -Y).
        let cap_mesh = meshes.add(
            Circle::new(wheel.radius as f32).mesh().resolution(32).build(),
        );
        commands
            .spawn((
                Name::new(format!("{}::wheel[{}]::cap+", spec.name, idx)),
                Transform::from_xyz(0.0, (wheel.width * 0.5) as f32, 0.0)
                    .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
                Mesh3d(cap_mesh.clone()),
                MeshMaterial3d(cap_mat.clone()),
            ))
            .insert(ChildOf(wheel_entity));
        commands
            .spawn((
                Name::new(format!("{}::wheel[{}]::cap-", spec.name, idx)),
                Transform::from_xyz(0.0, -(wheel.width * 0.5) as f32, 0.0)
                    .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                Mesh3d(cap_mesh),
                MeshMaterial3d(cap_mat.clone()),
            ))
            .insert(ChildOf(wheel_entity));
    }

    // Parts — parented to the chassis so they inherit its pose.
    for part in &spec.parts {
        let [pr, pg, pb] = part.color;
        let p_color = Color::srgb(pr, pg, pb);
        // If this part's colour matches the chassis colour at spawn,
        // it's "bodywork" (cab, beams, crossbars, bumper, …) and we
        // tag it so the Properties colour picker re-tints it along
        // with the chassis. Contrast parts (dark roofs, wheels,
        // hitches with their own palette) stay untagged and retain
        // their original colour.
        let matches_chassis = (pr - spec.chassis.color[0]).abs() < 1e-4
            && (pg - spec.chassis.color[1]).abs() < 1e-4
            && (pb - spec.chassis.color[2]).abs() < 1e-4;
        let mesh = mesh_for(part.mesh, part.size, meshes);
        let mat = materials.add(StandardMaterial {
            base_color: p_color,
            perceptual_roughness: match part.kind {
                PartKind::Hitch => 0.3, // slightly glossy marker
                _ => 0.7,
            },
            metallic: match part.kind {
                PartKind::Hitch => 0.4,
                _ => 0.1,
            },
            ..default()
        });
        let mut ec = commands.spawn((
            Name::new(format!("{}::{}", spec.name, part.name)),
            Transform::from_xyz(
                part.position.x as f32,
                part.position.y as f32,
                part.position.z as f32,
            ),
            Mesh3d(mesh),
            MeshMaterial3d(mat),
        ));
        if matches_chassis {
            ec.insert(ChassisTinted { id });
        }
        ec.insert(ChildOf(root));
    }

    root
}

/// Pick a spawn Y that guarantees the chassis starts a bit above the
/// ground (wheels hang down, settle on contact). Dynamic so we don't
/// hard-code 1.4 for every preset regardless of size.
pub fn spawn_height_for(spec: &VehicleSpec) -> f64 {
    // Lowest point of the vehicle in chassis-local coordinates —
    // either the chassis bottom or the rest-length wheel bottom,
    // whichever hangs lower. Gantry robots (Robotti) mount their
    // wheels well below the chassis pod, so the plain chassis-half
    // formula would spawn them partially buried.
    let chassis_bottom = -spec.chassis.size.y * 0.5;
    let mut lowest = chassis_bottom;
    for w in &spec.wheels {
        let wheel_bottom = w.chassis_connection.y
            - w.suspension_rest_length as f64
            - w.radius as f64;
        if wheel_bottom < lowest {
            lowest = wheel_bottom;
        }
    }
    // Keep ~0.2 m of air under the lowest point so the suspension
    // has room to settle under gravity without punching through.
    // Also honour the legacy 0.8 m clearance under the chassis
    // bottom used by every existing preset — so their spawn
    // behaviour doesn't change.
    let height_by_lowest = (-lowest) + 0.2;
    let height_by_chassis = spec.chassis.size.y * 0.5 + 0.8;
    height_by_lowest.max(height_by_chassis)
}

/// Procedural tyre-tread texture — **exactly one chevron period**, so
/// the material can tile it `circumference / TYRE_STRIPE_ARC_M` times
/// around the wheel via `uv_transform`.  Sampler set to `Repeat` on
/// the U axis so tiling works.
///
/// UV convention: `u` wraps around the wheel; `v` runs along the axle.
/// Apex of the "^" sits on the tyre centre line (`v = 0.5`).
fn make_tyre_tread_texture() -> Image {
    const W: u32 = 64;
    const H: u32 = 64;
    const CHEVRON_SLOPE: f32 = 0.55;   // sharper V — visible bend from apex to edges
    const STRIPE_FRACTION: f32 = 0.40; // chunky tread block

    let base:  [u8; 4] = [18, 18, 20, 255];
    let tread: [u8; 4] = [70, 70, 72, 255];

    let mut data = Vec::with_capacity((W * H * 4) as usize);
    for vp in 0..H {
        let fv = vp as f32 / H as f32;
        let dv = (fv - 0.5).abs();
        for up in 0..W {
            let fu = up as f32 / W as f32;
            let u_shifted = (fu + dv * CHEVRON_SLOPE).rem_euclid(1.0);
            let c = if u_shifted < STRIPE_FRACTION { tread } else { base };
            data.extend_from_slice(&c);
        }
    }
    let mut img = Image::new(
        Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    // Repeat on U so `uv_transform` tiling actually tiles; clamp on V
    // so the chevron apex stays dead-centre on the tyre.
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::ClampToEdge,
        ..ImageSamplerDescriptor::default()
    });
    img
}

/// Non-physics translucent preview of a vehicle — same meshes/parts
/// as the real one, but with alpha-blended materials and no
/// `VehicleBody` / rapier tagging. Used for the "drag-to-place" UX:
/// the ghost follows the cursor until the user commits with a click.
pub fn spawn_vehicle_ghost(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    spec: &VehicleSpec,
) -> Entity {
    let alpha = 0.45;
    let [r, g, b] = spec.chassis.color;
    let chassis_color = Color::srgba(r, g, b, alpha);
    let tread_tex     = images.add(make_tyre_tread_texture());

    let mut root_cmd = commands.spawn((
        Name::new(format!("{}-ghost", spec.name)),
        Transform::default(),
        GhostTag,
    ));
    if spec.chassis.render_chassis {
        let chassis_mesh = mesh_for(spec.chassis.mesh, spec.chassis.size, meshes);
        let chassis_mat = materials.add(StandardMaterial {
            base_color: chassis_color,
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 0.7,
            metallic: 0.1,
            ..default()
        });
        root_cmd.insert((Mesh3d(chassis_mesh), MeshMaterial3d(chassis_mat)));
    }
    let root = root_cmd.id();

    // Wheels as children of the ghost root — at rest (suspension
    // fully extended) so the silhouette reads as a settled vehicle.
    // Cylinder default axis is +Y; rotate 90° around Z so the axle
    // lies along X.
    // Shared cap material for the ghost preview (translucent).
    let ghost_cap_mat = materials.add(StandardMaterial {
        base_color: Color::srgba(0.06, 0.06, 0.07, alpha),
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    for wheel in &spec.wheels {
        let circumference = std::f32::consts::TAU * wheel.radius as f32;
        let uv_tile = (circumference / TYRE_STRIPE_ARC_M).max(1.0);
        let mat = materials.add(StandardMaterial {
            base_color: Color::srgba(1.0, 1.0, 1.0, alpha),
            base_color_texture: Some(tread_tex.clone()),
            uv_transform: Affine2::from_scale(Vec2::new(uv_tile, 1.0)),
            alpha_mode: AlphaMode::Blend,
            ..default()
        });
        let side_mesh = meshes.add(
            Cylinder::new(wheel.radius as f32, wheel.width as f32)
                .mesh()
                .resolution(32)
                .without_caps()
                .build(),
        );
        let cap_mesh = meshes.add(
            Circle::new(wheel.radius as f32).mesh().resolution(32).build(),
        );
        let wy = (wheel.chassis_connection.y - wheel.suspension_rest_length as f64) as f32;
        let wheel_parent = commands
            .spawn((
                Transform::from_xyz(
                    wheel.chassis_connection.x as f32,
                    wy,
                    wheel.chassis_connection.z as f32,
                )
                .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
                Mesh3d(side_mesh),
                MeshMaterial3d(mat),
                GhostTag,
            ))
            .insert(ChildOf(root))
            .id();
        commands
            .spawn((
                Transform::from_xyz(0.0, (wheel.width * 0.5) as f32, 0.0)
                    .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
                Mesh3d(cap_mesh.clone()),
                MeshMaterial3d(ghost_cap_mat.clone()),
                GhostTag,
            ))
            .insert(ChildOf(wheel_parent));
        commands
            .spawn((
                Transform::from_xyz(0.0, -(wheel.width * 0.5) as f32, 0.0)
                    .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                Mesh3d(cap_mesh),
                MeshMaterial3d(ghost_cap_mat.clone()),
                GhostTag,
            ))
            .insert(ChildOf(wheel_parent));
    }

    // Body parts — children of the chassis root with local offsets.
    for part in &spec.parts {
        let [pr, pg, pb] = part.color;
        let p_color = Color::srgba(pr, pg, pb, alpha);
        let mesh = mesh_for(part.mesh, part.size, meshes);
        let mat = materials.add(StandardMaterial {
            base_color: p_color,
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: match part.kind {
                PartKind::Hitch => 0.3,
                _ => 0.7,
            },
            metallic: match part.kind {
                PartKind::Hitch => 0.4,
                _ => 0.1,
            },
            ..default()
        });
        commands
            .spawn((
                Transform::from_xyz(
                    part.position.x as f32,
                    part.position.y as f32,
                    part.position.z as f32,
                ),
                Mesh3d(mesh),
                MeshMaterial3d(mat),
                GhostTag,
            ))
            .insert(ChildOf(root));
    }

    root
}
