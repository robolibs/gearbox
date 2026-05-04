//! `usd-world-demo <path1.usd> [path2.usd ...]` — load one or more
//! USD assets into a gearbox-shaped Bevy app.
//!
//! Multiple paths are mounted side by side along +X, spacing 2 m
//! between centres. The world layer (sky, sun, grid, axes, ground
//! collider) comes from `gearbox-world`; USD loading + physics come
//! from `usd_bevy`.

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::scene::SceneRoot;
use gearbox_world::WorldPlugin;
use usd_bevy::physics::{PhysicsActive, RapierAdapterPlugin};
use usd_bevy::{UsdAsset, UsdLoaderSettings, UsdPlugin};

const MOUNT_SPACING: f32 = 2.0;

#[derive(Resource, Clone)]
struct PendingLoads(Vec<PendingAsset>);

#[derive(Clone)]
struct PendingAsset {
    abs_path: PathBuf,
    mount_point: Vec3,
    search_path: PathBuf,
}

#[derive(Resource, Default)]
struct LoadedAssets(Vec<LoadedAsset>);

struct LoadedAsset {
    handle: Handle<UsdAsset>,
    mount_point: Vec3,
    spawned: bool,
    label: String,
}

fn main() {
    let raw_paths: Vec<String> = std::env::args().skip(1).collect();
    if raw_paths.is_empty() {
        eprintln!("usage: usd-world-demo <path1.usd> [path2.usd ...]");
        std::process::exit(2);
    }

    let pending: Vec<PendingAsset> = raw_paths
        .iter()
        .enumerate()
        .map(|(i, raw)| {
            let path = PathBuf::from(raw);
            let abs = if path.is_absolute() {
                path
            } else {
                std::env::current_dir().unwrap().join(path)
            };
            let parent = abs.parent().expect("asset has parent dir").to_path_buf();
            PendingAsset {
                abs_path: abs,
                mount_point: Vec3::new(i as f32 * MOUNT_SPACING, 0.0, 0.0),
                search_path: parent,
            }
        })
        .collect();

    let title = format!(
        "usd-world-demo — {}",
        pending
            .iter()
            .map(|p| p.abs_path.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" + ")
    );

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title,
                    resolution: (1400u32, 900u32).into(),
                    ..default()
                }),
                ..default()
            })
            // Asset root `/` so per-asset absolute paths work without
            // a common-ancestor calculation. Bevy 0.18 forbids paths
            // outside `file_path` by default — `Allow` is required
            // for arbitrary absolute load paths.
            .set(AssetPlugin {
                file_path: "/".to_string(),
                unapproved_path_mode: bevy::asset::UnapprovedPathMode::Allow,
                ..default()
            }),
    )
    .add_plugins(WorldPlugin::default())
    .add_plugins(UsdPlugin)
    .add_plugins(RapierAdapterPlugin)
    .insert_resource(PhysicsActive(true))
    .insert_resource(PendingLoads(pending))
    .init_resource::<LoadedAssets>()
    .add_systems(Startup, request_usd_loads)
    .add_systems(Update, spawn_scenes_when_loaded);

    app.run();
}

fn request_usd_loads(
    asset_server: Res<AssetServer>,
    pending: Res<PendingLoads>,
    mut loaded: ResMut<LoadedAssets>,
) {
    for asset in &pending.0 {
        let search = vec![asset.search_path.clone()];
        let load_path = asset.abs_path.to_string_lossy().into_owned();
        let handle: Handle<UsdAsset> = asset_server.load_with_settings::<UsdAsset, _>(
            load_path.clone(),
            move |s: &mut UsdLoaderSettings| {
                s.search_paths = search.clone();
            },
        );
        let label = asset
            .abs_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| load_path.clone());
        info!("requested USD load: {label} at mount={:?}", asset.mount_point);
        loaded.0.push(LoadedAsset {
            handle,
            mount_point: asset.mount_point,
            spawned: false,
            label,
        });
    }
}

fn spawn_scenes_when_loaded(
    mut commands: Commands,
    mut loaded: ResMut<LoadedAssets>,
    usd_assets: Res<Assets<UsdAsset>>,
) {
    for entry in loaded.0.iter_mut() {
        if entry.spawned {
            continue;
        }
        let Some(asset) = usd_assets.get(&entry.handle) else {
            continue;
        };
        commands.spawn((
            SceneRoot(asset.scene.clone()),
            Transform::from_translation(entry.mount_point),
        ));
        entry.spawned = true;
        info!(
            "spawned {} at {:?}: default_prim={:?}, layer_count={}",
            entry.label, entry.mount_point, asset.default_prim, asset.layer_count
        );
    }
}
