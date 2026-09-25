//! Simulated sensors on authored `role = "sensor"` links: Molla GPU IMU and
//! LiDAR sampled from the Molla physics scene at their own simulated-time
//! rates and streamed on `/machines/<machine_id>/sensors/<link>`.
//!
//! The rig of a machine snapshots the physics scene's geometry once per scene
//! revision and copies body poses/velocities at each sample; GPU results are
//! collected without blocking on later frames. The physics lock is held only
//! for those host copies, never across GPU waits.

use std::collections::{HashMap, HashSet};
use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::sync::Arc;
use std::time::Duration;

use bevy::camera::primitives::Aabb;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::render::renderer::{RenderAdapterInfo, RenderDevice, RenderQueue};
use gearbox_api::datapod::robot::Imu;
use gearbox_api::datapod::{Acceleration, Quaternion, Velocity};
use gearbox_api::{
    CAMERA_COLOR, CAMERA_DEPTH, CameraFrame, GearboxBus, LIDAR_CHUNK_PULSES, LidarScan, Props,
    measurement_kind,
};
use molla_compute::{AdapterInfo, Engine};
use molla_core::BodyId as MollaBodyId;
use molla_math::{Mat3 as MMat3, Quat as MQuat, Transform as MTransform, Vec3 as MVec3};
use molla_sensors::readback::{ReadbackConfig, ReadbackRing, ReadbackStamp, ReadbackTicket};
use molla_sensors::{
    CameraChannels, ImuMount, ImuSensor, LidarBatch, LidarPattern, LidarScan as MollaLidarScan,
    ScanMetadata, ScheduledSample, SensorScene, SensorSceneOptions, SensorSchedule,
    SensorScheduler, TiledCamera, TiledCameraConfig, UnsupportedGeometryPolicy,
};
use molla_sim::runtime::{ColliderGeometry, RigidScene};
use molla_sim::{Model, State};
use molla_storage::{Buffer, BufferUsage};
use usd_bevy::UsdPrimRef;

use crate::controller::{ControllerInventory, MachineAgentKeys, find_prim_entity};
use crate::links::{LinkSpec, LinkTree, SensorKind, SensorSpec};
use crate::sensor_radio::RadioLink;
use crate::sensor_generic::{
    GenericSample, GenericSensors, SensorEnvironment, is_generic, measurement, recognition,
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
                run_machine_sensors.after(bevy::transform::TransformSystems::Propagate),
            );
    }
}

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

impl VisualColors<'_, '_> {
    pub fn color(&self, entity: Entity) -> Option<[f32; 3]> {
        let mut root = entity;
        for _ in 0..VISUAL_SEARCH_LEVELS {
            if let Some(color) = self.subtree_color(root) {
                return Some(color);
            }
            root = self.parents.get(root).ok()?.parent();
        }
        None
    }

    fn subtree_color(&self, root: Entity) -> Option<[f32; 3]> {
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
                && let Some(material) = self.materials.get(&material.0)
            {
                let w = aabb.map_or(1.0, |b| {
                    let h = b.half_extents;
                    (h.x * h.y + h.y * h.z + h.z * h.x).max(1e-6)
                });
                let color = self.material_color(material);
                for i in 0..3 {
                    sum[i] += color[i] * w;
                }
                weight += w;
            }
            if let Ok(children) = self.children.get(entity) {
                stack.extend(children.iter());
            }
        }
        (weight > 0.0).then(|| sum.map(|s| s / weight))
    }

    fn material_color(&self, material: &StandardMaterial) -> [f32; 3] {
        let base = material.base_color.to_srgba();
        let texel = material
            .base_color_texture
            .as_ref()
            .and_then(|texture| self.images.get(texture))
            .and_then(mean_texel)
            .unwrap_or([1.0; 3]);
        [base.red * texel[0], base.green * texel[1], base.blue * texel[2]]
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
    warned_backend: bool,
}

impl MachineSensorRigs {
    pub fn has_rig(&self, machine_id: &str) -> bool {
        self.rigs.contains_key(machine_id)
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
}

struct Mount {
    resolved: ResolvedMount,
    name: String,
    body: MollaBodyId,
    published: u64,
    undelivered: u64,
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
    signature: Vec<ResolvedMount>,
    scene: SensorScene,
    imu_model: Model,
    state: State,
    scheduler: SensorScheduler,
    mounts: Vec<Mount>,
    imu: Option<ImuSensor>,
    imu_mounts: Vec<usize>,
    last_imu_time: Option<f64>,
    lidar: Option<LidarBatch>,
    cameras: Vec<(usize, TiledCamera)>,
    ring: ReadbackRing,
    pending: Vec<Pending>,
    render_requests: Vec<RenderRequest>,
    generic: GenericSensors,
    /// Generic readings awaiting publication.
    ready: Vec<(usize, gearbox_api::Measurement)>,
    /// Recognition object of each scene shape, and each object's label.
    objects: Vec<Option<u32>>,
    object_labels: Vec<String>,
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
    Imu { due: Vec<(usize, MQuat)> },
    Lidar { scans: Vec<PendingScan> },
    Camera { mount: usize, sample: u64 },
}

struct PendingScan {
    mount: usize,
    sample: u64,
    ray_offset: usize,
    ray_count: usize,
    meta: ScanMetadata,
    world: MTransform,
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
    parents: Query<&ChildOf>,
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
                let raw: &wgpu::AdapterInfo = &info.0;
                AdapterInfo::from_wgpu(raw.clone())
            });
            rigs.engine = Some(Engine::from_device(
                Arc::new(device.wgpu_device().clone()),
                Arc::new((**queue.0).clone()),
                info,
            ));
        }
        let engine = rigs.engine.as_ref().expect("sensor engine");

        let mut revision = None;
        physics.with_molla_scene(&mut |scene| revision = Some(scene.revision()));
        let Some(revision) = revision else {
            if !rigs.warned_backend {
                warn!(
                    "gearbox-sensors: `{}` authors sensor links, but only the molla physics backend samples them",
                    machine.id
                );
                rigs.warned_backend = true;
            }
            continue;
        };
        let resolved = match resolve_mounts(
            &physics,
            &machine.links,
            scene_root,
            &prims,
            &parents,
            &transforms,
        ) {
            Ok(resolved) => resolved,
            Err(error) => {
                if rigs.failed.insert(machine.id.clone(), revision) != Some(revision) {
                    error!("gearbox-sensors: `{}`: {error}", machine.id);
                }
                continue;
            }
        };
        let stale = rigs.rigs.get(&machine.id).is_none_or(|rig| {
            rig.revision != revision || mounts_changed(&rig.signature, &resolved)
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
                .filter_map(|prim| find_prim_entity(scene_root, prim, &prims, &parents))
                .filter_map(|entity| physics.entity_to_body.get(&entity).copied())
                .collect();
            // `<machine>:<body prim>` for machine parts, the body prim otherwise.
            let label = |entity: Entity| {
                let lineage = || std::iter::once(entity).chain(parents.iter_ancestors(entity));
                let body = lineage().find(|e| physics.entity_to_body.contains_key(e)).unwrap_or(entity);
                let path = prims.get(body).ok()?.1.path.to_string();
                let machine = lineage().find_map(|e| {
                    inventory.machines.iter().find(|m| m.scene_root == Some(e)).map(|m| m.id.as_str())
                });
                Some(machine.map_or_else(|| path.clone(), |id| format!("{id}:{path}")))
            };
            match Rig::build(
                engine,
                &**physics,
                resolved,
                revision,
                sim_time,
                &|entity| visuals.color(entity),
                &label,
                &own,
            ) {
                Ok(rig) => {
                    info!(
                        "gearbox-sensors: `{}` samples {} sensor link(s) on scene revision {revision}",
                        machine.id,
                        rig.mounts.len()
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
            if let Err(error) = rig.sample(engine, &**physics, sim_time, &environment) {
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
                    "gearbox-sensors: `{}`: {} samples, {:.3} ms sample + {:.3} ms collect per sample, {:.1} KB up, {:.1} KB down per sample, {} frames, {} dropped in the last {:.0} s",
                    machine.id,
                    s.samples,
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

/// Every authored sensor link of a machine with its body and mount pose.
fn resolve_mounts(
    physics: &PhysicsWorld,
    links: &LinkTree,
    scene_root: Entity,
    prims: &Query<(Entity, &UsdPrimRef)>,
    parents: &Query<&ChildOf>,
    transforms: &Query<&GlobalTransform>,
) -> Result<Vec<ResolvedMount>, String> {
    let mut mounts = Vec::new();
    for (index, link) in links.links.iter().enumerate() {
        let Some(spec) = link.sensor else {
            continue;
        };
        let body_prim = rigid_body_prim(links, link)
            .ok_or_else(|| format!("sensor link `{}` has no rigid-body ancestor", link.name))?;
        let body_entity = find_prim_entity(scene_root, body_prim, prims, parents)
            .ok_or_else(|| format!("sensor link `{}`: no entity for `{body_prim}`", link.name))?;
        let sensor_entity = find_prim_entity(scene_root, &link.prim_path, prims, parents)
            .ok_or_else(|| format!("sensor link `{}` has no entity", link.name))?;
        let body = *physics
            .entity_to_body
            .get(&body_entity)
            .ok_or_else(|| format!("sensor link `{}`: `{body_prim}` has no body", link.name))?;
        let (Ok(body_gt), Ok(sensor_gt)) =
            (transforms.get(body_entity), transforms.get(sensor_entity))
        else {
            return Err(format!("sensor link `{}` has no transforms yet", link.name));
        };
        let local = body_gt.affine().inverse() * sensor_gt.affine();
        let (_, rotation, translation) = local.to_scale_rotation_translation();
        if !rotation.is_finite() || !translation.is_finite() {
            return Err(format!(
                "sensor link `{}` has a non-finite mount",
                link.name
            ));
        }
        let joint = (spec.kind == SensorKind::Position)
            .then(|| {
                let body_link = links.links.iter().find(|l| l.body_prim.as_deref() == Some(body_prim))?;
                let parent_link = links.get(body_link.parent.as_deref()?)?;
                let parent_prim = rigid_body_prim(links, parent_link)?;
                let parent_entity = find_prim_entity(scene_root, parent_prim, prims, parents)?;
                let parent_body = *physics.entity_to_body.get(&parent_entity)?;
                physics.joint_between(parent_body, body)
            })
            .flatten();
        if spec.kind == SensorKind::Position && joint.is_none() {
            return Err(format!("position sensor `{}` has no joint above it", link.name));
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
    Ok(mounts)
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

impl Rig {
    /// Snapshots the Molla scene and creates the GPU sensors of `mounts`;
    /// `colors` gives a collider entity's display colour for the cameras,
    /// `labels` its object name for recognition, and `own_bodies` the
    /// machine's bodies, which its radars and recognition ignore.
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        engine: &Engine,
        physics: &dyn PhysicsBackend,
        mounts: Vec<ResolvedMount>,
        revision: u64,
        sim_time: f64,
        colors: &dyn Fn(Entity) -> Option<[f32; 3]>,
        labels: &dyn Fn(Entity) -> Option<String>,
        own_bodies: &[BodyId],
    ) -> Result<Self, String> {
        let device = engine.device().ok_or("sensor engine has no device")?;
        let queue = engine.queue().ok_or("sensor engine has no queue")?;
        let handles = mounts
            .iter()
            .map(|m| {
                physics
                    .molla_body_handle(m.body)
                    .ok_or_else(|| format!("sensor link `{}`: body is not a molla body", m.name))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut snapshot = None;
        physics.with_molla_scene(&mut |scene| {
            snapshot = Some(snapshot_scene(engine, scene, &handles, colors));
        });
        let (scene, imu_model, mut state, bodies) =
            snapshot.ok_or("physics backend is not molla")??;
        let own: HashSet<u64> = own_bodies
            .iter()
            .filter_map(|&b| physics.molla_body_handle(b))
            .map(|h| h.to_bits())
            .collect();
        let (mut joints, mut targets) = (HashMap::new(), Vec::new());
        let recognition = mounts.iter().any(|m| m.spec.kind == SensorKind::Camera && m.spec.recognition);
        let (mut objects, mut object_labels) = (Vec::new(), Vec::new());
        physics.with_molla_scene(&mut |rigid| {
            if recognition {
                (objects, object_labels) = shape_objects(rigid, &own, labels);
            }
            for (i, mount) in mounts.iter().enumerate() {
                let Some(joint) = mount.joint else { continue };
                if let Some(index) = rigid
                    .joints()
                    .find(|(h, _)| h.to_bits() == joint.0)
                    .and_then(|(h, _)| rigid.joint_index(h))
                {
                    joints.insert(i, index.index());
                }
            }
            targets = rigid
                .bodies()
                .filter(|(h, desc)| {
                    desc.kind == molla_sim::runtime::BodyKind::Dynamic && !own.contains(&h.to_bits())
                })
                .filter_map(|(h, _)| rigid.body_index(h))
                .collect();
        });
        let specs: Vec<SensorSpec> = mounts.iter().map(|m| m.spec).collect();
        let generic = GenericSensors::build(&specs, joints, targets)?;
        state
            .upload_to_device(device, queue)
            .map_err(|e| format!("sensor state upload: {e}"))?;
        if !scene.unsupported_shapes().is_empty() {
            warn!(
                "gearbox-sensors: {} collider shape(s) are invisible to sensors: {:?}",
                scene.unsupported_shapes().len(),
                scene
                    .unsupported_shapes()
                    .iter()
                    .map(|s| s.kind)
                    .collect::<HashSet<_>>()
            );
        }

        let signature = mounts.clone();
        let mounts: Vec<Mount> = mounts
            .into_iter()
            .zip(bodies)
            .map(|(resolved, body)| Mount {
                name: resolved.name.clone(),
                resolved,
                body,
                published: 0,
                undelivered: 0,
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
            Some(ImuSensor::new(engine, &imu_model, &descs).map_err(|e| format!("imu: {e}"))?)
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
            Some(
                LidarBatch::new(engine, lidars.len(), lidar_rays)
                    .map_err(|e| format!("lidar: {e}"))?,
            )
        };
        let mut cameras = Vec::new();
        let mut camera_bytes = 0u64;
        for (i, mount) in mounts.iter().enumerate() {
            let spec = mount.resolved.spec;
            let traced = camera_channels(&spec);
            if spec.kind != SensorKind::Camera || traced.is_empty() {
                continue;
            }
            let config = TiledCameraConfig {
                width: spec.width,
                height: spec.height,
                fov_y_radians: spec.vfov,
                channels: CameraChannels(traced.iter().fold(0, |all, c| all | c.0)),
                max_distance: spec.range_m,
                ..Default::default()
            };
            let camera = TiledCamera::new(engine, config, 1)
                .map_err(|e| format!("camera `{}`: {e}", mount.name))?;
            let texel: u64 = traced
                .iter()
                .map(|&c| if c == CameraChannels::DEPTH { 8 } else { 4 })
                .sum();
            camera_bytes = camera_bytes.max(u64::from(spec.width) * u64::from(spec.height) * texel);
            cameras.push((i, camera));
        }
        let bytes_per_slot = (imu_mounts.len() as u64 * 32)
            .max(lidar_rays as u64 * 24)
            .max(camera_bytes)
            .max(8);
        let slots = 2 * (1 + lidars.len() + cameras.len()) + 2;
        let ring = ReadbackRing::new(
            engine,
            ReadbackConfig {
                slots,
                bytes_per_slot,
                max_total_bytes: bytes_per_slot * slots as u64,
            },
        )
        .map_err(|e| format!("sensor readback: {e}"))?;
        Ok(Self {
            revision,
            signature,
            scene,
            imu_model,
            state,
            scheduler,
            mounts,
            imu,
            imu_mounts,
            last_imu_time: None,
            lidar,
            cameras,
            ring,
            pending: Vec::new(),
            render_requests: Vec::new(),
            generic,
            ready: Vec::new(),
            objects,
            object_labels,
            stats: RigStats::default(),
            last_report: 0.0,
        })
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

    /// Issues every sample due at `sim_time` without waiting on the GPU
    /// (generic ray sensors cast synchronously).
    pub fn sample(
        &mut self,
        engine: &Engine,
        physics: &dyn PhysicsBackend,
        sim_time: f64,
        environment: &SensorEnvironment,
    ) -> Result<(), String> {
        let due: Vec<ScheduledSample> = self
            .scheduler
            .advance(Duration::from_secs_f64(sim_time))
            .map_err(|e| e.to_string())?
            .to_vec();
        if due.is_empty() {
            return Ok(());
        }
        let mut copied = Err("physics backend is not molla".to_string());
        let state = &mut self.state;
        physics.with_molla_scene(&mut |scene| copied = copy_state(scene.state(), state));
        copied?;
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

        let imu_due: Vec<(usize, MQuat)> = due
            .iter()
            .map(|s| s.sensor_id as usize)
            .filter(|&i| self.mounts[i].resolved.spec.kind.uses_imu())
            .map(|i| (i, world_pose(&self.mounts[i]).rotation))
            .collect();
        if let (Some(imu), false) = (self.imu.as_mut(), imu_due.is_empty()) {
            let fastest = self
                .imu_mounts
                .iter()
                .map(|&i| self.mounts[i].resolved.spec.rate_hz)
                .fold(f32::MIN, f32::max);
            let dt = self
                .last_imu_time
                .map(|t| sim_time - t)
                .filter(|dt| *dt > 0.0)
                .unwrap_or(1.0 / fastest as f64);
            self.last_imu_time = Some(sim_time);
            let readings = imu
                .dispatch(engine, &self.imu_model, &self.state, dt)
                .map_err(|e| format!("imu dispatch: {e}"))?;
            if let Some(readings) = readings {
                let n = self.imu_mounts.len();
                let sources = [
                    readings.accelerometer.readback_source(0..n),
                    readings.gyroscope.readback_source(0..n),
                ]
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
                let stamp = ReadbackStamp {
                    sensor_id: u64::MAX,
                    sample_id: due[0].sample_id,
                    scene_revision: self.revision,
                    simulation_time: Some(sim_time),
                };
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
        }

        let lidar_due: Vec<(usize, u64)> = due
            .iter()
            .map(|s| (s.sensor_id as usize, s.sample_id))
            .filter(|(i, _)| self.mounts[*i].resolved.spec.kind == SensorKind::Lidar)
            .collect();
        if let (Some(lidar), false) = (self.lidar.as_mut(), lidar_due.is_empty()) {
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
            let view = lidar
                .dispatch_scene(engine, &self.scene, &self.state, &scans)
                .map_err(|e| format!("lidar dispatch: {e}"))?;
            let n = view.hits.ray_count;
            let sources = [
                view.hits.point.readback_source(0..n),
                view.hits.distance.readback_source(0..n),
                view.hits.shape.readback_source(0..n),
            ]
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
            let pending_scans: Vec<PendingScan> = view
                .scans
                .iter()
                .zip(&lidar_due)
                .zip(worlds)
                .map(|((range, &(mount, sample)), world)| PendingScan {
                    mount,
                    sample,
                    ray_offset: range.ray_offset as usize,
                    ray_count: range.ray_count as usize,
                    meta: match range.scan.pattern {
                        LidarPattern::Terrestrial(meta) => meta,
                        LidarPattern::Aerial(_) => unreachable!("terrestrial scans only"),
                    },
                    world,
                })
                .collect();
            let stamp = ReadbackStamp {
                sensor_id: lidar_due[0].0 as u64,
                sample_id: lidar_due[0].1,
                scene_revision: self.revision,
                simulation_time: Some(sim_time),
            };
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

        let camera_due: Vec<(usize, u64)> = due
            .iter()
            .map(|s| (s.sensor_id as usize, s.sample_id))
            .filter(|(i, _)| self.mounts[*i].resolved.spec.kind == SensorKind::Camera)
            .collect();
        for (mount, sample) in camera_due {
            let spec = self.mounts[mount].resolved.spec;
            if spec.color && spec.render.is_bevy() {
                self.render_requests.push(RenderRequest {
                    mount,
                    sample,
                    sim_time,
                });
            }
            let Some(camera) = self
                .cameras
                .iter_mut()
                .find(|(m, _)| *m == mount)
                .map(|(_, camera)| camera)
            else {
                continue;
            };
            let world = world_pose(&self.mounts[mount]);
            let pose = MTransform::new(
                world.position,
                (world.rotation * camera_in_link()).normalize(),
            );
            camera
                .set_camera_pose(0, pose)
                .map_err(|e| format!("camera pose: {e}"))?;
            camera
                .dispatch_scene(engine, &self.scene, &self.state)
                .map_err(|e| format!("camera dispatch: {e}"))?;
            let pixels = spec.width as usize * spec.height as usize;
            let sources = camera_channels(&spec)
                .into_iter()
                .map(|channel| camera.readback_source(channel, 0..pixels))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            let stamp = ReadbackStamp {
                sensor_id: mount as u64,
                sample_id: sample,
                scene_revision: self.revision,
                simulation_time: Some(sim_time),
            };
            match self
                .ring
                .try_enqueue(&sources, stamp)
                .map_err(|e| e.to_string())?
            {
                Some(ticket) => self.pending.push(Pending {
                    ticket,
                    sim_time,
                    kind: PendingKind::Camera { mount, sample },
                }),
                None => self.stats.dropped += 1,
            }
        }

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
        if !generic_due.is_empty() {
            let generic = &mut self.generic;
            let mut result = None;
            physics.with_molla_scene(&mut |rigid| {
                result = Some(generic.sample(rigid, environment, &generic_due, sim_time));
            });
            let readings = result.ok_or("physics backend is not molla")??;
            self.ready.extend(readings);
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
                PendingKind::Camera { mount, sample } => {
                    let m = &mut self.mounts[mount];
                    let spec = m.resolved.spec;
                    let pixels = (spec.width * spec.height) as usize;
                    let color = molla_color(&spec);
                    let traced = camera_channels(&spec);
                    let slot = |c: CameraChannels| traced.iter().position(|&t| t == c);
                    let colors: Vec<u32> = slot(CameraChannels::COLOR)
                        .map_or_else(Vec::new, |i| frame.channel(i).unwrap_or_default());
                    let depths: Vec<f64> = slot(CameraChannels::DEPTH)
                        .map_or_else(Vec::new, |i| frame.channel(i).unwrap_or_default());
                    let shapes: Vec<u32> = slot(CameraChannels::SHAPE_INDEX)
                        .map_or_else(Vec::new, |i| frame.channel(i).unwrap_or_default());
                    if (color && colors.len() != pixels)
                        || (spec.depth && depths.len() != pixels)
                        || (spec.recognition && shapes.len() != pixels)
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
                    if color {
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
                        let objects = &self.objects;
                        let seen = molla_sensors::recognize(
                            spec.width,
                            spec.height,
                            spec.vfov as f64,
                            &depths,
                            &shapes,
                            &|shape| objects.get(shape as usize).copied().flatten().map(u64::from),
                            1,
                        );
                        let mut reading = recognition(&name, link_index, pending.sim_time, sample, &seen, &self.object_labels);
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
                        let mut delivered = true;
                        for (chunk, chunk_ranges) in ranges.chunks(LIDAR_CHUNK_PULSES).enumerate() {
                            let offset = chunk * LIDAR_CHUNK_PULSES;
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
                                props: Props::from_pairs(&[("name", mount.name.as_str())])
                                    .into_bytes(),
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
) -> (Vec<Option<u32>>, Vec<String>) {
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
    (shapes, labels)
}

/// Geometry, the IMU's body model and a state copy, taken under the physics lock.
fn snapshot_scene(
    engine: &Engine,
    scene: &RigidScene,
    handles: &[molla_sim::runtime::BodyHandle],
    colors: &dyn Fn(Entity) -> Option<[f32; 3]>,
) -> Result<(SensorScene, Model, State, Vec<MollaBodyId>), String> {
    let bodies = handles
        .iter()
        .map(|&h| scene.body_index(h).ok_or("sensor body left the scene"))
        .collect::<Result<Vec<_>, _>>()?;
    let source = scene.model();
    let mut sensor_scene = SensorScene::from_model(
        engine,
        source,
        SensorSceneOptions {
            unsupported: UnsupportedGeometryPolicy::ReportAndSkip,
            ..Default::default()
        },
    )
    .map_err(|e| format!("sensor scene: {e}"))?;
    sensor_scene
        .set_shape_colors(engine, &shape_colors(scene, colors))
        .map_err(|e| format!("sensor scene colours: {e}"))?;
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
    Ok((sensor_scene, imu_model, state, bodies))
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
            kind: SensorKind::Lidar,
            rate_hz: 10.0,
            columns: 4,
            rows: 1,
            hfov: std::f32::consts::TAU,
            vfov: 0.0,
            range_m: 50.0,
            width: 128,
            height: 96,
            render: crate::links::CameraRender::Geometry,
            color: true,
            depth: true,
            recognition: false,
            variant: crate::links::SensorVariant::Default,
            rays: 1,
            aperture: 0.0,
            min_range_m: 1.0,
            channel: 0,
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
            kind: SensorKind::Lidar,
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
            variant: crate::links::SensorVariant::Default,
            rays: 1,
            aperture: 0.0,
            min_range_m: 1.0,
            channel: 0,
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
        let mut rig =
            Rig::build(&engine, &*world.backend, mounts, revision, 0.0, &|_| None, &label, &[]).unwrap();
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
                    sources: vec![sun.clone()],
                    sky: 50.0,
                    up: MVec3::Y,
                },
                contacts: crate::sensor_generic::contact_forces(&*world.backend),
            };
            rig.sample(&engine, &*world.backend, world.simulated_seconds, &environment)
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
                        imu = Some(reading.clone());
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
    }
}
