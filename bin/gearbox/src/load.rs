//! Multi-USD loading: CLI args + 📂 ribbon button → asset_server →
//! `SceneRoot` with `LoadedAsset` marker. The marker also tags the
//! root for the UI's pick-and-gizmo wiring.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use bevy::scene::SceneRoot;
use serde::{Deserialize, Serialize};
use usd_bevy::{UsdAsset, UsdLoaderSettings};
use zenoh::Wait;

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
    transform: Transform,
    namespace: Option<String>,
    spawned: bool,
}

#[derive(Resource, Default)]
struct Inflight(Vec<InflightLoad>);

/// Generic runtime USD load request.
///
/// This intentionally says nothing about tractors, bales, robots, etc. The
/// caller gives Gearbox a USD path and an optional placement transform; the
/// usual USD loader/discovery path decides whether that USD also contains a
/// machine/controller.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeUsdLoadWire {
    #[serde(default)]
    pub category: String,
    pub usd_path: String,
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default)]
    pub z: f32,
    #[serde(default)]
    pub yaw_deg: f32,
    #[serde(default)]
    pub label: Option<String>,
    /// Optional runtime machine namespace for instance-specific controller
    /// topics when the same USD is spawned multiple times.
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub remove: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeUsdLoadedWire {
    pub usd_path: String,
    pub label: String,
    pub namespace: Option<String>,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw_deg: f32,
}

#[derive(Resource)]
struct RuntimeUsdLoader {
    session: Arc<zenoh::Session>,
    inbox: Arc<Mutex<VecDeque<RuntimeUsdLoadWire>>>,
    _legacy_spawn_subscriber: zenoh::pubsub::Subscriber<()>,
    _load_subscriber: zenoh::pubsub::Subscriber<()>,
}

impl RuntimeUsdLoader {
    fn open() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let session = Arc::new(zenoh::open(zenoh::Config::default()).wait()?);
        let inbox: Arc<Mutex<VecDeque<RuntimeUsdLoadWire>>> = Arc::new(Mutex::new(VecDeque::new()));
        let legacy_spawn_inbox_cb = Arc::clone(&inbox);
        let legacy_spawn_subscriber = session
            .declare_subscriber("gearbox/usd/spawn")
            .callback(move |sample| {
                let bytes = sample.payload().to_bytes();
                match decode::<RuntimeUsdLoadWire>(bytes.as_ref()) {
                    Ok(req) => {
                        if let Ok(mut q) = legacy_spawn_inbox_cb.lock() {
                            q.push_back(req);
                        }
                    }
                    Err(err) => {
                        eprintln!("gearbox-load: bad legacy USD spawn payload: {err}");
                    }
                }
            })
            .wait()?;
        let load_inbox_cb = Arc::clone(&inbox);
        let load_subscriber = session
            .declare_subscriber("gearbox/usd/load/**")
            .callback(move |sample| {
                let bytes = sample.payload().to_bytes();
                match decode::<RuntimeUsdLoadWire>(bytes.as_ref()) {
                    Ok(req) => {
                        let category = req.category.as_str();
                        if matches!(category, "machine" | "robot") {
                            if let Ok(mut q) = load_inbox_cb.lock() {
                                q.push_back(req);
                            }
                        }
                    }
                    Err(err) => {
                        eprintln!("gearbox-load: bad USD load payload: {err}");
                    }
                }
            })
            .wait()?;
        Ok(Self {
            session,
            inbox,
            _legacy_spawn_subscriber: legacy_spawn_subscriber,
            _load_subscriber: load_subscriber,
        })
    }

    fn drain_inbox(&self) -> Vec<RuntimeUsdLoadWire> {
        match self.inbox.lock() {
            Ok(mut q) => q.drain(..).collect(),
            Err(_) => Vec::new(),
        }
    }

    fn publish_loaded(&self, ev: &RuntimeUsdLoadedWire) {
        let Ok(bytes) = encode(ev) else { return };
        if let Err(err) = self
            .session
            .put("gearbox/usd/spawned", bytes.clone())
            .wait()
        {
            warn!("gearbox-load: failed to publish legacy gearbox/usd/spawned: {err}");
        }
        if let Err(err) = self.session.put("gearbox/usd/loaded", bytes).wait() {
            warn!("gearbox-load: failed to publish gearbox/usd/loaded: {err}");
        }
    }
}

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
            .add_systems(Startup, open_runtime_usd_loader)
            .add_systems(
                Update,
                (
                    drain_load_queue,
                    drain_runtime_usd_loader,
                    spawn_when_loaded,
                ),
            );
    }
}

fn open_runtime_usd_loader(mut commands: Commands) {
    match RuntimeUsdLoader::open() {
        Ok(api) => {
            commands.insert_resource(api);
            info!("gearbox-load: USD machine loader ready (gearbox/usd/load/<id>)");
        }
        Err(err) => {
            warn!("gearbox-load: runtime USD loader disabled: {err}");
        }
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
        let label = abs
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| abs.to_string_lossy().into_owned());
        let n = inflight.0.len();
        // Stagger mounts on a 4 m grid so multiple loads don't pile.
        let cols = ((n as f32 + 1.0).sqrt().ceil() as usize).max(1);
        let r = n / cols;
        let c = n % cols;
        let half = (cols as f32 - 1.0) * 0.5;
        let mount = Vec3::new((c as f32 - half) * 4.0, 0.0, r as f32 * 4.0 - 2.0);
        queue_usd_load(
            &asset_server,
            &mut inflight,
            abs,
            label,
            Transform::from_translation(mount),
            None,
        );
    }
}

fn drain_runtime_usd_loader(
    asset_server: Res<AssetServer>,
    api: Option<Res<RuntimeUsdLoader>>,
    mut inflight: ResMut<Inflight>,
    mut physics_active: ResMut<usd_bevy::physics::PhysicsActive>,
) {
    let Some(api) = api else { return };
    for req in api.drain_inbox() {
        if req.remove {
            continue;
        }
        let category = if req.category.is_empty() {
            "machine"
        } else {
            req.category.as_str()
        };
        if !matches!(category, "machine" | "robot") {
            continue;
        }
        if !physics_active.0 {
            physics_active.0 = true;
            info!("gearbox-load: enabling physics for runtime machine USD load");
        }
        let path = resolve_spawn_path(&req.usd_path);
        let label = req.label.clone().unwrap_or_else(|| {
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| req.usd_path.clone())
        });
        let transform = Transform {
            translation: Vec3::new(req.x, req.y, req.z),
            rotation: Quat::from_rotation_y(req.yaw_deg.to_radians()),
            ..default()
        };
        queue_usd_load(
            &asset_server,
            &mut inflight,
            path.clone(),
            label.clone(),
            transform,
            req.namespace.clone(),
        );
        api.publish_loaded(&RuntimeUsdLoadedWire {
            usd_path: path.to_string_lossy().into_owned(),
            label,
            namespace: req.namespace,
            x: req.x,
            y: req.y,
            z: req.z,
            yaw_deg: req.yaw_deg,
        });
    }
}

fn queue_usd_load(
    asset_server: &AssetServer,
    inflight: &mut Inflight,
    path: PathBuf,
    label: String,
    transform: Transform,
    namespace: Option<String>,
) {
    let parent = path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let search = vec![parent];
    let load_path = path.to_string_lossy().into_owned();
    let handle: Handle<UsdAsset> = asset_server.load_with_settings::<UsdAsset, _>(
        load_path,
        move |s: &mut UsdLoaderSettings| {
            s.search_paths = search.clone();
        },
    );
    info!(
        "Load USD: {label} → translation={:?} yaw={:.1}°",
        transform.translation,
        transform.rotation.to_euler(EulerRot::YXZ).0.to_degrees(),
    );
    inflight.0.push(InflightLoad {
        handle,
        path,
        label,
        transform,
        namespace,
        spawned: false,
    });
}

fn resolve_spawn_path(raw: &str) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        return path;
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let direct = cwd.join(&path);
    if direct.exists() {
        return direct;
    }

    let from_assets = default_asset_root().join(&path);
    if from_assets.exists() {
        return from_assets;
    }

    direct
}

pub fn default_asset_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets")
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
                entry.transform,
                LoadedAsset {
                    path: entry.path.clone(),
                    label: entry.label.clone(),
                },
                UsdAssetHandle(entry.handle.clone()),
            ))
            .id();
        entry.spawned = true;
        info!(
            "Spawned {} at {:?}",
            entry.label, entry.transform.translation
        );

        match discover_machines_from_usd(&entry.path) {
            Ok(mut machines) => {
                if let Some(namespace) = entry.namespace.as_deref() {
                    apply_runtime_namespace(&mut machines, namespace);
                }
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

fn apply_runtime_namespace(
    machines: &mut [crate::controller::MachineInstanceSpec],
    namespace: &str,
) {
    for machine in machines {
        machine.id = namespace.to_string();
        for controller in &mut machine.controllers {
            controller.namespace = namespace.to_string();
        }
    }
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, ciborium::ser::Error<std::io::Error>> {
    let mut buf = Vec::new();
    ciborium::into_writer(value, &mut buf)?;
    Ok(buf)
}

fn decode<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, ciborium::de::Error<std::io::Error>> {
    ciborium::from_reader(bytes)
}
