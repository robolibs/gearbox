//! `franka-demo <path/to/franka.usd>` — minimum viable Isaac-Sim-style
//! loop:
//!
//! 1. **Gearbox owns the world.** A `gearbox_physics::Sim` resource
//!    sits in the Bevy `World` with gravity, a ground plane, and the
//!    franka rigid bodies loaded via `gearbox_usd::load_usd_into_sim`.
//! 2. **`usd_bevy` renders visuals only.** `UsdPlugin` projects the
//!    USD scene into Bevy entities (meshes, materials, transforms).
//!    The Rapier adapter from `usd_bevy` is **not** registered — we
//!    don't want a second physics world.
//! 3. **Reconciliation.** Each frame we step the gearbox sim, then
//!    walk every entity carrying a `UsdPrimRef` and copy the matching
//!    rapier body's pose into its `Transform`. The visual entity
//!    becomes a "view" of the gearbox-owned body.
//!
//! This binary is the proof that the boundary works: USD is the asset
//! format, gearbox owns the simulation, Bevy renders. No coupling
//! between the rendering and physics besides the prim-path map.

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::scene::SceneRoot;
use gearbox_physics::Sim;
use gearbox_usd::SceneDescriptor;
use usd_bevy::{UsdAsset, UsdLoaderSettings, UsdPlugin, UsdPrimRef};

#[derive(Resource)]
struct PhysicsSim(Sim);

/// Whether `step_gearbox_sim` advances the sim. Starts paused so the
/// franka stays in its authored rest pose — without joints every
/// link is an independent free body and gravity scatters them
/// instantly. Press Space to toggle.
#[derive(Resource)]
struct SimPaused(bool);

impl Default for SimPaused {
    fn default() -> Self {
        Self(true)
    }
}

#[derive(Resource, Default)]
struct UsdSceneDesc(SceneDescriptor);

#[derive(Resource)]
struct AssetPath(PathBuf);

/// Filesystem dir openusd searches when resolving relative references
/// authored inside the stage (Props/, Materials/, etc.). Without this
/// the loader sees `layer_count = 1` and the visual is empty.
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

    // Bevy's AssetServer wants assets relative to a root; we use the
    // file's parent dir and pass just the filename.
    let asset_root = asset_path
        .parent()
        .expect("asset has parent dir")
        .to_path_buf();
    let asset_filename = asset_path
        .file_name()
        .expect("asset has filename")
        .to_string_lossy()
        .into_owned();

    // Build the gearbox sim FIRST so it lands as a Bevy resource at
    // startup. Ground plane + franka bodies → a world that's already
    // primed before the first frame.
    let mut sim = Sim::new();
    sim.add_ground_plane(50.0);
    let descriptor = gearbox_usd::load_usd_into_sim(&asset_path, &mut sim)
        .expect("loading USD into gearbox sim");
    info!(
        "gearbox sim primed: {} body(ies) from {}",
        descriptor.bodies.len(),
        asset_path.display()
    );

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: format!("franka-demo (gearbox sim) — {}", asset_path.display()),
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
    .add_plugins(UsdPlugin)
    .insert_resource(PhysicsSim(sim))
    .insert_resource(SimPaused::default())
    .insert_resource(UsdSceneDesc(descriptor))
    .insert_resource(AssetPath(asset_path.clone()))
    .insert_resource(UsdSearchPaths(vec![asset_root.clone()]))
    .add_systems(Startup, (spawn_camera_and_light, request_usd_load(asset_filename)))
    .add_systems(
        Update,
        (
            spawn_scene_when_loaded,
            toggle_sim_pause,
            step_gearbox_sim,
            // `reconcile_visuals_to_sim` is intentionally OFF until we
            // teach it about the USD parent chain. usd_bevy spawns
            // every prim under a root-basis transform (Z→Y rotation +
            // metersPerUnit scale) and a hierarchy of intermediate
            // Xforms; naively overwriting a leaf's local Transform
            // with the body's world pose ignores all of that and
            // visually scrambles the arm. Step 4 of the gearbox
            // integration adds the proper write-back.
        ),
    );

    app.run();
}

fn spawn_camera_and_light(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // The franka authors `upAxis = Z`, but `usd_bevy` projects it
    // into Bevy's native Y-up via the root-basis transform. So once
    // the visual is on screen, "up" really is +Y. Camera + ground
    // accordingly assume Y-up — matches every other Bevy 3D scene.
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

    // Gearbox-owned world surface. Bevy's `Plane3d::default()` is the
    // XZ plane (Y-up native), which is exactly what we want — no
    // rotation required.
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

/// `Startup` system: kicks off the async USD load through Bevy's
/// `AssetServer`. The actual `SceneRoot` spawn happens later in
/// `spawn_scene_when_loaded` once the `UsdAsset` is resident — we
/// need its inner `Handle<Scene>`, which doesn't exist until the
/// loader finishes.
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
        info!("requested visual load: {filename} (search: {:?})", search.0);
    }
}

/// Once the `UsdAsset` has materialised, grab its inner `Handle<Scene>`
/// and spawn it as a `SceneRoot`. Bevy clones the scene under the
/// root entity, attaching every projected prim (mesh, material,
/// `UsdPrimRef`, …).
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

fn step_gearbox_sim(mut sim: ResMut<PhysicsSim>, paused: Res<SimPaused>, time: Res<Time>) {
    if paused.0 {
        return;
    }
    // Cap dt to avoid the spiral-of-death after a long pause.
    let dt = time.delta_secs_f64().min(1.0 / 30.0);
    sim.0.step(dt);
}

fn toggle_sim_pause(keys: Res<ButtonInput<KeyCode>>, mut paused: ResMut<SimPaused>) {
    if keys.just_pressed(KeyCode::Space) {
        paused.0 = !paused.0;
        info!(
            "sim {}",
            if paused.0 { "PAUSED" } else { "RUNNING" }
        );
    }
}

/// For every entity carrying a `UsdPrimRef`, look up the matching
/// rapier body in the descriptor and copy its pose into `Transform`.
/// USD authors `Z up` for franka but Bevy is `Y up`; we deal with
/// that by reading `metersPerUnit` / `upAxis` later — for now we just
/// trust the loaded transforms to put us roughly in the right place.
fn reconcile_visuals_to_sim(
    sim: Res<PhysicsSim>,
    desc: Res<UsdSceneDesc>,
    mut prims: Query<(&UsdPrimRef, &mut Transform)>,
) {
    for (prim, mut tr) in &mut prims {
        let Some(handle) = desc.0.body(&prim.path) else {
            continue;
        };
        let Some(body) = sim.0.bodies.get(handle) else {
            continue;
        };
        let t = body.translation();
        let r = body.rotation();
        tr.translation = Vec3::new(t.x as f32, t.y as f32, t.z as f32);
        tr.rotation = Quat::from_xyzw(r.x as f32, r.y as f32, r.z as f32, r.w as f32);
    }
}
