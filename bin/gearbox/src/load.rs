//! Multi-USD loading: CLI args + 📂 ribbon button → asset_server →
//! `SceneRoot` with `LoadedAsset` marker. The marker also tags the
//! root for the UI's pick-and-gizmo wiring.

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::scene::SceneRoot;
use usd_bevy::{UsdAsset, UsdLoaderSettings};

use crate::controller::{ControllerInventory, discover_machines_from_usd, log_discovered_machines};

/// Tag on every `SceneRoot` the loader spawns. Carries the source
/// path so the tree / inspector can show where it came from.
#[derive(Component, Debug, Clone)]
pub struct LoadedAsset {
    pub path: PathBuf,
    pub label: String,
}

/// Companion to `LoadedAsset`: the `Handle<UsdAsset>` that produced
/// the spawned scene. The viewer panels look the asset up via
/// `Assets<UsdAsset>::get(handle)` for stage metadata, variants,
/// cameras, etc.
#[derive(Component, Debug, Clone)]
pub struct UsdAssetHandle(pub Handle<UsdAsset>);

/// Push a path here to load + spawn it next frame. The 📂 button
/// (and CLI seeding) both write to this queue.
#[derive(Resource, Default)]
pub struct LoadQueue(pub Vec<PathBuf>);

/// Tracking entry per in-flight or already-spawned load.
struct InflightLoad {
    handle: Handle<UsdAsset>,
    path: PathBuf,
    label: String,
    mount: Vec3,
    spawned: bool,
}

#[derive(Resource, Default)]
struct Inflight(Vec<InflightLoad>);

pub struct LoadPlugin {
    pub cli_paths: Vec<PathBuf>,
}

impl Plugin for LoadPlugin {
    fn build(&self, app: &mut App) {
        let cli = self.cli_paths.clone();
        app.init_resource::<LoadQueue>()
            .init_resource::<Inflight>()
            .add_systems(Startup, move |mut q: ResMut<LoadQueue>| {
                q.0.extend(cli.clone());
            })
            .add_systems(Update, (drain_load_queue, spawn_when_loaded));
    }
}

fn drain_load_queue(
    asset_server: Res<AssetServer>,
    mut queue: ResMut<LoadQueue>,
    mut inflight: ResMut<Inflight>,
) {
    if queue.0.is_empty() {
        return;
    }
    for abs in queue.0.drain(..) {
        let parent = abs
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let search = vec![parent];
        let load_path = abs.to_string_lossy().into_owned();
        let handle: Handle<UsdAsset> = asset_server.load_with_settings::<UsdAsset, _>(
            load_path.clone(),
            move |s: &mut UsdLoaderSettings| {
                s.search_paths = search.clone();
            },
        );
        let label = abs
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or(load_path);
        let n = inflight.0.len();
        // Stagger mounts on a 4 m grid so multiple loads don't pile.
        let cols = ((n as f32 + 1.0).sqrt().ceil() as usize).max(1);
        let r = n / cols;
        let c = n % cols;
        let half = (cols as f32 - 1.0) * 0.5;
        let mount = Vec3::new((c as f32 - half) * 4.0, 0.0, r as f32 * 4.0 - 2.0);
        info!("Load USD: {label} → mount={mount:?}");
        inflight.0.push(InflightLoad {
            handle,
            path: abs,
            label,
            mount,
            spawned: false,
        });
    }
}

fn spawn_when_loaded(
    mut commands: Commands,
    mut inflight: ResMut<Inflight>,
    mut controller_inventory: ResMut<ControllerInventory>,
    usd_assets: Res<Assets<UsdAsset>>,
) {
    for entry in inflight.0.iter_mut() {
        if entry.spawned {
            continue;
        }
        let Some(asset) = usd_assets.get(&entry.handle) else {
            continue;
        };
        let scene_root = commands
            .spawn((
                Name::new(entry.label.clone()),
                SceneRoot(asset.scene.clone()),
                Transform::from_translation(entry.mount),
                LoadedAsset {
                    path: entry.path.clone(),
                    label: entry.label.clone(),
                },
                UsdAssetHandle(entry.handle.clone()),
            ))
            .id();
        entry.spawned = true;
        info!("Spawned {} at {:?}", entry.label, entry.mount);

        match discover_machines_from_usd(&entry.path) {
            Ok(machines) => {
                log_discovered_machines(&entry.label, &machines);
                controller_inventory.push_loaded_asset(
                    scene_root,
                    entry.label.clone(),
                    entry.path.to_string_lossy(),
                    machines,
                );
            }
            Err(err) => {
                warn!(
                    "gearbox-control: failed to scan {} for machine controllers: {err}",
                    entry.path.display()
                );
            }
        }
    }
}
