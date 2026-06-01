//! Multi-USD loading: CLI args + 📂 ribbon button → asset_server →
//! `SceneRoot` with `LoadedAsset` marker. The marker also tags the
//! root for the UI's pick-and-gizmo wiring.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bevy::math::{DQuat, DVec3};
use bevy::prelude::*;
use bevy::scene::SceneRoot;
use bevy::transform::TransformSystems;
use rapier3d::prelude::Pose;
use serde::{Deserialize, Serialize};
use usd_bevy::{UsdAsset, UsdLoaderSettings};
use zenoh::Wait;

use crate::controller::{ControllerInventory, discover_machines_from_usd, log_discovered_machines};
use crate::world::terrain_height_m;

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
    activate_physics_after_sync: bool,
    spawned: bool,
}

#[derive(Resource, Default)]
struct Inflight(Vec<InflightLoad>);

#[derive(Component, Debug, Clone, Copy)]
struct MachinePhysicsSyncPending {
    activate_after_sync: bool,
    frames_waited: u32,
}

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
    #[serde(default)]
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
    _load_subscriber: zenoh::pubsub::Subscriber<()>,
}

impl RuntimeUsdLoader {
    fn open() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let session = Arc::new(zenoh::open(zenoh::Config::default()).wait()?);
        let inbox: Arc<Mutex<VecDeque<RuntimeUsdLoadWire>>> = Arc::new(Mutex::new(VecDeque::new()));
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
                    clear_runtime_usd_loads_on_reset_system,
                    drain_load_queue,
                    drain_runtime_usd_loader,
                    spawn_when_loaded,
                ),
            )
            .add_systems(
                PostUpdate,
                sync_pending_machine_physics_to_scene_transforms.after(TransformSystems::Propagate),
            );
    }
}

fn clear_runtime_usd_loads_on_reset_system(
    messages: Option<MessageReader<gearbox_api::SimResetRequest>>,
    mut commands: Commands,
    mut inflight: ResMut<Inflight>,
    mut controller_inventory: ResMut<ControllerInventory>,
    mut physics: ResMut<usd_bevy::physics::PhysicsWorld>,
    loaded_roots: Query<Entity, With<LoadedAsset>>,
    children_q: Query<&Children>,
) {
    let Some(mut messages) = messages else { return };
    if messages.read().count() == 0 {
        return;
    }

    inflight.0.clear();
    controller_inventory.machines.clear();

    let physics = physics.as_mut();
    let mut cleared = 0usize;
    for root in loaded_roots.iter() {
        remove_loaded_usd_physics(root, physics, &children_q);
        commands.entity(root).despawn();
        cleared += 1;
    }
    if cleared > 0 {
        info!("gearbox-load: cleared {cleared} runtime USD load(s)");
    }
}

fn remove_loaded_usd_physics(
    root: Entity,
    physics: &mut usd_bevy::physics::PhysicsWorld,
    children_q: &Query<&Children>,
) {
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if let Some(handle) = physics.entity_to_body.remove(&entity) {
            let _ = physics.bodies.remove(
                handle,
                &mut physics.islands,
                &mut physics.colliders,
                &mut physics.impulse_joints,
                &mut physics.multibody_joints,
                true,
            );
        }
        if let Some(collider) = physics.entity_to_collider.remove(&entity) {
            physics
                .colliders
                .remove(collider, &mut physics.islands, &mut physics.bodies, false);
        }
        if let Ok(children) = children_q.get(entity) {
            stack.extend(children.iter());
        }
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
        let x = (c as f32 - half) * 4.0;
        let z = r as f32 * 4.0 - 2.0;
        let mount = Vec3::new(x, terrain_height_m(x, z), z);
        queue_usd_load(
            &asset_server,
            &mut inflight,
            abs,
            label,
            Transform::from_translation(mount),
            None,
            None,
            Vec::new(),
            false,
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
        if physics_active.0 {
            physics_active.0 = false;
            info!("gearbox-load: pausing physics until new machine USD is aligned to terrain");
        }
        let path = resolve_spawn_path(&req.usd_path);
        let (load_path, source_path, extra_search_paths) = hotload_runtime_usd_path(&path);
        let label = req.label.clone().unwrap_or_else(|| {
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| req.usd_path.clone())
        });
        let mut transform = Transform {
            translation: Vec3::new(req.x, req.y, req.z),
            rotation: Quat::from_rotation_y(req.yaw_deg.to_radians()),
            ..default()
        };
        snap_grounded_machine_to_terrain(&mut transform);
        queue_usd_load(
            &asset_server,
            &mut inflight,
            load_path,
            label.clone(),
            transform,
            req.namespace.clone(),
            Some(source_path.clone()),
            extra_search_paths,
            true,
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
    source_path: Option<PathBuf>,
    extra_search_paths: Vec<PathBuf>,
    activate_physics_after_sync: bool,
) {
    let parent = path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let mut search = vec![parent];
    for extra in extra_search_paths {
        if !search.iter().any(|existing| existing == &extra) {
            search.push(extra);
        }
    }
    let source_path = source_path.unwrap_or_else(|| path.clone());
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
        path: source_path,
        label,
        transform,
        namespace,
        activate_physics_after_sync,
        spawned: false,
    });
}

fn hotload_runtime_usd_path(path: &Path) -> (PathBuf, PathBuf, Vec<PathBuf>) {
    let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
        return (
            path.to_path_buf(),
            path.to_path_buf(),
            default_search_paths(path),
        );
    };
    if !matches!(ext.to_ascii_lowercase().as_str(), "usd" | "usda" | "usdc") {
        return (
            path.to_path_buf(),
            path.to_path_buf(),
            default_search_paths(path),
        );
    }

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy())
        .unwrap_or_else(|| std::borrow::Cow::Borrowed("runtime"));
    let hotload_dir = std::env::temp_dir().join("gearbox_usd_hotload");
    let hotload_path = hotload_dir.join(format!("{stem}_{}_{}.{}", std::process::id(), stamp, ext));

    if let Err(err) = std::fs::create_dir_all(&hotload_dir) {
        warn!(
            "gearbox-load: failed to create hotload dir {}; using cached USD path: {err}",
            hotload_dir.display()
        );
        return (
            path.to_path_buf(),
            path.to_path_buf(),
            default_search_paths(path),
        );
    }
    if let Err(err) = std::fs::copy(path, &hotload_path) {
        warn!(
            "gearbox-load: failed to hotload-copy {}; using cached USD path: {err}",
            path.display()
        );
        return (
            path.to_path_buf(),
            path.to_path_buf(),
            default_search_paths(path),
        );
    }

    let mut search = default_search_paths(path);
    search.push(hotload_dir);
    (hotload_path, path.to_path_buf(), search)
}

fn default_search_paths(path: &Path) -> Vec<PathBuf> {
    path.parent()
        .map(|p| vec![p.to_path_buf()])
        .unwrap_or_default()
}

fn snap_grounded_machine_to_terrain(transform: &mut Transform) {
    // Runtime machine load requests use y=0 to mean "put this machine on the
    // ground". Once the world became a real heightfield, leaving y=0 made
    // tractors float above valleys or spawn half-buried inside hills. Treat
    // nonzero y as an explicit caller-provided vertical offset.
    if transform.translation.y.abs() < 0.001 {
        transform.translation.y =
            terrain_height_m(transform.translation.x, transform.translation.z);
    }
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
        let mut discovered_machines = match discover_machines_from_usd(&entry.path) {
            Ok(machines) => Some(machines),
            Err(err) => {
                warn!(
                    "gearbox-control: failed to scan {} for machine controllers: {err}",
                    entry.path.display()
                );
                None
            }
        };
        let is_machine_asset = discovered_machines
            .as_ref()
            .is_some_and(|machines| !machines.is_empty());
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
        if is_machine_asset {
            commands
                .entity(scene_root)
                .insert(MachinePhysicsSyncPending {
                    activate_after_sync: entry.activate_physics_after_sync,
                    frames_waited: 0,
                });
        }
        entry.spawned = true;
        info!(
            "Spawned {} at {:?}",
            entry.label, entry.transform.translation
        );

        if let Some(mut machines) = discovered_machines.take() {
            if let Some(namespace) = entry.namespace.as_deref() {
                apply_runtime_namespace(&mut machines, namespace);
            }
            log_discovered_machines(&entry.label, &machines);
            if !machines.is_empty() {
                controller_inventory.push_loaded_asset(
                    scene_root,
                    entry.label.clone(),
                    entry.path.to_string_lossy(),
                    machines,
                );
            }
        }
    }
}

fn sync_pending_machine_physics_to_scene_transforms(
    mut commands: Commands,
    mut pending: Query<(
        Entity,
        &mut Transform,
        &mut MachinePhysicsSyncPending,
        Option<&Name>,
    )>,
    children: Query<&Children>,
    globals: Query<&GlobalTransform>,
    names: Query<&Name>,
    mut physics: ResMut<usd_bevy::physics::PhysicsWorld>,
    mut physics_active: ResMut<usd_bevy::physics::PhysicsActive>,
) {
    for (root, mut root_transform, mut pending, name) in pending.iter_mut() {
        let descendants = collect_descendants(root, &children);
        let body_entities = descendants
            .into_iter()
            .filter_map(|entity| {
                physics
                    .entity_to_body
                    .get(&entity)
                    .copied()
                    .map(|h| (entity, h))
            })
            .collect::<Vec<_>>();

        if body_entities.is_empty() {
            pending.frames_waited += 1;
            if pending.frames_waited == 120 {
                warn!(
                    "gearbox-load: waiting for physics bodies before terrain-aligning {}",
                    name.map(|n| n.as_str()).unwrap_or("<unnamed machine>")
                );
            }
            continue;
        }

        let mut synced = 0usize;
        for (entity, handle) in body_entities {
            let Ok(gt) = globals.get(entity) else {
                continue;
            };
            let Some(body) = physics.bodies.get_mut(handle) else {
                continue;
            };
            let transform = gt.compute_transform();
            body.set_position(
                Pose {
                    translation: DVec3::new(
                        transform.translation.x as f64,
                        transform.translation.y as f64,
                        transform.translation.z as f64,
                    ),
                    rotation: DQuat::from_xyzw(
                        transform.rotation.x as f64,
                        transform.rotation.y as f64,
                        transform.rotation.z as f64,
                        transform.rotation.w as f64,
                    ),
                },
                true,
            );
            body.set_linvel(DVec3::ZERO, true);
            body.set_angvel(DVec3::ZERO, true);
            synced += 1;
        }

        if synced == 0 {
            continue;
        }

        propagate_body_positions_to_colliders(physics.as_mut());

        let collider_entities = collect_descendants(root, &children)
            .into_iter()
            .filter_map(|entity| {
                physics
                    .entity_to_collider
                    .get(&entity)
                    .copied()
                    .map(|handle| (entity, handle))
            })
            .collect::<Vec<_>>();
        if let Some(delta_y) = terrain_contact_alignment_delta(&physics, &collider_entities, &names)
        {
            root_transform.translation.y += delta_y as f32;
            for handle in physics
                .entity_to_body
                .iter()
                .filter(|(entity, _)| is_descendant_or_self(root, **entity, &children))
                .map(|(_, handle)| *handle)
                .collect::<Vec<_>>()
            {
                if let Some(body) = physics.bodies.get_mut(handle) {
                    let mut pose = *body.position();
                    pose.translation.y += delta_y;
                    body.set_position(pose, true);
                    body.set_linvel(DVec3::ZERO, true);
                    body.set_angvel(DVec3::ZERO, true);
                }
            }
            propagate_body_positions_to_colliders(physics.as_mut());
            info!(
                "gearbox-load: terrain contact adjusted {} by {delta_y:+.3} m",
                name.map(|n| n.as_str()).unwrap_or("<unnamed machine>")
            );
        }

        commands.entity(root).remove::<MachinePhysicsSyncPending>();
        if pending.activate_after_sync {
            physics_active.0 = true;
            info!(
                "gearbox-load: terrain-aligned {} physics bodies for {}; enabling physics",
                synced,
                name.map(|n| n.as_str()).unwrap_or("<unnamed machine>")
            );
        } else {
            info!(
                "gearbox-load: terrain-aligned {} physics bodies for {}",
                synced,
                name.map(|n| n.as_str()).unwrap_or("<unnamed machine>")
            );
        }
    }
}

fn propagate_body_positions_to_colliders(physics: &mut usd_bevy::physics::PhysicsWorld) {
    let bodies = &physics.bodies;
    let colliders = &mut physics.colliders;
    bodies.propagate_modified_body_positions_to_colliders(colliders);
}

fn terrain_contact_alignment_delta(
    physics: &usd_bevy::physics::PhysicsWorld,
    collider_entities: &[(Entity, rapier3d::prelude::ColliderHandle)],
    names: &Query<&Name>,
) -> Option<f64> {
    let mut tire_handles = Vec::new();
    let mut fallback_handles = Vec::new();
    for (entity, handle) in collider_entities {
        fallback_handles.push(*handle);
        let lower = names
            .get(*entity)
            .map(|name| name.as_str().to_ascii_lowercase())
            .unwrap_or_default();
        if lower.contains("tire") || lower.contains("tyre") || lower.contains("wheel") {
            tire_handles.push(*handle);
        }
    }

    let handles = if tire_handles.is_empty() {
        &fallback_handles
    } else {
        &tire_handles
    };

    let mut min_clearance = f64::INFINITY;
    for handle in handles {
        let Some(collider) = physics.colliders.get(*handle) else {
            continue;
        };
        let aabb = collider.compute_aabb();
        let ground =
            max_terrain_height_under_aabb(aabb.mins.x, aabb.maxs.x, aabb.mins.z, aabb.maxs.z);
        let clearance = aabb.mins.y - ground;
        min_clearance = min_clearance.min(clearance);
    }

    if !min_clearance.is_finite() {
        return None;
    }

    // Leave a tiny positive clearance so the first physics tick settles the
    // tyres onto the heightfield instead of starting with interpenetration.
    let desired_clearance = 0.03;
    let delta = desired_clearance - min_clearance;
    if delta.abs() > 0.002 {
        Some(delta)
    } else {
        None
    }
}

fn max_terrain_height_under_aabb(min_x: f64, max_x: f64, min_z: f64, max_z: f64) -> f64 {
    let samples = [
        ((min_x + max_x) * 0.5, (min_z + max_z) * 0.5),
        (min_x, min_z),
        (min_x, max_z),
        (max_x, min_z),
        (max_x, max_z),
    ];
    samples
        .into_iter()
        .map(|(x, z)| terrain_height_m(x as f32, z as f32) as f64)
        .fold(f64::NEG_INFINITY, f64::max)
}

fn is_descendant_or_self(root: Entity, candidate: Entity, children: &Query<&Children>) -> bool {
    if root == candidate {
        return true;
    }
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        let Ok(kids) = children.get(entity) else {
            continue;
        };
        for child in kids.iter() {
            if child == candidate {
                return true;
            }
            stack.push(child);
        }
    }
    false
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
