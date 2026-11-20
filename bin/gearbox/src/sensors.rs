//! Simulated sensors on authored `role = "sensor"` links: Molla GPU IMU,
//! LiDAR, batched cameras, GPU annotations and async GPU ray sensors sampled
//! from the shared Molla sensor scene at their own simulated-time rates and
//! streamed on `/machines/<machine_id>/sensors/<link>`.
//!
//! The rig of a machine snapshots the physics scene's geometry once per scene
//! revision and copies body poses/velocities at each sample, then encodes all
//! due GPU sensors into one [`SensorCommandBatch`] submission and collects
//! results asynchronously on later frames without holding the physics lock.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::sync::Arc;
use std::time::Duration;

use bevy::camera::primitives::Aabb;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::render::renderer::{RenderAdapterInfo, RenderDevice, RenderQueue};
use bevy::tasks::{AsyncComputeTaskPool, Task};
use gearbox_api::datapod::robot::Imu;
use gearbox_api::datapod::{Acceleration, Quaternion, Velocity};
use gearbox_api::{
    CAMERA_COLOR, CAMERA_DEPTH, CameraFrame, GearboxBus, LidarScan, Props,
    measurement_kind,
};
use molla_compute::{AdapterInfo, Engine};
use molla_core::BodyId as MollaBodyId;
use molla_math::{Mat3 as MMat3, Quat as MQuat, Transform as MTransform, Vec3 as MVec3};
use molla_sensors::readback::{ReadbackConfig, ReadbackRing, ReadbackStamp, ReadbackTicket};
use molla_sensors::{
    AnnotationConfig, AnnotationCounts, AnnotationProcessor, CameraBatch, CameraBatchLimits,
    CameraBatchOutputPool, CameraChannels, CameraSpec, DeviceBoundingBox, EncodedCameraBatch,
    ImuMount, ImuSensor, LidarBatch, LidarPattern, LidarScan as MollaLidarScan, PhysicalOptics,
    RadarTarget, Raycaster, RecognizedObject, ScanMetadata, ScheduledSample, SensorCommandBatch,
    SensorFramePool, SensorFramePoolConfig, SensorScene, SensorSceneOptions,
    SensorSchedule, SensorScheduler, ShapeLabel, TiledCameraConfig, UnsupportedGeometryPolicy,
};
use molla_sim::runtime::{ColliderGeometry, RigidScene, SceneStamp};
use molla_sim::{Model, State};
use molla_storage::{Buffer, BufferUsage};
use usd_bevy::UsdPrimRef;

use crate::controller::{ControllerInventory, MachineAgentKeys};
use crate::prim_index::PrimIndex;
use crate::links::{LinkSpec, LinkTree, SensorKind, SensorSpec};
use crate::sensor_radio::RadioLink;
use crate::sensor_generic::{
    GenericSample, GenericSensors, PendingRayPlan, SensorEnvironment, is_generic, measurement,
    recognition,
};
use crate::physics::backend::{BodyId, PhysicsBackend};
use crate::physics::{PhysicsActive, PhysicsWorld};

pub struct SensorsPlugin;

impl Plugin for SensorsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MachineSensorRigs>()
            .add_plugins(crate::sensor_cameras::SensorCamerasPlugin)
            .add_systems(
                PostUpdate,
                (
                    crate::sensor_capture::bump_scene_epoch_on_reset,
                    run_machine_sensors.after(bevy::transform::TransformSystems::Propagate),
                )
                    .chain(),
            );
    }
}

/// The name recognition gives a collider's entity in place of its prim path.
#[derive(Component, Clone, Debug)]
pub struct SensorLabel(pub String);

/// Authored radar cross-section (m²) of a rigid body entity. A positive value
/// overrides the default 1 m² on dynamic bodies and makes fixed or kinematic
/// bodies detectable by radar; zero excludes the body from radar detection.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct RadarCrossSection(pub f64);

/// Albedo for colliders without a matching visual: terrain and entity-less
/// ground, then everything else.
const GROUND_COLOR: [f32; 3] = [0.36, 0.45, 0.24];
const NEUTRAL_COLOR: [f32; 3] = [0.62, 0.62, 0.6];
/// Entities visited per visual-colour search, and ancestors climbed.
const VISUAL_SEARCH_NODES: usize = 4096;
const VISUAL_SEARCH_LEVELS: usize = 3;

/// Display colours of collider entities for the sensor cameras: a collider
/// takes the size-weighted mean of the visible mesh materials under itself,
/// else under its nearest ancestor that has any (invisible physics proxies
/// then take their body's look).
#[derive(SystemParam)]
pub struct VisualColors<'w, 's> {
    parents: Query<'w, 's, &'static ChildOf>,
    children: Query<'w, 's, &'static Children>,
    meshes: Query<
        'w,
        's,
        (
            &'static MeshMaterial3d<StandardMaterial>,
            Option<&'static Aabb>,
            Option<&'static Visibility>,
        ),
    >,
    materials: Res<'w, Assets<StandardMaterial>>,
    images: Res<'w, Assets<Image>>,
}

/// Subtree and material colours already averaged within one scene build.
#[derive(Default)]
struct ColorMemo {
    subtrees: RefCell<HashMap<Entity, Option<[f32; 3]>>>,
    materials: RefCell<HashMap<AssetId<StandardMaterial>, [f32; 3]>>,
}

impl VisualColors<'_, '_> {
    /// A colour lookup for one scene build, sharing every subtree and
    /// material average between the colliders it colours.
    pub fn memo(&self) -> impl Fn(Entity) -> Option<[f32; 3]> + '_ {
        let memo = ColorMemo::default();
        move |entity| self.color(entity, &memo)
    }

    fn color(&self, entity: Entity, memo: &ColorMemo) -> Option<[f32; 3]> {
        let mut root = entity;
        for _ in 0..VISUAL_SEARCH_LEVELS {
            if let Some(color) = self.subtree_color(root, memo) {
                return Some(color);
            }
            root = self.parents.get(root).ok()?.parent();
        }
        None
    }

    fn subtree_color(&self, root: Entity, memo: &ColorMemo) -> Option<[f32; 3]> {
        let cached = memo.subtrees.borrow().get(&root).copied();
        if let Some(color) = cached {
            return color;
        }
        let (mut sum, mut weight) = ([0.0f32; 3], 0.0f32);
        let mut stack = vec![root];
        let mut visited = 0;
        while let Some(entity) = stack.pop() {
            visited += 1;
            if visited > VISUAL_SEARCH_NODES {
                break;
            }
            if let Ok((material, aabb, visibility)) = self.meshes.get(entity)
                && visibility != Some(&Visibility::Hidden)
                && let Some(color) = self.material_color(material.id(), memo)
            {
                let w = aabb.map_or(1.0, |b| {
                    let h = b.half_extents;
                    (h.x * h.y + h.y * h.z + h.z * h.x).max(1e-6)
                });
                for i in 0..3 {
                    sum[i] += color[i] * w;
                }
                weight += w;
            }
            if let Ok(children) = self.children.get(entity) {
                stack.extend(children.iter());
            }
        }
        let color = (weight > 0.0).then(|| sum.map(|s| s / weight));
        memo.subtrees.borrow_mut().insert(root, color);
        color
    }

    fn material_color(&self, id: AssetId<StandardMaterial>, memo: &ColorMemo) -> Option<[f32; 3]> {
        let cached = memo.materials.borrow().get(&id).copied();
        if cached.is_some() {
            return cached;
        }
        let material = self.materials.get(id)?;
        let base = material.base_color.to_srgba();
        let texel = material
            .base_color_texture
            .as_ref()
            .and_then(|texture| self.images.get(texture))
            .and_then(mean_texel)
            .unwrap_or([1.0; 3]);
        let color = [base.red * texel[0], base.green * texel[1], base.blue * texel[2]];
        memo.materials.borrow_mut().insert(id, color);
        Some(color)
    }
}

/// Mean colour of an 8-bit RGBA texture still held on the CPU.
fn mean_texel(image: &Image) -> Option<[f32; 3]> {
    let data = image.data.as_ref()?;
    if !matches!(
        image.texture_descriptor.format,
        wgpu::TextureFormat::Rgba8UnormSrgb | wgpu::TextureFormat::Rgba8Unorm
    ) || data.len() < 4
    {
        return None;
    }
    let stride = (data.len() / 4 / 4096).max(1);
    let (mut sum, mut count) = ([0.0f32; 3], 0.0f32);
    for texel in data.chunks_exact(4).step_by(stride) {
        for i in 0..3 {
            sum[i] += texel[i] as f32 / 255.0;
        }
        count += 1.0;
    }
    Some(sum.map(|s| s / count))
}

/// Sensor rigs per machine id over one engine that adopts the render device.
#[derive(Resource, Default)]
pub struct MachineSensorRigs {
    engine: Option<Engine>,
    rigs: HashMap<String, Rig>,
    /// Scene revision at which a machine's rig build last failed.
    failed: HashMap<String, u64>,
    /// Scene revision at which an individual `(machine_id, link_name)` last failed.
    failed_links: HashMap<(String, String), u64>,
    /// The sensor scene every rig samples.
    scene: Option<SharedScene>,
    /// The next scene, building off the frame.
    building: Option<SceneBuild>,
}

/// A sensor scene of one physics model, over one order of bodies.
pub struct SharedScene {
    model: SceneStamp,
    bodies: Vec<u64>,
    shape_bodies: Arc<[MollaBodyId]>,
    scene: Arc<SensorScene>,
}

/// What recognition names in a scene: each shape's object and each object's label.
#[derive(Default)]
pub struct SceneObjects {
    shapes: Vec<Option<u32>>,
    labels: Vec<String>,
}

/// A newer model's scene building on a worker while the rigs sample the
/// last one, with what each rig will recognise in it.
struct SceneBuild {
    model: SceneStamp,
    bodies: Vec<u64>,
    shape_bodies: Arc<[MollaBodyId]>,
    objects: HashMap<String, Arc<SceneObjects>>,
    radar_targets: HashMap<String, Vec<RadarTarget>>,
    task: Task<Result<SensorScene, String>>,
}

/// The sensor scene of the physics world's model, built once per model
/// however many machines sample it.
fn shared_scene<'a>(
    engine: &Engine,
    cached: &'a mut Option<SharedScene>,
    physics: &dyn PhysicsBackend,
    model: &SceneStamp,
    colors: &dyn Fn(Entity) -> Option<[f32; 3]>,
) -> Result<&'a SharedScene, String> {
    if !cached.as_ref().is_some_and(|shared| &shared.model == model) {
        let (scene, bodies, shape_bodies) = physics.molla(|rigid| {
            sensor_scene(engine, rigid, colors)
                .map(|scene| (scene, body_order(rigid), shape_bodies(rigid)))
        })?;
        *cached = Some(SharedScene {
            model: model.clone(),
            bodies,
            shape_bodies,
            scene: Arc::new(scene),
        });
    }
    Ok(cached.as_ref().expect("scene just built"))
}

/// Hands a finished scene build to every rig over the same bodies not
/// already on the physics world's `model`.
fn swap_built_scene(rigs: &mut MachineSensorRigs, model: &SceneStamp) {
    let Some(build) = rigs.building.as_mut() else { return };
    let Some(done) = bevy::tasks::block_on(bevy::tasks::poll_once(&mut build.task)) else { return };
    let build = rigs.building.take().expect("build polled");
    let scene = match done {
        Ok(scene) => Arc::new(scene),
        Err(error) => {
            warn!("gearbox-sensors: {error}");
            return;
        }
    };
    report_unsupported(&scene);
    for (id, rig) in rigs.rigs.iter_mut() {
        if let Some(objects) = build.objects.get(id)
            && rig.bodies == build.bodies
            && rig.model.as_ref() != Some(model)
        {
            rig.scene = scene.clone();
            rig.shape_bodies = build.shape_bodies.clone();
            rig.objects = objects.clone();
            if let Some(targets) = build.radar_targets.get(id) {
                rig.generic.set_radar_targets(targets.clone());
            }
            rig.model = Some(build.model.clone());
        }
    }
    if rigs.scene.as_ref().is_none_or(|shared| &shared.model != model) {
        rigs.scene = Some(SharedScene {
            model: build.model,
            bodies: build.bodies,
            shape_bodies: build.shape_bodies,
            scene,
        });
    }
}

impl MachineSensorRigs {
    pub fn has_rig(&self, machine_id: &str) -> bool {
        self.rigs.contains_key(machine_id)
    }

    /// The prim entities of every machine's sensor links.
    pub fn sensor_entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.rigs.values().flat_map(|rig| rig.mounts.iter().filter_map(|m| m.resolved.entity))
    }

    /// Published samples per sensor link of a machine, for diagnostics.
    pub fn published(&self, machine_id: &str) -> Vec<(String, u64)> {
        self.rigs
            .get(machine_id)
            .map(|rig| {
                rig.mounts
                    .iter()
                    .map(|m| (m.name.clone(), m.published))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Quaternion of the `odom` world remap `(ros_x, ros_y, ros_z) = (sim_z, sim_x, sim_y)`.
const SIM_TO_ROS: MQuat = MQuat::from_xyzw(0.5, 0.5, 0.5, 0.5);

/// A sensor link tied to the physics body it rides on.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedMount {
    pub link: usize,
    pub name: String,
    pub spec: SensorSpec,
    pub body: BodyId,
    /// The link prim's entity, which Bevy-rendered cameras are parented to.
    pub entity: Option<Entity>,
    /// A position sensor's joint: the one between its rigid body and the
    /// rigid body above.
    pub joint: Option<crate::physics::backend::JointId>,
    /// Sensor frame in the body frame.
    pub local: MTransform,
}

/// One reading ready to publish for the named sensor link.
pub enum SensorReading<'a> {
    Imu(&'a Imu),
    Lidar(&'a LidarScan),
    Camera(&'a CameraFrame),
    Measurement(&'a gearbox_api::Measurement),
}

/// Per-rig cost counters, reported every ten seconds.
#[derive(Default, Clone, Copy)]
pub struct RigStats {
    pub samples: u64,
    pub sample_seconds: f64,
    pub collect_seconds: f64,
    pub upload_bytes: u64,
    pub readback_bytes: u64,
    pub frames: u64,
    pub dropped: u64,
    pub gpu_batches: u64,
    pub gpu_dispatches: u64,
}

const POOL_SLOTS: usize = 4;
const MIN_GENERIC_RAY_CAPACITY: u32 = 64;
const MAX_ANNOTATION_LABELS: u32 = 65_536;
const MAX_ANNOTATION_BUFFER_BYTES: u64 = 64 * 1024 * 1024;

/// Traced cameras sharing one sample rate, batched into one [`CameraBatch`]
/// over the currently subscribed subset so unsubscribed cameras cost nothing
/// and homogeneous cameras trace together.
struct CameraBucket {
    rate_bits: u32,
    mounts: Vec<usize>,
    active: Vec<usize>,
    batch: Option<(CameraBatch, CameraBatchOutputPool)>,
}

/// GPU raycaster and output pool for async generic ray sensors (distance,
/// light, radar).
struct GenericRayGpu {
    capacity: u32,
    raycaster: Raycaster,
    pool: SensorFramePool,
}

/// GPU annotation processor chained directly off [`CameraBatch`] outputs for
/// camera recognition links.
struct AnnotationGpu {
    processor: AnnotationProcessor,
    pool: SensorFramePool,
    max_labels: u32,
}

struct Mount {
    resolved: ResolvedMount,
    name: String,
    body: MollaBodyId,
    published: u64,
    undelivered: u64,
    /// Whether a sweep's capture metadata was refused, so the warning shows once.
    capture_refused: bool,
}

impl Mount {
    /// Counts a sample the sink accepted, or warns once when it did not.
    fn record(&mut self, delivered: bool, hits: u32) {
        if delivered {
            self.published += 1;
            return;
        }
        if self.undelivered == 0 {
            warn!(
                "gearbox-sensors: sensor link `{}` has no reachable publisher; readings ({hits} hits) are dropped",
                self.name
            );
        }
        self.undelivered += 1;
    }
}

pub struct Rig {
    revision: u64,
    /// The machine id LiDAR capture metadata names; none is attached while empty.
    pub machine: String,
    /// The compiled model the rig's scene was built from: poses and controls
    /// it samples as they come; a new model over the same bodies hands it a
    /// new scene, over other bodies builds the rig again.
    model: Option<SceneStamp>,
    signature: Vec<ResolvedMount>,
    scene: Arc<SensorScene>,
    shape_bodies: Arc<[MollaBodyId]>,
    imu_model: Model,
    state: State,
    scheduler: SensorScheduler,
    mounts: Vec<Mount>,
    imu: Option<(ImuSensor, SensorFramePool)>,
    imu_mounts: Vec<usize>,
    last_imu_time: Option<f64>,
    lidar: Option<(LidarBatch, SensorFramePool)>,
    camera_buckets: Vec<CameraBucket>,
    max_camera_bytes: u64,
    annotation: Option<AnnotationGpu>,
    annotation_synced: Option<Arc<SceneObjects>>,
    generic_rays: Option<GenericRayGpu>,
    ring: ReadbackRing,
    ring_bytes_per_slot: u64,
    ring_slots: usize,
    pending: Vec<Pending>,
    render_requests: Vec<RenderRequest>,
    generic: GenericSensors,
    /// Generic readings awaiting publication.
    ready: Vec<(usize, gearbox_api::Measurement)>,
    /// What recognition names in the scene, the bodies the scene was built
    /// over, and the machine's own bodies, which recognition leaves out.
    objects: Arc<SceneObjects>,
    bodies: Vec<u64>,
    own: HashSet<u64>,
    recognition: bool,
    pub stats: RigStats,
    last_report: f64,
}

/// A Bevy-rendered colour image due for a camera link, stamped with the same
/// sample number and simulated time as the link's Molla depth.
pub struct RenderRequest {
    pub mount: usize,
    pub sample: u64,
    pub sim_time: f64,
}

/// Whether the Molla trace of a camera link produces its colour channel.
fn molla_color(spec: &SensorSpec) -> bool {
    spec.color && !spec.render.is_bevy()
}

/// Channels the Molla trace of a camera link reads back, in order:
/// recognition needs depth and shape indices.
fn camera_channels(spec: &SensorSpec) -> Vec<CameraChannels> {
    [
        (molla_color(spec), CameraChannels::COLOR),
        (spec.depth || spec.recognition, CameraChannels::DEPTH),
        (spec.recognition, CameraChannels::SHAPE_INDEX),
    ]
    .into_iter()
    .filter(|(wanted, _)| *wanted)
    .map(|(_, channel)| channel)
    .collect()
}

/// Splits one four-byte-per-pixel channel of a row-major camera image into
/// row bands that fit a shared-memory message and hands each to `send`;
/// false when any band was refused.
#[allow(clippy::too_many_arguments)]
pub fn camera_chunks(
    name: &str,
    link_index: u32,
    spec: &SensorSpec,
    sim_time: f64,
    stamp_ms: u32,
    sample: u64,
    channel: u32,
    pixels: &[u8],
    send: &mut dyn FnMut(&CameraFrame) -> bool,
) -> bool {
    let (width, height) = (spec.width, spec.height);
    let rows_per_chunk = CameraFrame::chunk_rows(width);
    let row_bytes = width as usize * 4;
    let props = Props::from_pairs(&[("name", name)]).into_bytes();
    let mut delivered = true;
    for row_offset in (0..height).step_by(rows_per_chunk as usize) {
        let rows = rows_per_chunk.min(height - row_offset);
        let first = row_offset as usize * row_bytes;
        let wire = CameraFrame {
            sim_time_s: sim_time,
            fov_y_rad: spec.vfov as f64,
            max_range_m: spec.range_m as f64,
            link_index,
            width,
            height,
            row_offset,
            rows,
            channel,
            stamp_ms,
            sample: sample as u32,
            data: pixels[first..first + rows as usize * row_bytes].to_vec(),
            props: props.clone(),
        };
        delivered &= send(&wire);
    }
    delivered
}

struct Pending {
    ticket: ReadbackTicket,
    sim_time: f64,
    kind: PendingKind,
}

enum PendingKind {
    Imu {
        due: Vec<(usize, MQuat)>,
    },
    Lidar {
        scans: Vec<PendingScan>,
    },
    /// Generic ray sensors (distance, light, radar) evaluated asynchronously on GPU.
    GenericRays {
        plan: PendingRayPlan,
        shape_bodies: Arc<[MollaBodyId]>,
    },
    /// Camera readback carrying pixel channels and optional GPU annotation boxes/counts.
    Camera {
        mount: usize,
        sample: u64,
        objects: Arc<SceneObjects>,
        has_color: bool,
        has_depth: bool,
        has_shape: bool,
        has_annotation: bool,
        has_boxes: bool,
    },
}

struct PendingScan {
    mount: usize,
    sample: u64,
    ray_offset: usize,
    ray_count: usize,
    meta: ScanMetadata,
    world: MTransform,
    capture: Option<crate::sensor_capture::CapturePose>,
}

/// `<machine>:<body prim>` for machine parts, the body prim otherwise.
fn part_label(
    entity: Entity,
    sensor_labels: &Query<&SensorLabel>,
    parents: &Query<&ChildOf>,
    prims: &Query<(Entity, &UsdPrimRef)>,
    physics: &PhysicsWorld,
    inventory: &ControllerInventory,
) -> Option<String> {
    if let Ok(label) = sensor_labels.get(entity) {
        return Some(label.0.clone());
    }
    let body = std::iter::once(entity)
        .chain(parents.iter_ancestors(entity))
        .find(|e| physics.entity_to_body.contains_key(e))
        .unwrap_or(entity);
    let path = prims.get(body).ok()?.1.path.to_string();
    let machine = inventory.holding(entity, prims, parents);
    Some(machine.map_or_else(|| path.clone(), |m| format!("{}:{path}", m.id)))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_machine_sensors(
    mut rigs: ResMut<MachineSensorRigs>,
    inventory: Res<ControllerInventory>,
    keys: Res<MachineAgentKeys>,
    bus: Option<ResMut<GearboxBus>>,
    physics: Res<PhysicsWorld>,
    active: Res<PhysicsActive>,
    time: Res<Time>,
    device: Option<Res<RenderDevice>>,
    queue: Option<Res<RenderQueue>>,
    adapter: Option<Res<RenderAdapterInfo>>,
    prims: Query<(Entity, &UsdPrimRef)>,
    (parents, sensor_labels, radar_rcs, air, prim_index): (
        Query<&ChildOf>,
        Query<&SensorLabel>,
        Query<&RadarCrossSection>,
        Option<Res<crate::sensor_generic::AirField>>,
        Res<PrimIndex>,
    ),
    transforms: Query<&GlobalTransform>,
    visuals: VisualColors,
    mut renders: ResMut<crate::sensor_cameras::SensorRenderQueue>,
    daylight: crate::sensor_generic::Daylight,
) {
    renders.live.clear();
    let Some(mut bus) = bus else { return };
    let rigs = &mut *rigs;
    let live: HashSet<&str> = inventory.machines.iter().map(|m| m.id.as_str()).collect();
    rigs.rigs.retain(|id, _| live.contains(id.as_str()));
    rigs.failed.retain(|id, _| live.contains(id.as_str()));
    rigs.failed_links.retain(|(id, _), _| live.contains(id.as_str()));
    if rigs.rigs.is_empty() {
        rigs.scene = None;
        rigs.building = None;
    }
    let model = physics.molla(|scene| scene.model_stamp());
    let bodies = physics.molla(body_order);
    swap_built_scene(rigs, &model);
    let label = |entity: Entity| {
        part_label(
            entity,
            &sensor_labels,
            &parents,
            &prims,
            &physics,
            &inventory,
        )
    };
    let rcs = |entity: Entity| radar_rcs.get(entity).ok().map(|r| r.0);
    let stamp_ms = (time.elapsed_secs_f64() * 1000.0) as u32;
    let sim_time = physics.simulated_seconds;
    let touch = inventory.machines.iter().any(|m| {
        m.links
            .links
            .iter()
            .any(|l| l.sensor.is_some_and(|s| s.kind == SensorKind::Touch))
    });
    let mut carriers = Vec::new();
    let environment = SensorEnvironment {
        light: daylight.light(),
        lamps: daylight.lamps(),
        air: air.as_deref().cloned().unwrap_or_default(),
        contacts: if touch {
            crate::sensor_generic::contact_forces(&**physics)
        } else {
            Vec::new()
        },
    };

    for machine in &inventory.machines {
        if !machine.links.links.iter().any(|l| l.sensor.is_some()) {
            rigs.rigs.remove(&machine.id);
            continue;
        }
        let Some(scene_root) = machine.scene_root else {
            continue;
        };
        let Some(machine_id) = keys
            .0
            .iter()
            .find(|(_, k)| k.scene_root == scene_root && k.machine_id == machine.id)
            .map(|(id, _)| id.clone())
        else {
            continue;
        };
        let Some(agent) = bus.machines.get_mut(&machine_id) else {
            continue;
        };
        if rigs.engine.is_none() {
            let (Some(device), Some(queue)) = (device.as_deref(), queue.as_deref()) else {
                continue;
            };
            let info = adapter.as_deref().map(|info| {
                let raw: &wgpu::AdapterInfo = &info;
                AdapterInfo::from_wgpu(raw.clone())
            });
            rigs.engine = Some(Engine::from_device(
                Arc::new(device.wgpu_device().clone()),
                Arc::new(wgpu::Queue::clone(&queue)),
                info,
            ));
        }
        let engine = rigs.engine.as_ref().expect("sensor engine");

        let revision = physics.molla(|scene| scene.revision());
        let resolution = resolve_mounts(
            &physics,
            &machine.links,
            scene_root,
            &prim_index,
            &transforms,
        );
        for (link_name, error) in &resolution.failed {
            let key = (machine.id.clone(), link_name.clone());
            if rigs.failed_links.insert(key, revision) != Some(revision) {
                error!("gearbox-sensors: `{}`: {error}", machine.id);
            }
        }
        for mount in &resolution.mounts {
            rigs.failed_links
                .remove(&(machine.id.clone(), mount.name.clone()));
        }
        let resolved = resolution.mounts;
        if resolved.is_empty() {
            rigs.rigs.remove(&machine.id);
            continue;
        }
        // A rig over the same bodies samples its scene until a newer one is
        // built off the frame.
        let stale = rigs.rigs.get(&machine.id).is_none_or(|rig| {
            mounts_changed(&rig.signature, &resolved) || rig.model.as_ref() != Some(&model) && rig.bodies != bodies
        });
        if stale {
            if rigs.failed.get(&machine.id) == Some(&revision) {
                continue;
            }
            rigs.rigs.remove(&machine.id);
            let own: Vec<BodyId> = machine
                .links
                .links
                .iter()
                .filter_map(|l| l.body_prim.as_deref())
                .filter_map(|prim| prim_index.find(scene_root, prim))
                .filter_map(|entity| physics.entity_to_body.get(&entity).copied())
                .collect();
            let started = std::time::Instant::now();
            let built = shared_scene(engine, &mut rigs.scene, &**physics, &model, &visuals.memo())
                .and_then(|shared| {
                    Rig::build(
                        engine,
                        &**physics,
                        resolved,
                        revision,
                        sim_time,
                        shared,
                        &label,
                        &rcs,
                        &own,
                    )
                });
            match built {
                Ok(mut rig) => {
                    rig.model = Some(model.clone());
                    rig.machine = machine.id.clone();
                    info!(
                        "gearbox-sensors: `{}` samples {} sensor link(s) on scene revision {revision}, built in {:.0} ms",
                        machine.id,
                        rig.mounts.len(),
                        started.elapsed().as_secs_f64() * 1000.0
                    );
                    rigs.failed.remove(&machine.id);
                    rigs.rigs.insert(machine.id.clone(), rig);
                }
                Err(error) => {
                    error!("gearbox-sensors: `{}`: {error}", machine.id);
                    rigs.failed.insert(machine.id.clone(), revision);
                    continue;
                }
            }
        }
        let rig = rigs.rigs.get_mut(&machine.id).expect("rig just built");
        carriers.push((machine.id.clone(), machine_id.clone()));
        if active.0 {
            let started = std::time::Instant::now();
            let mut wanted = |link: &str| agent.sensor_subscribed(link);
            if let Err(error) = rig.sample(engine, &**physics, sim_time, &environment, &mut wanted) {
                warn!("gearbox-sensors: `{}` sample failed: {error}", machine.id);
            }
            rig.stats.sample_seconds += started.elapsed().as_secs_f64();
        }
        let started = std::time::Instant::now();
        rig.collect(stamp_ms, &mut |link, reading| match reading {
            SensorReading::Imu(imu) => agent.publish_sensor_imu(link, imu),
            SensorReading::Lidar(scan) => agent.publish_sensor_lidar(link, scan),
            SensorReading::Camera(frame) => agent.publish_sensor_camera(link, frame),
            SensorReading::Measurement(reading) => agent.publish_sensor_measurement(link, reading),
        });
        rig.stats.collect_seconds += started.elapsed().as_secs_f64();
        for camera in rig.bevy_cameras() {
            renders.live.insert(crate::sensor_cameras::CameraKey {
                bus_key: machine_id.clone(),
                link: camera.name.clone(),
            });
        }
        for request in rig.take_render_requests() {
            let mount = rig.mount(request.mount);
            renders.jobs.push(crate::sensor_cameras::RenderJob {
                key: crate::sensor_cameras::CameraKey {
                    bus_key: machine_id.clone(),
                    link: mount.name.clone(),
                },
                link_index: mount.link as u32,
                spec: mount.spec,
                entity: mount.entity,
                sample: request.sample,
                sim_time: request.sim_time,
                stamp_ms,
            });
        }
        let now = time.elapsed_secs_f64();
        if now - rig.last_report >= 10.0 {
            let s = rig.stats;
            if s.samples > 0 {
                info!(
                    "gearbox-sensors: `{}`: {} samples ({} gpu batches, {} dispatches), {:.3} ms sample + {:.3} ms collect per sample, {:.1} KB up, {:.1} KB down per sample, {} frames, {} dropped in the last {:.0} s",
                    machine.id,
                    s.samples,
                    s.gpu_batches,
                    s.gpu_dispatches,
                    1000.0 * s.sample_seconds / s.samples as f64,
                    1000.0 * s.collect_seconds / s.samples as f64,
                    s.upload_bytes as f64 / 1024.0 / s.samples as f64,
                    s.readback_bytes as f64 / 1024.0 / s.samples as f64,
                    s.frames,
                    s.dropped,
                    now - rig.last_report
                );
            }
            rig.stats = RigStats::default();
            rig.last_report = now;
        }
    }
    if rigs.building.is_none()
        && let Some(engine) = rigs.engine.as_ref()
    {
        let behind: Vec<(String, Option<&HashSet<u64>>, &HashSet<u64>)> = rigs
            .rigs
            .iter()
            .filter(|(_, rig)| rig.model.as_ref() != Some(&model) && rig.bodies == bodies)
            .map(|(id, rig)| (id.clone(), rig.recognition.then_some(&rig.own), &rig.own))
            .collect();
        if !behind.is_empty() {
            let started = physics.molla(|rigid| {
                let objects = behind
                    .iter()
                    .map(|(id, rec_own, _)| {
                        (
                            id.clone(),
                            Arc::new(rec_own.map_or_else(SceneObjects::default, |own| {
                                shape_objects(rigid, own, &label)
                            })),
                        )
                    })
                    .collect();
                let targets = behind
                    .iter()
                    .map(|(id, _, own)| (id.clone(), radar_targets(rigid, own, &rcs)))
                    .collect();
                let shape_bodies = shape_bodies(rigid);
                build_scene_off_frame(engine, rigid, shape_colors(rigid, &visuals.memo()))
                    .map(|task| SceneBuild {
                        model: model.clone(),
                        bodies: bodies.clone(),
                        shape_bodies,
                        objects,
                        radar_targets: targets,
                        task,
                    })
            });
            match started {
                Ok(build) => rigs.building = Some(build),
                Err(error) => warn!("gearbox-sensors: {error}"),
            }
        }
    }
    exchange_packets(rigs, &mut bus, &carriers, &**physics, sim_time, stamp_ms);
}

/// Carries this frame's emitter requests of every machine to the receiver
/// links that hear them; requests naming no emitter link are dropped.
fn exchange_packets(
    rigs: &mut MachineSensorRigs,
    bus: &mut GearboxBus,
    carriers: &[(String, String)],
    physics: &dyn PhysicsBackend,
    sim_time: f64,
    stamp_ms: u32,
) {
    let (mut emitters, mut receivers) = (Vec::new(), Vec::new());
    for (id, key) in carriers {
        let Some(rig) = rigs.rigs.get(id) else { continue };
        for (mount, resolved) in rig.radio_mounts() {
            let Some(world) = crate::sensor_radio::world_pose(physics, resolved.body, &resolved.local) else {
                continue;
            };
            let link = RadioLink {
                machine: key.clone(),
                mount,
                name: resolved.name.clone(),
                link_index: resolved.link as u32,
                body: resolved.body,
                world,
                endpoint: crate::sensor_radio::endpoint(&resolved.spec),
            };
            if resolved.spec.kind == SensorKind::Emitter {
                emitters.push(link);
            } else {
                receivers.push(link);
            }
        }
    }
    let mut sent = Vec::new();
    for (key, agent) in &mut bus.machines {
        for request in agent.emits.drain(..) {
            let name = request.link();
            match emitters.iter().position(|e| &e.machine == key && e.name == name) {
                Some(emitter) => sent.push((emitter, request.data)),
                None => debug!("gearbox-sensors: `{key}` has no emitter link `{name}`; packet dropped"),
            }
        }
    }
    if sent.is_empty() || receivers.is_empty() {
        return;
    }
    let rig_of: HashMap<&str, &str> = carriers.iter().map(|(id, key)| (key.as_str(), id.as_str())).collect();
    for (receiver, mut reading) in crate::sensor_radio::exchange(physics, &emitters, &receivers, sent, sim_time) {
        let link = &receivers[receiver];
        let Some(rig) = rig_of.get(link.machine.as_str()).and_then(|id| rigs.rigs.get_mut(*id)) else {
            continue;
        };
        reading.stamp_ms = stamp_ms;
        reading.sample = rig.packets(link.mount) as u32;
        let delivered = bus
            .machines
            .get_mut(&link.machine)
            .is_some_and(|agent| agent.publish_sensor_measurement(&link.name, &reading));
        rig.record(link.mount, delivered);
    }
}

/// Rotation from Molla's camera axes (+X right, +Y up, -Z forward) into a
/// REP-103 link frame (+X forward, +Y left, +Z up).
fn camera_in_link() -> MQuat {
    MQuat::from_mat3(&MMat3::from_cols(
        MVec3::new(0.0, -1.0, 0.0),
        MVec3::new(0.0, 0.0, 1.0),
        MVec3::new(-1.0, 0.0, 0.0),
    ))
    .normalize()
}

/// Per-machine mount resolution result: healthy mounts proceed even when a
/// sibling sensor link is misconfigured.
pub struct MountResolution {
    pub mounts: Vec<ResolvedMount>,
    /// Structural errors per skipped link: `(link_name, reason)`.
    pub failed: Vec<(String, String)>,
}

/// Every authored sensor link of a machine with its body and mount pose.
/// Links that fail validation are returned in `failed` rather than aborting
/// the machine's healthy links; links whose entities/bodies are still spawning
/// are skipped until the next frame.
fn resolve_mounts(
    physics: &PhysicsWorld,
    links: &LinkTree,
    scene_root: Entity,
    prim_index: &PrimIndex,
    transforms: &Query<&GlobalTransform>,
) -> MountResolution {
    let mut mounts = Vec::new();
    let mut failed = Vec::new();
    for (index, link) in links.links.iter().enumerate() {
        let Some(spec) = link.sensor else {
            continue;
        };
        let Some(body_prim) = rigid_body_prim(links, link) else {
            failed.push((
                link.name.clone(),
                format!("sensor link `{}` has no rigid-body ancestor", link.name),
            ));
            continue;
        };
        let Some(body_entity) = prim_index.find(scene_root, body_prim) else {
            continue;
        };
        let Some(sensor_entity) = prim_index.find(scene_root, &link.prim_path) else {
            continue;
        };
        let Some(&body) = physics.entity_to_body.get(&body_entity) else {
            continue;
        };
        let (Ok(body_gt), Ok(sensor_gt)) =
            (transforms.get(body_entity), transforms.get(sensor_entity))
        else {
            continue;
        };
        let local = body_gt.affine().inverse() * sensor_gt.affine();
        let (_, rotation, translation) = local.to_scale_rotation_translation();
        if !rotation.is_finite() || !translation.is_finite() {
            failed.push((
                link.name.clone(),
                format!("sensor link `{}` has a non-finite mount", link.name),
            ));
            continue;
        }
        let joint = (spec.kind == SensorKind::Position)
            .then(|| {
                let body_link = links.links.iter().find(|l| l.body_prim.as_deref() == Some(body_prim))?;
                let parent_link = links.get(body_link.parent.as_deref()?)?;
                let parent_prim = rigid_body_prim(links, parent_link)?;
                let parent_entity = prim_index.find(scene_root, parent_prim)?;
                let parent_body = *physics.entity_to_body.get(&parent_entity)?;
                physics.joint_between(parent_body, body)
            })
            .flatten();
        if spec.kind == SensorKind::Position && joint.is_none() {
            failed.push((
                link.name.clone(),
                format!("position sensor `{}` has no joint above it", link.name),
            ));
            continue;
        }
        mounts.push(ResolvedMount {
            link: index,
            name: link.name.clone(),
            spec,
            body,
            entity: Some(sensor_entity),
            joint,
            local: MTransform::new(
                MVec3::new(
                    translation.x as f64,
                    translation.y as f64,
                    translation.z as f64,
                ),
                MQuat::from_xyzw(
                    rotation.x as f64,
                    rotation.y as f64,
                    rotation.z as f64,
                    rotation.w as f64,
                )
                .normalize(),
            ),
        });
    }
    MountResolution { mounts, failed }
}

/// Whether a mount set differs beyond the f32 jitter of resolved poses.
fn mounts_changed(built: &[ResolvedMount], resolved: &[ResolvedMount]) -> bool {
    built.len() != resolved.len()
        || built.iter().zip(resolved).any(|(a, b)| {
            a.link != b.link
                || a.name != b.name
                || a.spec != b.spec
                || a.body != b.body
                || a.entity != b.entity
                || (a.local.position - b.local.position).length() > 1e-3
                || a.local.rotation.dot(b.local.rotation).abs() < 1.0 - 1e-6
        })
}

/// The nearest link at or above `link` that owns a rigid body.
fn rigid_body_prim<'a>(links: &'a LinkTree, link: &'a LinkSpec) -> Option<&'a str> {
    let mut current = link;
    for _ in 0..64 {
        if let Some(body) = &current.body_prim {
            return Some(body);
        }
        current = links.get(current.parent.as_deref()?)?;
    }
    None
}

fn camera_config(spec: &SensorSpec, traced: &[CameraChannels]) -> TiledCameraConfig {
    TiledCameraConfig {
        width: spec.width,
        height: spec.height,
        fov_y_radians: spec.vfov,
        channels: CameraChannels(traced.iter().fold(0, |all, c| all | c.0)),
        max_distance: spec.range_m,
        optics: PhysicalOptics {
            lens_diameter_m: spec.lens_diameter_m,
            focal_distance_m: spec.focal_distance_m,
            aperture_samples: spec.aperture_samples.max(1),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn shape_labels_for(objects: &SceneObjects) -> Vec<ShapeLabel> {
    objects
        .shapes
        .iter()
        .enumerate()
        .filter_map(|(shape_id, obj)| {
            obj.map(|class_id| ShapeLabel {
                shape_id: shape_id as u32,
                class_id,
            })
        })
        .collect()
}

fn build_annotation_gpu(
    engine: &Engine,
    mounts: &[Mount],
    objects: &Arc<SceneObjects>,
) -> Option<AnnotationGpu> {
    let rec_mounts: Vec<&SensorSpec> = mounts
        .iter()
        .map(|m| &m.resolved.spec)
        .filter(|s| s.kind == SensorKind::Camera && s.recognition)
        .collect();
    if rec_mounts.is_empty() {
        return None;
    }
    let max_pixels: u64 = rec_mounts
        .iter()
        .map(|s| u64::from(s.width) * u64::from(s.height))
        .sum();
    let max_images = rec_mounts.len() as u32;
    let labels = shape_labels_for(objects);
    let max_labels = (labels.len() as u32).max(16).next_power_of_two();
    if max_pixels == 0 || max_pixels > u32::MAX as u64 || max_labels > MAX_ANNOTATION_LABELS {
        return None;
    }
    let config = AnnotationConfig {
        max_pixels: max_pixels as u32,
        max_images,
        max_labels,
        max_buffer_bytes: MAX_ANNOTATION_BUFFER_BYTES,
    };
    let mut processor = match AnnotationProcessor::new(engine, config) {
        Ok(p) => p,
        Err(error) => {
            warn!("gearbox-sensors: GPU annotation unavailable, falling back to CPU: {error}");
            return None;
        }
    };
    if let Err(error) = processor.set_labels(&labels) {
        warn!("gearbox-sensors: GPU annotation labels failed: {error}");
        return None;
    }
    let slot_bytes = max_pixels * 8
        + u64::from(max_images) * u64::from(max_labels) * 32
        + u64::from(max_images) * 16;
    let pool = match processor.output_pool(
        engine,
        SensorFramePoolConfig {
            slots: POOL_SLOTS,
            max_total_bytes: (slot_bytes.max(64)) * POOL_SLOTS as u64 * 2,
        },
    ) {
        Ok(pool) => pool,
        Err(error) => {
            warn!("gearbox-sensors: GPU annotation pool failed: {error}");
            return None;
        }
    };
    Some(AnnotationGpu {
        processor,
        pool,
        max_labels,
    })
}

fn build_generic_ray_gpu(engine: &Engine, capacity: u32) -> Result<GenericRayGpu, String> {
    let capacity = capacity.max(MIN_GENERIC_RAY_CAPACITY);
    let raycaster =
        Raycaster::new(engine, capacity).map_err(|e| format!("generic raycaster: {e}"))?;
    let slot_bytes = u64::from(capacity) * 96;
    let pool = raycaster
        .output_pool(
            engine,
            SensorFramePoolConfig {
                slots: POOL_SLOTS,
                max_total_bytes: slot_bytes * POOL_SLOTS as u64 * 2,
            },
        )
        .map_err(|e| format!("generic raycaster pool: {e}"))?;
    Ok(GenericRayGpu {
        capacity,
        raycaster,
        pool,
    })
}

fn build_camera_batch(
    engine: &Engine,
    mounts: &[Mount],
    active: &[usize],
) -> Result<(CameraBatch, CameraBatchOutputPool), String> {
    let specs: Vec<CameraSpec> = active
        .iter()
        .map(|&idx| {
            let spec = mounts[idx].resolved.spec;
            let traced = camera_channels(&spec);
            CameraSpec {
                sensor_id: idx as u64,
                config: camera_config(&spec, &traced),
                pose: MTransform::IDENTITY,
            }
        })
        .collect();
    let total_pixels: u64 = specs
        .iter()
        .map(|s| u64::from(s.config.width) * u64::from(s.config.height))
        .sum();
    let limits = CameraBatchLimits {
        max_cameras: specs.len().max(64),
        max_groups: specs.len().clamp(16, 64),
        max_total_pixels: total_pixels.max(4_000_000),
    };
    let batch =
        CameraBatch::new(engine, &specs, limits).map_err(|e| format!("camera batch: {e}"))?;
    let max_total_bytes = total_pixels
        .saturating_mul(32)
        .saturating_mul(POOL_SLOTS as u64)
        .saturating_mul(2)
        .max(1024);
    let pool = batch
        .output_pool(
            engine,
            SensorFramePoolConfig {
                slots: POOL_SLOTS,
                max_total_bytes,
            },
        )
        .map_err(|e| format!("camera batch pool: {e}"))?;
    Ok((batch, pool))
}

/// Group cropped rows `start_row..start_row+rows` by object id using the same
/// geometry as [`molla_sensors::recognize`].
#[allow(clippy::too_many_arguments)]
fn recognize_rows(
    width: u32,
    height: u32,
    fov_y: f64,
    start_row: u32,
    rows: u32,
    depth: &[f64],
    shape: &[u32],
    object_of: &dyn Fn(u32) -> Option<u64>,
    min_pixels: u32,
) -> Vec<RecognizedObject> {
    struct Acc {
        pixels: u32,
        bbox: [u32; 4],
        sum: MVec3,
        lo: MVec3,
        hi: MVec3,
    }
    let mut objects: std::collections::BTreeMap<u64, Acc> = Default::default();
    let total = (width as usize * rows as usize)
        .min(depth.len())
        .min(shape.len());
    for index in 0..total {
        let (s, d) = (shape[index], depth[index]);
        if s == molla_sensors::recognition::NO_SHAPE || !d.is_finite() {
            continue;
        }
        let Some(id) = object_of(s) else { continue };
        let px = index as u32 % width;
        let py = start_row + index as u32 / width;
        let point = molla_sensors::recognition::pixel_ray(width, height, fov_y, px, py) * d;
        let acc = objects.entry(id).or_insert(Acc {
            pixels: 0,
            bbox: [px, py, px, py],
            sum: MVec3::ZERO,
            lo: point,
            hi: point,
        });
        acc.pixels += 1;
        acc.bbox = [
            acc.bbox[0].min(px),
            acc.bbox[1].min(py),
            acc.bbox[2].max(px),
            acc.bbox[3].max(py),
        ];
        acc.sum += point;
        acc.lo = acc.lo.min(point);
        acc.hi = acc.hi.max(point);
    }
    objects
        .into_iter()
        .filter(|(_, acc)| acc.pixels >= min_pixels.max(1))
        .map(|(id, acc)| RecognizedObject {
            id,
            pixels: acc.pixels,
            bbox: acc.bbox,
            position: acc.sum / acc.pixels as f64,
            size: acc.hi - acc.lo,
        })
        .collect()
}

fn bounding_row_span(boxes: &[DeviceBoundingBox], height: u32) -> Option<(u32, u32)> {
    let mut min_y = u32::MAX;
    let mut max_y = 0u32;
    let mut any = false;
    for b in boxes {
        if b.pixel_count == 0 {
            continue;
        }
        min_y = min_y.min(b.bounds[1]);
        max_y = max_y.max(b.bounds[3]);
        any = true;
    }
    if !any || height == 0 {
        return None;
    }
    let min_y = min_y.min(height - 1);
    let max_y = max_y.min(height - 1);
    Some((min_y, max_y))
}

impl Rig {
    /// Snapshots the Molla scene and creates the GPU sensors of `mounts`;
    /// `labels` gives a collider entity's object name for recognition, `rcs`
    /// gives authored radar cross-sections, and `own_bodies` the machine's
    /// bodies, which its radars and recognition ignore.
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        engine: &Engine,
        physics: &dyn PhysicsBackend,
        mounts: Vec<ResolvedMount>,
        revision: u64,
        sim_time: f64,
        shared: &SharedScene,
        labels: &dyn Fn(Entity) -> Option<String>,
        rcs: &dyn Fn(Entity) -> Option<f64>,
        own_bodies: &[BodyId],
    ) -> Result<Self, String> {
        let scene = shared.scene.clone();
        let shape_bodies = shared.shape_bodies.clone();
        let device = engine.device().ok_or("sensor engine has no device")?;
        let queue = engine.queue().ok_or("sensor engine has no queue")?;
        let mut valid_mounts = Vec::with_capacity(mounts.len());
        let mut handles = Vec::with_capacity(mounts.len());
        for m in mounts {
            match physics.molla_body_handle(m.body) {
                Some(handle) => {
                    handles.push(handle);
                    valid_mounts.push(m);
                }
                None => warn!(
                    "gearbox-sensors: skipping sensor link `{}`: unknown physics body",
                    m.name
                ),
            }
        }
        if valid_mounts.is_empty() {
            return Err("no sensor link has a valid physics body".into());
        }
        let (imu_model, mut state, bodies) =
            physics.molla(|scene| snapshot_bodies(engine, scene, &handles))?;
        let own: HashSet<u64> = own_bodies
            .iter()
            .filter_map(|&b| physics.molla_body_handle(b))
            .map(|h| h.to_bits())
            .collect();
        let (mut joints, mut targets) = (HashMap::new(), Vec::new());
        let recognition = valid_mounts
            .iter()
            .any(|m| m.spec.kind == SensorKind::Camera && m.spec.recognition);
        let mut objects = SceneObjects::default();
        physics.with_molla_scene(&mut |rigid| {
            if recognition {
                objects = shape_objects(rigid, &own, labels);
            }
            for (i, mount) in valid_mounts.iter().enumerate() {
                let Some(joint) = mount.joint else { continue };
                if let Some(index) = rigid
                    .joints()
                    .find(|(h, _)| h.to_bits() == joint.0)
                    .and_then(|(h, _)| rigid.joint_index(h))
                {
                    joints.insert(i, index.index());
                }
            }
            targets = radar_targets(rigid, &own, rcs);
        });
        let specs: Vec<SensorSpec> = valid_mounts.iter().map(|m| m.spec).collect();
        let generic = GenericSensors::build(&specs, joints, targets)?;
        state
            .upload_to_device(device, queue)
            .map_err(|e| format!("sensor state upload: {e}"))?;

        let signature = valid_mounts.clone();
        let mounts: Vec<Mount> = valid_mounts
            .into_iter()
            .zip(bodies)
            .map(|(resolved, body)| Mount {
                name: resolved.name.clone(),
                resolved,
                body,
                published: 0,
                undelivered: 0,
                capture_refused: false,
            })
            .collect();
        let schedules: Vec<SensorSchedule> = mounts
            .iter()
            .enumerate()
            .map(|(i, m)| SensorSchedule {
                sensor_id: i as u64,
                period: Duration::from_secs_f64(1.0 / m.resolved.spec.rate_hz as f64),
                phase: Duration::ZERO,
            })
            .collect();
        let scheduler =
            SensorScheduler::new(Duration::from_secs_f64(sim_time), revision, &schedules)
                .map_err(|e| format!("sensor schedule: {e}"))?;

        let imu_mounts: Vec<usize> = mounts
            .iter()
            .enumerate()
            .filter(|(_, m)| m.resolved.spec.kind.uses_imu())
            .map(|(i, _)| i)
            .collect();
        let imu = if imu_mounts.is_empty() {
            None
        } else {
            let descs: Vec<ImuMount> = imu_mounts
                .iter()
                .map(|&i| ImuMount {
                    body: mounts[i].body,
                    local_pose: mounts[i].resolved.local,
                })
                .collect();
            let sensor = ImuSensor::new(engine, &imu_model, &descs)
                .map_err(|e| format!("imu: {e}"))?;
            let max_total_bytes = (imu_mounts.len() as u64 * 32 * POOL_SLOTS as u64 * 2).max(256);
            let pool = sensor
                .output_pool(
                    engine,
                    SensorFramePoolConfig {
                        slots: POOL_SLOTS,
                        max_total_bytes,
                    },
                )
                .map_err(|e| format!("imu pool: {e}"))?;
            Some((sensor, pool))
        };
        let lidars: Vec<&Mount> = mounts
            .iter()
            .filter(|m| m.resolved.spec.kind == SensorKind::Lidar)
            .collect();
        let lidar_rays: u32 = lidars
            .iter()
            .map(|m| m.resolved.spec.rows * m.resolved.spec.columns)
            .sum();
        let lidar = if lidars.is_empty() {
            None
        } else {
            let batch = LidarBatch::new(engine, lidars.len(), lidar_rays)
                .map_err(|e| format!("lidar: {e}"))?;
            let max_total_bytes = (u64::from(lidar_rays.max(1)) * 128 * POOL_SLOTS as u64 * 2).max(1024);
            let pool = batch
                .output_pool(
                    engine,
                    SensorFramePoolConfig {
                        slots: POOL_SLOTS,
                        max_total_bytes,
                    },
                )
                .map_err(|e| format!("lidar pool: {e}"))?;
            Some((batch, pool))
        };
        let mut camera_buckets: Vec<CameraBucket> = Vec::new();
        let mut camera_bytes = 0u64;
        let mut camera_count = 0usize;
        for (i, mount) in mounts.iter().enumerate() {
            let spec = mount.resolved.spec;
            let traced = camera_channels(&spec);
            if spec.kind != SensorKind::Camera || traced.is_empty() {
                continue;
            }
            camera_count += 1;
            let texel: u64 = traced
                .iter()
                .map(|&c| if c == CameraChannels::NORMAL { 16 } else { 4 })
                .sum();
            camera_bytes = camera_bytes.max(u64::from(spec.width) * u64::from(spec.height) * texel);
            let rate_bits = spec.rate_hz.to_bits();
            if let Some(bucket) = camera_buckets.iter_mut().find(|b| b.rate_bits == rate_bits) {
                bucket.mounts.push(i);
            } else {
                camera_buckets.push(CameraBucket {
                    rate_bits,
                    mounts: vec![i],
                    active: Vec::new(),
                    batch: None,
                });
            }
        }
        let objects = Arc::new(objects);
        let annotation = build_annotation_gpu(engine, &mounts, &objects);
        let annotation_synced = recognition.then(|| objects.clone());
        let annotation_bytes = annotation.as_ref().map_or(0u64, |a| {
            u64::from(a.max_labels) * 32 + 16
        });

        let needs_generic_rays = mounts.iter().any(|m| {
            matches!(
                m.resolved.spec.kind,
                SensorKind::Distance | SensorKind::Light | SensorKind::Radar
            )
        });
        let generic_ray_est: u32 = mounts
            .iter()
            .map(|m| match m.resolved.spec.kind {
                SensorKind::Distance => m.resolved.spec.rays.max(1),
                SensorKind::Light => 8,
                SensorKind::Radar => generic.radar_target_count().max(16) as u32,
                _ => 0,
            })
            .sum();
        let generic_rays = if needs_generic_rays {
            Some(build_generic_ray_gpu(engine, generic_ray_est)?)
        } else {
            None
        };
        let generic_ray_bytes = generic_rays
            .as_ref()
            .map_or(0u64, |g| u64::from(g.capacity) * 12);

        let ring_bytes_per_slot = (imu_mounts.len() as u64 * 32)
            .max(lidar_rays as u64 * 24)
            .max(camera_bytes + annotation_bytes)
            .max(generic_ray_bytes)
            .max(64);
        let ring_slots = (2 * (1 + lidars.len() + camera_count * 2 + 2) + 4).clamp(4, 64);
        let ring = ReadbackRing::new(
            engine,
            ReadbackConfig {
                slots: ring_slots,
                bytes_per_slot: ring_bytes_per_slot,
                max_total_bytes: ring_bytes_per_slot * ring_slots as u64,
            },
        )
        .map_err(|e| format!("sensor readback: {e}"))?;
        Ok(Self {
            revision,
            machine: String::new(),
            model: None,
            signature,
            scene,
            shape_bodies,
            imu_model,
            state,
            scheduler,
            mounts,
            imu,
            imu_mounts,
            last_imu_time: None,
            lidar,
            camera_buckets,
            max_camera_bytes: camera_bytes,
            annotation,
            annotation_synced,
            generic_rays,
            ring,
            ring_bytes_per_slot,
            ring_slots,
            pending: Vec::new(),
            render_requests: Vec::new(),
            generic,
            ready: Vec::new(),
            objects,
            bodies: shared.bodies.clone(),
            own,
            recognition,
            stats: RigStats::default(),
            last_report: 0.0,
        })
    }

    fn ensure_ring_slot_bytes(&mut self, engine: &Engine, needed: u64) -> Result<(), String> {
        if needed <= self.ring_bytes_per_slot {
            return Ok(());
        }
        let next = needed.next_power_of_two().max(64);
        let ring = ReadbackRing::new(
            engine,
            ReadbackConfig {
                slots: self.ring_slots,
                bytes_per_slot: next,
                max_total_bytes: next * self.ring_slots as u64,
            },
        )
        .map_err(|e| format!("sensor readback resize: {e}"))?;
        self.pending.clear();
        self.ring = ring;
        self.ring_bytes_per_slot = next;
        Ok(())
    }

    /// Bevy colour renders requested by the samples issued since the last call.
    pub fn take_render_requests(&mut self) -> Vec<RenderRequest> {
        std::mem::take(&mut self.render_requests)
    }

    pub fn mount(&self, index: usize) -> &ResolvedMount {
        &self.mounts[index].resolved
    }

    /// Receiver and emitter links by mount index.
    pub fn radio_mounts(&self) -> impl Iterator<Item = (usize, &ResolvedMount)> {
        self.mounts
            .iter()
            .enumerate()
            .map(|(i, m)| (i, &m.resolved))
            .filter(|(_, r)| matches!(r.spec.kind, SensorKind::Receiver | SensorKind::Emitter))
    }

    /// Packets a receiver link has been handed so far.
    pub fn packets(&self, mount: usize) -> u64 {
        let m = &self.mounts[mount];
        m.published + m.undelivered
    }

    /// Counts a receiver packet as published or dropped.
    pub fn record(&mut self, mount: usize, delivered: bool) {
        self.mounts[mount].record(delivered, 1);
    }

    /// Camera links whose colour Bevy renders.
    pub fn bevy_cameras(&self) -> impl Iterator<Item = &ResolvedMount> {
        self.mounts
            .iter()
            .map(|m| &m.resolved)
            .filter(|r| r.spec.kind == SensorKind::Camera && r.spec.color && r.spec.render.is_bevy())
    }

    /// Issues every sample due at `sim_time` in one [`SensorCommandBatch`]
    /// without waiting on the GPU. A link samples only while `wanted` says
    /// someone asks for its readings; radio links always do.
    pub fn sample(
        &mut self,
        engine: &Engine,
        physics: &dyn PhysicsBackend,
        sim_time: f64,
        environment: &SensorEnvironment,
        wanted: &mut dyn FnMut(&str) -> bool,
    ) -> Result<(), String> {
        if let Some(device) = engine.device() {
            let _ = device.poll(wgpu::PollType::Poll);
        }
        let mut due: Vec<ScheduledSample> = self
            .scheduler
            .advance(Duration::from_secs_f64(sim_time))
            .map_err(|e| e.to_string())?
            .to_vec();
        let mounts = &self.mounts;
        due.retain(|s| {
            let mount = &mounts[s.sensor_id as usize].resolved;
            matches!(mount.spec.kind, SensorKind::Receiver | SensorKind::Emitter) || wanted(&mount.name)
        });
        if due.is_empty() {
            return Ok(());
        }
        let state = &mut self.state;
        physics.molla(|scene| copy_state(scene.state(), state))?;
        let queue = engine.queue().ok_or("sensor engine has no queue")?;
        self.state.body_q.upload(queue).map_err(|e| e.to_string())?;
        self.state
            .body_qd
            .upload(queue)
            .map_err(|e| e.to_string())?;
        let body_q = self
            .state
            .body_q
            .host()
            .map_err(|e| e.to_string())?
            .to_vec();
        let world_pose = |mount: &Mount| body_q[mount.body.index()].compose(&mount.resolved.local);

        // Sync GPU annotation labels once per distinct scene objects snapshot.
        if self.recognition
            && self
                .annotation_synced
                .as_ref()
                .is_none_or(|s| !Arc::ptr_eq(s, &self.objects))
        {
            self.annotation_synced = Some(self.objects.clone());
            let labels = shape_labels_for(&self.objects);
            let fits = self
                .annotation
                .as_ref()
                .is_some_and(|a| labels.len() <= a.max_labels as usize);
            if fits {
                if let Some(ann) = self.annotation.as_mut()
                    && ann.processor.set_labels(&labels).is_err()
                {
                    self.annotation = None;
                }
            } else {
                self.annotation = build_annotation_gpu(engine, &self.mounts, &self.objects);
                if let Some(ann) = self.annotation.as_ref() {
                    let needed = self.max_camera_bytes + u64::from(ann.max_labels) * 32 + 16;
                    self.ensure_ring_slot_bytes(engine, needed)?;
                }
            }
        }

        // Non-ray generic sensors run immediately; distance/light/radar rays
        // plan into one async GPU raycast encoded in this tick's command batch.
        let generic_due: Vec<GenericSample<'_>> = due
            .iter()
            .filter(|s| is_generic(self.mounts[s.sensor_id as usize].resolved.spec.kind))
            .map(|s| {
                let m = &self.mounts[s.sensor_id as usize];
                GenericSample {
                    mount: s.sensor_id as usize,
                    name: &m.name,
                    link_index: m.resolved.link as u32,
                    spec: m.resolved.spec,
                    body: m.body,
                    world: world_pose(m),
                    sample: s.sample_id,
                }
            })
            .collect();
        let ray_plan = if generic_due.is_empty() {
            PendingRayPlan::default()
        } else {
            let mut read_joint = |joint: usize| {
                physics.molla(|rigid| {
                    molla_sensors::joint_reading(rigid.model(), rigid.state(), joint)
                        .map_err(|e| format!("position joint {joint}: {e}"))
                })
            };
            let (ready, plan) = self.generic.sample_and_plan_rays(
                &self.imu_model,
                &self.state,
                &mut read_joint,
                environment,
                &generic_due,
                sim_time,
            )?;
            self.ready.extend(ready);
            plan
        };
        if !ray_plan.is_empty() {
            let needed_rays = ray_plan.origins.len() as u32;
            let grow = self
                .generic_rays
                .as_ref()
                .is_none_or(|g| g.capacity < needed_rays);
            if grow {
                let cap = needed_rays.max(MIN_GENERIC_RAY_CAPACITY).next_power_of_two();
                self.generic_rays = Some(build_generic_ray_gpu(engine, cap)?);
                self.ensure_ring_slot_bytes(engine, u64::from(cap) * 12)?;
            }
        }

        // Prepare due camera mounts, Bevy render requests, and CameraBatch buckets.
        let camera_due: Vec<(usize, u64)> = due
            .iter()
            .map(|s| (s.sensor_id as usize, s.sample_id))
            .filter(|(i, _)| self.mounts[*i].resolved.spec.kind == SensorKind::Camera)
            .collect();
        for &(mount, sample) in &camera_due {
            let spec = self.mounts[mount].resolved.spec;
            if spec.color && spec.render.is_bevy() {
                self.render_requests.push(RenderRequest {
                    mount,
                    sample,
                    sim_time,
                });
            }
        }
        let mut due_buckets: Vec<(usize, Vec<(usize, u64)>)> = Vec::new();
        for (bucket_idx, bucket) in self.camera_buckets.iter_mut().enumerate() {
            let mut entries: Vec<(usize, u64)> = camera_due
                .iter()
                .copied()
                .filter(|(m, _)| bucket.mounts.contains(m))
                .collect();
            if entries.is_empty() {
                continue;
            }
            entries.sort_unstable_by_key(|(m, _)| *m);
            let want: Vec<usize> = entries.iter().map(|(m, _)| *m).collect();
            if bucket.active != want || bucket.batch.is_none() {
                bucket.batch = Some(build_camera_batch(engine, &self.mounts, &want)?);
                bucket.active = want;
            }
            if let Some((batch, _)) = bucket.batch.as_mut() {
                for &(mount, _) in &entries {
                    let world = world_pose(&self.mounts[mount]);
                    let pose = MTransform::new(
                        world.position,
                        (world.rotation * camera_in_link()).normalize(),
                    );
                    batch
                        .set_camera_pose(mount as u64, pose)
                        .map_err(|e| format!("camera pose: {e}"))?;
                }
            }
            due_buckets.push((bucket_idx, entries));
        }

        // Prepare due IMU and LiDAR inputs before opening the shared command batch.
        let imu_due: Vec<(usize, MQuat)> = due
            .iter()
            .map(|s| s.sensor_id as usize)
            .filter(|&i| self.mounts[i].resolved.spec.kind.uses_imu())
            .map(|i| (i, world_pose(&self.mounts[i]).rotation))
            .collect();
        let imu_dt = if self.imu.is_some() && !imu_due.is_empty() {
            let fastest = self
                .imu_mounts
                .iter()
                .map(|&i| self.mounts[i].resolved.spec.rate_hz)
                .fold(f32::MIN, f32::max);
            let slowest_due = imu_due
                .iter()
                .map(|&(i, _)| self.mounts[i].resolved.spec.rate_hz)
                .fold(f32::MAX, f32::min);
            // An IMU that skipped samples re-primes instead of differencing across the gap.
            if self.last_imu_time.is_some_and(|t| sim_time - t > 2.0 / slowest_due as f64) {
                if let Some((imu, _)) = self.imu.as_mut() {
                    imu.reset();
                }
                self.last_imu_time = None;
            }
            Some(
                self.last_imu_time
                    .map(|t| sim_time - t)
                    .filter(|dt| *dt > 0.0)
                    .unwrap_or(1.0 / fastest as f64),
            )
        } else {
            None
        };

        let lidar_due: Vec<(usize, u64)> = due
            .iter()
            .map(|s| (s.sensor_id as usize, s.sample_id))
            .filter(|(i, _)| self.mounts[*i].resolved.spec.kind == SensorKind::Lidar)
            .collect();
        let (lidar_scans, lidar_worlds) = if self.lidar.is_some() && !lidar_due.is_empty() {
            let mut scans = Vec::with_capacity(lidar_due.len());
            let mut worlds = Vec::with_capacity(lidar_due.len());
            for &(i, _) in &lidar_due {
                let world = world_pose(&self.mounts[i]);
                scans.push(MollaLidarScan {
                    sensor_id: i as u64,
                    pattern: LidarPattern::Terrestrial(scan_metadata(
                        &self.mounts[i].resolved.spec,
                        &world,
                    )),
                });
                worlds.push(world);
            }
            (scans, worlds)
        } else {
            (Vec::new(), Vec::new())
        };

        // Encode every due GPU producer (IMU, LiDAR, batched cameras, GPU
        // annotations, generic GPU rays) into ONE command batch submission.
        let (
            submission,
            encoded_imu,
            encoded_lidar,
            encoded_cameras,
            encoded_ann,
            ann_entries,
            encoded_rays,
            imu_backpressured,
            dropped_count,
        ) = {
            let mut cmd =
                SensorCommandBatch::new(engine).map_err(|e| format!("sensor batch: {e}"))?;
            let mut dropped_count = 0u64;
            let mut imu_backpressured = false;

            let encoded_imu = if let (Some((imu, pool)), Some(dt)) = (self.imu.as_mut(), imu_dt) {
                let stamp = ReadbackStamp {
                    sensor_id: u64::MAX,
                    sample_id: due[0].sample_id,
                    scene_revision: self.revision,
                    simulation_time: Some(sim_time),
                };
                match imu
                    .encode(&mut cmd, engine, &self.imu_model, &self.state, dt, pool, stamp)
                    .map_err(|e| format!("imu encode: {e}"))?
                {
                    Some(sample) => Some(sample),
                    None => {
                        imu_backpressured = true;
                        dropped_count += 1;
                        None
                    }
                }
            } else {
                None
            };

            let encoded_lidar = if let (Some((lidar, pool)), false) =
                (self.lidar.as_mut(), lidar_scans.is_empty())
            {
                let stamp = ReadbackStamp {
                    sensor_id: lidar_due[0].0 as u64,
                    sample_id: lidar_due[0].1,
                    scene_revision: self.revision,
                    simulation_time: Some(sim_time),
                };
                match lidar
                    .encode_scene(
                        &mut cmd,
                        engine,
                        &self.scene,
                        &self.state,
                        &lidar_scans,
                        pool,
                        stamp,
                    )
                    .map_err(|e| format!("lidar encode: {e}"))?
                {
                    Some(batch) => Some(batch),
                    None => {
                        dropped_count += 1;
                        None
                    }
                }
            } else {
                None
            };

            let mut encoded_cameras: Vec<(Vec<(usize, u64)>, EncodedCameraBatch)> =
                Vec::with_capacity(due_buckets.len());
            for (bucket_idx, bucket) in self.camera_buckets.iter_mut().enumerate() {
                let Some((_, entries)) = due_buckets.iter().find(|(i, _)| *i == bucket_idx) else {
                    continue;
                };
                if let Some((batch, pool)) = bucket.batch.as_mut() {
                    match batch
                        .encode_scene(&mut cmd, engine, &self.scene, &self.state, pool, Some(sim_time))
                        .map_err(|e| format!("camera encode: {e}"))?
                    {
                        Some(encoded) => encoded_cameras.push((entries.clone(), encoded)),
                        None => dropped_count += entries.len() as u64,
                    }
                }
            }

            // Chain GPU AnnotationProcessor directly off pending camera outputs.
            let mut ann_entries: Vec<(usize, usize)> = Vec::new();
            let encoded_ann = if let Some(ann) = self.annotation.as_mut() {
                let mut jobs = Vec::new();
                for (cam_idx, (entries, encoded)) in encoded_cameras.iter().enumerate() {
                    for &(mount, _) in entries {
                        if self.mounts[mount].resolved.spec.recognition
                            && let Ok(job) = encoded.annotation_job(mount as u64)
                        {
                            ann_entries.push((cam_idx, mount));
                            jobs.push(job);
                        }
                    }
                }
                if jobs.is_empty() {
                    None
                } else {
                    let stamp = ReadbackStamp {
                        sensor_id: u64::MAX - 1,
                        sample_id: due[0].sample_id,
                        scene_revision: self.revision,
                        simulation_time: Some(sim_time),
                    };
                    match ann
                        .processor
                        .encode_pending(&mut cmd, &jobs, &mut ann.pool, stamp)
                        .map_err(|e| format!("annotation encode: {e}"))?
                    {
                        Some(encoded) => Some(encoded),
                        None => {
                            ann_entries.clear();
                            None
                        }
                    }
                }
            } else {
                None
            };

            let encoded_rays = if let (Some(rays), false) =
                (self.generic_rays.as_mut(), ray_plan.is_empty())
            {
                let stamp = ReadbackStamp {
                    sensor_id: u64::MAX - 2,
                    sample_id: due[0].sample_id,
                    scene_revision: self.revision,
                    simulation_time: Some(sim_time),
                };
                match rays
                    .raycaster
                    .encode_scene(
                        &mut cmd,
                        engine,
                        &self.scene,
                        &self.state,
                        &ray_plan.origins,
                        &ray_plan.directions,
                        ray_plan.max_distance,
                        &mut rays.pool,
                        stamp,
                    )
                    .map_err(|e| format!("generic ray encode: {e}"))?
                {
                    Some(encoded) => Some(encoded),
                    None => {
                        dropped_count += 1;
                        None
                    }
                }
            } else {
                None
            };

            let submission = cmd.submit().map_err(|e| format!("sensor batch submit: {e}"))?;
            (
                submission,
                encoded_imu,
                encoded_lidar,
                encoded_cameras,
                encoded_ann,
                ann_entries,
                encoded_rays,
                imu_backpressured,
                dropped_count,
            )
        };

        self.stats.dropped += dropped_count;
        self.stats.gpu_batches += submission.stats.submissions;
        self.stats.gpu_dispatches += submission.stats.kernel_dispatches;

        if imu_dt.is_some() && !imu_backpressured {
            self.last_imu_time = Some(sim_time);
        }

        // Enqueue async readbacks from the submitted direct-output frames.
        if let Some(imu_sample) = encoded_imu
            && !imu_sample.priming
        {
            let frame = submission
                .frame(&imu_sample.frame)
                .map_err(|e| e.to_string())?;
            let n = self.imu_mounts.len();
            let sources = [
                frame
                    .channel::<MVec3>(0)
                    .and_then(|c| c.readback_source(0..n)),
                frame
                    .channel::<MVec3>(1)
                    .and_then(|c| c.readback_source(0..n)),
            ]
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
            let stamp = frame.stamp();
            match self
                .ring
                .try_enqueue(&sources, stamp)
                .map_err(|e| e.to_string())?
            {
                Some(ticket) => self.pending.push(Pending {
                    ticket,
                    sim_time,
                    kind: PendingKind::Imu { due: imu_due },
                }),
                None => self.stats.dropped += 1,
            }
        }

        if let Some(lidar_batch) = encoded_lidar {
            let frame = submission
                .frame(&lidar_batch.frame)
                .map_err(|e| e.to_string())?;
            let n = lidar_batch.ray_count;
            let sources = [
                frame
                    .channel::<MVec3>(4)
                    .and_then(|c| c.readback_source(0..n)),
                frame
                    .channel::<f64>(2)
                    .and_then(|c| c.readback_source(0..n)),
                frame
                    .channel::<i32>(3)
                    .and_then(|c| c.readback_source(0..n)),
            ]
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
            let pending_scans: Vec<PendingScan> = lidar_batch
                .scans
                .iter()
                .zip(&lidar_due)
                .zip(lidar_worlds)
                .map(|((range, &(mount, sample)), world)| PendingScan {
                    mount,
                    sample,
                    ray_offset: range.ray_offset as usize,
                    ray_count: range.ray_count as usize,
                    meta: match range.scan.pattern {
                        LidarPattern::Terrestrial(meta) => meta,
                        LidarPattern::Aerial(_) => unreachable!("terrestrial scans only"),
                    },
                    capture: crate::sensor_capture::CapturePose::dispatched(&world),
                    world,
                })
                .collect();
            let stamp = frame.stamp();
            match self
                .ring
                .try_enqueue(&sources, stamp)
                .map_err(|e| e.to_string())?
            {
                Some(ticket) => self.pending.push(Pending {
                    ticket,
                    sim_time,
                    kind: PendingKind::Lidar {
                        scans: pending_scans,
                    },
                }),
                None => self.stats.dropped += 1,
            }
        }

        if let Some(ray_query) = encoded_rays {
            let frame = submission
                .frame(&ray_query.frame)
                .map_err(|e| e.to_string())?;
            let n = ray_plan.origins.len();
            let sources = [
                frame
                    .channel::<f64>(2)
                    .and_then(|c| c.readback_source(0..n)),
                frame
                    .channel::<i32>(3)
                    .and_then(|c| c.readback_source(0..n)),
            ]
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
            let stamp = frame.stamp();
            match self
                .ring
                .try_enqueue(&sources, stamp)
                .map_err(|e| e.to_string())?
            {
                Some(ticket) => self.pending.push(Pending {
                    ticket,
                    sim_time,
                    kind: PendingKind::GenericRays {
                        plan: ray_plan,
                        shape_bodies: self.shape_bodies.clone(),
                    },
                }),
                None => self.stats.dropped += 1,
            }
        }

        let ann_frame = encoded_ann
            .as_ref()
            .map(|ann| submission.frame(&ann.frame).map_err(|e| e.to_string()))
            .transpose()?;

        for (cam_idx, (entries, encoded)) in encoded_cameras.iter().enumerate() {
            for &(mount, sample) in entries {
                let spec = self.mounts[mount].resolved.spec;
                let Some((layout, group)) = encoded.image(mount as u64) else {
                    continue;
                };
                let frame = submission.frame(&group.frame).map_err(|e| e.to_string())?;
                let pixels = layout.pixels.clone();
                let color = molla_color(&spec);
                let has_depth = spec.depth || spec.recognition;
                let has_shape = spec.recognition;
                let ann_pos = ann_entries
                    .iter()
                    .position(|&(c, m)| c == cam_idx && m == mount);
                let stamp = ReadbackStamp {
                    sensor_id: mount as u64,
                    sample_id: sample,
                    scene_revision: self.revision,
                    simulation_time: Some(sim_time),
                };
                let mut sources = Vec::new();
                if color {
                    sources.push(
                        frame
                            .channel::<u32>(1)
                            .and_then(|c| c.readback_source(pixels.clone()))
                            .map_err(|e| e.to_string())?,
                    );
                }
                if has_depth {
                    sources.push(
                        frame
                            .channel::<f64>(0)
                            .and_then(|c| c.readback_source(pixels.clone()))
                            .map_err(|e| e.to_string())?,
                    );
                }
                if has_shape {
                    sources.push(
                        frame
                            .channel::<u32>(4)
                            .and_then(|c| c.readback_source(pixels.clone()))
                            .map_err(|e| e.to_string())?,
                    );
                }
                let mut has_annotation = false;
                let mut has_boxes = false;
                if let (Some(ann_image), Some(ann), Some(af)) =
                    (ann_pos, encoded_ann.as_ref(), ann_frame)
                {
                    let ann_layout = &ann.layouts[ann_image];
                    has_annotation = true;
                    sources.push(
                        af.channel::<u32>(3)
                            .and_then(|c| c.readback_source(ann_image * 4..ann_image * 4 + 4))
                            .map_err(|e| e.to_string())?,
                    );
                    if !ann_layout.boxes.is_empty() {
                        has_boxes = true;
                        sources.push(
                            af.channel::<DeviceBoundingBox>(2)
                                .and_then(|c| c.readback_source(ann_layout.boxes.clone()))
                                .map_err(|e| e.to_string())?,
                        );
                    }
                }
                match self
                    .ring
                    .try_enqueue(&sources, stamp)
                    .map_err(|e| e.to_string())?
                {
                    Some(ticket) => self.pending.push(Pending {
                        ticket,
                        sim_time,
                        kind: PendingKind::Camera {
                            mount,
                            sample,
                            objects: self.objects.clone(),
                            has_color: color,
                            has_depth,
                            has_shape,
                            has_annotation,
                            has_boxes,
                        },
                    }),
                    None => self.stats.dropped += 1,
                }
            }
        }

        self.stats.samples += 1;
        self.stats.upload_bytes += (self.state.body_q.len() * 64) as u64;
        Ok(())
    }

    /// Hands every completed readback to `sink`; pending ones wait for later calls.
    pub fn collect(
        &mut self,
        stamp_ms: u32,
        sink: &mut dyn FnMut(&str, SensorReading<'_>) -> bool,
    ) {
        for (mount, mut reading) in self.ready.drain(..) {
            reading.stamp_ms = stamp_ms;
            let m = &mut self.mounts[mount];
            let delivered = sink(&m.name, SensorReading::Measurement(&reading));
            self.stats.frames += 1;
            m.record(delivered, reading.count.max(1));
        }
        if let Err(error) = self.ring.poll_releases() {
            warn!("gearbox-sensors: readback: {error}");
            self.pending.clear();
            self.ring.reset();
            return;
        }
        let mut kept = Vec::with_capacity(self.pending.len());
        for pending in self.pending.drain(..) {
            let frame = match self.ring.poll(&pending.ticket) {
                Ok(Some(frame)) => frame,
                Ok(None) => {
                    kept.push(pending);
                    continue;
                }
                Err(error) => {
                    warn!("gearbox-sensors: readback: {error}");
                    continue;
                }
            };
            self.stats.readback_bytes += frame.byte_len() as u64;
            match pending.kind {
                PendingKind::GenericRays { plan, shape_bodies } => {
                    let distances: Vec<f64> = frame.channel(0).unwrap_or_default();
                    let shapes: Vec<i32> = frame.channel(1).unwrap_or_default();
                    match self.generic.resolve_rays(
                        plan,
                        &distances,
                        &shapes,
                        &shape_bodies,
                        pending.sim_time,
                    ) {
                        Ok(readings) => {
                            for (mount, mut reading) in readings {
                                reading.stamp_ms = stamp_ms;
                                let m = &mut self.mounts[mount];
                                let delivered = sink(&m.name, SensorReading::Measurement(&reading));
                                self.stats.frames += 1;
                                m.record(delivered, reading.count.max(1));
                            }
                        }
                        Err(error) => warn!("gearbox-sensors: {error}"),
                    }
                }
                PendingKind::Camera {
                    mount,
                    sample,
                    objects,
                    has_color,
                    has_depth,
                    has_shape,
                    has_annotation,
                    has_boxes,
                } => {
                    let m = &mut self.mounts[mount];
                    let spec = m.resolved.spec;
                    let pixels = (spec.width * spec.height) as usize;
                    let mut ch = 0usize;
                    let colors: Vec<u32> = if has_color {
                        let v = frame.channel(ch).unwrap_or_default();
                        ch += 1;
                        v
                    } else {
                        Vec::new()
                    };
                    let depths: Vec<f64> = if has_depth {
                        let v = frame.channel(ch).unwrap_or_default();
                        ch += 1;
                        v
                    } else {
                        Vec::new()
                    };
                    let shapes: Vec<u32> = if has_shape {
                        let v = frame.channel(ch).unwrap_or_default();
                        ch += 1;
                        v
                    } else {
                        Vec::new()
                    };
                    let ann_counts = if has_annotation {
                        let words: Vec<u32> = frame.channel(ch).unwrap_or_default();
                        ch += 1;
                        AnnotationCounts::from_words(&words).ok()
                    } else {
                        None
                    };
                    let ann_boxes: Vec<DeviceBoundingBox> = if has_boxes {
                        frame.channel(ch).unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    if (has_color && colors.len() != pixels)
                        || (has_depth && depths.len() != pixels)
                        || (has_shape && shapes.len() != pixels)
                    {
                        warn!(
                            "gearbox-sensors: camera `{}` returned {} colors and {} depths for {}x{}",
                            m.name,
                            colors.len(),
                            depths.len(),
                            spec.width,
                            spec.height
                        );
                        continue;
                    }
                    let name = m.name.clone();
                    let link_index = m.resolved.link as u32;
                    let mut send = |wire: &CameraFrame| sink(&name, SensorReading::Camera(wire));
                    let mut delivered = true;
                    if has_color {
                        let bytes: Vec<u8> = colors.iter().flat_map(|c| c.to_le_bytes()).collect();
                        delivered &= camera_chunks(
                            &name, link_index, &spec, pending.sim_time, stamp_ms, sample,
                            CAMERA_COLOR, &bytes, &mut send,
                        );
                    }
                    if spec.depth {
                        let bytes: Vec<u8> = depths
                            .iter()
                            .map(|d| {
                                if *d >= 0.0 && *d < spec.range_m as f64 {
                                    *d as f32
                                } else {
                                    f32::INFINITY
                                }
                            })
                            .flat_map(|d| d.to_le_bytes())
                            .collect();
                        delivered &= camera_chunks(
                            &name, link_index, &spec, pending.sim_time, stamp_ms, sample,
                            CAMERA_DEPTH, &bytes, &mut send,
                        );
                    }
                    if spec.recognition {
                        let seen = match ann_counts {
                            Some(c) if c.visible_instances == 0 => Vec::new(),
                            Some(_) => match bounding_row_span(&ann_boxes, spec.height) {
                                Some((min_y, max_y)) => {
                                    let rows = max_y - min_y + 1;
                                    let start = min_y as usize * spec.width as usize;
                                    let end = (max_y as usize + 1) * spec.width as usize;
                                    recognize_rows(
                                        spec.width,
                                        spec.height,
                                        spec.vfov as f64,
                                        min_y,
                                        rows,
                                        &depths[start..end],
                                        &shapes[start..end],
                                        &|shape| {
                                            objects
                                                .shapes
                                                .get(shape as usize)
                                                .copied()
                                                .flatten()
                                                .map(u64::from)
                                        },
                                        1,
                                    )
                                }
                                None => Vec::new(),
                            },
                            None => molla_sensors::recognize(
                                spec.width,
                                spec.height,
                                spec.vfov as f64,
                                &depths,
                                &shapes,
                                &|shape| {
                                    objects
                                        .shapes
                                        .get(shape as usize)
                                        .copied()
                                        .flatten()
                                        .map(u64::from)
                                },
                                1,
                            ),
                        };
                        let mut reading = recognition(&name, link_index, pending.sim_time, sample, &seen, &objects.labels);
                        reading.stamp_ms = stamp_ms;
                        delivered &= sink(&name, SensorReading::Measurement(&reading));
                    }
                    self.stats.frames += 1;
                    m.record(delivered, 1);
                }
                PendingKind::Imu { due } => {
                    let accel: Vec<MVec3> = frame.channel(0).unwrap_or_default();
                    let gyro: Vec<MVec3> = frame.channel(1).unwrap_or_default();
                    for (mount, orientation) in due {
                        let Some(slot) = self.imu_mounts.iter().position(|&m| m == mount) else {
                            continue;
                        };
                        let (Some(a), Some(g)) = (accel.get(slot), gyro.get(slot)) else {
                            continue;
                        };
                        let channel = match self.mounts[mount].resolved.spec.kind {
                            SensorKind::Accelerometer => Some((measurement_kind::ACCELEROMETER, *a)),
                            SensorKind::Gyro => Some((measurement_kind::GYRO, *g)),
                            _ => None,
                        };
                        if let Some((kind, v)) = channel {
                            let m = &mut self.mounts[mount];
                            let mut reading = measurement(
                                &m.name,
                                m.resolved.link as u32,
                                kind,
                                pending.sim_time,
                                0,
                                vec![v.x, v.y, v.z],
                            );
                            reading.stamp_ms = stamp_ms;
                            let delivered = sink(&m.name, SensorReading::Measurement(&reading));
                            m.record(delivered, 1);
                            continue;
                        }
                        let q = (SIM_TO_ROS * orientation).normalize();
                        let reading = Imu::new(
                            Velocity {
                                vx: g.x,
                                vy: g.y,
                                vz: g.z,
                            },
                            Acceleration {
                                ax: a.x,
                                ay: a.y,
                                az: a.z,
                            },
                            Quaternion::new(q.w, q.x, q.y, q.z),
                        );
                        let mount = &mut self.mounts[mount];
                        let delivered = sink(&mount.name, SensorReading::Imu(&reading));
                        mount.record(delivered, 1);
                    }
                }
                PendingKind::Lidar { scans } => {
                    let points: Vec<MVec3> = frame.channel(0).unwrap_or_default();
                    let distances: Vec<f64> = frame.channel(1).unwrap_or_default();
                    let shapes: Vec<i32> = frame.channel(2).unwrap_or_default();
                    for scan in scans {
                        let range = scan.ray_offset..scan.ray_offset + scan.ray_count;
                        if range.end > points.len()
                            || range.end > distances.len()
                            || range.end > shapes.len()
                        {
                            continue;
                        }
                        let inverse = scan.world.inverse();
                        let mut ranges = Vec::with_capacity(scan.ray_count);
                        let mut local = Vec::with_capacity(scan.ray_count);
                        let mut hits = 0u32;
                        for i in range {
                            if shapes[i] >= 0 && distances[i].is_finite() {
                                hits += 1;
                                let p = inverse.transform_point(points[i]);
                                ranges.push(distances[i] as f32);
                                local.push([p.x as f32, p.y as f32, p.z as f32]);
                            } else {
                                ranges.push(f32::INFINITY);
                                local.push([f32::NAN; 3]);
                            }
                        }
                        let mount = &mut self.mounts[scan.mount];
                        let spec = mount.resolved.spec;
                        let capture = scan
                            .capture
                            .as_ref()
                            .filter(|_| !self.machine.is_empty())
                            .map(|c| c.native(&self.machine, &mount.name, pending.sim_time, scan.sample));
                        let (props, capacity, refused) =
                            crate::sensor_capture::chunk_props(&mount.name, capture.as_ref());
                        if let Some(error) = refused
                            && !std::mem::replace(&mut mount.capture_refused, true)
                        {
                            warn!("gearbox-sensors: `{}` LiDAR capture pose left out: {error}", mount.name);
                        }
                        let mut delivered = true;
                        for (chunk, chunk_ranges) in ranges.chunks(capacity).enumerate() {
                            let offset = chunk * capacity;
                            let chunk_points = &local[offset..offset + chunk_ranges.len()];
                            let mut wire = LidarScan {
                                sim_time_s: pending.sim_time,
                                theta_min: scan.meta.theta_range.0,
                                theta_max: scan.meta.theta_range.1,
                                phi_min: scan.meta.phi_range.0,
                                phi_max: scan.meta.phi_range.1,
                                max_range_m: scan.meta.max_distance,
                                link_index: mount.resolved.link as u32,
                                rows: spec.rows,
                                cols: spec.columns,
                                pulse_offset: offset as u32,
                                pulse_count: chunk_ranges.len() as u32,
                                hits: chunk_ranges.iter().filter(|r| r.is_finite()).count() as u32,
                                stamp_ms,
                                sample: scan.sample as u32,
                                ranges: chunk_ranges.to_vec(),
                                points: Vec::new(),
                                props: props.clone(),
                            };
                            wire.set_points(chunk_points);
                            delivered &= sink(&mount.name, SensorReading::Lidar(&wire));
                        }
                        self.stats.frames += 1;
                        mount.record(delivered, hits);
                    }
                }
            }
        }
        self.pending = kept;
    }
}

/// Per-shape albedo in model order: terrain and entity-less colliders are
/// ground, the rest take their entity's visual colour when it has one.
fn shape_colors(scene: &RigidScene, colors: &dyn Fn(Entity) -> Option<[f32; 3]>) -> Vec<MVec3> {
    let rgb = |c: [f32; 3]| MVec3::new(c[0].into(), c[1].into(), c[2].into());
    let mut shapes = vec![rgb(NEUTRAL_COLOR); scene.model().shape_count];
    let (mut visual, mut ground, mut neutral) = (0, 0, 0);
    for (handle, collider) in scene.colliders() {
        let color = if collider.user_tag == 0
            || matches!(collider.geometry, ColliderGeometry::Heightfield(_))
        {
            ground += 1;
            GROUND_COLOR
        } else if let Some(color) = colors(Entity::from_bits(collider.user_tag)) {
            visual += 1;
            color
        } else {
            neutral += 1;
            NEUTRAL_COLOR
        };
        for shape in scene.collider_indices(handle).unwrap_or(&[]) {
            if let Some(slot) = shapes.get_mut(shape.index()) {
                *slot = rgb(color);
            }
        }
    }
    info!("gearbox-sensors: collider colours: {visual} from visuals, {ground} ground, {neutral} neutral");
    shapes
}

/// Recognisable objects: each rigid body not in `own` is one, labelled by
/// its first collider `label` names. Returns each shape's object and the
/// labels; heightfields and bodiless untagged ground are no object.
fn shape_objects(
    scene: &RigidScene,
    own: &HashSet<u64>,
    label: &dyn Fn(Entity) -> Option<String>,
) -> SceneObjects {
    let mut shapes = vec![None; scene.model().shape_count];
    let (mut ids, mut labels) = (HashMap::new(), Vec::<String>::new());
    for (handle, collider) in scene.colliders() {
        let ground = collider.user_tag == 0 && collider.parent.is_none();
        if ground || matches!(collider.geometry, ColliderGeometry::Heightfield(_)) {
            continue;
        }
        let key = collider.parent.map_or(handle.to_bits() | 1 << 63, |b| b.to_bits());
        if own.contains(&key) {
            continue;
        }
        let id = *ids.entry(key).or_insert_with(|| {
            labels.push(String::new());
            labels.len() as u32 - 1
        });
        let slot = &mut labels[id as usize];
        if slot.is_empty()
            && collider.user_tag != 0
            && let Some(name) = label(Entity::from_bits(collider.user_tag))
        {
            *slot = name;
        }
        for shape in scene.collider_indices(handle).unwrap_or(&[]) {
            if let Some(slot) = shapes.get_mut(shape.index()) {
                *slot = Some(id);
            }
        }
    }
    for (id, slot) in labels.iter_mut().enumerate() {
        if slot.is_empty() {
            *slot = format!("object{id}");
        }
    }
    SceneObjects { shapes, labels }
}

/// Geometry, the IMU's body model and a state copy, taken under the physics lock.
/// The geometry and colours sensors trace in, from the physics model.
fn sensor_scene(
    engine: &Engine,
    scene: &RigidScene,
    colors: &dyn Fn(Entity) -> Option<[f32; 3]>,
) -> Result<SensorScene, String> {
    let mut sensor_scene = SensorScene::from_model(engine, scene.model(), scene_options()).map_err(|e| format!("sensor scene: {e}"))?;
    sensor_scene
        .set_shape_colors(engine, &shape_colors(scene, colors))
        .map_err(|e| format!("sensor scene colours: {e}"))?;
    report_unsupported(&sensor_scene);
    Ok(sensor_scene)
}

fn scene_options() -> SensorSceneOptions {
    SensorSceneOptions { unsupported: UnsupportedGeometryPolicy::ReportAndSkip, ..Default::default() }
}

fn report_unsupported(scene: &SensorScene) {
    let unsupported = scene.unsupported_shapes();
    if !unsupported.is_empty() {
        let kinds: HashSet<_> = unsupported.iter().map(|s| s.kind).collect();
        warn!("gearbox-sensors: {} collider shape(s) are invisible to sensors: {kinds:?}", unsupported.len());
    }
}

/// Starts building the sensor scene of the physics world's model on a
/// worker, from a copy of the shapes it traces, coloured as `colors` says.
fn build_scene_off_frame(
    engine: &Engine,
    rigid: &RigidScene,
    colors: Vec<MVec3>,
) -> Result<Task<Result<SensorScene, String>>, String> {
    let source = rigid.model();
    let mut copy = Model::empty();
    copy.body_count = source.body_count;
    copy.shape_count = source.shape_count;
    copy.shape_geometry = source.shape_geometry.clone();
    copy.shape_body = Buffer::from_vec(source.shape_body.host().map_err(|e| e.to_string())?.to_vec(), BufferUsage::Storage);
    copy.shape_transform = Buffer::from_vec(source.shape_transform.host().map_err(|e| e.to_string())?.to_vec(), BufferUsage::Storage);
    copy.shape_color = Buffer::from_vec(colors, BufferUsage::Storage);
    copy.meshes = source.meshes.clone();
    copy.hulls = source.hulls.clone();
    copy.heightfields = source.heightfields.clone();
    let device = engine.device().ok_or("sensor engine has no device")?.clone();
    let queue = engine.queue().ok_or("sensor engine has no queue")?.clone();
    Ok(AsyncComputeTaskPool::get().spawn(async move {
        let engine = Engine::from_device(device, queue, None);
        SensorScene::from_model(&engine, &copy, scene_options()).map_err(|e| format!("sensor scene: {e}"))
    }))
}

/// The physics bodies in index order: a rig samples a scene only over the
/// bodies it was built with.
fn body_order(rigid: &RigidScene) -> Vec<u64> {
    let mut bodies: Vec<_> = rigid.bodies().filter_map(|(h, _)| rigid.body_index(h).map(|i| (i.index(), h.to_bits()))).collect();
    bodies.sort_unstable();
    bodies.into_iter().map(|(_, h)| h).collect()
}

/// Owning body per shape index in the compiled physics model, used to map
/// GPU raycast shape hits back to [`MollaBodyId`].
fn shape_bodies(rigid: &RigidScene) -> Arc<[MollaBodyId]> {
    rigid
        .model()
        .shape_body
        .host()
        .map(|bodies| {
            bodies
                .iter()
                .map(|&b| {
                    if b >= 0 {
                        MollaBodyId::from_index(b as usize)
                    } else {
                        MollaBodyId::NONE
                    }
                })
                .collect::<Vec<_>>()
                .into()
        })
        .unwrap_or_else(|_| Arc::from([]))
}

/// Bodies detectable by a machine's radars: every non-own dynamic body
/// defaults to 1 m², overridden by an authored [`RadarCrossSection`] (0 opts
/// out), and fixed/kinematic bodies with a positive authored cross-section
/// act as static radar reflectors.
fn radar_targets(
    rigid: &RigidScene,
    own: &HashSet<u64>,
    rcs: &dyn Fn(Entity) -> Option<f64>,
) -> Vec<RadarTarget> {
    let mut out = Vec::new();
    for (handle, desc) in rigid.bodies() {
        if own.contains(&handle.to_bits()) {
            continue;
        }
        let authored = (desc.user_tag != 0)
            .then(|| rcs(Entity::from_bits(desc.user_tag)))
            .flatten();
        let cross_section = match authored {
            Some(v) if v.is_finite() && v > 0.0 => Some(v),
            Some(_) => None,
            None if desc.kind == molla_sim::runtime::BodyKind::Dynamic => Some(1.0),
            None => None,
        };
        if let Some(cross_section) = cross_section
            && let Some(body) = rigid.body_index(handle)
        {
            out.push(RadarTarget {
                body,
                cross_section,
            });
        }
    }
    out
}

/// A rig's own copy of the bodies: the IMU's model and the state it samples.
fn snapshot_bodies(
    engine: &Engine,
    scene: &RigidScene,
    handles: &[molla_sim::runtime::BodyHandle],
) -> Result<(Model, State, Vec<MollaBodyId>), String> {
    let bodies = handles
        .iter()
        .map(|&h| scene.body_index(h).ok_or("sensor body left the scene"))
        .collect::<Result<Vec<_>, _>>()?;
    let source = scene.model();
    let mut imu_model = Model::empty();
    imu_model.body_count = source.body_count;
    let host = |b: &Buffer<MVec3>| b.host().map(|h| h.to_vec());
    imu_model.body_com = Buffer::from_vec(
        host(&source.body_com).map_err(|e| e.to_string())?,
        BufferUsage::Storage,
    );
    imu_model.body_world = Buffer::from_vec(
        source
            .body_world
            .host()
            .map_err(|e| e.to_string())?
            .to_vec(),
        BufferUsage::Storage,
    );
    imu_model.gravity = Buffer::from_vec(
        host(&source.gravity).map_err(|e| e.to_string())?,
        BufferUsage::Storage,
    );
    imu_model.initial_body_q = Buffer::from_vec(
        source
            .initial_body_q
            .host()
            .map_err(|e| e.to_string())?
            .to_vec(),
        BufferUsage::Storage,
    );
    let device = engine.device().ok_or("sensor engine has no device")?;
    let queue = engine.queue().ok_or("sensor engine has no queue")?;
    imu_model
        .upload_to_device(device, queue)
        .map_err(|e| format!("imu model upload: {e}"))?;
    let state = scene.state().clone_host().map_err(|e| e.to_string())?;
    Ok((imu_model, state, bodies))
}

/// Copies the physics body poses and twists into the rig's own state.
fn copy_state(source: &State, target: &mut State) -> Result<(), String> {
    let (q, qd) = (
        source.body_q.host().map_err(|e| e.to_string())?,
        source.body_qd.host().map_err(|e| e.to_string())?,
    );
    let (tq, tqd) = (
        target.body_q.host_mut().map_err(|e| e.to_string())?,
        target.body_qd.host_mut().map_err(|e| e.to_string())?,
    );
    if q.len() != tq.len() || qd.len() != tqd.len() {
        return Err("body count changed without a scene revision".into());
    }
    tq.copy_from_slice(q);
    tqd.copy_from_slice(qd);
    Ok(())
}

/// A sweep along the link's +X with +Z as zenith; azimuth turns towards +Y.
fn scan_metadata(spec: &SensorSpec, world: &MTransform) -> ScanMetadata {
    let mut meta = ScanMetadata::new(world.position, spec.rows, spec.columns);
    meta.forward = world.rotation * MVec3::X;
    meta.up = world.rotation * MVec3::Z;
    let half_v = spec.vfov as f64 * 0.5;
    let hfov = spec.hfov as f64;
    meta.theta_range = (FRAC_PI_2 - half_v, FRAC_PI_2 + half_v);
    meta.phi_range = if hfov >= TAU - 1e-6 {
        (-PI, PI - TAU / spec.columns as f64)
    } else {
        (-hfov * 0.5, hfov * 0.5)
    };
    meta.max_distance = spec.range_m as f64;
    meta
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::backend::{BodyDesc, ColliderDesc, DVec3, JointDesc, JointKind, Pose, Shape};
    use crate::physics::molla::MollaBackend;

    #[test]
    fn sim_to_ros_permutes_axes_like_odom() {
        let sim = MVec3::new(1.0, 2.0, 3.0);
        let ros = SIM_TO_ROS * sim;
        assert!(
            (ros - MVec3::new(3.0, 1.0, 2.0)).length() < 1e-12,
            "{ros:?}"
        );
    }

    #[test]
    fn lidar_sweeps_forward_with_zenith_up() {
        let spec = SensorSpec {
            rate_hz: 10.0,
            columns: 4,
            rows: 1,
            hfov: std::f32::consts::TAU,
            vfov: 0.0,
            range_m: 50.0,
            render: crate::links::CameraRender::Geometry,
            ..SensorSpec::defaults(SensorKind::Lidar, false)
        };
        let world = MTransform::new(MVec3::new(1.0, 2.0, 3.0), MQuat::IDENTITY);
        let meta = scan_metadata(&spec, &world);
        assert_eq!(meta.origin, world.position);
        assert!((meta.rc_to_direction(0, 2) - MVec3::X).length() < 1e-6);
        assert!((meta.rc_to_direction(0, 3) - MVec3::Y).length() < 1e-6);
        assert!((meta.rc_to_direction(0, 0) - MVec3::NEG_X).length() < 1e-6);
        assert_eq!(meta.max_distance, 50.0);
        let narrow = SensorSpec {
            columns: 3,
            rows: 3,
            hfov: std::f32::consts::FRAC_PI_2,
            vfov: std::f32::consts::FRAC_PI_2,
            ..spec
        };
        let meta = scan_metadata(&narrow, &world);
        assert!((meta.rc_to_direction(1, 1) - MVec3::X).length() < 1e-6);
        assert!((meta.rc_to_direction(0, 1) - (MVec3::X + MVec3::Z).normalize()).length() < 1e-6);
        assert!((meta.rc_to_direction(1, 2) - (MVec3::X + MVec3::Y).normalize()).length() < 1e-6);
    }

    #[test]
    fn collider_colours_follow_visuals_with_ground_and_neutral_fallbacks() {
        let mut world = PhysicsWorld::with_backend(Box::new(MollaBackend::default()));
        let body = world.insert_body(BodyDesc::fixed());
        let cube = |entity: Option<Entity>| {
            let desc = ColliderDesc::new(Shape::Cuboid {
                half_extents: DVec3::splat(0.5),
            })
            .parent(body);
            match entity {
                Some(entity) => desc.entity(entity),
                None => desc,
            }
        };
        let (painted, bare) = (Entity::from_raw_u32(7).unwrap(), Entity::from_raw_u32(8).unwrap());
        for entity in [Some(painted), Some(bare), None] {
            world.insert_collider(cube(entity)).unwrap();
        }
        let mut colors = Vec::new();
        world.with_molla_scene(&mut |scene| {
            colors = shape_colors(scene, &|entity| (entity == painted).then_some([1.0, 0.0, 0.0]));
        });
        let rgb = |c: [f32; 3]| MVec3::new(c[0].into(), c[1].into(), c[2].into());
        assert_eq!(colors, [rgb([1.0, 0.0, 0.0]), rgb(NEUTRAL_COLOR), rgb(GROUND_COLOR)]);
    }

    #[test]
    fn remembered_colours_stay_with_their_own_subtree() {
        let mut world = World::new();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<Assets<Image>>();
        let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
        let red = materials.add(StandardMaterial::from_color(Color::srgb(1.0, 0.0, 0.0)));
        let blue = materials.add(StandardMaterial::from_color(Color::srgb(0.0, 0.0, 1.0)));
        let tray = |world: &mut World, material: &Handle<StandardMaterial>| {
            let tray = world.spawn(MeshMaterial3d(material.clone())).id();
            let plants = [world.spawn_empty().id(), world.spawn_empty().id()];
            world.entity_mut(tray).add_children(&plants);
            plants
        };
        let [red_a, red_b] = tray(&mut world, &red);
        let [blue_a, _] = tray(&mut world, &blue);
        let mut state = bevy::ecs::system::SystemState::<VisualColors>::new(&mut world);
        let visuals = state.get(&world).expect("visual colours are valid");
        let color = visuals.memo();
        let near = |c: Option<[f32; 3]>, want: [f32; 3]| {
            c.is_some_and(|c| c.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-5))
        };
        assert!(near(color(red_a), [1.0, 0.0, 0.0]), "{:?}", color(red_a));
        assert!(near(color(blue_a), [0.0, 0.0, 1.0]), "{:?}", color(blue_a));
        assert!(near(color(red_b), [1.0, 0.0, 0.0]), "{:?}", color(red_b));
    }

    /// A fixed sensor body 1 m over a ground slab with a wall 4.5 m ahead of
    /// the links, which are yawed so that their +Z is the sim +Y up axis: the
    /// IMU reads the reaction to gravity along its +Z and the horizontal LiDAR
    /// ranges the wall in front while missing everywhere else. The generic
    /// sensors face north under a sun 45° up ahead, a crate rests in radar
    /// view and a hinged arm swings down.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn imu_and_lidar_sample_the_molla_scene() {
        let engine = Engine::new_gpu().expect("GPU adapter");
        let mut world = PhysicsWorld::with_backend(Box::new(MollaBackend::default()));
        let ground = world.insert_body(BodyDesc::fixed());
        world
            .insert_collider(
                ColliderDesc::new(Shape::Cuboid {
                    half_extents: DVec3::new(20.0, 0.5, 20.0),
                })
                .parent(ground)
                .translation(DVec3::new(0.0, -0.5, 0.0)),
            )
            .unwrap();
        let wall = world.insert_body(BodyDesc::fixed());
        world
            .insert_collider(
                ColliderDesc::new(Shape::Cuboid {
                    half_extents: DVec3::new(0.5, 2.0, 5.0),
                })
                .parent(wall)
                .translation(DVec3::new(5.0, 2.0, 0.0)),
            )
            .unwrap();
        let mut sensor_body = BodyDesc::fixed();
        sensor_body.pose = Pose::from_translation(DVec3::Y);
        let carrier = world.insert_body(sensor_body);
        let cube = |world: &mut PhysicsWorld, body, half: f64| {
            world
                .insert_collider(
                    ColliderDesc::new(Shape::Cuboid {
                        half_extents: DVec3::splat(half),
                    })
                    .density(1000.0)
                    .parent(body),
                )
                .unwrap();
        };
        let crate_body =
            world.insert_body(BodyDesc::dynamic().pose(Pose::from_translation(DVec3::new(3.0, 0.25, -0.5))));
        let crate_entity = Entity::from_raw_u32(42).unwrap();
        world
            .insert_collider(
                ColliderDesc::new(Shape::Cuboid {
                    half_extents: DVec3::splat(0.25),
                })
                .density(1000.0)
                .parent(crate_body)
                .entity(crate_entity),
            )
            .unwrap();
        let base = world.insert_body(BodyDesc::fixed().pose(Pose::from_translation(DVec3::new(-10.0, 3.0, 0.0))));
        let arm = world.insert_body(BodyDesc::dynamic().pose(Pose::from_translation(DVec3::new(-9.0, 3.0, 0.0))));
        cube(&mut world, arm, 0.1);
        let hinge = world.backend.insert_joint(
            base,
            arm,
            JointDesc::new(
                JointKind::Revolute { axis: DVec3::Z },
                Pose::IDENTITY,
                Pose::from_translation(DVec3::NEG_X),
            ),
        );
        let mount = MTransform::new(MVec3::ZERO, MQuat::from_rotation_x(-FRAC_PI_2));
        let lidar_spec = SensorSpec {
            rate_hz: 50.0,
            columns: 8,
            rows: 1,
            hfov: std::f32::consts::TAU,
            vfov: 0.0,
            range_m: 20.0,
            width: 96,
            height: 64,
            render: crate::links::CameraRender::Geometry,
            color: true,
            depth: true,
            recognition: false,
            ..SensorSpec::defaults(SensorKind::Lidar, false)
        };
        let mounts = vec![
            ResolvedMount {
                link: 1,
                name: "imu_link".into(),
                spec: SensorSpec {
                    kind: SensorKind::Imu,
                    rate_hz: 100.0,
                    ..lidar_spec
                },
                body: carrier,
                entity: None,
                joint: None,
                local: mount,
            },
            ResolvedMount {
                link: 2,
                name: "lidar_link".into(),
                spec: lidar_spec,
                body: carrier,
                entity: None,
                joint: None,
                local: mount,
            },
            ResolvedMount {
                link: 3,
                name: "camera_link".into(),
                spec: SensorSpec {
                    kind: SensorKind::Camera,
                    rate_hz: 20.0,
                    vfov: 60f32.to_radians(),
                    width: 32,
                    height: 24,
                    ..lidar_spec
                },
                body: carrier,
                entity: None,
                joint: None,
                local: mount,
            },
        ];
        let generic = |link: usize, name: &str, kind, body, joint, local, spec: SensorSpec| ResolvedMount {
            link,
            name: name.into(),
            spec: SensorSpec {
                kind,
                rate_hz: 50.0,
                ..spec
            },
            body,
            entity: None,
            joint,
            local,
        };
        let radar = SensorSpec {
            hfov: 90f32.to_radians(),
            vfov: 40f32.to_radians(),
            ..lidar_spec
        };
        let force = SensorSpec {
            variant: crate::links::SensorVariant::Force3d,
            ..lidar_spec
        };
        let seer = SensorSpec {
            vfov: 60f32.to_radians(),
            width: 32,
            height: 24,
            color: false,
            depth: false,
            recognition: true,
            ..lidar_spec
        };
        let at_rest = MTransform::IDENTITY;
        let mut mounts = mounts;
        mounts.extend([
            generic(4, "accel_link", SensorKind::Accelerometer, carrier, None, mount, lidar_spec),
            generic(5, "gyro_link", SensorKind::Gyro, carrier, None, mount, lidar_spec),
            generic(6, "inertial_link", SensorKind::InertialUnit, carrier, None, mount, lidar_spec),
            generic(7, "compass_link", SensorKind::Compass, carrier, None, mount, lidar_spec),
            generic(8, "distance_link", SensorKind::Distance, carrier, None, mount, lidar_spec),
            generic(9, "light_link", SensorKind::Light, carrier, None, mount, lidar_spec),
            generic(10, "radar_link", SensorKind::Radar, carrier, None, mount, radar),
            generic(11, "touch_link", SensorKind::Touch, crate_body, None, at_rest, force),
            generic(12, "hinge_link", SensorKind::Position, arm, Some(hinge), at_rest, lidar_spec),
            generic(13, "seer_link", SensorKind::Camera, carrier, None, mount, seer),
        ]);
        let mut revision = 0;
        world.with_molla_scene(&mut |scene| revision = scene.revision());
        let label = |entity| (entity == crate_entity).then(|| "crate".to_string());
        let mut shared = None;
        let model = world.backend.molla(|scene| scene.model_stamp());
        let shared = shared_scene(&engine, &mut shared, &*world.backend, &model, &|_| None).unwrap();
        let mut rig = Rig::build(
            &engine,
            &*world.backend,
            mounts,
            revision,
            0.0,
            shared,
            &label,
            &|_| None,
            &[],
        )
        .unwrap();
        let mut imu = None;
        let mut lidar = None;
        let mut depth = None;
        let mut color = None;
        let mut measured: HashMap<String, gearbox_api::Measurement> = HashMap::new();
        let sun = molla_sensors::LightSource::Directional {
            towards: MVec3::new(1.0, 1.0, 0.0).normalize(),
            irradiance: 500.0,
        };
        for _ in 0..12 {
            world.step();
            let environment = SensorEnvironment {
                light: molla_sensors::LightEnvironment {
                    sources: vec![sun],
                    sky: 50.0,
                    up: MVec3::Y,
                },
                contacts: crate::sensor_generic::contact_forces(&*world.backend),
                ..Default::default()
            };
            rig.sample(&engine, &*world.backend, world.simulated_seconds, &environment, &mut |_| true)
                .unwrap();
            engine
                .device()
                .unwrap()
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: Some(Duration::from_secs(5)),
                })
                .unwrap();
            rig.collect(0, &mut |link, reading| {
                match reading {
                    SensorReading::Imu(reading) => {
                        assert_eq!(link, "imu_link");
                        imu = Some(*reading);
                    }
                    SensorReading::Lidar(scan) => {
                        assert_eq!(link, "lidar_link");
                        assert_eq!((scan.pulse_offset, scan.pulse_count), (0, 8));
                        assert!(scan.is_last());
                        lidar = Some(scan.clone());
                    }
                    SensorReading::Camera(frame) => {
                        assert_eq!(link, "camera_link");
                        assert_eq!((frame.width, frame.height, frame.row_offset), (32, 24, 0));
                        assert!(frame.is_last());
                        if frame.channel == CAMERA_DEPTH {
                            depth = Some(frame.depths());
                        } else {
                            color = Some(frame.colors());
                        }
                    }
                    SensorReading::Measurement(m) => {
                        assert_eq!(m.name(), link);
                        measured.insert(link.to_string(), m.clone());
                    }
                }
                true
            });
        }
        let values = |link: &str| measured.get(link).unwrap_or_else(|| panic!("no {link}")).values.clone();
        let near = |a: f64, b: f64, tol: f64| (a - b).abs() < tol;
        let accel = values("accel_link");
        assert!(near(accel[2], 9.81, 0.05) && near(accel[0], 0.0, 1e-3), "{accel:?}");
        assert!(values("gyro_link").iter().all(|v| v.abs() < 1e-6));
        let attitude = values("inertial_link");
        assert!(near(attitude[0], 0.0, 1e-9) && near(attitude[1], 0.0, 1e-9), "{attitude:?}");
        assert!(near(attitude[2], std::f64::consts::FRAC_PI_2, 1e-9), "{attitude:?}");
        let heading = values("compass_link");
        assert!(near(heading[0], 1.0, 1e-9) && near(heading[3], 0.0, 1e-9), "{heading:?}");
        let range = values("distance_link");
        assert!(near(range[0], 4.5, 1e-3), "{range:?}");
        let light = values("light_link");
        let direct = 500.0 * std::f64::consts::FRAC_1_SQRT_2;
        assert!(near(light[1], direct, 1e-6) && near(light[2], 25.0, 1e-9), "{light:?}");
        assert_eq!(light[3], 1.0);
        let radar = values("radar_link");
        assert_eq!(radar.len(), 5, "only the crate is a target in view: {radar:?}");
        assert!(near(radar[0], 9.8125f64.sqrt(), 0.05), "{radar:?}");
        assert!(near(radar[1], 0.5f64.atan2(3.0), 0.02), "{radar:?}");
        assert!(near(radar[2], (-0.75f64).atan2(9.25f64.sqrt()), 0.02), "{radar:?}");
        assert!(radar[3].abs() < 0.2, "{radar:?}");
        let touch = values("touch_link");
        assert!(touch[0] == 1.0 && touch[1] >= 1.0 && touch[2] > 0.0, "{touch:?}");
        assert!(touch[4] > 0.9 * touch[2], "the ground pushes the crate up: {touch:?}");
        let hinge = values("hinge_link");
        assert!(hinge[0].abs() > 1e-3 && hinge[1].abs() > 1e-3, "the arm swings down: {hinge:?}");
        let seen = &measured["seer_link"];
        let names: Vec<String> = seen.props().get("names").unwrap_or_default().lines().map(String::from).collect();
        let at = names.iter().position(|n| n == "crate").expect("the crate is recognised");
        let object: Vec<f64> = seen.records().nth(at).unwrap().to_vec();
        let (x, y, z) = (object[6], object[7], object[8]);
        assert!(object[1] >= 1.0 && object[4] < 32.0 && object[5] < 24.0, "{object:?}");
        assert!((-0.75..-0.25).contains(&x) && (-1.0..-0.5).contains(&y) && (-3.25..-2.7).contains(&z), "{object:?}");
        let depth = depth.expect("a depth frame");
        let color = color.expect("a color frame");
        assert_eq!((depth.len(), color.len()), (768, 768));
        let center = 12 * 32 + 16;
        assert!((depth[center] - 4.5).abs() < 0.05, "{}", depth[center]);
        assert!(depth[0].is_infinite() || depth[0] > 4.5, "{}", depth[0]);
        assert_ne!(color[center] & 0xff_ff_ff, 0);
        let imu = imu.expect("an IMU reading");
        assert!((imu.linear_acceleration.az - 9.81).abs() < 0.05, "{imu:?}");
        assert!(imu.linear_acceleration.ax.abs() < 1e-3 && imu.linear_acceleration.ay.abs() < 1e-3);
        assert!(imu.angular_velocity.vx.abs() < 1e-6 && imu.angular_velocity.vz.abs() < 1e-6);
        let q = MQuat::from_xyzw(
            imu.orientation.x,
            imu.orientation.y,
            imu.orientation.z,
            imu.orientation.w,
        );
        assert!(
            (q * MVec3::Z - MVec3::Z).length() < 1e-9,
            "sensor +Z is not ROS up: {q:?}"
        );
        assert!(
            (q * MVec3::X - MVec3::Y).length() < 1e-9,
            "sim +X is not ROS +Y: {q:?}"
        );
        let scan = lidar.expect("a LiDAR scan");
        assert_eq!((scan.rows, scan.cols), (1, 8));
        let ranges = scan.ranges.clone();
        let points = scan.points();
        assert_eq!(ranges.len(), 8);
        assert!((ranges[4] - 4.5).abs() < 1e-3, "{ranges:?}");
        assert!(
            (points[4][0] - 4.5).abs() < 1e-3 && points[4][1].abs() < 1e-3,
            "{points:?}"
        );
        assert!((ranges[3] - 4.5 * 2f32.sqrt()).abs() < 1e-3, "{ranges:?}");
        assert!((ranges[5] - 4.5 * 2f32.sqrt()).abs() < 1e-3, "{ranges:?}");
        assert!(
            ranges[0].is_infinite() && ranges[6].is_infinite(),
            "{ranges:?}"
        );
        assert!(points[0][0].is_nan());
        assert_eq!(scan.hits, 3, "{ranges:?}");
        assert!(rig.mounts.iter().all(|m| m.published >= 1));

        // Links sample only while someone asks for them.
        rig.take_render_requests();
        let run = |world: &mut PhysicsWorld, rig: &mut Rig, wanted: &dyn Fn(&str) -> bool| {
            let (mut asked, mut got) = (Vec::new(), Vec::new());
            for _ in 0..12 {
                world.step();
                let mut ask = |link: &str| {
                    asked.push(link.to_string());
                    wanted(link)
                };
                rig.sample(&engine, &*world.backend, world.simulated_seconds, &SensorEnvironment::default(), &mut ask)
                    .unwrap();
                let wait = wgpu::PollType::Wait { submission_index: None, timeout: Some(Duration::from_secs(5)) };
                engine.device().unwrap().poll(wait).unwrap();
                rig.collect(0, &mut |link, _| {
                    got.push(link.to_string());
                    true
                });
            }
            (asked, got)
        };
        let (asked, got) = run(&mut world, &mut rig, &|_| false);
        assert!(got.is_empty(), "nobody asked, yet {got:?} sent");
        assert!(["imu_link", "lidar_link", "camera_link"].iter().all(|l| asked.iter().any(|a| a == l)), "{asked:?}");
        assert!(rig.take_render_requests().is_empty());
        let (asked, got) = run(&mut world, &mut rig, &|link| link == "imu_link");
        let imu_asks = asked.iter().filter(|l| *l == "imu_link").count();
        assert!(imu_asks >= 2 && got.iter().all(|l| l == "imu_link"), "{got:?}");
        assert_eq!(got.len(), imu_asks - 1, "after a gap the IMU primes before it reads");
    }

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn a_rig_samples_its_scene_while_a_newer_one_builds_then_takes_it() {
        AsyncComputeTaskPool::get_or_init(bevy::tasks::TaskPool::new);
        let engine = Engine::new_gpu().expect("GPU adapter");
        let mut world = PhysicsWorld::with_backend(Box::new(MollaBackend::default()));
        let ground = world.insert_body(BodyDesc::fixed());
        let slab = Shape::Cuboid { half_extents: DVec3::new(20.0, 0.5, 20.0) };
        world.insert_collider(ColliderDesc::new(slab).parent(ground).translation(DVec3::new(0.0, -0.5, 0.0))).unwrap();
        let carrier = world.insert_body(BodyDesc::fixed().pose(Pose::from_translation(DVec3::Y)));
        let seer = SensorSpec {
            rate_hz: 50.0,
            vfov: 60f32.to_radians(),
            range_m: 20.0,
            width: 32,
            height: 24,
            render: crate::links::CameraRender::Geometry,
            color: false,
            depth: false,
            recognition: true,
            ..SensorSpec::defaults(SensorKind::Camera, false)
        };
        let mount = ResolvedMount {
            link: 1,
            name: "seer_link".into(),
            spec: seer,
            body: carrier,
            entity: None,
            joint: None,
            local: MTransform::new(MVec3::ZERO, MQuat::from_rotation_x(-FRAC_PI_2)),
        };
        let plant = Entity::from_raw_u32(7).unwrap();
        let label = |entity| (entity == plant).then(|| "plant:A/0".to_string());
        let mut rigs = MachineSensorRigs::default();
        let model = world.backend.molla(|scene| scene.model_stamp());
        let shared = shared_scene(&engine, &mut rigs.scene, &*world.backend, &model, &|_| None).unwrap();
        let mut rig = Rig::build(
            &engine,
            &*world.backend,
            vec![mount],
            0,
            0.0,
            shared,
            &label,
            &|_| None,
            &[],
        )
        .unwrap();
        rig.model = Some(model);
        rigs.rigs.insert("bot".into(), rig);
        let seen = |rigs: &mut MachineSensorRigs, world: &mut PhysicsWorld| {
            let mut names = String::new();
            for _ in 0..12 {
                world.step();
                let rig = rigs.rigs.get_mut("bot").unwrap();
                rig.sample(&engine, &*world.backend, world.simulated_seconds, &SensorEnvironment::default(), &mut |_| true).unwrap();
                let wait = wgpu::PollType::Wait { submission_index: None, timeout: Some(Duration::from_secs(5)) };
                engine.device().unwrap().poll(wait).unwrap();
                rig.collect(0, &mut |_, reading| {
                    if let SensorReading::Measurement(m) = reading {
                        names = m.props().get("names").unwrap_or_default().to_string();
                    }
                    true
                });
            }
            names
        };
        assert!(!seen(&mut rigs, &mut world).contains("plant"));

        // A plant's hull standing on no body changes the model, not the bodies.
        let hull = Shape::Cuboid { half_extents: DVec3::new(0.2, 0.3, 0.2) };
        let plant_hull = ColliderDesc { enabled: false, ..ColliderDesc::new(hull).translation(DVec3::new(3.0, 1.0, 0.0)).entity(plant) };
        world.insert_collider(plant_hull).unwrap();
        let model = world.backend.molla(|scene| scene.model_stamp());
        let bodies = world.backend.molla(body_order);
        assert_eq!(bodies, rigs.rigs["bot"].bodies);
        let build = world.backend.molla(|rigid| {
            let objects = HashMap::from([("bot".to_string(), Arc::new(shape_objects(rigid, &HashSet::new(), &label)))]);
            let targets = HashMap::from([("bot".to_string(), radar_targets(rigid, &HashSet::new(), &|_| None))]);
            let shape_bodies = shape_bodies(rigid);
            build_scene_off_frame(&engine, rigid, shape_colors(rigid, &|_| None)).map(|task| {
                SceneBuild {
                    model: model.clone(),
                    bodies,
                    shape_bodies,
                    objects,
                    radar_targets: targets,
                    task,
                }
            })
        });
        rigs.building = Some(build.unwrap());
        assert!(!seen(&mut rigs, &mut world).contains("plant"), "the rig samples its old scene meanwhile");
        for _ in 0..500 {
            swap_built_scene(&mut rigs, &model);
            if rigs.building.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(rigs.building.is_none(), "the scene builds");
        assert_eq!(rigs.rigs["bot"].model.as_ref(), Some(&model));
        assert!(seen(&mut rigs, &mut world).contains("plant:A/0"));
    }

    /// Moves the robot between dispatch and readback at a far, rotated site:
    /// points placed by their capture stay on the wall, the current pose does not.
    #[test]
    fn a_delayed_sweep_lands_where_it_was_captured_not_where_the_robot_is_now() {
        use crate::physics::backend::DQuat;
        use bevy::math::{DQuat as BQuat, DVec3 as BVec3};
        let engine = Engine::new_gpu().expect("GPU adapter");
        let home = gearbox_globe::Datum::at(52.370216, 4.895168);
        let far = gearbox_globe::Datum::at(-33.8688, 151.2093);
        crate::globe::set_site_frames(vec![home, far]);
        let offset = DVec3::X * gearbox_globe::physics_offset(1).x;
        let mut world = PhysicsWorld::with_backend(Box::new(MollaBackend::default()));
        let wall = world.insert_body(BodyDesc::fixed());
        let face = Shape::Cuboid { half_extents: DVec3::new(0.5, 3.0, 8.0) };
        world.insert_collider(ColliderDesc::new(face).parent(wall).translation(offset + DVec3::new(5.0, 1.0, 0.0))).unwrap();
        let start = Pose { translation: offset + DVec3::new(0.0, 1.0, 0.0), rotation: DQuat::from_rotation_y(0.4) };
        let carrier = world.insert_body(BodyDesc::fixed().pose(start));
        let spec = SensorSpec {
            rate_hz: 50.0,
            columns: 64,
            rows: 1,
            hfov: std::f32::consts::TAU,
            vfov: 0.0,
            range_m: 20.0,
            render: crate::links::CameraRender::Geometry,
            color: false,
            depth: false,
            ..SensorSpec::defaults(SensorKind::Lidar, false)
        };
        let mount = ResolvedMount {
            link: 1,
            name: "roof_lidar".into(),
            spec,
            body: carrier,
            entity: None,
            joint: None,
            local: MTransform::new(MVec3::new(0.2, 0.3, 0.0), MQuat::from_rotation_x(-FRAC_PI_2)),
        };
        let mut shared = None;
        let model = world.backend.molla(|scene| scene.model_stamp());
        let shared = shared_scene(&engine, &mut shared, &*world.backend, &model, &|_| None).unwrap();
        let mut rig = Rig::build(
            &engine,
            &*world.backend,
            vec![mount],
            0,
            0.0,
            shared,
            &|_| None,
            &|_| None,
            &[],
        )
        .unwrap();
        rig.machine = "rover".into();
        let mut sweeps = Vec::new();
        for _ in 0..12 {
            world.step();
            rig.sample(&engine, &*world.backend, world.simulated_seconds, &SensorEnvironment::default(), &mut |_| true)
                .unwrap();
            if rig.pending.is_empty() {
                continue;
            }
            let moved = Pose { translation: offset + DVec3::new(-2.0, 1.0, 1.5), rotation: DQuat::from_rotation_y(1.3) };
            world.backend.body_mut(carrier).unwrap().set_position(moved, true);
            let wait = wgpu::PollType::Wait { submission_index: None, timeout: Some(Duration::from_secs(5)) };
            engine.device().unwrap().poll(wait).unwrap();
            rig.collect(0, &mut |_, reading| {
                if let SensorReading::Lidar(scan) = reading {
                    sweeps.push(scan.clone());
                }
                true
            });
            break;
        }
        assert!(!sweeps.is_empty(), "a sweep was dispatched and read back");
        let now = world.backend.body(carrier).unwrap().position();
        let (mut on_wall, mut misplaced, mut worst) = (0, 0, 0.0f64);
        for scan in &sweeps {
            let capture = scan.native_capture().unwrap().expect("capture metadata");
            assert_eq!(capture.machine, "rover");
            assert_eq!(capture.transform.stamp.scene_epoch, crate::sensor_capture::scene_epoch());
            let [w, x, y, z] = capture.transform.datum.as_ref().unwrap().axes_to_ecef_wxyz;
            let axes = BQuat::from_xyzw(x, y, z, w);
            let origin = BVec3::from_array(capture.transform.datum.as_ref().unwrap().origin_ecef_m);
            let pose = &capture.transform.pose;
            let [w, x, y, z] = pose.rotation_wxyz;
            let rotation = BQuat::from_xyzw(x, y, z, w);
            for (i, range) in scan.ranges.iter().enumerate() {
                if !range.is_finite() {
                    continue;
                }
                let p = [0, 1, 2].map(|k| scan.points[3 * i + k] as f64);
                let ecef = origin + axes * (rotation * BVec3::from_array(p) + BVec3::from_array(pose.translation_m));
                let site = far.from_ecef(ecef);
                worst = worst.max((site.x - 4.5).abs());
                on_wall += 1;
                let link = DVec3::new(0.2, 0.3, 0.0) + DQuat::from_rotation_x(-FRAC_PI_2) * DVec3::from_array(p);
                let guessed = now.translation + now.rotation * link - offset;
                if (guessed.x - 4.5).abs() > 0.5 {
                    misplaced += 1;
                }
            }
        }
        println!("delayed sweep: {on_wall} hits, worst wall error {worst:.4} m, {misplaced} misplaced by the current pose");
        assert!(worst < 1e-3, "captured hits left the wall face by {worst} m");
        assert!(on_wall >= 8, "the wall faces a quarter of the ring: {on_wall} hits");
        assert!(misplaced > on_wall / 2, "the current pose would misplace the sweep");
    }

    #[test]
    fn a_part_is_labelled_by_its_own_machine_in_a_shared_stage() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        let (_, bodies) = crate::controller::spawn_fleet(&mut world, &["Eve", "Percy"]);
        let mut physics = PhysicsWorld::default();
        for &body in &bodies {
            let handle = physics.insert_body(BodyDesc::dynamic());
            physics.entity_to_body.insert(body, handle);
        }
        world.insert_resource(physics);
        let part = world.spawn(ChildOf(bodies[1])).id();
        let label = world
            .run_system_once(
                move |labels: Query<&SensorLabel>,
                      parents: Query<&ChildOf>,
                      prims: Query<(Entity, &UsdPrimRef)>,
                      physics: Res<PhysicsWorld>,
                      inventory: Res<ControllerInventory>| {
                    part_label(part, &labels, &parents, &prims, &physics, &inventory)
                },
            )
            .unwrap();
        assert_eq!(label.as_deref(), Some("percy:/Fleet/Percy/body"));
    }

    #[test]
    fn resolve_mounts_isolates_broken_links_from_healthy_siblings() {
        use crate::links::{LinkRole, LinkSpec, LinkTree};
        use bevy::ecs::system::RunSystemOnce;

        let mut world = World::new();
        PrimIndex::track(&mut world);
        let scene_root = world.spawn(GlobalTransform::IDENTITY).id();
        let chassis = world
            .spawn((GlobalTransform::IDENTITY, UsdPrimRef::new("/robot/chassis"), ChildOf(scene_root)))
            .id();
        let _imu = world
            .spawn((GlobalTransform::from_xyz(0.0, 1.0, 0.0), UsdPrimRef::new("/robot/chassis/imu"), ChildOf(chassis)))
            .id();
        let _bad_pos = world
            .spawn((GlobalTransform::IDENTITY, UsdPrimRef::new("/robot/chassis/bad_pos"), ChildOf(chassis)))
            .id();
        let _orphan = world
            .spawn((GlobalTransform::IDENTITY, UsdPrimRef::new("/robot/orphan_lidar"), ChildOf(scene_root)))
            .id();

        let mut physics = PhysicsWorld::with_backend(Box::new(MollaBackend::default()));
        let chassis_body = physics.insert_body(BodyDesc::dynamic());
        physics.entity_to_body.insert(chassis, chassis_body);
        world.insert_resource(physics);

        let mk_link = |name: &str, prim: &str, role, parent: Option<&str>, body: Option<&str>, sensor| LinkSpec {
            name: name.into(),
            prim_path: prim.into(),
            role,
            parent: parent.map(String::from),
            joint_prim: None,
            body_prim: body.map(String::from),
            static_offset: None,
            coupling: None,
            element: None,
            values: Vec::new(),
            sensor,
        };
        let tree = LinkTree {
            links: vec![
                mk_link("base_link", "/robot/chassis", LinkRole::Base, None, Some("/robot/chassis"), None),
                mk_link("imu", "/robot/chassis/imu", LinkRole::Sensor, Some("base_link"), None, Some(SensorSpec::defaults(SensorKind::Imu, false))),
                mk_link("bad_pos", "/robot/chassis/bad_pos", LinkRole::Sensor, Some("base_link"), None, Some(SensorSpec::defaults(SensorKind::Position, false))),
                mk_link("orphan_lidar", "/robot/orphan_lidar", LinkRole::Sensor, None, None, Some(SensorSpec::defaults(SensorKind::Lidar, false))),
            ],
            ..Default::default()
        };

        let resolution = world
            .run_system_once(
                move |physics: Res<PhysicsWorld>,
                      prim_index: Res<PrimIndex>,
                      transforms: Query<&GlobalTransform>| {
                    resolve_mounts(&physics, &tree, scene_root, &prim_index, &transforms)
                },
            )
            .unwrap();
        assert_eq!(resolution.mounts.len(), 1, "healthy IMU link still resolves");
        assert_eq!(resolution.mounts[0].name, "imu");
        assert_eq!(resolution.failed.len(), 2, "{:?}", resolution.failed);
        assert!(resolution.failed.iter().any(|(n, _)| n == "bad_pos"));
        assert!(resolution.failed.iter().any(|(n, _)| n == "orphan_lidar"));
    }

    #[test]
    fn attach_sensor_metadata_reads_rcs_and_label_from_usd() {
        let dir = std::env::temp_dir().join("gearbox-sensor-meta-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("meta-{}.usda", std::process::id()));
        std::fs::write(
            &path,
            r#"#usda 1.0
(
    defaultPrim = "World"
)
def Xform "World"
{
    def Cube "reflector"
    {
        float gearbox:radar:crossSection = 12.5
        token gearbox:sensor:label = "reflector_north"
    }
    def Cube "stealth"
    {
        float gearbox:value:radar_cross_section = 0
    }
}
"#,
        )
        .unwrap();
        let stage = openusd::usd::Stage::open(path.to_str().unwrap()).expect("open stage");
        let _ = std::fs::remove_file(&path);
        let mut world = World::new();
        let e1 = world.spawn_empty().id();
        let e2 = world.spawn_empty().id();
        crate::physics::attach::attach_sensor_metadata(&mut world, e1, &stage, "/World/reflector");
        crate::physics::attach::attach_sensor_metadata(&mut world, e2, &stage, "/World/stealth");
        assert_eq!(world.get::<RadarCrossSection>(e1), Some(&RadarCrossSection(12.5)));
        assert_eq!(world.get::<SensorLabel>(e1).map(|l| l.0.as_str()), Some("reflector_north"));
        assert_eq!(world.get::<RadarCrossSection>(e2), Some(&RadarCrossSection(0.0)));
        assert!(world.get::<SensorLabel>(e2).is_none());
    }

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn single_command_batch_combines_imu_lidar_batched_cameras_annotations_and_gpu_rays() {
        let engine = Engine::new_gpu().expect("GPU adapter");
        let mut world = PhysicsWorld::with_backend(Box::new(MollaBackend::default()));
        let ground = world.insert_body(BodyDesc::fixed());
        world
            .insert_collider(
                ColliderDesc::new(Shape::Cuboid {
                    half_extents: DVec3::new(20.0, 0.5, 20.0),
                })
                .parent(ground)
                .translation(DVec3::new(0.0, -0.5, 0.0)),
            )
            .unwrap();
        let carrier = world.insert_body(BodyDesc::fixed().pose(Pose::from_translation(DVec3::Y)));
        let box_entity = Entity::from_raw_u32(77).unwrap();
        let crate_body = world.insert_body(
            BodyDesc::dynamic()
                .pose(Pose::from_translation(DVec3::new(4.0, 0.5, 0.0)))
                .entity(box_entity),
        );
        world
            .insert_collider(
                ColliderDesc::new(Shape::Cuboid {
                    half_extents: DVec3::splat(0.4),
                })
                .density(500.0)
                .parent(crate_body)
                .entity(box_entity),
            )
            .unwrap();

        let mount = MTransform::new(MVec3::ZERO, MQuat::from_rotation_x(-FRAC_PI_2));
        let cam_spec = SensorSpec {
            rate_hz: 50.0,
            vfov: 60f32.to_radians(),
            range_m: 20.0,
            width: 32,
            height: 24,
            render: crate::links::CameraRender::Geometry,
            color: true,
            depth: true,
            recognition: true,
            ..SensorSpec::defaults(SensorKind::Camera, false)
        };
        let mounts = vec![
            ResolvedMount {
                link: 1,
                name: "imu".into(),
                spec: SensorSpec { rate_hz: 50.0, ..SensorSpec::defaults(SensorKind::Imu, false) },
                body: carrier,
                entity: None,
                joint: None,
                local: mount,
            },
            ResolvedMount {
                link: 2,
                name: "lidar".into(),
                spec: SensorSpec {
                    rate_hz: 50.0,
                    columns: 16,
                    rows: 1,
                    vfov: 0.0,
                    range_m: 20.0,
                    ..SensorSpec::defaults(SensorKind::Lidar, false)
                },
                body: carrier,
                entity: None,
                joint: None,
                local: mount,
            },
            ResolvedMount {
                link: 3,
                name: "cam_a".into(),
                spec: cam_spec,
                body: carrier,
                entity: None,
                joint: None,
                local: mount,
            },
            ResolvedMount {
                link: 4,
                name: "cam_b".into(),
                spec: cam_spec,
                body: carrier,
                entity: None,
                joint: None,
                local: mount,
            },
            ResolvedMount {
                link: 5,
                name: "dist".into(),
                spec: SensorSpec { rate_hz: 50.0, range_m: 20.0, ..SensorSpec::defaults(SensorKind::Distance, false) },
                body: carrier,
                entity: None,
                joint: None,
                local: mount,
            },
            ResolvedMount {
                link: 6,
                name: "radar".into(),
                spec: SensorSpec {
                    rate_hz: 50.0,
                    hfov: 90f32.to_radians(),
                    vfov: 40f32.to_radians(),
                    range_m: 20.0,
                    ..SensorSpec::defaults(SensorKind::Radar, false)
                },
                body: carrier,
                entity: None,
                joint: None,
                local: mount,
            },
        ];
        let mut shared = None;
        let model = world.backend.molla(|scene| scene.model_stamp());
        let shared = shared_scene(&engine, &mut shared, &*world.backend, &model, &|_| None).unwrap();
        let label = |e| (e == box_entity).then(|| "box77".to_string());
        let mut rig = Rig::build(&engine, &*world.backend, mounts, 0, 0.0, shared, &label, &|_| None, &[]).unwrap();
        assert!(rig.annotation.is_some(), "GPU annotation processor built");
        assert_eq!(rig.camera_buckets.len(), 1, "same-rate cameras share one bucket");

        // Sample on a tick where every 50 Hz sensor is due -> exactly 1 GPU batch.
        let mut seen = HashSet::new();
        for _ in 0..6 {
            world.step();
            let before_batches = rig.stats.gpu_batches;
            let before_samples = rig.stats.samples;
            rig.sample(&engine, &*world.backend, world.simulated_seconds, &SensorEnvironment::default(), &mut |_| true)
                .unwrap();
            if rig.stats.samples > before_samples {
                assert_eq!(
                    rig.stats.gpu_batches - before_batches,
                    1,
                    "all due GPU sensors encode into one SensorCommandBatch"
                );
            }
            let wait = wgpu::PollType::Wait { submission_index: None, timeout: Some(Duration::from_secs(5)) };
            engine.device().unwrap().poll(wait).unwrap();
            rig.collect(0, &mut |link, _| {
                seen.insert(link.to_string());
                true
            });
        }
        // Both homogeneous cameras share 1 group inside CameraBatch.
        assert_eq!(rig.camera_buckets[0].batch.as_ref().unwrap().0.group_count(), 1);
        for link in ["imu", "lidar", "cam_a", "cam_b", "dist", "radar"] {
            assert!(seen.contains(link), "missing reading from {link}: {seen:?}");
        }

        // Unsubscribing cam_b shrinks the active CameraBatch to cam_a only.
        let mut cam_links = HashSet::new();
        for _ in 0..4 {
            world.step();
            rig.sample(&engine, &*world.backend, world.simulated_seconds, &SensorEnvironment::default(), &mut |l| l == "cam_a")
                .unwrap();
            let wait = wgpu::PollType::Wait { submission_index: None, timeout: Some(Duration::from_secs(5)) };
            engine.device().unwrap().poll(wait).unwrap();
            rig.collect(0, &mut |link, _| {
                cam_links.insert(link.to_string());
                true
            });
        }
        assert_eq!(rig.camera_buckets[0].active.len(), 1);
        assert!(cam_links.contains("cam_a") && !cam_links.contains("cam_b"), "{cam_links:?}");
    }

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn radar_targets_respect_authored_rcs_opt_out_and_static_reflectors() {
        let engine = Engine::new_gpu().expect("GPU adapter");
        let mut world = PhysicsWorld::with_backend(Box::new(MollaBackend::default()));
        let carrier = world.insert_body(BodyDesc::fixed().pose(Pose::from_translation(DVec3::Y)));

        let e_default = Entity::from_raw_u32(10).unwrap();
        let e_strong = Entity::from_raw_u32(11).unwrap();
        let e_stealth = Entity::from_raw_u32(12).unwrap();
        let e_static = Entity::from_raw_u32(13).unwrap();

        let add_box = |world: &mut PhysicsWorld, desc: BodyDesc, entity: Entity| {
            let body = world.insert_body(desc.entity(entity));
            world
                .insert_collider(
                    ColliderDesc::new(Shape::Cuboid { half_extents: DVec3::splat(0.25) })
                        .density(1000.0)
                        .parent(body)
                        .entity(entity),
                )
                .unwrap();
            body
        };
        // Default 1 m² dynamic at 5 m ahead (left of center).
        add_box(&mut world, BodyDesc::dynamic().pose(Pose::from_translation(DVec3::new(5.0, 1.0, -1.0))), e_default);
        // Strong 10 m² dynamic at the exact same range (right of center) -> +10 dB.
        add_box(&mut world, BodyDesc::dynamic().pose(Pose::from_translation(DVec3::new(5.0, 1.0, 1.0))), e_strong);
        // Opted-out 0 m² dynamic at 3 m -> must not be detected.
        add_box(&mut world, BodyDesc::dynamic().pose(Pose::from_translation(DVec3::new(3.0, 1.0, 0.0))), e_stealth);
        // Fixed static reflector with 5 m² at (8, 1, 3.5) (clear of all foreground boxes) -> must be detected!
        add_box(&mut world, BodyDesc::fixed().pose(Pose::from_translation(DVec3::new(8.0, 1.0, 3.5))), e_static);

        let rcs = |e: Entity| match e {
            e if e == e_strong => Some(10.0),
            e if e == e_stealth => Some(0.0),
            e if e == e_static => Some(5.0),
            _ => None,
        };
        let mount = ResolvedMount {
            link: 1,
            name: "radar".into(),
            spec: SensorSpec {
                rate_hz: 50.0,
                hfov: 90f32.to_radians(),
                vfov: 40f32.to_radians(),
                range_m: 30.0,
                ..SensorSpec::defaults(SensorKind::Radar, false)
            },
            body: carrier,
            entity: None,
            joint: None,
            local: MTransform::new(MVec3::ZERO, MQuat::from_rotation_x(-FRAC_PI_2)),
        };
        let mut shared = None;
        let model = world.backend.molla(|scene| scene.model_stamp());
        let shared = shared_scene(&engine, &mut shared, &*world.backend, &model, &|_| None).unwrap();
        let mut rig = Rig::build(&engine, &*world.backend, vec![mount], 0, 0.0, shared, &|_| None, &rcs, &[]).unwrap();

        let mut reading = None;
        for _ in 0..4 {
            world.step();
            rig.sample(&engine, &*world.backend, world.simulated_seconds, &SensorEnvironment::default(), &mut |_| true)
                .unwrap();
            let wait = wgpu::PollType::Wait { submission_index: None, timeout: Some(Duration::from_secs(5)) };
            engine.device().unwrap().poll(wait).unwrap();
            rig.collect(0, &mut |_, r| {
                if let SensorReading::Measurement(m) = r {
                    reading = Some(m.clone());
                }
                true
            });
        }
        let m = reading.expect("radar reading");
        // Expect 3 targets: default (~5.1 m), strong (~5.1 m), static reflector (~8.0 m). Stealth at 3 m is absent.
        assert_eq!(m.count, 3, "values: {:?}", m.values);
        let recs: Vec<&[f64]> = m.records().collect();
        assert!(recs.iter().all(|r| r[0] > 4.5), "stealth target at 3 m must be excluded: {recs:?}");
        // Find the two ~5.1 m targets by azimuth sign (+az is left/-Z in sim, -az is right/+Z in sim).
        let (left, right) = if recs[0][1] > recs[1][1] { (recs[0], recs[1]) } else { (recs[1], recs[0]) };
        let power_diff_db = right[4] - left[4];
        assert!(
            (power_diff_db - 10.0).abs() < 0.25,
            "10 m² vs 1 m² at equal distance should differ by +10 dB, got {power_diff_db:.2} dB ({recs:?})"
        );
        assert!((recs[2][0] - (64.0 + 12.25f64).sqrt()).abs() < 0.1, "static reflector at ~8.73 m: {:?}", recs[2]);
    }

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn camera_physical_optics_renders_on_gpu() {
        let engine = Engine::new_gpu().expect("GPU adapter");
        let mut world = PhysicsWorld::with_backend(Box::new(MollaBackend::default()));
        let wall = world.insert_body(BodyDesc::fixed());
        world
            .insert_collider(
                ColliderDesc::new(Shape::Cuboid { half_extents: DVec3::new(0.5, 2.0, 5.0) })
                    .parent(wall)
                    .translation(DVec3::new(5.0, 1.0, 0.0)),
            )
            .unwrap();
        let carrier = world.insert_body(BodyDesc::fixed().pose(Pose::from_translation(DVec3::Y)));
        let spec = SensorSpec {
            rate_hz: 50.0,
            vfov: 60f32.to_radians(),
            range_m: 20.0,
            width: 32,
            height: 24,
            render: crate::links::CameraRender::Geometry,
            color: true,
            depth: true,
            lens_diameter_m: 0.08,
            focal_distance_m: 4.5,
            aperture_samples: 4,
            ..SensorSpec::defaults(SensorKind::Camera, false)
        };
        let mount = ResolvedMount {
            link: 1,
            name: "dof_cam".into(),
            spec,
            body: carrier,
            entity: None,
            joint: None,
            local: MTransform::new(MVec3::ZERO, MQuat::from_rotation_x(-FRAC_PI_2)),
        };
        let mut shared = None;
        let model = world.backend.molla(|scene| scene.model_stamp());
        let shared = shared_scene(&engine, &mut shared, &*world.backend, &model, &|_| None).unwrap();
        let mut rig = Rig::build(&engine, &*world.backend, vec![mount], 0, 0.0, shared, &|_| None, &|_| None, &[]).unwrap();

        let (mut color, mut depth) = (None, None);
        for _ in 0..4 {
            world.step();
            rig.sample(&engine, &*world.backend, world.simulated_seconds, &SensorEnvironment::default(), &mut |_| true)
                .unwrap();
            let wait = wgpu::PollType::Wait { submission_index: None, timeout: Some(Duration::from_secs(5)) };
            engine.device().unwrap().poll(wait).unwrap();
            rig.collect(0, &mut |_, r| {
                if let SensorReading::Camera(frame) = r {
                    if frame.channel == CAMERA_DEPTH {
                        depth = Some(frame.depths());
                    } else {
                        color = Some(frame.colors());
                    }
                }
                true
            });
        }
        let color = color.expect("DOF color frame");
        let depth = depth.expect("DOF depth frame");
        let center = 12 * 32 + 16;
        assert!((depth[center] - 4.5).abs() < 0.05, "center depth {}", depth[center]);
        assert_ne!(color[center] & 0xff_ff_ff, 0);
    }
}
