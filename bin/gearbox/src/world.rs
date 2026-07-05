//! The persistent world: horizon/background helpers, cloud shell,
//! atmospheric DistanceFog, sun with a single tight shadow cascade,
//! ChaseCamera configuration, and world-event publishing. The local
//! terrain surface itself is now a USD scene loaded by `load.rs`.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bevy::asset::RenderAssetUsages;
use bevy::ecs::entity::Entities;
use bevy::image::Image;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::light::{CascadeShadowConfigBuilder, DirectionalLightShadowMap, NotShadowCaster};
use bevy::mesh::VertexAttributeValues;
use bevy::pbr::{
    DistanceFog, ExtendedMaterial, FogFalloff, MaterialExtension, MaterialPlugin, StandardMaterial,
};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::shader::ShaderRef;
use bevy::transform::TransformSystems;
use bevy::window::PrimaryWindow;
use bevy_mara::{ChaseCamera, GroundGrid, apply_rig};
use rapier3d::math::{Rotation as DQuat, Vector as DVec3};
use rapier3d::prelude::{
    ColliderBuilder, ColliderHandle, Pose, RigidBodyBuilder, RigidBodyHandle, RigidBodyType,
};
use serde::Serialize;
use usd_bevy::physics::PhysicsWorld;
use zenoh::Wait;

/// Earth-radius planet sphere. The simulator was tuned for this
/// radius — vehicle wheel friction, camera fog distances, cloud
/// altitude, and shadow cascades all assume ~6 371 km.
const PLANET_RADIUS_M: f32 = 6_371_000.0;
/// Keep the old planet/horizon helper below the hilly local terrain.
/// Otherwise it reads as a flat plate under the terrain mesh.
const PLANET_VISUAL_DROP_M: f32 = 40.0;
/// Cloud deck height above the planet surface. ~4 km gives visible
/// separation from the terrain when zoomed out.
const CLOUD_ALTITUDE_M: f64 = 4_000.0;
const TERRAIN_FLAT_SPAWN_RADIUS_M: f32 = 24.0;
const TERRAIN_FULL_RELIEF_RADIUS_M: f32 = 55.0;
const TERRAIN_MIN_HEIGHT_M: f32 = -5.0;
const TERRAIN_MAX_HEIGHT_M: f32 = 10.0;
const FLAT_GROUND_HALF_EXTENT_M: f64 = 10_000.0;
const FLAT_GROUND_VISUAL_SIZE_M: f32 = 10_000.0;
const USD_TERRAIN_ACTIVATION_WARN_FRAMES: u32 = 120;

static USD_TERRAIN_LOADED: AtomicBool = AtomicBool::new(false);

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "../assets/shaders/terrain_material.wgsl");
        app.insert_resource(ClearColor(Color::srgb(0.55, 0.70, 0.86)))
            // 8 k shadow map (4× Bevy's 2048 default). One cascade has
            // to cover the whole ~100 m vehicle neighbourhood, so the
            // extra texels go directly into shadow sharpness.
            .insert_resource(DirectionalLightShadowMap { size: 8192 })
            .init_resource::<StaticUsdPropBodies>()
            .init_resource::<PublishedUsdPoses>()
            .add_plugins(MaterialPlugin::<AntiRepeatTerrainMaterial>::default())
            .add_systems(Startup, open_world_event_publisher)
            .add_systems(Startup, (spawn_world, spawn_flat_ground))
            .add_systems(Update, mark_new_usd_terrain_roots)
            .add_systems(Update, (chase_camera_control, chase_camera_zoom))
            .add_systems(Update, snap_new_usd_roots_to_terrain)
            .add_systems(Update, apply_anti_repeat_material_to_usd_terrain)
            .add_systems(Update, freeze_settled_static_usd_prop_bodies)
            .add_systems(Update, publish_loaded_usd_poses)
            .add_systems(Update, harvest_bales_on_machine_contact)
            .add_systems(Update, cleanup_terrain_collision_without_usd_terrain)
            .add_systems(Update, cleanup_static_usd_prop_bodies)
            .add_systems(
                PostUpdate,
                (
                    activate_usd_terrain_when_collider_ready.after(TransformSystems::Propagate),
                    align_new_grounded_usd_bounds_to_terrain
                        .after(activate_usd_terrain_when_collider_ready),
                ),
            );
    }
}

type AntiRepeatTerrainMaterial = ExtendedMaterial<StandardMaterial, AntiRepeatTerrainExtension>;

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
struct AntiRepeatTerrainExtension {
    #[texture(100)]
    #[sampler(101)]
    terrain_albedo: Handle<Image>,
    #[texture(102)]
    #[sampler(103)]
    terrain_height: Handle<Image>,
    #[texture(104)]
    #[sampler(105)]
    terrain_detail_albedo: Handle<Image>,
    #[texture(106)]
    #[sampler(107)]
    terrain_detail_height: Handle<Image>,
}

impl MaterialExtension for AntiRepeatTerrainExtension {
    fn fragment_shader() -> ShaderRef {
        "embedded://gearbox/../assets/shaders/terrain_material.wgsl".into()
    }
}

#[derive(Component, Debug, Clone, Copy)]
struct AntiRepeatTerrainMaterialApplied;

#[derive(Component, Debug, Clone, Copy)]
struct TerrainBoundsSnapPending {
    frames_waited: u32,
}

#[derive(Component, Debug, Clone, Copy)]
struct PendingUsdTerrainActivation {
    frames_waited: u32,
}

#[derive(Component, Debug, Clone, Copy)]
struct StaticUsdPhysicsProp {
    body: RigidBodyHandle,
    visual_top_offset_y: f32,
    frames_alive: u32,
}

#[derive(Resource, Debug, Clone, Copy)]
struct FlatGround {
    entity: Entity,
    collider: ColliderHandle,
}

#[derive(Resource, Debug, Clone, Copy)]
struct TerrainCollision {
    terrain: ColliderHandle,
    safety_floor: ColliderHandle,
}

#[derive(Resource, Default)]
struct StaticUsdPropBodies {
    handles: HashMap<Entity, (RigidBodyHandle, ColliderHandle)>,
}

#[derive(Resource, Clone)]
struct WorldEventPublisher {
    session: Arc<zenoh::Session>,
}

#[derive(Debug, Serialize)]
struct UsdHarvestedWire {
    id: String,
    bale_id: Option<u32>,
    x: f32,
    y: f32,
    z: f32,
}

/// Settled world pose of a loader-spawned static USD. Published once the prop
/// body freezes, so scripts read the *real* terrain-snapped + physics-settled
/// position instead of guessing. `top_y` is the world Y of the asset's visual
/// top — a caller can drop a marker right above it with no terrain math.
#[derive(Debug, Serialize)]
struct UsdPoseWire {
    id: String,
    x: f32,
    y: f32,
    z: f32,
    top_y: f32,
}

/// Prop entities whose settled pose has already been published. Keyed by
/// `Entity` (not runtime id) so that re-loading an id — a fresh entity — is
/// reported anew instead of being silently suppressed across script runs.
#[derive(Resource, Default)]
struct PublishedUsdPoses {
    published: std::collections::HashSet<Entity>,
}

const TERRAIN_BOUNDS_SNAP_SETTLE_FRAMES: u32 = 5;
const TERRAIN_PROP_CONTACT_CLEARANCE_M: f32 = 0.015;
const STATIC_PROP_MIN_DYNAMIC_FRAMES: u32 = 12;
const STATIC_PROP_FORCE_FREEZE_FRAMES: u32 = 45;
const STATIC_PROP_SETTLED_LINEAR_SPEED_MPS: f64 = 0.12;
const STATIC_PROP_SETTLED_ANGULAR_SPEED_RPS: f64 = 0.25;

fn open_world_event_publisher(mut commands: Commands) {
    match zenoh::open(zenoh::Config::default()).wait() {
        Ok(session) => {
            commands.insert_resource(WorldEventPublisher {
                session: Arc::new(session),
            });
            info!(
                "world: USD world events ready \
                 (gearbox/usd/harvested/<id>, gearbox/usd/pose/<id>)"
            );
        }
        Err(err) => {
            warn!("world: USD world events disabled: {err}");
        }
    }
}

fn spawn_world(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let radius = PLANET_RADIUS_M;
    let radius_f64 = PLANET_RADIUS_M as f64;

    // ── Planet sphere ────────────────────────────────────────────────
    // Warm sandy / tan ground colour. Higher UV resolution than a
    // toy sphere because this fills the whole horizon. It is lowered
    // below the local terrain so it cannot appear as a second flat
    // ground plate under the hilly field mesh.
    let planet_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.62, 0.48, 0.33),
        perceptual_roughness: 0.95,
        ..default()
    });
    let planet_mesh = meshes.add(Sphere::new(radius).mesh().uv(1024, 512));
    commands.spawn((
        Name::new("Planet"),
        Transform::from_xyz(0.0, -radius - PLANET_VISUAL_DROP_M, 0.0),
        Mesh3d(planet_mesh),
        MeshMaterial3d(planet_mat.clone()),
        NotShadowCaster,
        bevy::light::NotShadowReceiver,
    ));

    // ── Ground grid: off-looking flat grids make hilly terrain read as
    // floating over a plate, so keep it effectively invisible by
    // default; the UI toggle can still enable the grid resource.
    commands.insert_resource(GroundGrid {
        color: Color::srgba(80.0 / 255.0, 70.0 / 255.0, 70.0 / 255.0, 0.0),
        ..GroundGrid::default()
    });

    // ── Cloud shell ──────────────────────────────────────────────────
    spawn_cloud_shell(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut images,
        radius_f64,
    );

    // ── Sun + tight cascade ──────────────────────────────────────────
    // Single 100 m cascade so all texels land on the vehicle
    // neighbourhood. Steep angle for a clear horizontal direction.
    let sun_shadow = CascadeShadowConfigBuilder {
        num_cascades: 1,
        minimum_distance: 0.1,
        maximum_distance: 100.0,
        first_cascade_far_bound: 100.0,
        overlap_proportion: 0.0,
    }
    .build();
    commands.spawn((
        Name::new("Sun"),
        Transform::from_xyz(5.0, 50.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
        DirectionalLight {
            illuminance: 10_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        sun_shadow,
    ));

    // ── Atmospheric fog ──────────────────────────────────────────────
    let fog = DistanceFog {
        color: Color::srgb(0.55, 0.70, 0.86),
        falloff: FogFalloff::Atmospheric {
            extinction: Vec3::new(0.00008, 0.00012, 0.00020),
            inscattering: Vec3::new(0.00010, 0.00015, 0.00025),
        },
        ..default()
    };

    // ── Camera ──────────────────────────────────────────────────────
    let chase = ChaseCamera {
        focus: Vec3::new(0.0, 0.5, 0.0),
        distance: 14.0,
        elevation: 25_f32.to_radians(),
        max_distance: radius * 3.0,
        ..default()
    };
    let mut camera_transform = Transform::from_xyz(0.0, 8.0, -15.0).looking_at(Vec3::ZERO, Vec3::Y);
    apply_rig(&chase, &mut camera_transform);

    commands.spawn((
        Name::new("Camera"),
        Camera3d::default(),
        camera_transform,
        Projection::Perspective(PerspectiveProjection {
            near: 0.1,
            far: radius * 2.5,
            ..default()
        }),
        fog,
        AmbientLight {
            color: Color::WHITE,
            brightness: 120.0,
            ..default()
        },
        chase,
    ));
}

fn chase_camera_control(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    primary_window: Query<&Window, With<PrimaryWindow>>,
    mut pan_anchor: Local<Option<Vec2>>,
    mut lift_anchor: Local<Option<Vec2>>,
    mut orbit_anchor: Local<Option<Vec2>>,
    mut cameras: Query<(&mut ChaseCamera, &mut Transform)>,
) {
    let middle_pressed = mouse_buttons.pressed(MouseButton::Middle);
    let left_pressed = mouse_buttons.pressed(MouseButton::Left);
    let right_pressed = mouse_buttons.pressed(MouseButton::Right);

    let lift_active = middle_pressed && (left_pressed || right_pressed);
    let pan_active = middle_pressed && !lift_active;
    let orbit_active = left_pressed && right_pressed && !middle_pressed;

    if !pan_active {
        *pan_anchor = None;
    }
    if !lift_active {
        *lift_anchor = None;
    }
    if !orbit_active {
        *orbit_anchor = None;
    }

    let cursor_position = primary_window
        .single()
        .ok()
        .and_then(|w| w.cursor_position());
    let mut pan_delta = Vec2::ZERO;
    if pan_active && let Some(pos) = cursor_position {
        if let Some(anchor) = *pan_anchor {
            pan_delta = pos - anchor;
        }
        *pan_anchor = Some(pos);
    }

    let mut lift_delta = 0.0_f32;
    if lift_active && let Some(pos) = cursor_position {
        if let Some(anchor) = *lift_anchor {
            lift_delta = (pos - anchor).y;
        }
        *lift_anchor = Some(pos);
    }

    let mut orbit_delta = Vec2::ZERO;
    if orbit_active && let Some(pos) = cursor_position {
        if let Some(anchor) = *orbit_anchor {
            orbit_delta = pos - anchor;
        }
        *orbit_anchor = Some(pos);
    }

    if pan_delta == Vec2::ZERO && lift_delta == 0.0 && orbit_delta == Vec2::ZERO {
        return;
    }

    for (mut cam, mut transform) in &mut cameras {
        if pan_delta != Vec2::ZERO {
            let pan_speed = cam.distance * cam.pan_sensitivity;
            let forward = Vec3::new(cam.yaw.sin(), 0.0, cam.yaw.cos());
            let right = Vec3::new(forward.z, 0.0, -forward.x);
            cam.focus += (-right * pan_delta.x - forward * pan_delta.y) * pan_speed;
        }
        if lift_delta != 0.0 {
            cam.focus.y += lift_delta * cam.distance * cam.pan_sensitivity;
        }
        if orbit_delta != Vec2::ZERO {
            cam.yaw -= orbit_delta.x * cam.orbit_speed;
            cam.elevation += orbit_delta.y * cam.orbit_speed;
            cam.elevation = cam.elevation.clamp(cam.min_elevation, cam.max_elevation);
        }
        apply_rig(&cam, &mut transform);
    }
}

fn chase_camera_zoom(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut wheel: MessageReader<MouseWheel>,
    mut zoom_target: Local<Option<f64>>,
    mut cameras: Query<(&mut ChaseCamera, &mut Transform)>,
) {
    if keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight) {
        wheel.read().for_each(drop);
        return;
    }
    let mut scroll_delta = 0.0_f64;
    for event in wheel.read() {
        scroll_delta += match event.unit {
            MouseScrollUnit::Line => event.y as f64,
            MouseScrollUnit::Pixel => event.y as f64 / 32.0,
        };
    }

    let Ok((mut cam, mut transform)) = cameras.single_mut() else {
        return;
    };
    let target = zoom_target.get_or_insert(cam.distance as f64);
    if scroll_delta != 0.0 {
        let log_target = target.max(0.1).log10();
        let new_log = log_target - scroll_delta * cam.zoom_step;
        *target = 10f64
            .powf(new_log)
            .clamp(cam.min_distance as f64, cam.max_distance as f64);
    }

    let dt = time.delta_secs_f64();
    let log_current = (cam.distance as f64).max(0.1).ln();
    let log_target = target.max(0.1).ln();
    let log_diff = log_target - log_current;
    if log_diff.abs() > 1e-4 {
        let new_log = log_current + log_diff * (6.0 * dt).min(0.9);
        cam.distance = new_log.exp() as f32;
        apply_rig(&cam, &mut transform);
    } else if log_diff.abs() > 1e-5 {
        cam.distance = *target as f32;
        apply_rig(&cam, &mut transform);
    }
}

fn spawn_flat_ground(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut physics: ResMut<PhysicsWorld>,
) {
    USD_TERRAIN_LOADED.store(false, Ordering::Relaxed);

    let material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.48, 0.42, 0.30),
        perceptual_roughness: 0.98,
        ..default()
    });
    let mesh = meshes.add(Cuboid::new(
        FLAT_GROUND_VISUAL_SIZE_M,
        0.02,
        FLAT_GROUND_VISUAL_SIZE_M,
    ));
    let entity = commands
        .spawn((
            Name::new("FlatGround"),
            Transform::from_xyz(0.0, -0.01, 0.0),
            Mesh3d(mesh),
            MeshMaterial3d(material),
            NotShadowCaster,
        ))
        .id();

    let collider =
        ColliderBuilder::cuboid(FLAT_GROUND_HALF_EXTENT_M, 0.02, FLAT_GROUND_HALF_EXTENT_M)
            .translation(DVec3::new(0.0, -0.02, 0.0))
            .friction(1.0)
            .restitution(0.0)
            .build();
    let collider = physics.colliders.insert(collider);
    commands.insert_resource(FlatGround { entity, collider });
}

fn mark_new_usd_terrain_roots(
    mut commands: Commands,
    terrain_roots: Query<(Entity, &Name), Added<usd_bevy::UsdSceneRoot>>,
) {
    for (entity, name) in terrain_roots.iter() {
        if !is_usd_terrain_root_name(name.as_str()) {
            continue;
        }
        commands
            .entity(entity)
            .insert(PendingUsdTerrainActivation { frames_waited: 0 });
        info!(
            "world: USD terrain scene loaded; keeping flat fallback until terrain collider is ready"
        );
    }
}

fn activate_usd_terrain_when_collider_ready(
    mut commands: Commands,
    mut terrain_roots: Query<(Entity, &Name, &mut PendingUsdTerrainActivation)>,
    children: Query<&Children>,
    meshes: Res<Assets<Mesh>>,
    terrain_meshes: Query<(Entity, &GlobalTransform, &Mesh3d)>,
    flat: Option<Res<FlatGround>>,
    terrain_collision: Option<Res<TerrainCollision>>,
    mut physics: ResMut<PhysicsWorld>,
) {
    let mut flat_ground = flat.as_deref().copied();
    let mut terrain_collision_ready = terrain_collision.is_some();
    for (root, name, mut pending) in terrain_roots.iter_mut() {
        pending.frames_waited = pending.frames_waited.saturating_add(1);
        if !terrain_collision_ready && is_usd_terrain_scene_instantiated(root, &children) {
            if let Some(collision) = attach_gearbox_terrain_trimesh(
                root,
                &children,
                &meshes,
                &terrain_meshes,
                physics.as_mut(),
            ) {
                commands.insert_resource(collision);
                terrain_collision_ready = true;
            }
        }
        if !terrain_collision_ready {
            if pending.frames_waited == USD_TERRAIN_ACTIVATION_WARN_FRAMES {
                warn!(
                    "world: still waiting for visible USD terrain mesh on {}; keeping flat fallback active",
                    name.as_str()
                );
            }
            continue;
        }

        USD_TERRAIN_LOADED.store(true, Ordering::Relaxed);
        let removed_flat = if let Some(flat) = flat_ground.take() {
            remove_flat_ground(&mut commands, physics.as_mut(), flat);
            commands.remove_resource::<FlatGround>();
            true
        } else {
            false
        };
        commands
            .entity(root)
            .remove::<PendingUsdTerrainActivation>();
        if removed_flat {
            info!(
                "world: USD terrain collider ready for {}; removed default flat ground",
                name.as_str()
            );
        } else {
            info!(
                "world: USD terrain collider ready for {}; default flat ground was already gone",
                name.as_str()
            );
        }
    }
}

fn is_usd_terrain_root_name(name: &str) -> bool {
    name == "WorldTerrain" || name.to_ascii_lowercase().contains("terrain")
}

fn is_usd_terrain_scene_instantiated(root: Entity, children: &Query<&Children>) -> bool {
    collect_descendants(root, children).len() > 1
}

fn apply_anti_repeat_material_to_usd_terrain(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<AntiRepeatTerrainMaterial>>,
    terrain_roots: Query<(Entity, &Name), With<usd_bevy::UsdSceneRoot>>,
    children: Query<&Children>,
    terrain_meshes: Query<Entity, (With<Mesh3d>, Without<AntiRepeatTerrainMaterialApplied>)>,
    mut material_handle: Local<Option<Handle<AntiRepeatTerrainMaterial>>>,
) {
    let handle = material_handle
        .get_or_insert_with(|| {
            materials.add(ExtendedMaterial {
                base: StandardMaterial {
                    double_sided: true,
                    cull_mode: None,
                    perceptual_roughness: 0.98,
                    metallic: 0.0,
                    ..default()
                },
                extension: AntiRepeatTerrainExtension {
                    terrain_albedo: asset_server.load(asset_path(
                        "textures/terrain/Ground001/Ground001_1K-JPG_Color.jpg",
                    )),
                    terrain_height: asset_server.load(asset_path(
                        "textures/terrain/Ground001/Ground001_1K-JPG_Displacement.jpg",
                    )),
                    terrain_detail_albedo: asset_server.load(asset_path(
                        "textures/terrain/Ground003/Ground003_1K-JPG_Color.jpg",
                    )),
                    terrain_detail_height: asset_server.load(asset_path(
                        "textures/terrain/Ground003/Ground003_1K-JPG_Displacement.jpg",
                    )),
                },
            })
        })
        .clone();

    let mut applied = 0usize;
    for (root, name) in terrain_roots.iter() {
        if !is_usd_terrain_root_name(name.as_str())
            || !is_usd_terrain_scene_instantiated(root, &children)
        {
            continue;
        }
        for entity in collect_descendants(root, &children) {
            if terrain_meshes.get(entity).is_err() {
                continue;
            }
            commands
                .entity(entity)
                .remove::<MeshMaterial3d<StandardMaterial>>()
                .insert((
                    MeshMaterial3d(handle.clone()),
                    AntiRepeatTerrainMaterialApplied,
                ));
            applied += 1;
        }
    }
    if applied > 0 {
        info!("world: applied anti-repeating terrain material to {applied} USD terrain mesh(es)");
    }
}

fn asset_path(relative: &str) -> String {
    crate::load::default_asset_root()
        .join(relative)
        .to_string_lossy()
        .into_owned()
}

fn attach_gearbox_terrain_trimesh(
    root: Entity,
    children: &Query<&Children>,
    meshes: &Assets<Mesh>,
    terrain_meshes: &Query<(Entity, &GlobalTransform, &Mesh3d)>,
    physics: &mut PhysicsWorld,
) -> Option<TerrainCollision> {
    let Some((vertices, indices)) =
        terrain_trimesh_from_visible_mesh(root, children, meshes, terrain_meshes)
    else {
        return None;
    };
    remove_terrain_descendant_colliders(root, children, physics);
    let Some(terrain) = ColliderBuilder::trimesh(vertices, indices).ok() else {
        warn!("world: failed to build exact Rapier trimesh collider for USD terrain");
        return None;
    };
    let terrain = physics
        .colliders
        .insert(terrain.friction(1.4).restitution(0.0).build());
    physics.entity_to_collider.insert(root, terrain);

    // Belt-and-braces catch floor below the lowest authored terrain. It
    // should never be contacted in normal use, but it prevents assets from
    // disappearing forever if a future USD terrain asset has a hole or loads
    // slower than its dynamic bodies.
    let safety_y = TERRAIN_MIN_HEIGHT_M as f64 - 1.0;
    let safety_floor =
        ColliderBuilder::cuboid(FLAT_GROUND_HALF_EXTENT_M, 0.10, FLAT_GROUND_HALF_EXTENT_M)
            .translation(DVec3::new(0.0, safety_y, 0.0))
            .friction(1.2)
            .restitution(0.0)
            .build();
    let safety_floor = physics.colliders.insert(safety_floor);

    info!("world: attached exact visible-mesh Rapier trimesh collider for USD terrain");
    Some(TerrainCollision {
        terrain,
        safety_floor,
    })
}

fn terrain_trimesh_from_visible_mesh(
    root: Entity,
    children: &Query<&Children>,
    meshes: &Assets<Mesh>,
    terrain_meshes: &Query<(Entity, &GlobalTransform, &Mesh3d)>,
) -> Option<(Vec<DVec3>, Vec<[u32; 3]>)> {
    let mut best = None;
    let mut best_area = 0.0f32;
    for entity in collect_descendants(root, children) {
        let Ok((_entity, gt, mesh3d)) = terrain_meshes.get(entity) else {
            continue;
        };
        let Some(mesh) = meshes.get(&mesh3d.0) else {
            continue;
        };
        let Some(candidate) = trimesh_candidate_from_mesh(gt, mesh) else {
            continue;
        };
        if candidate.area > best_area {
            best_area = candidate.area;
            best = Some(candidate);
        }
    }
    best.map(|candidate| (candidate.vertices, candidate.indices))
}

struct TrimeshCandidate {
    vertices: Vec<DVec3>,
    indices: Vec<[u32; 3]>,
    area: f32,
}

fn trimesh_candidate_from_mesh(gt: &GlobalTransform, mesh: &Mesh) -> Option<TrimeshCandidate> {
    let positions = match mesh.attribute(Mesh::ATTRIBUTE_POSITION)? {
        VertexAttributeValues::Float32x3(positions) => positions,
        _ => return None,
    };
    if positions.len() < 4 {
        return None;
    }

    let m = gt.to_matrix();
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let vertices = positions
        .iter()
        .map(|p| {
            let w = m.transform_point3(Vec3::from(*p));
            min = min.min(w);
            max = max.max(w);
            DVec3::new(w.x as f64, w.y as f64, w.z as f64)
        })
        .collect::<Vec<_>>();

    let indices = if let Some(indices) = mesh.indices() {
        let raw = indices.iter().map(|i| i as u32).collect::<Vec<_>>();
        raw.chunks_exact(3)
            .map(|tri| [tri[0], tri[1], tri[2]])
            .collect::<Vec<_>>()
    } else {
        (0..vertices.len() / 3)
            .map(|i| [(i * 3) as u32, (i * 3 + 1) as u32, (i * 3 + 2) as u32])
            .collect::<Vec<_>>()
    };
    if indices.is_empty() {
        return None;
    }

    let size = max - min;
    let area = size.x.abs() * size.z.abs();
    if area <= 0.0 {
        return None;
    }
    Some(TrimeshCandidate {
        vertices,
        indices,
        area,
    })
}

fn remove_terrain_descendant_colliders(
    root: Entity,
    children: &Query<&Children>,
    physics: &mut PhysicsWorld,
) {
    let stale = collect_descendants(root, children)
        .into_iter()
        .filter_map(|entity| {
            physics
                .entity_to_collider
                .remove(&entity)
                .map(|handle| (entity, handle))
        })
        .collect::<Vec<_>>();
    let colliders = &mut physics.colliders;
    let islands = &mut physics.islands;
    let bodies = &mut physics.bodies;
    for (_entity, handle) in stale {
        colliders.remove(handle, islands, bodies, true);
    }
}

fn remove_flat_ground(commands: &mut Commands, physics: &mut PhysicsWorld, flat: FlatGround) {
    commands.entity(flat.entity).despawn();
    let colliders = &mut physics.colliders;
    let islands = &mut physics.islands;
    let bodies = &mut physics.bodies;
    colliders.remove(flat.collider, islands, bodies, true);
}

fn snap_new_usd_roots_to_terrain(
    mut commands: Commands,
    mut roots: Query<(Entity, &Name, &mut Transform), Added<Name>>,
) {
    for (entity, name, mut tr) in roots.iter_mut() {
        if !name.as_str().starts_with("UsdLoad[") {
            continue;
        }
        // The generic USD loader API documents Y as "height above ground".
        // Convert that offset to absolute Bevy Y for every static load, not
        // just y=0. This keeps target indicators above bales on hills instead
        // of disappearing inside the terrain.
        let requested_ground_offset = tr.translation.y;
        tr.translation.y =
            terrain_height_m(tr.translation.x, tr.translation.z) + requested_ground_offset;

        // For grounded USD assets (bales use y=0), do a second pass after the
        // scene is instantiated and mesh AABBs exist. Asset origins are not
        // guaranteed to be at the bottom, so origin-only terrain snapping can
        // still leave them floating or half buried.
        if requested_ground_offset.abs() < 0.001 {
            commands
                .entity(entity)
                .insert(TerrainBoundsSnapPending { frames_waited: 0 });
        }
    }
}

fn align_new_grounded_usd_bounds_to_terrain(
    mut commands: Commands,
    mut roots: Query<(Entity, &Name, &mut Transform, &mut TerrainBoundsSnapPending)>,
    children: Query<&Children>,
    meshes: Res<Assets<Mesh>>,
    mut physics: ResMut<PhysicsWorld>,
    mut prop_bodies: ResMut<StaticUsdPropBodies>,
    bounds: Query<(
        &GlobalTransform,
        Option<&usd_bevy::UsdSceneRoot>,
        Option<&StaticUsdPhysicsProp>,
        Option<&Mesh3d>,
        Option<&usd_bevy::UsdLocalExtent>,
        Option<&bevy::camera::primitives::Aabb>,
    )>,
) {
    for (root, name, mut root_transform, mut pending) in roots.iter_mut() {
        pending.frames_waited += 1;
        let Some(extent) = loaded_usd_world_extent(root, &children, &meshes, &bounds) else {
            if pending.frames_waited > 240 {
                warn!(
                    "world: no bounds found to terrain-align {}; keeping origin snap",
                    name.as_str()
                );
                commands.entity(root).remove::<TerrainBoundsSnapPending>();
            }
            continue;
        };
        if pending.frames_waited < TERRAIN_BOUNDS_SNAP_SETTLE_FRAMES {
            continue;
        }

        let root_translation_before_adjustment = root_transform.translation;
        let delta_y = TERRAIN_PROP_CONTACT_CLEARANCE_M - extent.min_terrain_clearance;
        if delta_y.abs() > 0.002 {
            root_transform.translation.y += delta_y;
            info!(
                "world: terrain contact adjusted {} by {delta_y:+.3} m (clearance={:+.3})",
                name.as_str(),
                extent.min_terrain_clearance
            );
        }
        if extent.has_scene_root && !extent.has_static_prop_body {
            let (body, collider) = attach_static_usd_prop_body(
                root,
                &root_transform,
                root_translation_before_adjustment,
                &extent,
                physics.as_mut(),
            );
            commands.entity(root).insert(StaticUsdPhysicsProp {
                body,
                visual_top_offset_y: extent.max.y - root_translation_before_adjustment.y,
                frames_alive: 0,
            });
            prop_bodies.handles.insert(root, (body, collider));
            info!(
                "world: attached gravity collider to {} (radius={:.3}, half_length={:.3})",
                name.as_str(),
                extent.collider_radius,
                extent.collider_half_length
            );
        }
        commands.entity(root).remove::<TerrainBoundsSnapPending>();
    }
}

#[derive(Debug, Clone, Copy)]
struct WorldExtent {
    min: Vec3,
    max: Vec3,
    min_terrain_clearance: f32,
    has_scene_root: bool,
    has_static_prop_body: bool,
    collider_radius: f32,
    collider_half_length: f32,
}

fn loaded_usd_world_extent(
    root: Entity,
    children: &Query<&Children>,
    meshes: &Assets<Mesh>,
    bounds: &Query<(
        &GlobalTransform,
        Option<&usd_bevy::UsdSceneRoot>,
        Option<&StaticUsdPhysicsProp>,
        Option<&Mesh3d>,
        Option<&usd_bevy::UsdLocalExtent>,
        Option<&bevy::camera::primitives::Aabb>,
    )>,
) -> Option<WorldExtent> {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut min_terrain_clearance = f32::INFINITY;
    let mut count = 0usize;
    let mut has_scene_root = false;
    let mut has_static_prop_body = false;
    for entity in collect_descendants(root, children) {
        let Ok((gt, scene_root, prop_body, mesh3d, local_extent, aabb)) = bounds.get(entity) else {
            continue;
        };
        has_scene_root |= scene_root.is_some();
        has_static_prop_body |= prop_body.is_some();
        if let Some(mesh3d) = mesh3d
            && let Some(mesh) = meshes.get(&mesh3d.0)
            && let Some((mesh_min, mesh_max, mesh_clearance)) =
                mesh_world_bounds_and_terrain_clearance(gt, mesh)
        {
            min = min.min(mesh_min);
            max = max.max(mesh_max);
            min_terrain_clearance = min_terrain_clearance.min(mesh_clearance);
            count += 1;
        } else if let Some(aabb) = aabb {
            // Prefer Bevy's AABB because it is computed from the actual mesh
            // vertices after usd_bevy has converted USD Z-up geometry into
            // Gearbox/Bevy Y-up geometry. Authored USD extents stay in the
            // original prim space on some assets (notably the bale.usdz), so
            // using them for ground contact can put the object visibly above
            // or below the terrain.
            let center = Vec3::from(aabb.center);
            let half = Vec3::from(aabb.half_extents);
            let (box_min, box_max, box_clearance) =
                local_box_world_bounds_and_terrain_clearance(gt, center - half, center + half);
            min = min.min(box_min);
            max = max.max(box_max);
            min_terrain_clearance = min_terrain_clearance.min(box_clearance);
            count += 1;
        } else if let Some(local_extent) = local_extent {
            let (box_min, box_max, box_clearance) = local_box_world_bounds_and_terrain_clearance(
                gt,
                Vec3::new(
                    local_extent.min[0],
                    local_extent.min[1],
                    local_extent.min[2],
                ),
                Vec3::new(
                    local_extent.max[0],
                    local_extent.max[1],
                    local_extent.max[2],
                ),
            );
            min = min.min(box_min);
            max = max.max(box_max);
            min_terrain_clearance = min_terrain_clearance.min(box_clearance);
            count += 1;
        }
    }
    if !(count > 0 && min_terrain_clearance.is_finite()) {
        return None;
    }
    let size = max - min;
    let collider_radius = (size.y * 0.5).clamp(0.1, 2.0);
    let horizontal_long = size.x.max(size.z);
    let collider_half_length = (horizontal_long * 0.5 - collider_radius).max(0.0);
    Some(WorldExtent {
        min,
        max,
        min_terrain_clearance,
        has_scene_root,
        has_static_prop_body,
        collider_radius,
        collider_half_length,
    })
}

fn attach_static_usd_prop_body(
    root: Entity,
    root_transform: &Transform,
    root_translation_before_adjustment: Vec3,
    extent: &WorldExtent,
    physics: &mut PhysicsWorld,
) -> (RigidBodyHandle, ColliderHandle) {
    let center = (extent.min + extent.max) * 0.5;
    let root_pos = root_transform.translation;
    let local_center =
        root_transform.rotation.inverse() * (center - root_translation_before_adjustment);
    let size = extent.max - extent.min;
    let along_x = size.x >= size.z;
    let body = RigidBodyBuilder::dynamic()
        .pose(Pose {
            translation: DVec3::new(root_pos.x as f64, root_pos.y as f64, root_pos.z as f64),
            rotation: DQuat::from_xyzw(
                root_transform.rotation.x as f64,
                root_transform.rotation.y as f64,
                root_transform.rotation.z as f64,
                root_transform.rotation.w as f64,
            ),
        })
        .linvel(DVec3::ZERO)
        .angvel(DVec3::ZERO)
        .linear_damping(4.0)
        .angular_damping(8.0)
        .can_sleep(true)
        .build();
    let body_handle = physics.bodies.insert(body);
    let mut collider = if along_x {
        ColliderBuilder::capsule_x(
            extent.collider_half_length as f64,
            extent.collider_radius as f64,
        )
    } else {
        ColliderBuilder::capsule_z(
            extent.collider_half_length as f64,
            extent.collider_radius as f64,
        )
    };
    collider = collider
        .translation(DVec3::new(
            local_center.x as f64,
            local_center.y as f64,
            local_center.z as f64,
        ))
        .density(80.0)
        .friction(1.2)
        .restitution(0.05);
    let collider_handle =
        physics
            .colliders
            .insert_with_parent(collider.build(), body_handle, &mut physics.bodies);
    physics.entity_to_body.insert(root, body_handle);
    physics.entity_to_collider.insert(root, collider_handle);
    (body_handle, collider_handle)
}

/// Publish the settled world pose of every loader-spawned static USD prop —
/// once, when its physics body freezes. Scripts subscribe to
/// `gearbox/usd/pose/**` and drive off these authoritative positions instead
/// of guessing where a terrain-snapped, physics-settled asset ended up. This
/// replaces the old proximity-matching marker system: the world reports where
/// things *are*; it never decides what anything targets.
fn publish_loaded_usd_poses(
    physics: Res<PhysicsWorld>,
    publisher: Option<Res<WorldEventPublisher>>,
    mut published: ResMut<PublishedUsdPoses>,
    props: Query<(Entity, &Name, &Transform, &StaticUsdPhysicsProp)>,
) {
    let Some(publisher) = publisher.as_deref() else {
        return;
    };
    // Forget props that no longer exist (harvested / unloaded). A later load
    // that reuses the same runtime id is a fresh entity, so its pose is then
    // published anew rather than suppressed.
    let live: std::collections::HashSet<Entity> = props.iter().map(|(e, ..)| e).collect();
    published.published.retain(|entity| live.contains(entity));

    for (entity, name, tr, prop) in props.iter() {
        if published.published.contains(&entity) {
            continue;
        }
        let Some(id) = parse_loaded_usd_id(name.as_str()) else {
            continue;
        };
        // Wait for the prop to come to rest. `freeze_settled_static_usd_prop_bodies`
        // flips a settled body to `Fixed`, so a non-dynamic body means the
        // pose reported here is the final resting pose.
        let settled = physics
            .bodies
            .get(prop.body)
            .is_some_and(|body| !body.is_dynamic());
        if !settled {
            continue;
        }
        let pos = tr.translation;
        let top_y = pos.y + prop.visual_top_offset_y;
        publisher.publish_loaded_usd_pose(id, pos, top_y);
        published.published.insert(entity);
    }
}

fn harvest_bales_on_machine_contact(
    mut commands: Commands,
    mut physics: ResMut<PhysicsWorld>,
    mut prop_bodies: ResMut<StaticUsdPropBodies>,
    publisher: Option<Res<WorldEventPublisher>>,
    bales: Query<(Entity, &Name, &Transform, &StaticUsdPhysicsProp)>,
) {
    let prop_body_handles = prop_bodies
        .handles
        .values()
        .map(|(body, _)| *body)
        .collect::<std::collections::HashSet<_>>();
    let mut touched = Vec::new();
    for (entity, name, tr, prop) in bales.iter() {
        let Some(bale_id) = parse_loaded_bale_id(name.as_str()) else {
            continue;
        };
        let Some((_body, collider)) = prop_bodies.handles.get(&entity).copied() else {
            continue;
        };
        let hit_non_prop_body = physics
            .narrow_phase
            .contact_pairs_with(collider)
            .filter(|pair| pair.has_any_active_contact())
            .any(|pair| {
                let other_collider = if pair.collider1 == collider {
                    pair.collider2
                } else {
                    pair.collider1
                };
                physics
                    .colliders
                    .get(other_collider)
                    .and_then(|collider| collider.parent())
                    .is_some_and(|body| {
                        body != prop.body
                            && !prop_body_handles.contains(&body)
                            && physics
                                .bodies
                                .get(body)
                                .is_some_and(|body| body.is_dynamic())
                    })
            });
        if hit_non_prop_body {
            touched.push((entity, bale_id, tr.translation));
        }
    }

    for (entity, bale_id, pos) in touched {
        remove_static_prop_body(entity, physics.as_mut(), prop_bodies.as_mut());
        commands.entity(entity).despawn();
        if let Some(publisher) = publisher.as_deref() {
            publisher.publish_bale_harvested(&bale_id, pos);
        }
    }
}

fn parse_loaded_bale_id(name: &str) -> Option<String> {
    let rest = name.strip_prefix("UsdLoad[bale_")?;
    let end = rest.find(']')?;
    Some(rest[..end].to_string())
}

/// Extract the loader runtime id from a `UsdLoad[<id>]::…` entity name.
fn parse_loaded_usd_id(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("UsdLoad[")?;
    let end = rest.find(']')?;
    Some(&rest[..end])
}

impl WorldEventPublisher {
    fn publish_bale_harvested(&self, bale_id: &str, pos: Vec3) {
        let id = format!("bale_{bale_id}");
        let event = UsdHarvestedWire {
            id: id.clone(),
            bale_id: bale_id.parse::<u32>().ok(),
            x: pos.x,
            y: pos.y,
            z: pos.z,
        };
        let Ok(bytes) = encode(&event) else {
            return;
        };
        let topic = format!("gearbox/usd/harvested/{id}");
        // BLOCK congestion control: a dropped harvest event would leave the
        // controlling script unaware that a bale was collected, so its tractor
        // would keep targeting a bale that no longer exists. Harvest events
        // are infrequent, so blocking briefly here costs nothing.
        if let Err(err) = self
            .session
            .put(topic.clone(), bytes)
            .congestion_control(zenoh::qos::CongestionControl::Block)
            .wait()
        {
            warn!("world: failed to publish {topic}: {err}");
        }
    }

    fn publish_loaded_usd_pose(&self, id: &str, pos: Vec3, top_y: f32) {
        let event = UsdPoseWire {
            id: id.to_string(),
            x: pos.x,
            y: pos.y,
            z: pos.z,
            top_y,
        };
        let Ok(bytes) = encode(&event) else {
            return;
        };
        let topic = format!("gearbox/usd/pose/{id}");
        // BLOCK congestion control: a dropped pose would leave a script with
        // no position for that object — it could never be targeted. Each prop
        // publishes its pose exactly once, so blocking briefly is free.
        if let Err(err) = self
            .session
            .put(topic.clone(), bytes)
            .congestion_control(zenoh::qos::CongestionControl::Block)
            .wait()
        {
            warn!("world: failed to publish {topic}: {err}");
        }
    }
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, ciborium::ser::Error<std::io::Error>> {
    let mut buf = Vec::new();
    ciborium::into_writer(value, &mut buf)?;
    Ok(buf)
}

fn freeze_settled_static_usd_prop_bodies(
    mut props: Query<(Entity, &mut StaticUsdPhysicsProp)>,
    mut physics: ResMut<PhysicsWorld>,
) {
    for (_entity, mut prop) in props.iter_mut() {
        prop.frames_alive = prop.frames_alive.saturating_add(1);
        let Some(body) = physics.bodies.get_mut(prop.body) else {
            continue;
        };
        if !body.is_dynamic() {
            continue;
        }
        if prop.frames_alive < STATIC_PROP_MIN_DYNAMIC_FRAMES {
            continue;
        }

        let linear_speed = body.linvel().length();
        let angular_speed = body.angvel().length();
        let settled = linear_speed < STATIC_PROP_SETTLED_LINEAR_SPEED_MPS
            && angular_speed < STATIC_PROP_SETTLED_ANGULAR_SPEED_RPS;
        let timed_out = prop.frames_alive >= STATIC_PROP_FORCE_FREEZE_FRAMES;
        if settled || timed_out {
            body.set_linvel(DVec3::ZERO, true);
            body.set_angvel(DVec3::ZERO, true);
            body.set_body_type(RigidBodyType::Fixed, true);
        }
    }
}

fn cleanup_static_usd_prop_bodies(
    entities: &Entities,
    mut physics: ResMut<PhysicsWorld>,
    mut prop_bodies: ResMut<StaticUsdPropBodies>,
) {
    let stale = prop_bodies
        .handles
        .keys()
        .copied()
        .filter(|entity| !entities.contains(*entity))
        .collect::<Vec<_>>();
    for entity in stale {
        remove_static_prop_body(entity, physics.as_mut(), prop_bodies.as_mut());
    }
}

fn cleanup_terrain_collision_without_usd_terrain(
    mut commands: Commands,
    terrain_collision: Option<Res<TerrainCollision>>,
    terrain_roots: Query<&Name, With<usd_bevy::UsdSceneRoot>>,
    mut physics: ResMut<PhysicsWorld>,
) {
    let Some(terrain_collision) = terrain_collision.as_deref().copied() else {
        return;
    };
    if terrain_roots
        .iter()
        .any(|name| is_usd_terrain_root_name(name.as_str()))
    {
        return;
    }

    physics
        .entity_to_collider
        .retain(|_, handle| *handle != terrain_collision.terrain);
    let physics = physics.as_mut();
    let colliders = &mut physics.colliders;
    let islands = &mut physics.islands;
    let bodies = &mut physics.bodies;
    colliders.remove(terrain_collision.terrain, islands, bodies, true);
    colliders.remove(terrain_collision.safety_floor, islands, bodies, true);
    commands.remove_resource::<TerrainCollision>();
    USD_TERRAIN_LOADED.store(false, Ordering::Relaxed);
    info!("world: removed terrain collision because USD terrain is no longer loaded");
}

fn remove_static_prop_body(
    entity: Entity,
    physics: &mut PhysicsWorld,
    prop_bodies: &mut StaticUsdPropBodies,
) {
    if let Some((body, _collider)) = prop_bodies.handles.remove(&entity) {
        physics.entity_to_body.remove(&entity);
        physics.entity_to_collider.remove(&entity);
        physics.bodies.remove(
            body,
            &mut physics.islands,
            &mut physics.colliders,
            &mut physics.impulse_joints,
            &mut physics.multibody_joints,
            true,
        );
    }
}

fn mesh_world_bounds_and_terrain_clearance(
    gt: &GlobalTransform,
    mesh: &Mesh,
) -> Option<(Vec3, Vec3, f32)> {
    let positions = match mesh.attribute(Mesh::ATTRIBUTE_POSITION)? {
        VertexAttributeValues::Float32x3(positions) => positions,
        _ => return None,
    };
    if positions.is_empty() {
        return None;
    }

    let m = gt.to_matrix();
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut min_clearance = f32::INFINITY;
    for p in positions {
        let w = m.transform_point3(Vec3::from(*p));
        min = min.min(w);
        max = max.max(w);
        min_clearance = min_clearance.min(w.y - terrain_height_m(w.x, w.z));
    }
    Some((min, max, min_clearance))
}

fn local_box_world_bounds_and_terrain_clearance(
    gt: &GlobalTransform,
    local_min: Vec3,
    local_max: Vec3,
) -> (Vec3, Vec3, f32) {
    let m = gt.to_matrix();
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut min_clearance = f32::INFINITY;
    for i in 0..8 {
        let c = Vec3::new(
            if i & 1 == 0 { local_min.x } else { local_max.x },
            if i & 2 == 0 { local_min.y } else { local_max.y },
            if i & 4 == 0 { local_min.z } else { local_max.z },
        );
        let w = m.transform_point3(c);
        min = min.min(w);
        max = max.max(w);
        min_clearance = min_clearance.min(w.y - terrain_height_m(w.x, w.z));
    }
    (min, max, min_clearance)
}

fn collect_descendants(root: Entity, children: &Query<&Children>) -> Vec<Entity> {
    let mut out = vec![root];
    let mut cursor = 0usize;
    while cursor < out.len() {
        let entity = out[cursor];
        cursor += 1;
        if let Ok(kids) = children.get(entity) {
            out.extend(kids.iter());
        }
    }
    out
}

pub fn terrain_height_m(x: f32, z: f32) -> f32 {
    if !USD_TERRAIN_LOADED.load(Ordering::Relaxed) {
        return 0.0;
    }
    terrain_height_formula_m(x, z)
}

fn terrain_height_formula_m(x: f32, z: f32) -> f32 {
    let raw = terrain_hills_raw_m(x, z) + terrain_visible_local_relief_m(x, z);
    let origin = terrain_hills_raw_m(0.0, 0.0) + terrain_visible_local_relief_m(0.0, 0.0);
    let distance_from_spawn = (x * x + z * z).sqrt();
    let spawn_fade = smoothstep_range(
        TERRAIN_FLAT_SPAWN_RADIUS_M,
        TERRAIN_FULL_RELIEF_RADIUS_M,
        distance_from_spawn,
    );

    ((raw - origin) * spawn_fade).clamp(TERRAIN_MIN_HEIGHT_M, TERRAIN_MAX_HEIGHT_M)
}

fn terrain_hills_raw_m(x: f32, z: f32) -> f32 {
    // Visible crop-field rolls. Keep this CPU-side so the visible mesh
    // and Rapier heightfield collider are exactly the same shape.
    //
    // The first attempt used 3-6 m hills over hundreds of metres; on an
    // 8 km field from the default camera that reads basically flat. Use
    // stronger nearby rolls so the terrain is unmistakably a mesh, not a
    // texture floating over a plate.
    let rolling_noise = (fbm_world(x * 0.0017 + 12.7, z * 0.0017 - 8.4, 5) - 0.5) * 4.0;
    rolling_noise
        + smooth_hill(x, z, 45.0, 45.0, 70.0, 4.0)
        + smooth_hill(x, z, -70.0, 55.0, 85.0, 3.4)
        + smooth_hill(x, z, 80.0, -65.0, 95.0, 3.0)
        + smooth_hill(x, z, 260.0, -360.0, 300.0, -2.8)
        + smooth_hill(x, z, -340.0, 260.0, 340.0, -2.3)
        + smooth_hill(x, z, 620.0, -430.0, 520.0, 3.5)
        + smooth_hill(x, z, -720.0, 560.0, 560.0, 3.3)
        + smooth_hill(x, z, 1_250.0, 900.0, 850.0, 2.8)
        + smooth_hill(x, z, -1_250.0, -850.0, 900.0, 2.6)
        + smooth_hill(x, z, 2_050.0, -1_450.0, 1_100.0, 2.4)
        + smooth_hill(x, z, -2_150.0, 1_450.0, 1_150.0, 2.3)
}

fn terrain_visible_local_relief_m(x: f32, z: f32) -> f32 {
    // Deliberately obvious local relief near the default camera/tractor.
    // These are real vertex heights, not shader displacement. The
    // `terrain_height_m` origin subtraction keeps exact spawn at y=0 while
    // leaving a clearly visible hill/valley profile tens of metres around
    // the origin.
    let rolling_ridges = (x * 0.020).sin() * (z * 0.015 + 0.7).sin() * 1.2;
    rolling_ridges
        + smooth_hill(x, z, 32.0, 42.0, 38.0, 8.8)
        + smooth_hill(x, z, -44.0, 36.0, 44.0, -4.8)
        + smooth_hill(x, z, 64.0, -42.0, 52.0, 6.2)
        + smooth_hill(x, z, -72.0, -58.0, 60.0, -3.9)
        + smooth_hill(x, z, 0.0, 115.0, 80.0, 5.4)
}

fn smoothstep_range(edge0: f32, edge1: f32, value: f32) -> f32 {
    let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn smooth_hill(x: f32, z: f32, cx: f32, cz: f32, radius: f32, height: f32) -> f32 {
    let dx = x - cx;
    let dz = z - cz;
    let d2 = dx * dx + dz * dz;
    height * (-d2 / (2.0 * radius * radius)).exp()
}

/// Translucent cloud shell — a UV sphere at `planet_radius + 4 km`,
/// double-sided so it reads from inside (ground level overcast) and
/// outside (orbital cloud bands), `NotShadowCaster` so it doesn't
/// blow up the directional cascade.
fn spawn_cloud_shell(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    planet_radius: f64,
) {
    let shell_radius = planet_radius + CLOUD_ALTITUDE_M;
    let mesh = meshes.add(Sphere::new(shell_radius as f32).mesh().uv(256, 128));
    let cloud_tex = images.add(make_cloud_texture());
    let material = materials.add(StandardMaterial {
        base_color: Color::srgba(1.0, 1.0, 1.0, 0.92),
        base_color_texture: Some(cloud_tex),
        alpha_mode: AlphaMode::Blend,
        unlit: false,
        double_sided: true,
        cull_mode: None,
        perceptual_roughness: 1.0,
        metallic: 0.0,
        ..default()
    });
    commands.spawn((
        Name::new("CloudShell"),
        Transform::from_xyz(0.0, -planet_radius as f32, 0.0),
        Mesh3d(mesh),
        MeshMaterial3d(material),
        NotShadowCaster,
    ));
}

fn make_cloud_texture() -> Image {
    const W: u32 = 1024;
    const H: u32 = 512;
    let mut data = Vec::with_capacity((W * H * 4) as usize);
    let coverage: f32 = 0.55;
    let max_alpha: f32 = 0.92;
    for y in 0..H {
        for x in 0..W {
            let u = x as f32 / W as f32;
            let v = y as f32 / H as f32;
            let n = fbm_tileable(u, v);
            let t = ((n - (1.0 - coverage)) / coverage).clamp(0.0, 1.0);
            let a = (t * t * (3.0 - 2.0 * t)) * max_alpha;
            data.extend_from_slice(&[255, 255, 255, (a * 255.0) as u8]);
        }
    }
    Image::new(
        Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
}

fn fbm_tileable(u: f32, v: f32) -> f32 {
    use std::f32::consts::TAU;
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut freq: f32 = 3.0;
    let mut phase = 0.0;
    for _ in 0..5 {
        let fu = u * TAU * freq;
        let fv = v * std::f32::consts::PI * freq;
        sum += amp * ((fu + phase).sin() * fv.sin());
        amp *= 0.55;
        freq *= 2.07;
        phase += 1.73;
    }
    (sum * 0.5 + 0.5).clamp(0.0, 1.0)
}

fn fbm_world(x: f32, z: f32, octaves: u32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut freq = 1.0;
    let mut norm = 0.0;
    for _ in 0..octaves {
        sum += amp * value_noise_2d(x * freq, z * freq);
        norm += amp;
        amp *= 0.5;
        freq *= 2.03;
    }
    (sum / norm).clamp(0.0, 1.0)
}

fn value_noise_2d(x: f32, z: f32) -> f32 {
    let xi = x.floor() as i32;
    let zi = z.floor() as i32;
    let xf = x - xi as f32;
    let zf = z - zi as f32;
    let sx = smoothstep(xf);
    let sz = smoothstep(zf);

    let a = hash2(xi, zi);
    let b = hash2(xi + 1, zi);
    let c = hash2(xi, zi + 1);
    let d = hash2(xi + 1, zi + 1);
    let ab = a + (b - a) * sx;
    let cd = c + (d - c) * sx;
    ab + (cd - ab) * sz
}

fn hash2(x: i32, z: i32) -> f32 {
    let mut n = x as u32;
    n = n.wrapping_mul(0x9E37_79B1);
    n ^= (z as u32).wrapping_mul(0x85EB_CA77);
    n ^= n >> 16;
    n = n.wrapping_mul(0xC2B2_AE3D);
    n ^= n >> 15;
    n as f32 / u32::MAX as f32
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::Ordering;

    use super::{TERRAIN_MAX_HEIGHT_M, TERRAIN_MIN_HEIGHT_M, USD_TERRAIN_LOADED, terrain_height_m};

    static TERRAIN_FLAG_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn terrain_height_is_flat_until_usd_terrain_is_active() {
        let _lock = TERRAIN_FLAG_TEST_LOCK.lock().unwrap();
        USD_TERRAIN_LOADED.store(false, Ordering::Relaxed);
        assert_eq!(terrain_height_m(35.0, 45.0), 0.0);
    }

    #[test]
    fn terrain_height_has_visible_local_relief() {
        let _lock = TERRAIN_FLAG_TEST_LOCK.lock().unwrap();
        USD_TERRAIN_LOADED.store(true, Ordering::Relaxed);
        let mut min_h = f32::MAX;
        let mut max_h = f32::MIN;
        for zi in -20..=20 {
            for xi in -20..=20 {
                let h = terrain_height_m(xi as f32 * 25.0, zi as f32 * 25.0);
                min_h = min_h.min(h);
                max_h = max_h.max(h);
            }
        }

        assert!(
            terrain_height_m(0.0, 0.0).abs() < 0.001,
            "terrain origin must stay at y=0 for sane default spawning"
        );
        assert!(
            terrain_height_m(5.0, 0.0).abs() < 0.001 && terrain_height_m(0.0, 5.0).abs() < 0.001,
            "exact spawn pad must stay flat so tractors can spawn cleanly"
        );
        assert!(
            max_h <= TERRAIN_MAX_HEIGHT_M + 0.001,
            "terrain max height exceeded cap: max={max_h:.2}"
        );
        assert!(
            min_h >= TERRAIN_MIN_HEIGHT_M - 0.001,
            "terrain min height exceeded cap: min={min_h:.2}"
        );
        assert!(
            max_h - min_h > 4.0,
            "local terrain relief is too subtle: min={min_h:.2}, max={max_h:.2}, span={:.2}",
            max_h - min_h
        );
        USD_TERRAIN_LOADED.store(false, Ordering::Relaxed);
    }
}
