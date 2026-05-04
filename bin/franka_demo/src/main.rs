//! `franka-demo <path/to/file.usd>` — load a USD into a Bevy app via
//! `usd_bevy`, with `usd_bevy`'s own `RapierAdapterPlugin` driving the
//! physics. Gearbox owns the *world* (ground plane, sky, lights)
//! around the loaded asset.
//!
//! No bespoke USD→rapier translator, no reconciliation system — the
//! `usd_bevy` plugin already does all of that correctly (basis
//! conversion, hierarchy-aware writeback, joint drives, real
//! collider shapes). This binary's only job is to wire it up
//! alongside the world-layer entities that gearbox would normally
//! provide.

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::scene::SceneRoot;
use rapier3d::prelude::*;
use usd_bevy::physics::{PhysicsActive, PhysicsWorld, RapierAdapterPlugin};
use usd_bevy::{UsdAsset, UsdLoaderSettings, UsdPlugin};

#[derive(Resource, Clone)]
struct UsdSearchPaths(Vec<PathBuf>);

#[derive(Resource)]
struct UsdRoot {
    handle: Handle<UsdAsset>,
    scene_spawned: bool,
}

fn main() {
    let asset_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: franka-demo <path/to/file.usd>");

    let asset_root = asset_path
        .parent()
        .expect("asset has parent dir")
        .to_path_buf();
    let asset_filename = asset_path
        .file_name()
        .expect("asset has filename")
        .to_string_lossy()
        .into_owned();

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: format!("franka-demo (gearbox world + usd_bevy physics) — {}", asset_path.display()),
                    resolution: (1400u32, 900u32).into(),
                    ..default()
                }),
                ..default()
            })
            .set(AssetPlugin {
                file_path: asset_root.to_string_lossy().into_owned(),
                ..default()
            }),
    )
    // The two pieces of the USD stack:
    // - `UsdPlugin`: file → `UsdAsset` → projected Bevy entity tree
    //   (meshes, materials, lights, marker components like
    //   `UsdRigidBody`, `UsdCollider`, `UsdPhysicsJoint`).
    // - `RapierAdapterPlugin`: marker components → its own `PhysicsWorld`
    //   resource (rapier sets), steps physics, writes poses back to
    //   Bevy `Transform` in `PostUpdate` with full hierarchy awareness.
    .add_plugins(UsdPlugin)
    .add_plugins(RapierAdapterPlugin)
    // `RapierAdapterPlugin` defaults to PAUSED — usdview wires a UI
    // toggle for that. The demo wants physics live from frame 0 so
    // gravity pulls on the franka right away; override here.
    .insert_resource(PhysicsActive(true))
    .insert_resource(UsdSearchPaths(vec![asset_root.clone()]))
    .add_systems(
        Startup,
        (
            spawn_world,
            spawn_physics_ground,
            request_usd_load(asset_filename),
        ),
    )
    .add_systems(Update, spawn_scene_when_loaded);

    app.run();
}

/// Gearbox-owned world layer: camera, sky, ambient + directional
/// light, visual ground plane. The "permanent stuff that's there
/// regardless of which USD is loaded" — Isaac-Sim-style.
fn spawn_world(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        Camera {
            clear_color: bevy::camera::ClearColorConfig::Custom(Color::srgb(0.55, 0.7, 0.85)),
            ..default()
        },
        Transform::from_xyz(2.0, 1.5, 2.5).looking_at(Vec3::new(0.0, 0.5, 0.0), Vec3::Y),
    ));

    commands.spawn((
        DirectionalLight {
            illuminance: 15_000.0,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(5.0, 10.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.insert_resource(bevy::light::GlobalAmbientLight {
        brightness: 400.0,
        ..default()
    });

    let ground_mesh = meshes.add(Plane3d::default().mesh().size(50.0, 50.0));
    let ground_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.45, 0.5, 0.4),
        perceptual_roughness: 0.95,
        ..default()
    });
    commands.spawn((
        Mesh3d(ground_mesh),
        MeshMaterial3d(ground_mat),
        Transform::IDENTITY,
    ));
}

/// Static ground collider in `usd_bevy::physics::PhysicsWorld` so
/// dynamic USD bodies actually rest on the visual floor instead of
/// falling forever. 50 m × 50 m cuboid, top surface flush with Y=0.
fn spawn_physics_ground(mut world: ResMut<PhysicsWorld>) {
    use bevy::math::DVec3;
    let ground = ColliderBuilder::cuboid(50.0, 0.5, 50.0)
        .translation(DVec3::new(0.0, -0.5, 0.0))
        .friction(1.0)
        .build();
    world.colliders.insert(ground);
}

fn request_usd_load(
    filename: String,
) -> impl Fn(Commands, Res<AssetServer>, Res<UsdSearchPaths>) {
    move |mut commands: Commands, asset_server: Res<AssetServer>, search: Res<UsdSearchPaths>| {
        let search_paths = search.0.clone();
        let handle: Handle<UsdAsset> = asset_server
            .load_with_settings::<UsdAsset, _>(filename.clone(), move |s: &mut UsdLoaderSettings| {
                s.search_paths = search_paths.clone();
            });
        commands.insert_resource(UsdRoot {
            handle,
            scene_spawned: false,
        });
        info!("requested visual load: {filename}");
    }
}

fn spawn_scene_when_loaded(
    mut commands: Commands,
    mut root: ResMut<UsdRoot>,
    usd_assets: Res<Assets<UsdAsset>>,
) {
    if root.scene_spawned {
        return;
    }
    let Some(asset) = usd_assets.get(&root.handle) else {
        return;
    };
    commands.spawn((SceneRoot(asset.scene.clone()), Transform::IDENTITY));
    root.scene_spawned = true;
    info!(
        "scene spawned: default_prim={:?}, layer_count={}",
        asset.default_prim, asset.layer_count
    );
}
