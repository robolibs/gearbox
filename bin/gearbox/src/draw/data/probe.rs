use std::io::Read;
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use bevy::camera::{RenderTarget, visibility::RenderLayers};
use bevy::prelude::*;
use bevy::render::{
    render_resource::{TextureFormat, TextureUsages},
    view::{
        RetainedViewEntity,
        screenshot::{Screenshot, save_to_disk},
    },
};
use gearbox_api::draw_data::{GpuCharge, RenderCompletion};
use gearbox_api::wire::draw_data::{DataKey, ViewIdentity};
use mara::ui::modules::bevy::ChaseCamera;
use serde::{Deserialize, Serialize};

use super::worker::{Dataset, Page};

const NAMESPACE: &str = "draw-multiview-probe";

#[path = "probe_cpu.rs"]
mod cpu;

#[path = "probe_bench.rs"]
mod bench;

#[path = "probe_motion.rs"]
mod motion;

#[derive(Component)]
pub(super) struct ProbeCamera;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    Configure,
    Capture,
    Remove,
    Recreate,
    BeginMotion,
    EndMotion,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct Control {
    sequence: u64,
    action: Action,
    active: bool,
    max_generation: u64,
    #[serde(default)]
    gpu_headroom: Option<u64>,
    #[serde(default)]
    cpu_headroom: Option<u64>,
    #[serde(default)]
    pause_receipts: bool,
    #[serde(default)]
    hold_full: bool,
    #[serde(default)]
    secondary_size: Option<[u32; 2]>,
    #[serde(default)]
    secondary_east_m: Option<f32>,
}

#[derive(Clone, Copy, Serialize)]
struct Stamp {
    view: ViewIdentity,
    incarnation: u64,
    generation: u64,
    frame: u64,
    source_samples: u64,
    selected_items: u64,
}

#[derive(Clone, Copy, Default, Serialize)]
struct Report {
    sequence: u64,
    views: [Option<ViewIdentity>; 2],
    samples: [u32; 2],
    completed: [Option<Stamp>; 2],
    delayed_pages: u64,
    secondary_alive: bool,
    active: bool,
    gpu_reserved: u64,
    cpu: cpu::Report,
    receipts: ReceiptReport,
    scheduler: SchedulerReport,
    allocations: Allocations,
    uploads: Uploads,
    legacy: LegacyReport,
    backdrops: [Backdrop; 2],
    lod: [Option<LodReport>; 2],
    camera_views: [Option<u64>; 2],
    motion: motion::Audit,
}

#[derive(Clone, Copy, Default, Serialize)]
pub(super) struct LodReport {
    pub valid: u64,
    pub represented: u64,
    pub selected: u64,
    pub culled: u64,
    /// Full pages whose LOD threshold the point budget raised above the view's error.
    pub budget_limited_pages: u64,
    pub max_error_px: Option<f64>,
    pub pages: u64,
    pub preparing: bool,
    pub coarse: bool,
    pub full_pages: u64,
    pub coarse_pages: u64,
    pub source_page_bounds: Option<[u64; 2]>,
    pub selection_view: u64,
    pub verified_ordinals: bool,
}

#[derive(Clone, Copy, Default, Serialize)]
pub(super) struct ReceiptReport {
    pub frame: u64,
    pub paused: bool,
    pub in_flight: usize,
}

#[derive(Clone, Copy, Default, Serialize)]
struct SchedulerReport {
    frame: u64,
    groups: [Option<GroupUpload>; 8],
    overflow: u64,
}

#[derive(Clone, Copy, Serialize)]
struct GroupUpload {
    typed: bool,
    selection: bool,
    epoch: u64,
    id: u64,
    bytes: u64,
    first_frame: u64,
    last_frame: u64,
}

#[derive(Clone, Copy, Default, Serialize)]
pub(super) struct Backdrop {
    pub bytes: u64,
    pub size: [u32; 2],
    pub stride: u32,
}

#[derive(Clone, Copy, Default, Serialize)]
pub(super) struct LegacyReport {
    pub frame: u64,
    pub points: u64,
    pub pages: u64,
    pub ready: u64,
    pub queued: [u64; 2],
    pub positions: u64,
    pub attributes: u64,
    pub uniforms: u64,
    pub gpu_bytes: u64,
    pub auxiliary_gpu_bytes: u64,
    pub staging_gpu_bytes: u64,
    pub retiring_gpu_bytes: u64,
    pub other_gpu_bytes: u64,
    pub charged_cpu_bytes: u64,
    pub uploaded: [u64; 3],
    pub frame_upload: u64,
    pub max_frame_upload: u64,
}

#[derive(Clone, Copy, Default, Serialize)]
struct Uploads {
    positions: u64,
    attributes: u64,
    coarse_positions: u64,
    coarse_attributes: u64,
    coarse_indices: u64,
    styles: u64,
    placements: u64,
    indices: u64,
}

pub(super) enum UploadKind {
    Positions,
    Attributes,
    CoarsePositions,
    CoarseAttributes,
    CoarseIndices,
    Styles,
    Placements,
    Indices,
}

#[cfg(test)]
#[test]
fn allocation_snapshots_preserve_per_kind_upload_totals() {
    let probe = Probe {
        cpu: Default::default(),
        pause_receipts: AtomicBool::new(false),
        hold_full: AtomicBool::new(false),
        secondary: AtomicU64::new(u64::MAX),
        max_generation: AtomicU64::new(0),
        delayed: AtomicU64::new(0),
        report: Mutex::new(Report::default()),
    };
    for (kind, size) in [
        (UploadKind::Positions, 16),
        (UploadKind::Attributes, 32),
        (UploadKind::Styles, 64),
        (UploadKind::Placements, 128),
    ] {
        probe.uploaded(kind, size);
    }
    probe.allocations(Allocations::default());
    probe.uploaded(UploadKind::Placements, 128);
    let report = probe.report.lock().unwrap();
    assert_eq!(report.uploads.positions, 16);
    assert_eq!(report.uploads.attributes, 32);
    assert_eq!(report.uploads.styles, 64);
    assert_eq!(report.uploads.placements, 256);
}

#[derive(Clone, Copy, Default, Serialize)]
pub(super) struct Allocations {
    pub demanded_position_bytes: u64,
    pub page_loads: u64,
    pub evicted_pages: u64,
    pub indices: u64,
    pub lod_views: u64,
    pub selected_points: u64,
    pub positions: u64,
    pub attributes: u64,
    pub styles: u64,
    pub coarse_positions: u64,
    pub coarse_attributes: u64,
    pub coarse_pages: u64,
    pub coarse_points: u64,
    pub placements: u64,
    pub points: u64,
    pub pages: u64,
    pub gpu_bytes: u64,
    pub auxiliary_gpu_bytes: u64,
    pub staging_gpu_bytes: u64,
    pub retiring_gpu_bytes: u64,
    pub other_gpu_bytes: u64,
    pub gpu_peak: u64,
    pub uploaded: u64,
    pub frame_upload: u64,
    pub max_frame_upload: u64,
}

#[cfg(test)]
#[test]
fn legacy_snapshots_keep_distinct_upload_totals_and_frame_progress() {
    let probe = Probe {
        cpu: Default::default(),
        pause_receipts: AtomicBool::new(false),
        hold_full: AtomicBool::new(false),
        secondary: AtomicU64::new(u64::MAX),
        max_generation: AtomicU64::new(0),
        delayed: AtomicU64::new(0),
        report: Mutex::new(Report::default()),
    };
    for (index, bytes) in [16, 32, 128].into_iter().enumerate() {
        probe.legacy_uploaded(index, bytes);
    }
    probe.legacy(LegacyReport {
        points: 1,
        frame_upload: 176,
        ..Default::default()
    });
    probe.legacy_uploaded(2, 128);
    let primary = Entity::from_raw_u32(10).unwrap();
    let secondary = Entity::from_raw_u32(11).unwrap();
    probe
        .secondary
        .store(secondary.to_bits(), Ordering::Release);
    probe.legacy_queued(RetainedViewEntity::new(primary.into(), None, 0), 1);
    probe.legacy_queued(RetainedViewEntity::new(secondary.into(), None, 0), 2);
    assert_eq!(probe.report.lock().unwrap().legacy.queued, [1, 2]);
    probe.legacy(LegacyReport {
        points: 1,
        frame_upload: 128,
        ..Default::default()
    });
    assert_eq!(probe.report.lock().unwrap().legacy.queued, [1, 2]);
    probe.begin_legacy_queue();
    let report = probe.report.lock().unwrap();
    assert_eq!(report.legacy.uploaded, [16, 32, 256]);
    assert_eq!(report.legacy.frame, 2);
    assert_eq!(report.legacy.max_frame_upload, 176);
    assert_eq!(report.legacy.queued, [0, 0]);
    assert_eq!(
        report.uploads.positions
            + report.uploads.attributes
            + report.uploads.styles
            + report.uploads.placements,
        0
    );
}

pub(super) struct Probe {
    pub cpu: cpu::Pressure,
    pause_receipts: AtomicBool,
    /// Views draw every page from its coarse tier while set.
    hold_full: AtomicBool,
    secondary: AtomicU64,
    max_generation: AtomicU64,
    delayed: AtomicU64,
    report: Mutex<Report>,
}

impl Probe {
    pub fn camera(&self, view: RetainedViewEntity, stamp: u64) {
        let index =
            usize::from(view.main_entity.id().to_bits() == self.secondary.load(Ordering::Acquire));
        self.report
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .camera_views[index] = Some(stamp);
    }

    pub fn motion_frame(&self, limits: &gearbox_api::wire::draw_data::DataLimits, cpu: u64) {
        let mut report = self.report.lock().unwrap_or_else(|p| p.into_inner());
        let (frame, allocations, cuts, cameras) = (
            report.scheduler.frame,
            report.allocations,
            report.lod,
            report.camera_views,
        );
        report
            .motion
            .observe(frame, limits, cpu, allocations, cuts, cameras);
    }

    pub fn clear_lod(&self) {
        self.report.lock().unwrap_or_else(|p| p.into_inner()).lod = [None; 2];
    }

    pub fn lod(&self, view: RetainedViewEntity, value: LodReport) {
        let index =
            usize::from(view.main_entity.id().to_bits() == self.secondary.load(Ordering::Acquire));
        self.report.lock().unwrap_or_else(|p| p.into_inner()).lod[index] = Some(value);
    }

    pub fn begin_upload_frame(&self) {
        self.report
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .scheduler
            .frame += 1;
    }

    pub fn group_uploaded(&self, key: super::upload::Key, bytes: u64) {
        if bytes == 0 {
            return;
        }
        let (typed, selection, id, epoch) = match key {
            super::upload::Key::Typed(id) => (true, false, id, 0),
            super::upload::Key::Legacy(id) | super::upload::Key::Octree(id) => {
                (false, false, id, 0)
            }
            super::upload::Key::Selection(id, epoch) => (true, true, id, epoch),
        };
        let mut report = self.report.lock().unwrap_or_else(|p| p.into_inner());
        let scheduler = &mut report.scheduler;
        let index = scheduler
            .groups
            .iter()
            .position(|group| {
                group.is_some_and(|group| {
                    group.typed == typed
                        && group.selection == selection
                        && group.id == id
                        && group.epoch == epoch
                })
            })
            .or_else(|| scheduler.groups.iter().position(Option::is_none));
        if let Some(index) = index {
            let group = scheduler.groups[index].get_or_insert(GroupUpload {
                typed,
                selection,
                epoch,
                id,
                bytes: 0,
                first_frame: scheduler.frame,
                last_frame: scheduler.frame,
            });
            group.bytes += bytes;
            group.last_frame = scheduler.frame;
        } else {
            scheduler.overflow += 1;
        }
    }

    #[cfg(test)]
    pub(super) fn for_tests() -> Self {
        Self {
            cpu: Default::default(),
            pause_receipts: AtomicBool::new(false),
            hold_full: AtomicBool::new(false),
            secondary: AtomicU64::new(u64::MAX),
            max_generation: AtomicU64::new(0),
            delayed: AtomicU64::new(0),
            report: Mutex::new(Report::default()),
        }
    }

    pub fn configure_receipts(&self, paused: bool) {
        self.pause_receipts.store(paused, Ordering::Release);
    }

    #[cfg(test)]
    pub(super) fn receipt_snapshot(&self) -> ReceiptReport {
        self.report.lock().unwrap().receipts
    }

    pub fn receipts_paused(&self) -> bool {
        self.pause_receipts.load(Ordering::Acquire)
    }

    pub fn holds_full(&self) -> bool {
        self.hold_full.load(Ordering::Acquire)
    }

    pub fn receipts(&self, value: ReceiptReport) {
        self.report
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .receipts = value;
    }

    pub fn backdrop(&self, view: RetainedViewEntity, value: Backdrop) {
        let index =
            usize::from(view.main_entity.id().to_bits() == self.secondary.load(Ordering::Acquire));
        self.report
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .backdrops[index] = value;
    }
    pub fn legacy_uploaded(&self, index: usize, bytes: u64) {
        let mut report = self.report.lock().unwrap_or_else(|p| p.into_inner());
        report.legacy.uploaded[index] += bytes;
    }

    pub fn legacy(&self, mut value: LegacyReport) {
        let mut report = self.report.lock().unwrap_or_else(|p| p.into_inner());
        value.frame = report.legacy.frame + 1;
        value.uploaded = report.legacy.uploaded;
        value.queued = report.legacy.queued;
        value.max_frame_upload = report.legacy.max_frame_upload.max(value.frame_upload);
        report.legacy = value;
    }

    pub fn legacy_queued(&self, view: RetainedViewEntity, count: usize) {
        let secondary = view.main_entity.id().to_bits() == self.secondary.load(Ordering::Acquire);
        let mut report = self.report.lock().unwrap_or_else(|p| p.into_inner());
        report.legacy.queued[usize::from(secondary)] = count as u64;
    }

    pub fn begin_legacy_queue(&self) {
        self.report
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .legacy
            .queued = [0, 0];
    }

    pub fn uploaded(&self, kind: UploadKind, bytes: usize) {
        let mut report = self.report.lock().unwrap_or_else(|p| p.into_inner());
        let count = match kind {
            UploadKind::Positions => &mut report.uploads.positions,
            UploadKind::Attributes => &mut report.uploads.attributes,
            UploadKind::CoarsePositions => &mut report.uploads.coarse_positions,
            UploadKind::CoarseAttributes => &mut report.uploads.coarse_attributes,
            UploadKind::CoarseIndices => &mut report.uploads.coarse_indices,
            UploadKind::Styles => &mut report.uploads.styles,
            UploadKind::Placements => &mut report.uploads.placements,
            UploadKind::Indices => &mut report.uploads.indices,
        };
        *count += bytes as u64;
    }

    pub fn allocations(&self, mut value: Allocations) {
        let mut report = self.report.lock().unwrap_or_else(|p| p.into_inner());
        value.max_frame_upload = report.allocations.max_frame_upload.max(value.frame_upload);
        report.allocations = value;
    }

    pub fn view(&self, retained: RetainedViewEntity, identity: ViewIdentity, samples: u32) {
        let secondary =
            retained.main_entity.id().to_bits() == self.secondary.load(Ordering::Acquire);
        let index = usize::from(secondary);
        let mut report = self.report.lock().unwrap_or_else(|p| p.into_inner());
        if report.views[index] != Some(identity) {
            report.completed[index] = None;
            report.lod[index] = None;
        }
        report.views[index] = Some(identity);
        report.samples[index] = samples;
    }

    pub fn allow(&self, view: RetainedViewEntity, dataset: &Dataset, page: &Page) -> bool {
        let maximum = self.max_generation.load(Ordering::Acquire);
        let delay = maximum != 0
            && dataset.key().namespace == NAMESPACE
            && dataset.revision.generation > maximum
            && page.appearance.geometry.descriptor().address.page_id == 1
            && view.main_entity.id().to_bits() == self.secondary.load(Ordering::Acquire);
        if delay {
            self.delayed.fetch_add(1, Ordering::Relaxed);
        }
        !delay
    }

    pub fn completed(&self, key: &DataKey, value: RenderCompletion) {
        if key.namespace != NAMESPACE {
            return;
        }
        let mut report = self.report.lock().unwrap_or_else(|p| p.into_inner());
        let Some(index) = report
            .views
            .iter()
            .position(|view| *view == Some(value.target.view))
        else {
            return;
        };
        if report.completed[index].is_some_and(|old| old.frame >= value.frame) {
            return;
        }
        report.completed[index] = Some(Stamp {
            view: value.target.view,
            incarnation: value.target.dataset.incarnation,
            generation: value.target.dataset.generation,
            frame: value.frame,
            source_samples: value.source_samples,
            selected_items: value.selected_items,
        });
        report.motion.completed(
            index,
            value.frame,
            value.source_samples,
            value.selected_items,
        );
    }
}

#[derive(Resource)]
pub(super) struct ProbeState(pub Arc<Probe>);

#[derive(Resource)]
pub(super) struct Driver {
    root: PathBuf,
    camera: Option<Entity>,
    image: Option<Handle<Image>>,
    control: Control,
    poll: Timer,
    gpu_hold: Option<GpuCharge>,
}

pub(super) fn install(app: &mut App) {
    let Ok(root) = std::env::var("GEARBOX_DRAW_RESOURCE_PROBE")
        .or_else(|_| std::env::var("GEARBOX_DRAW_MULTIVIEW_PROBE"))
    else {
        return;
    };
    let root = PathBuf::from(root)
        .canonicalize()
        .expect("probe directory must exist");
    assert!(root.is_dir(), "probe path must be a directory");
    bench::install(app, &root);
    app.insert_resource(ProbeState(Arc::new(Probe {
        cpu: Default::default(),
        pause_receipts: AtomicBool::new(false),
        hold_full: AtomicBool::new(false),
        secondary: AtomicU64::new(u64::MAX),
        max_generation: AtomicU64::new(0),
        delayed: AtomicU64::new(0),
        report: Mutex::new(Report::default()),
    })))
    .insert_resource(Driver {
        root,
        camera: None,
        image: None,
        control: Control {
            sequence: 0,
            action: Action::Remove,
            active: true,
            max_generation: 0,
            gpu_headroom: None,
            cpu_headroom: None,
            pause_receipts: false,
            hold_full: false,
            secondary_size: None,
            secondary_east_m: None,
        },
        poll: Timer::from_seconds(0.1, TimerMode::Repeating),
        gpu_hold: None,
    })
    .add_systems(
        PostUpdate,
        drive.after(bevy::transform::TransformSystems::Propagate),
    );
}

pub(super) fn drive(
    mut commands: Commands,
    mut driver: ResMut<Driver>,
    probe: Res<ProbeState>,
    time: Res<Time>,
    store: Res<gearbox_api::DrawStoreRes>,
    mut images: ResMut<Assets<Image>>,
    primary: Query<
        (&GlobalTransform, &Projection, &RenderTarget, &RenderLayers),
        (With<ChaseCamera>, Without<ProbeCamera>),
    >,
    mut cameras: Query<(&mut Transform, &mut GlobalTransform, &mut Camera), With<ProbeCamera>>,
) {
    let Ok((transform, projection, target, layers)) = primary.single() else {
        return;
    };
    if let Some(entity) = driver.camera
        && let Ok((mut local, mut global, mut camera)) = cameras.get_mut(entity)
    {
        *local = transform.compute_transform();
        local.translation += super::super::enu_to_site().rotation
            * Vec3::X
            * driver.control.secondary_east_m.unwrap_or(0.0);
        *global = GlobalTransform::from(*local);
        camera.is_active = driver.control.active;
    }
    if !driver.poll.tick(time.delta()).just_finished() {
        return;
    }
    let request = std::fs::File::open(driver.root.join("control.json"))
        .ok()
        .and_then(|file| {
            let mut bytes = Vec::new();
            file.take(1025).read_to_end(&mut bytes).ok()?;
            (bytes.len() <= 1024)
                .then(|| serde_json::from_slice::<Control>(&bytes).ok())
                .flatten()
        });
    if let Some(control) = request.filter(|control| {
        control.sequence > driver.control.sequence
            && control
                .secondary_east_m
                .is_none_or(|value| value.is_finite() && value.abs() <= 1_000_000.0)
            && control
                .secondary_size
                .is_none_or(|size| size.into_iter().all(|n| (64..=4096).contains(&n)))
    }) {
        probe.0.cpu.configure(control.cpu_headroom);
        probe.0.configure_receipts(control.pause_receipts);
        probe.0.hold_full.store(control.hold_full, Ordering::Release);
        if control.gpu_headroom != driver.control.gpu_headroom {
            driver.gpu_hold = None;
            if let Some(headroom) = control.gpu_headroom {
                let backend = store.backend();
                let gpu = backend.gpu();
                let bytes = gpu
                    .limit()
                    .saturating_sub(gpu.usage().bytes)
                    .saturating_sub(headroom);
                let Some(hold) = gpu.reserve(bytes) else {
                    return;
                };
                driver.gpu_hold = Some(hold);
            }
        }
        let recreate = matches!(control.action, Action::Remove | Action::Recreate);
        if recreate {
            if let Some(entity) = driver.camera.take() {
                commands.entity(entity).try_despawn();
            }
            driver.image = None;
            probe.0.secondary.store(u64::MAX, Ordering::Release);
            let mut report = probe.0.report.lock().unwrap_or_else(|p| p.into_inner());
            report.views[1] = None;
            report.completed[1] = None;
            report.backdrops[1] = Backdrop::default();
        }
        if !matches!(control.action, Action::Remove) && driver.camera.is_none() {
            let [width, height] = control.secondary_size.unwrap_or([640, 480]);
            let mut image =
                Image::new_target_texture(width, height, TextureFormat::Rgba8UnormSrgb, None);
            image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
            let image = images.add(image);
            let camera = commands
                .spawn((
                    Name::new("Drawing multiview probe"),
                    ProbeCamera,
                    Camera3d::default(),
                    Camera {
                        is_active: control.active,
                        order: -500,
                        ..default()
                    },
                    RenderTarget::Image(image.clone().into()),
                    projection.clone(),
                    layers.clone(),
                    transform.compute_transform(),
                    *transform,
                    Msaa::Off,
                    bevy::render::view::NoIndirectDrawing,
                    bevy::light::cluster::ClusterConfig::None,
                    bevy_weather::WeatherOptOut,
                ))
                .id();
            driver.camera = Some(camera);
            driver.image = Some(image);
            probe.0.secondary.store(camera.to_bits(), Ordering::Release);
        }
        probe
            .0
            .max_generation
            .store(control.max_generation, Ordering::Release);
        driver.control = control;
        if matches!(control.action, Action::BeginMotion | Action::EndMotion) {
            let mut report = probe.0.report.lock().unwrap_or_else(|p| p.into_inner());
            if matches!(control.action, Action::BeginMotion) {
                let sources = report.lod.map(|cut| cut.map_or(0, |cut| cut.valid));
                let frame = report.scheduler.frame;
                report.motion.begin(control.sequence, frame, sources);
            } else {
                report.motion.end();
            }
        }
        if matches!(control.action, Action::Capture) {
            commands
                .spawn(Screenshot(target.clone()))
                .observe(save_to_disk(
                    driver.root.join(format!("{}-a.png", control.sequence)),
                ));
            if let Some(image) = &driver.image {
                commands
                    .spawn(Screenshot(RenderTarget::Image(image.clone().into())))
                    .observe(save_to_disk(
                        driver.root.join(format!("{}-b.png", control.sequence)),
                    ));
            }
        }
    }
    let report = {
        let mut report = probe.0.report.lock().unwrap_or_else(|p| p.into_inner());
        report.sequence = driver.control.sequence;
        report.delayed_pages = probe.0.delayed.load(Ordering::Relaxed);
        report.secondary_alive = driver.camera.is_some();
        report.active = driver.control.active;
        report.gpu_reserved = driver.gpu_hold.as_ref().map_or(0, GpuCharge::bytes);
        report.cpu = probe.0.cpu.report(store.backend().budget());
        *report
    };
    if let Ok(bytes) = serde_json::to_vec(&report) {
        let temporary = driver.root.join("report.tmp");
        if std::fs::write(&temporary, bytes).is_ok() {
            let _ = std::fs::rename(temporary, driver.root.join("report.json"));
        }
    }
}
