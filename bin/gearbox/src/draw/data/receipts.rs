use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
};

use bevy::camera::{ManualTextureViewHandle, RenderTarget, visibility::RenderLayers};
use bevy::core_pipeline::{
    core_3d::main_transparent_pass_3d,
    schedule::{Core3d, Core3dSystems},
};
use bevy::prelude::*;
use bevy::render::{
    Extract, ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems,
    renderer::{RenderQueue, ViewQuery},
    view::{ExtractedView, RetainedViewEntity},
};
use bevy::window::{PrimaryWindow, WindowRef};
use gearbox_api::draw_data::{Charge, DrawingBackend, RenderCompletion, StorageError};
use gearbox_api::wire::draw_data::{
    DatasetKind, FrontierDisposition, ViewIdentity, VisibilityTarget,
};
use mara::ui::modules::bevy::ChaseCamera;

use super::frontier::{Frontiers, Offered};
use super::runtime::{Node, Shared};
use super::worker::{Dataset, charge};

#[cfg(test)]
#[path = "receipt_tests.rs"]
mod tests;

const MAX_VIEWS: usize = 16;
const MAX_IN_FLIGHT: usize = 2;

struct Limiter {
    count: AtomicUsize,
    _charge: Charge,
}

struct Permit(Arc<Limiter>);
impl Permit {
    fn acquire(limiter: &Arc<Limiter>) -> Option<Self> {
        limiter
            .count
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < MAX_IN_FLIGHT).then_some(count + 1)
            })
            .ok()?;
        Some(Self(limiter.clone()))
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.count.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone, Copy)]
pub(super) struct RegisteredView {
    pub retained: RetainedViewEntity,
    surface: ViewSurface,
    pub identity: ViewIdentity,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewSurface {
    Window(Entity),
    Embedded(ManualTextureViewHandle),
    Image(AssetId<Image>),
}

#[derive(Clone, Copy)]
struct ViewCandidate {
    retained: RetainedViewEntity,
    surface: ViewSurface,
    primary: bool,
}

#[derive(Resource)]
pub(super) struct ViewRoster {
    entries: [Option<ViewCandidate>; MAX_VIEWS],
    count: usize,
    overflow: bool,
}

impl Default for ViewRoster {
    fn default() -> Self {
        Self {
            entries: [None; MAX_VIEWS],
            count: 0,
            overflow: false,
        }
    }
}

#[cfg(feature = "draw-probes")]
type CameraQuery = (
    Entity,
    &'static RenderTarget,
    Option<&'static RenderLayers>,
    Has<ChaseCamera>,
    Has<super::probe::ProbeCamera>,
);
#[cfg(not(feature = "draw-probes"))]
type CameraQuery = (
    Entity,
    &'static RenderTarget,
    Option<&'static RenderLayers>,
    Has<ChaseCamera>,
);

fn extract_views(
    mut roster: ResMut<ViewRoster>,
    cameras: Extract<
        Query<CameraQuery, (With<Camera3d>, Without<crate::sensor_cameras::SensorCamera>)>,
    >,
    windows: Extract<Query<(Entity, Has<PrimaryWindow>), With<Window>>>,
) {
    *roster = ViewRoster::default();
    let primary = windows
        .iter()
        .find_map(|(entity, primary)| primary.then_some(entity));
    for camera in &cameras {
        #[cfg(feature = "draw-probes")]
        let (entity, target, layers, chase, probe) = camera;
        #[cfg(not(feature = "draw-probes"))]
        let (entity, target, layers, chase) = camera;
        #[cfg(not(feature = "draw-probes"))]
        let probe = false;
        if !layers
            .is_some_and(|layers| layers.intersects(&RenderLayers::layer(super::super::DRAW_LAYER)))
        {
            continue;
        }
        let surface = match target {
            RenderTarget::Window(window) => {
                let window = match window {
                    WindowRef::Primary => primary,
                    WindowRef::Entity(entity) => Some(*entity),
                };
                let Some(window) = window.filter(|entity| windows.get(*entity).is_ok()) else {
                    continue;
                };
                ViewSurface::Window(window)
            }
            RenderTarget::TextureView(handle) if chase => ViewSurface::Embedded(*handle),
            RenderTarget::Image(image) if chase || probe => ViewSurface::Image(image.handle.id()),
            _ => continue,
        };
        if roster.count == MAX_VIEWS {
            roster.overflow = true;
            break;
        }
        let index = roster.count;
        roster.entries[index] = Some(ViewCandidate {
            retained: RetainedViewEntity::new(entity.into(), None, 0),
            surface,
            primary: chase
                && match surface {
                    ViewSurface::Window(window) => Some(window) == primary,
                    _ => true,
                },
        });
        roster.count += 1;
    }
    let count = roster.count;
    roster.entries[..count].sort_unstable_by_key(|entry| {
        let entry = entry.as_ref().unwrap();
        (!entry.primary, entry.retained.main_entity.id().to_bits())
    });
}

struct ViewMark {
    view: RegisteredView,
    finished: AtomicBool,
}

struct Entry {
    dataset: Arc<Dataset>,
    view: usize,
    expected: usize,
    selected_items: u64,
    drawn: AtomicUsize,
}

struct PageMark {
    view: usize,
    entity: Entity,
    entry: usize,
    required: u8,
    drawn: AtomicU8,
}

struct OctreeEntry {
    identity: Arc<super::octree::Identity>,
    view: usize,
    expected: usize,
    drawn: AtomicUsize,
}

struct OctreeMark {
    view: usize,
    entity: Entity,
    entry: usize,
    drawn: AtomicBool,
}

struct Frame {
    number: u64,
    views: Vec<ViewMark>,
    entries: Vec<Entry>,
    pages: Vec<PageMark>,
    octrees: Vec<OctreeEntry>,
    octree_pages: Vec<OctreeMark>,
    _charge: Charge,
    _permit: Permit,
}

impl Frame {
    fn drawn(&self, view: RetainedViewEntity, entity: Entity, pass: u8) {
        let Some(view) = self
            .views
            .iter()
            .position(|mark| mark.view.retained == view)
        else {
            return;
        };
        let Ok(index) = self
            .pages
            .binary_search_by_key(&(view, entity.to_bits()), |page| {
                (page.view, page.entity.to_bits())
            })
        else {
            return;
        };
        let page = &self.pages[index];
        let pass = pass & page.required;
        let previous = page.drawn.fetch_or(pass, Ordering::AcqRel);
        if previous & page.required != page.required
            && (previous | pass) & page.required == page.required
        {
            self.entries[page.entry]
                .drawn
                .fetch_add(1, Ordering::Release);
        }
    }

    fn completion(&self, index: usize) -> Option<RenderCompletion> {
        let entry = &self.entries[index];
        let view = &self.views[entry.view];
        if !view.finished.load(Ordering::Acquire)
            || entry.drawn.load(Ordering::Acquire) != entry.expected
        {
            return None;
        }
        let dataset = &entry.dataset;
        let counts = dataset.generation.counts();
        let selected_items = entry.selected_items;
        Some(RenderCompletion {
            target: VisibilityTarget {
                dataset: dataset.revision,
                view: view.view.identity,
            },
            frame: self.number,
            disposition: if counts.valid_samples == 0 {
                FrontierDisposition::Empty
            } else if selected_items == 0 {
                FrontierDisposition::Culled
            } else {
                FrontierDisposition::Drawn
            },
            source_samples: counts.source_samples,
            selected_items,
        })
    }

    fn drawn_octree(&self, view: RetainedViewEntity, entity: Entity) {
        let Some(view) = self
            .views
            .iter()
            .position(|mark| mark.view.retained == view)
        else {
            return;
        };
        let Ok(index) = self
            .octree_pages
            .binary_search_by_key(&(view, entity.to_bits()), |page| {
                (page.view, page.entity.to_bits())
            })
        else {
            return;
        };
        let page = &self.octree_pages[index];
        if !page.drawn.swap(true, Ordering::AcqRel) {
            self.octrees[page.entry]
                .drawn
                .fetch_add(1, Ordering::Release);
        }
    }

    fn octree_completion(&self, index: usize) -> Option<RenderCompletion> {
        let entry = &self.octrees[index];
        let view = &self.views[entry.view];
        if !view.finished.load(Ordering::Acquire)
            || entry.drawn.load(Ordering::Acquire) != entry.expected
        {
            return None;
        }
        let identity = &entry.identity;
        let drawn = entry.expected > 0;
        Some(RenderCompletion {
            target: VisibilityTarget {
                dataset: identity.revision,
                view: view.view.identity,
            },
            frame: self.number,
            disposition: if identity.cells == 0 {
                FrontierDisposition::Empty
            } else if !drawn {
                FrontierDisposition::Culled
            } else {
                FrontierDisposition::Drawn
            },
            source_samples: identity.leaves,
            selected_items: if drawn {
                identity.cells.min(identity.leaves)
            } else {
                0
            },
        })
    }

    /// Whether any cloud or compact-instance dataset finished this frame.
    fn completes(&self) -> bool {
        (0..self.entries.len()).any(|index| self.completion(index).is_some())
            || (0..self.octrees.len()).any(|index| self.octree_completion(index).is_some())
    }

    fn publish(&self, backend: &DrawingBackend) {
        for index in 0..self.entries.len() {
            if let Some(completion) = self.completion(index) {
                let dataset = &self.entries[index].dataset;
                if let Err(error) = backend.complete_render_frame(dataset.key(), completion) {
                    warn!("cloud frame receipt: {error}");
                }
            }
        }
        for index in 0..self.octrees.len() {
            if let Some(completion) = self.octree_completion(index) {
                let key = &self.octrees[index].identity.key;
                if let Err(error) = backend.complete_render_frame(key, completion) {
                    warn!("octree frame receipt: {error}");
                }
            }
        }
    }
}

#[derive(Resource, Default)]
pub(super) struct Receipts {
    views: Vec<RegisteredView>,
    epoch: u64,
    next_view: u64,
    frame: u64,
    current: Option<Arc<Frame>>,
    limiter: Option<Arc<Limiter>>,
    _charge: Option<Charge>,
}

impl Receipts {
    pub fn views(&self) -> &[RegisteredView] {
        &self.views
    }

    pub fn drawn(&self, view: RetainedViewEntity, entity: Entity, pass: u8) {
        if let Some(frame) = &self.current {
            frame.drawn(view, entity, pass);
        }
    }

    pub fn drawn_octree(&self, view: RetainedViewEntity, entity: Entity) {
        if let Some(frame) = &self.current {
            frame.drawn_octree(view, entity);
        }
    }
}

pub(super) fn install(app: &mut App) {
    app.sub_app_mut(RenderApp)
        .init_resource::<Receipts>()
        .init_resource::<ViewRoster>()
        .add_systems(ExtractSchedule, extract_views)
        .add_systems(RenderStartup, reset)
        .add_systems(Render, submit.in_set(RenderSystems::Cleanup))
        .add_systems(
            Core3d,
            finish_view
                .after(main_transparent_pass_3d)
                .in_set(Core3dSystems::MainPass),
        );
    #[cfg(feature = "draw-probes")]
    app.sub_app_mut(RenderApp)
        .add_systems(Render, observe.after(submit).in_set(RenderSystems::Cleanup));
}

#[cfg(feature = "draw-probes")]
fn observe(receipts: Res<Receipts>, shared: Option<Res<Shared>>) {
    if let Some(shared) = shared
        && let Some(probe) = &shared.probe
    {
        probe.receipts(super::probe::ReceiptReport {
            frame: receipts.frame,
            paused: probe.receipts_paused(),
            in_flight: receipts
                .limiter
                .as_ref()
                .map_or(0, |limiter| limiter.count.load(Ordering::Acquire)),
        });
    }
}

fn reset(
    mut receipts: ResMut<Receipts>,
    shared: Option<Res<Shared>>,
    mut frontiers: ResMut<Frontiers>,
    offered: Option<Res<Offered>>,
) {
    receipts.epoch = receipts.epoch.checked_add(1).expect("view epoch exhausted");
    receipts.views.clear();
    receipts.current = None;
    frontiers.0 = None;
    if let Some(offered) = offered {
        offered.retention.select(None);
    }
    if let Some(shared) = shared {
        let _ = shared.backend.sync_render_views(&[], None);
    }
}

pub(super) fn begin(
    mut receipts: ResMut<Receipts>,
    shared: Option<Res<Shared>>,
    roster: Res<ViewRoster>,
) {
    receipts.current = None;
    let Some(shared) = shared else {
        return;
    };
    if let Err(error) = register(&mut receipts, &shared.backend, &roster) {
        warn!("cloud frame receipt admission: {error}");
    }
}

fn register(
    receipts: &mut Receipts,
    backend: &DrawingBackend,
    roster: &ViewRoster,
) -> Result<(), StorageError> {
    if receipts.limiter.is_none() {
        backend
            .enable_renderer_visibility(MAX_VIEWS, &[DatasetKind::Cloud, DatasetKind::Octree, DatasetKind::Surfels, DatasetKind::MeshBlocks, DatasetKind::Volume, DatasetKind::Elevation])?;
        let metadata = charge(
            backend.budget(),
            size_of::<Receipts>()
                + size_of::<ViewRoster>()
                + MAX_VIEWS * size_of::<RegisteredView>(),
        )?;
        receipts
            .views
            .try_reserve_exact(MAX_VIEWS)
            .map_err(|_| StorageError::Allocation)?;
        if receipts.views.capacity() != MAX_VIEWS {
            return Err(StorageError::Allocation);
        }
        let limiter = charge(backend.budget(), size_of::<Limiter>() + 16)?;
        receipts.limiter = Some(Arc::new(Limiter {
            count: AtomicUsize::new(0),
            _charge: limiter,
        }));
        receipts._charge = Some(metadata);
    }
    sync_views(receipts, backend, roster)?;
    receipts.frame = receipts
        .frame
        .checked_add(1)
        .ok_or(StorageError::Allocation)?;
    Ok(())
}

pub(super) fn prepare_frame(
    mut receipts: ResMut<Receipts>,
    shared: Option<Res<Shared>>,
    frontiers: Res<Frontiers>,
    offered: Option<Res<Offered>>,
    nodes: Query<(Entity, &Node)>,
    views: Query<(&ExtractedView, &Msaa)>,
    selections: Option<Res<super::render::selection::Views>>,
    octree_nodes: Query<(Entity, &super::octree::Node)>,
    octree_offered: Option<Res<super::octree::Offered>>,
) {
    let (Some(shared), Some(offered)) = (shared, offered) else {
        return;
    };
    #[cfg(feature = "draw-probes")]
    if shared
        .probe
        .as_ref()
        .is_some_and(|probe| probe.receipts_paused())
    {
        return;
    }
    if let Err(error) = prepare(
        &mut receipts,
        &shared.backend,
        &frontiers,
        &offered,
        &nodes,
        &views,
        selections.as_deref(),
        &octree_nodes,
        octree_offered.as_deref(),
    ) {
        warn!("cloud frame receipt admission: {error}");
    }
}

fn prepare(
    receipts: &mut Receipts,
    backend: &DrawingBackend,
    frontiers: &Frontiers,
    offered: &Offered,
    nodes: &Query<(Entity, &Node)>,
    views: &Query<(&ExtractedView, &Msaa)>,
    selections: Option<&super::render::selection::Views>,
    octree_nodes: &Query<(Entity, &super::octree::Node)>,
    octree_offered: Option<&super::octree::Offered>,
) -> Result<(), StorageError> {
    let active = frontiers.0.as_ref();
    let octrees: &[Arc<super::octree::Identity>] =
        octree_offered.map_or(&[], |offered| offered.0.as_slice());
    let count = receipts.views.len();
    if receipts.limiter.is_none() || receipts.views.is_empty() {
        return Ok(());
    }
    let page_count = nodes.iter().filter(|(_, node)| node.placed).count() * count;
    let entry_count = receipts
        .views
        .iter()
        .filter_map(|view| active.and_then(|active| active.find(view.retained, view.identity)))
        .map(|set| {
            set.datasets
                .read()
                .iter()
                .filter(|data| offered.live(data))
                .count()
        })
        .sum::<usize>();
    let octree_count = octrees.len() * count;
    let octree_page_count = octree_nodes.iter().filter(|(_, node)| node.visible).count() * count;
    if entry_count == 0 && octree_count == 0 {
        return Ok(());
    }
    let Some(permit) = Permit::acquire(receipts.limiter.as_ref().unwrap()) else {
        return Ok(());
    };
    let metadata = charge(
        backend.budget(),
        size_of::<Frame>()
            + 16
            + count * size_of::<ViewMark>()
            + entry_count * size_of::<Entry>()
            + page_count * size_of::<PageMark>()
            + octree_count * size_of::<OctreeEntry>()
            + octree_page_count * size_of::<OctreeMark>(),
    )?;
    let mut frame = Frame {
        number: receipts.frame,
        views: Vec::new(),
        entries: Vec::new(),
        pages: Vec::new(),
        octrees: Vec::new(),
        octree_pages: Vec::new(),
        _charge: metadata,
        _permit: permit,
    };
    frame
        .octrees
        .try_reserve_exact(octree_count)
        .map_err(|_| StorageError::Allocation)?;
    frame
        .octree_pages
        .try_reserve_exact(octree_page_count)
        .map_err(|_| StorageError::Allocation)?;
    frame
        .views
        .try_reserve_exact(count)
        .map_err(|_| StorageError::Allocation)?;
    frame
        .entries
        .try_reserve_exact(entry_count)
        .map_err(|_| StorageError::Allocation)?;
    frame
        .pages
        .try_reserve_exact(page_count)
        .map_err(|_| StorageError::Allocation)?;
    if frame.views.capacity() != count
        || frame.entries.capacity() != entry_count
        || frame.pages.capacity() != page_count
    {
        return Err(StorageError::Allocation);
    }
    for (index, view) in receipts.views.iter().enumerate() {
        let samples = views
            .iter()
            .find_map(|(camera, msaa)| {
                (camera.retained_view_entity == view.retained).then_some(msaa.samples())
            })
            .unwrap_or(1);
        frame.views.push(ViewMark {
            view: *view,
            finished: AtomicBool::new(false),
        });
        for identity in octrees {
            let entry = frame.octrees.len();
            let mut expected = 0;
            for (entity, _) in octree_nodes
                .iter()
                .filter(|(_, node)| node.visible && Arc::ptr_eq(&node.dataset, identity))
            {
                frame.octree_pages.push(OctreeMark {
                    view: index,
                    entity,
                    entry,
                    drawn: AtomicBool::new(false),
                });
                expected += 1;
            }
            frame.octrees.push(OctreeEntry {
                identity: identity.clone(),
                view: index,
                expected,
                drawn: AtomicUsize::new(0),
            });
        }
        let Some(selected) = active.and_then(|active| active.find(view.retained, view.identity))
        else {
            continue;
        };
        let start = frame.entries.len();
        for dataset in selected
            .datasets
            .read()
            .iter()
            .filter(|data| offered.live(data))
        {
            let (expected, selected_items) = if let Some(selections) = selections {
                let Some(counts) = selections.counts(view.retained, view.identity, dataset) else {
                    continue;
                };
                counts
            } else {
                (
                    dataset.render_pages(),
                    dataset.pages.iter().map(|page| page.clip.selected).sum(),
                )
            };
            frame.entries.push(Entry {
                dataset: dataset.clone(),
                view: index,
                expected,
                selected_items,
                drawn: AtomicUsize::new(0),
            });
        }
        for (entity, node) in nodes.iter().filter(|(_, node)| node.placed) {
            if selections.is_some_and(|selections| selections.page(view.retained, entity).is_none())
            {
                continue;
            }
            if let Ok(dataset) = frame.entries[start..]
                .binary_search_by_key(&(Arc::as_ptr(&node.dataset) as usize), |entry| {
                    Arc::as_ptr(&entry.dataset) as usize
                })
            {
                frame.pages.push(PageMark {
                    view: index,
                    entity,
                    entry: start + dataset,
                    required: node
                        .dataset
                        .alpha
                        .required(node.page.appearance.alpha, samples),
                    drawn: AtomicU8::new(0),
                });
            }
        }
    }
    frame
        .pages
        .sort_unstable_by_key(|page| (page.view, page.entity.to_bits()));
    frame
        .octree_pages
        .sort_unstable_by_key(|page| (page.view, page.entity.to_bits()));
    receipts.current = Some(Arc::new(frame));
    Ok(())
}

fn sync_views(
    receipts: &mut Receipts,
    backend: &DrawingBackend,
    roster: &ViewRoster,
) -> Result<(), StorageError> {
    if roster.overflow {
        backend.sync_render_views(&[], None)?;
        receipts.views.clear();
        return Err(StorageError::Invalid("more than 16 drawing views".into()));
    }
    let candidates = &roster.entries[..roster.count];
    let matches = |view: &RegisteredView, candidate: &ViewCandidate| {
        view.retained == candidate.retained && view.surface == candidate.surface
    };
    receipts.views.retain(|view| {
        candidates
            .iter()
            .flatten()
            .any(|candidate| matches(view, candidate))
    });
    for candidate in candidates.iter().flatten() {
        if !receipts.views.iter().any(|view| matches(view, candidate)) {
            receipts.next_view = receipts
                .next_view
                .checked_add(1)
                .ok_or(StorageError::Allocation)?;
            receipts.views.push(RegisteredView {
                retained: candidate.retained,
                surface: candidate.surface,
                identity: ViewIdentity {
                    id: receipts.next_view,
                    epoch: receipts.epoch.max(1),
                },
            });
        }
    }
    receipts.views.sort_unstable_by_key(|view| {
        candidates.iter().position(|candidate| {
            candidate
                .as_ref()
                .is_some_and(|candidate| matches(view, candidate))
        })
    });
    let mut identities = [ViewIdentity { id: 1, epoch: 1 }; MAX_VIEWS];
    for (identity, view) in identities.iter_mut().zip(&receipts.views) {
        *identity = view.identity;
    }
    backend.sync_render_views(
        &identities[..receipts.views.len()],
        receipts.views.first().map(|view| view.identity),
    )
}

fn finish_view(view: ViewQuery<&ExtractedView>, receipts: Res<Receipts>) {
    let view = view.into_inner();
    if let Some(frame) = &receipts.current
        && let Some(mark) = frame
            .views
            .iter()
            .find(|mark| mark.view.retained == view.retained_view_entity)
    {
        mark.finished.store(true, Ordering::Release);
    }
}

fn submit(mut receipts: ResMut<Receipts>, shared: Option<Res<Shared>>, queue: Res<RenderQueue>) {
    let (Some(frame), Some(shared)) = (receipts.current.take(), shared) else {
        return;
    };
    if !frame.completes() {
        return;
    }
    let backend = shared.backend.clone();
    #[cfg(feature = "draw-probes")]
    let probe = shared.probe.clone();
    queue.on_submitted_work_done(move || {
        #[cfg(feature = "draw-probes")]
        if let Some(probe) = probe {
            for (index, entry) in frame.entries.iter().enumerate() {
                if let Some(completion) = frame.completion(index) {
                    probe.completed(entry.dataset.key(), completion);
                }
            }
        }
        frame.publish(&backend);
    });
}
