use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};

use bevy::math::DMat4;
use bevy::tasks::{AsyncComputeTaskPool, Task, TaskPool, block_on};

use super::super::{lod, upload};
use super::*;
use gearbox_api::draw_data::StorageError;
use gearbox_api::wire::draw_data::ViewIdentity;

#[cfg(test)]
#[path = "lod_render_tests.rs"]
mod tests;

#[path = "residency.rs"]
mod residency;

pub(super) struct Admission {
    plan: residency::Plan,
    hollow: List<lod::PageKey>,
    stamp: u64,
    wanted: u64,
}

impl Admission {
    fn tier(&self, key: lod::PageKey) -> Tier {
        self.plan
            .pages
            .binary_search_by_key(&key, |page| page.key)
            .ok()
            .filter(|index| self.plan.pages[*index].full)
            .filter(|_| self.hollow.binary_search(&key).is_err())
            .map_or(Tier::Coarse, |_| Tier::Full)
    }

    /// Pages the plan admits at full detail, whether or not their payload is resident yet.
    pub fn wanted(&self) -> impl Iterator<Item = lod::PageKey> + Clone + '_ {
        self.plan
            .pages
            .iter()
            .filter(|page| page.full)
            .map(|page| page.key)
    }

    pub fn wanted_stamp(&self) -> u64 {
        self.wanted
    }
}

pub(super) fn admission<'a>(
    shared: &Shared,
    cameras: impl Iterator<Item = (ViewIdentity, &'a ExtractedView)>,
    view_count: usize,
    nodes: &Query<(Entity, &MainEntity, &Node)>,
    offered: &Offered,
) -> Result<Admission, Issue> {
    let budget = shared.backend.budget();
    let mut views =
        List::new(budget, view_count, MemoryClass::Scratch).map_err(|_| Issue::CpuBudget)?;
    for (identity, view) in cameras {
        views
            .push((identity, view_request(view), 0u64))
            .map_err(|_| Issue::CpuBudget)?;
    }
    views.sort_unstable_by_key(|(identity, _, _)| (identity.id, identity.epoch));
    let eligible = |node: &Node| node.placed && offered.live(&node.dataset);
    let mut pages = List::new(
        budget,
        nodes.iter().filter(|(_, _, node)| eligible(node)).count(),
        MemoryClass::Metadata,
    )
    .map_err(|_| Issue::CpuBudget)?;
    let mut demands = List::new(
        budget,
        pages
            .capacity()
            .checked_mul(views.len())
            .ok_or(Issue::CpuBudget)?,
        MemoryClass::Scratch,
    )
    .map_err(|_| Issue::CpuBudget)?;
    let mut selected_nodes =
        List::new(budget, pages.capacity(), MemoryClass::Scratch).map_err(|_| Issue::CpuBudget)?;
    for (entity, _, node) in nodes.iter().filter(|(_, _, node)| eligible(node)) {
        let geometry = &node.page.appearance.geometry;
        let glyph = node_glyph(node);
        if !views.iter().any(|(_, view, _)| {
            view.page_error(&geometry.metadata, node.world_from_local.as_dmat4(), glyph)
                .is_some()
        }) {
            continue;
        }
        selected_nodes
            .push((node_key(node), node.dataset.revision.generation, entity))
            .map_err(|_| Issue::CpuBudget)?;
    }
    newest_pages(&mut selected_nodes);
    let mut hollow = List::new(
        budget,
        selected_nodes
            .iter()
            .filter(|(_, _, entity)| {
                nodes
                    .get(*entity)
                    .is_ok_and(|(_, _, node)| !node.page.appearance.geometry.resident())
            })
            .count(),
        MemoryClass::Scratch,
    )
    .map_err(|_| Issue::CpuBudget)?;
    for (key, _, entity) in &selected_nodes {
        if nodes
            .get(*entity)
            .is_ok_and(|(_, _, node)| !node.page.appearance.geometry.resident())
        {
            hollow.push(*key).map_err(|_| Issue::CpuBudget)?;
        }
    }
    hollow.sort_unstable();
    let mut styles = List::new(budget, selected_nodes.len(), MemoryClass::Scratch)
        .map_err(|_| Issue::CpuBudget)?;
    for (key, _, entity) in &selected_nodes {
        let (_, _, node) = nodes.get(*entity).map_err(|_| Issue::Deferred)?;
        let geometry = &node.page.appearance.geometry;
        let index = pages.len();
        styles
            .push(Arc::as_ptr(&node.dataset) as usize)
            .map_err(|_| Issue::CpuBudget)?;
        pages
            .push(residency::Page {
                key: *key,
                coarse_bytes: geometry.coarse.positions.values.len() as u64 * 48 + 512,
                full_bytes: geometry.len() as u64 * 48 + 512,
                full: false,
            })
            .map_err(|_| Issue::CpuBudget)?;
        for (view, (_, camera, coarse_points)) in views.iter_mut().enumerate() {
            if let Some(error) = camera.page_error(
                &geometry.metadata,
                node.world_from_local.as_dmat4(),
                node_glyph(node),
            ) {
                *coarse_points =
                    coarse_points.saturating_add(geometry.coarse.positions.values.len() as u64);
                demands
                    .push(residency::Demand {
                        view,
                        page: index,
                        error,
                    })
                    .map_err(|_| Issue::CpuBudget)?;
            }
        }
    }
    let limits = shared.backend.limits();
    let usage = shared.backend.gpu().usage();
    styles.sort_unstable();
    let style_count = usize::from(!styles.is_empty())
        + styles.windows(2).filter(|pair| pair[0] != pair[1]).count();
    let reserve = residency::headroom(
        limits.visible_points,
        views.iter().map(|(_, _, count)| *count),
        style_count,
        limits.upload_bytes_per_frame,
        usage.auxiliary,
        usage.other,
    );
    // While compact data exists, clouds leave it its demand and at least a quarter of the GPU.
    let compact = match shared.compact_demand.load(std::sync::atomic::Ordering::Relaxed) {
        0 => 0,
        demand => demand.max(shared.backend.gpu().limit() / 4),
    };
    let ceiling = shared.backend.gpu().limit().saturating_sub(reserve).saturating_sub(compact);
    let plan =
        residency::plan(budget, pages, demands, views.len(), ceiling).map_err(
            |error| match error {
                StorageError::Budget { .. } | StorageError::Allocation => Issue::CpuBudget,
                _ => Issue::GpuAllocation,
            },
        )?;
    let mut stamp = std::collections::hash_map::DefaultHasher::new();
    plan.bytes.hash(&mut stamp);
    for page in &plan.pages {
        (
            page.key.incarnation,
            page.key.scan,
            page.key.page,
            page.full,
        )
            .hash(&mut stamp);
    }
    let mut wanted = std::collections::hash_map::DefaultHasher::new();
    for page in plan.pages.iter().filter(|page| page.full) {
        (page.key.incarnation, page.key.scan, page.key.page).hash(&mut wanted);
    }
    for key in &hollow {
        (key.incarnation, key.scan, key.page).hash(&mut stamp);
    }
    Ok(Admission {
        plan,
        hollow,
        stamp: stamp.finish(),
        wanted: wanted.finish(),
    })
}

fn newest_pages(pages: &mut List<(lod::PageKey, u64, Entity)>) {
    pages.sort_unstable_by_key(|(key, generation, entity)| {
        (*key, std::cmp::Reverse(*generation), entity.to_bits())
    });
    let mut previous = None;
    pages.retain(|(key, _, _)| {
        let keep = previous != Some(*key);
        previous = Some(*key);
        keep
    });
}

#[derive(Clone, Copy, PartialEq)]
struct Placement {
    entity: Entity,
    key: lod::PageKey,
    world: Mat4,
    glyph: lod::Glyph,
}

struct Request {
    view: lod::View,
    placements: List<Placement>,
}

struct Index {
    values: List<u32>,
    buffer: OwnedBuffer,
    _charge: Charge,
}

pub(in crate::draw::data) struct Page {
    pub entity: Entity,
    pub tier: Tier,
    pub dataset: Arc<Dataset>,
    pub range: Range<u32>,
    pub selected: u64,
    pub group: Option<BindGroup>,
    key: lod::PageKey,
    geometry: std::sync::Weak<Geometry>,
    appearance: std::sync::Weak<Appearance>,
    values: Option<List<u32>>,
    index: Option<Arc<Index>>,
    written: u64,
    _charge: Charge,
}

impl Page {
    pub fn indexed(&self) -> bool {
        self.index.is_some() || self.values.is_some()
    }

    fn ready(&self) -> bool {
        !self.indexed() || self.group.is_some()
    }

    fn values(&self) -> Option<&[u32]> {
        self.index
            .as_ref()
            .map(|index| index.values.as_slice())
            .or_else(|| self.values.as_ref().map(List::as_slice))
    }
}

/// One page as a view draws it: instances `range` of its `tier` records,
/// through `values` when indexed, placed by `world`.
pub(in crate::draw::data) struct Drawn<'a> {
    pub entity: Entity,
    pub key: lod::PageKey,
    pub tier: Tier,
    pub dataset: &'a Arc<Dataset>,
    pub range: Range<u32>,
    pub values: Option<&'a [u32]>,
    pub geometry: Arc<Geometry>,
    pub appearance: Arc<Appearance>,
    pub world: Mat4,
}

struct Bundle {
    datasets: Arc<DatasetSet>,
    request: Request,
    history: Arc<lod::Cut>,
    pages: List<Page>,
    retry_stamp: Option<u64>,
    _charge: Charge,
}

impl Bundle {
    fn coarse(&self) -> bool {
        !self.pages.is_empty() && self.pages.iter().all(|page| page.tier == Tier::Coarse)
    }

    fn full(&self) -> bool {
        self.pages.iter().all(|page| page.tier == Tier::Full)
    }

    fn prune(&mut self, offered: &Offered) {
        self.datasets.datasets.prune();
        let datasets = self.datasets.datasets.read();
        if let Some(history) = Arc::get_mut(&mut self.history) {
            history.retain(|incarnation| {
                datasets
                    .iter()
                    .any(|dataset| dataset.revision.incarnation == incarnation)
            });
        }
        self.pages.retain(|page| offered.live(&page.dataset));
        if self.pages.is_empty() {
            self.pages = List::default();
        }
    }

    fn ready(&self, prepared: &[PreparedPage]) -> bool {
        self.ready_pages(prepared) == self.pages.len()
    }

    fn ready_pages(&self, prepared: &[PreparedPage]) -> usize {
        self.pages
            .iter()
            .filter(|page| {
                page.ready()
                    && prepared.iter().any(|candidate| {
                        candidate.entity == page.entity
                            && match page.tier {
                                Tier::Coarse => candidate.coarse,
                                Tier::Full => candidate.full,
                            }
                            && (!page.indexed()
                                || candidate.indexed_ids.iter().flatten().count()
                                    == candidate.ids.iter().flatten().count())
                    })
            })
            .count()
    }
}

struct State {
    identity: ViewIdentity,
    active: Option<Bundle>,
    pending: Option<Bundle>,
    job: Option<Job>,
    /// A page of `pending` was refused GPU memory since it was last checked.
    starved: AtomicBool,
    /// Most `pending` pages seen ready, and refused updates since that last grew.
    stalled: (usize, u32),
}

/// Refused updates without a new ready page after which the active cut gives way.
const STALLED_UPDATES: u32 = 30;

impl State {
    /// A pending cut refused GPU memory, and no longer progressing, while the active cut
    /// holds it replaces the active cut: pruning then frees the pages it does not share.
    fn release_starved(&mut self, prepared: &[PreparedPage]) {
        let Some(pending) = &self.pending else { return };
        let ready = pending.ready_pages(prepared);
        let refused = self.starved.swap(false, Ordering::Relaxed);
        if ready > self.stalled.0 {
            self.stalled = (ready, 0);
        } else if refused {
            self.stalled.1 += 1;
        }
        if self.active.is_some() && self.stalled.1 >= STALLED_UPDATES {
            tracing::debug!(target: "gearbox::draw::data::lod", view = self.identity.id, ready, "pending cut stalled on GPU memory; releasing the active cut");
            self.active = None;
            self.stalled.1 = 0;
        }
    }

    fn reset_starved(&mut self) {
        self.starved.store(false, Ordering::Relaxed);
        self.stalled = (0, 0);
    }
}

/// Full keys an admission plan draws at full detail, as a selection saw them.
struct Tiers(List<lod::PageKey>);

impl Tiers {
    fn of(budget: &CpuBudget, admission: &Admission) -> Result<Self, Issue> {
        let full = admission.plan.pages.iter().filter(|page| admission.tier(page.key) == Tier::Full);
        let mut keys = List::new(budget, full.clone().count(), MemoryClass::Scratch).map_err(|_| Issue::CpuBudget)?;
        for page in full {
            keys.push(page.key).map_err(|_| Issue::CpuBudget)?;
        }
        Ok(Self(keys))
    }

    /// Admitted full pages whose geometry `active` already draws at full detail.
    fn resident(
        budget: &CpuBudget,
        admission: &Admission,
        placements: &[Placement],
        nodes: &Query<(Entity, &MainEntity, &Node)>,
        active: Option<&Bundle>,
    ) -> Result<Self, Issue> {
        let held = |placement: &&Placement| {
            admission.tier(placement.key) == Tier::Full
                && nodes.get(placement.entity).is_ok_and(|(_, _, node)| {
                    let geometry = Arc::downgrade(&node.page.appearance.geometry);
                    active.is_some_and(|bundle| {
                        bundle.pages.iter().any(|page| page.tier == Tier::Full && std::sync::Weak::ptr_eq(&page.geometry, &geometry))
                    })
                })
        };
        let mut keys = List::new(budget, placements.iter().filter(held).count(), MemoryClass::Scratch).map_err(|_| Issue::CpuBudget)?;
        for placement in placements.iter().filter(held) {
            keys.push(placement.key).map_err(|_| Issue::CpuBudget)?;
        }
        Ok(Self(keys))
    }

    fn tier(&self, key: lod::PageKey) -> Tier {
        if self.0.binary_search(&key).is_ok() { Tier::Full } else { Tier::Coarse }
    }
}

/// Which tiers a background cut may draw at full detail, tried in this order when
/// a cut does not fit: the admission plan, then only pages whose full records are
/// already resident, then none.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tiering {
    Admitted,
    Resident,
    Coarse,
}

/// A cut selected off the render thread while the active cut keeps drawing.
struct Job {
    datasets: Arc<DatasetSet>,
    request: Option<Request>,
    tiers: Arc<Tiers>,
    tiering: Tiering,
    stamp: u64,
    cancel: Arc<AtomicBool>,
    task: Option<Task<Result<lod::Cut, Issue>>>,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[derive(Resource, Default)]
pub(in crate::draw::data) struct Views {
    states: Map<RetainedViewEntity, State>,
}

impl Views {
    fn source_pages(&self) -> impl Iterator<Item = &Page> {
        self.states
            .values()
            .flat_map(|state| [&state.active, &state.pending].into_iter().flatten())
            .flat_map(|bundle| bundle.pages.iter())
    }

    pub(super) fn sources(&self) -> impl Iterator<Item = (Entity, Tier)> + '_ {
        self.source_pages().map(|page| (page.entity, page.tier))
    }

    fn demand_stamp(&self, external: (u64, u64, u64)) -> u64 {
        let mut stamp = std::collections::hash_map::DefaultHasher::new();
        external.hash(&mut stamp);
        for page in self.source_pages() {
            (page.geometry.as_ptr(), page.appearance.as_ptr(), page.tier).hash(&mut stamp);
        }
        stamp.finish()
    }

    #[cfg(feature = "draw-probes")]
    pub(super) fn demanded_position_bytes(&self) -> u64 {
        self.source_pages()
            .enumerate()
            .filter(|(index, page)| {
                !self.source_pages().take(*index).any(|old| {
                    old.tier == page.tier && std::sync::Weak::ptr_eq(&old.geometry, &page.geometry)
                })
            })
            .filter_map(|(_, page)| {
                page.geometry
                    .upgrade()
                    .map(|geometry| (geometry, page.tier))
            })
            .map(|(geometry, tier)| {
                let count = match tier {
                    Tier::Full => geometry.positions.values.len(),
                    Tier::Coarse => geometry.coarse.positions.values.len(),
                };
                (count * size_of::<super::super::cloud::Position>()) as u64
            })
            .sum()
    }

    pub(super) fn needed(&self) -> impl Iterator<Item = Entity> + '_ {
        self.source_pages().map(|page| page.entity)
    }

    pub(super) fn needs(&self, entity: Entity) -> bool {
        self.needed().any(|needed| needed == entity)
    }

    pub(super) fn cancel_unpublished(&mut self, view: RetainedViewEntity) {
        if let Some(state) = self.states.get_mut(&view)
            && state.active.is_none()
        {
            state.pending = None;
        }
    }

    #[cfg(feature = "draw-probes")]
    pub(super) fn observe(&self, probe: &super::super::probe::Probe) {
        probe.clear_lod();
        for (view, state) in self.states.iter() {
            if let Some(active) = &state.active {
                let stats = active.history.statistics;
                probe.lod(
                    *view,
                    super::super::probe::LodReport {
                        valid: stats.valid,
                        represented: stats.represented,
                        selected: stats.selected,
                        culled: stats.culled,
                        pending_branches: stats.pending_branches,
                        budget_limited_branches: stats.budget_limited_branches,
                        max_error_px: stats.max_error_px.is_finite().then_some(stats.max_error_px),
                        pages: active.pages.len() as u64,
                        preparing: state.pending.is_some() || state.job.is_some(),
                        coarse: active.coarse(),
                        full_pages: active
                            .pages
                            .iter()
                            .filter(|page| page.tier == Tier::Full)
                            .count() as u64,
                        coarse_pages: active
                            .pages
                            .iter()
                            .filter(|page| page.tier == Tier::Coarse)
                            .count() as u64,
                        source_page_bounds: active
                            .pages
                            .iter()
                            .map(|page| page.key.page)
                            .min()
                            .zip(active.pages.iter().map(|page| page.key.page).max())
                            .map(|(first, last)| [first, last]),
                        selection_view: request_stamp(active.request.view),
                        verified_ordinals: true,
                    },
                );
            }
        }
    }

    #[cfg(feature = "draw-probes")]
    fn indices(&self) -> impl Iterator<Item = &Arc<Index>> {
        self.states
            .values()
            .flat_map(|state| [&state.active, &state.pending].into_iter().flatten())
            .flat_map(|bundle| bundle.pages.iter().filter_map(|page| page.index.as_ref()))
    }

    #[cfg(feature = "draw-probes")]
    pub(super) fn index_bytes(&self) -> u64 {
        self.indices()
            .enumerate()
            .filter(|(n, index)| {
                !self
                    .indices()
                    .take(*n)
                    .any(|previous| Arc::ptr_eq(previous, index))
            })
            .map(|(_, index)| index.buffer.buffer.size())
            .sum()
    }

    #[cfg(feature = "draw-probes")]
    pub(super) fn selected_points(&self) -> u64 {
        self.states
            .values()
            .filter_map(|state| state.active.as_ref())
            .flat_map(|bundle| &bundle.pages)
            .map(|page| page.range.len() as u64)
            .sum()
    }

    pub(super) fn report(&self, key: upload::Key, item: u64, issue: Issue, shared: &Shared) {
        let upload::Key::Selection(id, epoch) = key else {
            return;
        };
        let Some(state) = self
            .states
            .values()
            .find(|state| state.identity == ViewIdentity { id, epoch })
        else {
            return;
        };
        if let Some(page) = state
            .pending
            .as_ref()
            .and_then(|bundle| bundle.pages.get(item as usize))
        {
            issue.report(&shared.backend, &page.dataset);
            if issue == Issue::GpuAllocation {
                state.starved.store(true, Ordering::Relaxed);
            }
        }
    }

    /// Marks every view whose pending cut draws `entity` as refused GPU memory.
    pub(super) fn starve(&self, entity: Entity) {
        for state in self.states.values() {
            if state
                .pending
                .as_ref()
                .is_some_and(|bundle| bundle.pages.iter().any(|page| page.entity == entity))
            {
                state.starved.store(true, Ordering::Relaxed);
            }
        }
    }

    pub(super) fn prune(
        &mut self,
        budget: &CpuBudget,
        views: &[super::super::receipts::RegisteredView],
        offered: Option<&Offered>,
    ) {
        let Some(offered) = offered else {
            self.states.clear();
            return;
        };
        self.states.retain(|key, state| {
            if !views
                .iter()
                .any(|view| view.retained == *key && view.identity == state.identity)
            {
                return false;
            }
            for bundle in [&mut state.active, &mut state.pending]
                .into_iter()
                .flatten()
            {
                bundle.prune(offered);
            }
            [&state.active, &state.pending]
                .into_iter()
                .flatten()
                .any(|bundle| !bundle.datasets.datasets.read().is_empty())
        });
        let _ = self.states.compact(budget);
    }

    pub(super) fn count(&self) -> usize {
        self.states
            .values()
            .filter_map(|state| state.pending.as_ref())
            .map(|bundle| bundle.pages.iter().filter(|page| !page.ready()).count())
            .sum()
    }

    pub(super) fn jobs(&self) -> impl Iterator<Item = upload::Job> + '_ {
        self.states.values().flat_map(|state| {
            state.pending.iter().flat_map(move |bundle| {
                bundle
                    .pages
                    .iter()
                    .enumerate()
                    .filter(|(_, page)| !page.ready())
                    .map(move |(index, _)| {
                        upload::Job::new(
                            upload::Key::Selection(state.identity.id, state.identity.epoch),
                            index as u64,
                        )
                    })
            })
        })
    }

    pub(super) fn active_set(
        &self,
        key: RetainedViewEntity,
        identity: ViewIdentity,
    ) -> Option<Arc<DatasetSet>> {
        self.states
            .get(&key)
            .filter(|state| state.identity == identity)
            .and_then(|state| state.active.as_ref())
            .map(|bundle| bundle.datasets.clone())
    }

    pub(super) fn pending_set(
        &self,
        key: RetainedViewEntity,
        identity: ViewIdentity,
    ) -> Option<Arc<DatasetSet>> {
        self.states
            .get(&key)
            .filter(|state| state.identity == identity)
            .and_then(|state| state.pending.as_ref())
            .map(|bundle| bundle.datasets.clone())
    }

    pub(in crate::draw::data) fn page(
        &self,
        key: RetainedViewEntity,
        entity: Entity,
    ) -> Option<&Page> {
        self.states
            .get(&key)
            .and_then(|state| state.active.as_ref())
            .and_then(|bundle| bundle.pages.iter().find(|page| page.entity == entity))
    }

    /// Every page drawn for `key` this frame with its tier records and placement.
    pub(in crate::draw::data) fn drawn(&self, key: RetainedViewEntity) -> impl Iterator<Item = Drawn<'_>> {
        self.states
            .get(&key)
            .and_then(|state| state.active.as_ref())
            .into_iter()
            .flat_map(|bundle| {
                bundle.pages.iter().filter_map(|page| {
                    let placement = bundle.request.placements.iter().find(|p| p.entity == page.entity)?;
                    Some(Drawn {
                        entity: page.entity,
                        key: page.key,
                        tier: page.tier,
                        dataset: &page.dataset,
                        range: page.range.clone(),
                        values: page.values(),
                        geometry: page.geometry.upgrade()?,
                        appearance: page.appearance.upgrade()?,
                        world: placement.world,
                    })
                })
            })
    }

    pub(in crate::draw::data) fn counts(
        &self,
        key: RetainedViewEntity,
        identity: ViewIdentity,
        dataset: &Dataset,
    ) -> Option<(usize, u64)> {
        let bundle = self
            .states
            .get(&key)
            .filter(|state| state.identity == identity)?
            .active
            .as_ref()?;
        let address = dataset as *const Dataset as usize;
        bundle
            .datasets
            .datasets
            .read()
            .binary_search_by_key(&address, |data| Arc::as_ptr(data) as usize)
            .ok()?;
        Some(
            bundle
                .pages
                .iter()
                .filter(|page| Arc::as_ptr(&page.dataset) as usize == address)
                .fold((0, 0), |(count, selected), page| {
                    (count + 1, selected + page.selected)
                }),
        )
    }

    /// Advances one view's cut. The first cut is selected here; later ones run as a
    /// background job while the active cut keeps drawing, newest request first.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn update(
        &mut self,
        shared: &Shared,
        registered: &super::super::receipts::RegisteredView,
        view: &ExtractedView,
        datasets: Arc<DatasetSet>,
        nodes: &Query<(Entity, &MainEntity, &Node)>,
        prepared: &[PreparedPage],
        admission: &Admission,
    ) -> Result<(), Issue> {
        let budget = shared.backend.budget();
        let fresh = || State {
            identity: registered.identity,
            active: None,
            pending: None,
            job: None,
            starved: AtomicBool::new(false),
            stalled: (0, 0),
        };
        if !self.states.contains_key(&registered.retained) {
            self.states
                .insert(budget, registered.retained, fresh())
                .map_err(|_| Issue::CpuBudget)?;
        }
        let state = self.states.get_mut(&registered.retained).unwrap();
        if state.identity != registered.identity {
            *state = fresh();
        }
        if state
            .pending
            .as_ref()
            .is_some_and(|bundle| bundle.ready(prepared))
        {
            state.active = state.pending.take();
            state.reset_starved();
        }
        if state.pending.is_some() {
            state.release_starved(prepared);
            return Ok(());
        }
        let usage = shared.backend.gpu().usage();
        let stamp = self.demand_stamp((usage.auxiliary, usage.other, admission.stamp));
        let state = self.states.get(&registered.retained).unwrap();
        let current_request =
            request(budget, view, &datasets, nodes).map_err(|_| Issue::CpuBudget)?;
        if state.active.as_ref().is_some_and(|old| {
            same_datasets(&old.datasets, &datasets)
                && old.request.view == current_request.view
                && old.request.placements.as_slice() == current_request.placements.as_slice()
                && old.retry_stamp == Some(stamp)
        }) {
            self.states.get_mut(&registered.retained).unwrap().job = None;
            return Ok(());
        }
        let limits = lod::Limits {
            points: shared.backend.limits().visible_points,
            bytes: u64::from(shared.backend.limits().visible_points) * 4,
            bytes_per_point: 4,
        };
        let background = state.active.is_some();
        let pending = if background {
            let Some((pending, tiering)) = self.background(
                budget, registered, &datasets, current_request, stamp, nodes, limits, admission,
            )?
            else {
                return Ok(());
            };
            (pending, tiering)
        } else {
            let mut pending = build_admitted(
                budget,
                datasets.clone(),
                current_request,
                nodes,
                limits,
                |key| admission.tier(key),
                None,
            )?;
            pending.retry_stamp = Some(stamp);
            (pending, Tiering::Admitted)
        };
        let (mut pending, tiering) = pending;
        let headroom =
            shared.backend.limits().upload_bytes_per_frame + usage.auxiliary + usage.other;
        let fits = demand_bytes(|| self.source_pages().chain(pending.pages.iter()))
            .saturating_add(headroom)
            <= shared.backend.gpu().limit();
        if background {
            if !fits && tiering != Tiering::Coarse && !pending.coarse() {
                let next = request(budget, view, &datasets, nodes).map_err(|_| Issue::CpuBudget)?;
                let (tiers, tiering) = match tiering {
                    Tiering::Admitted => {
                        let active = self.states.get(&registered.retained).and_then(|state| state.active.as_ref());
                        (Tiers::resident(budget, admission, &pending.request.placements, nodes, active)?, Tiering::Resident)
                    }
                    _ => (Tiers::of(budget, admission)?, Tiering::Coarse),
                };
                tracing::debug!(target: "gearbox::draw::data::lod", view = registered.identity.id, selected = pending.history.statistics.selected, ?tiering, "background cut does not fit; narrowing");
                return self.spawn(budget, registered, datasets, next, stamp, limits, Arc::new(tiers), tiering, nodes);
            }
        } else if !fits || !pending.ready(prepared) {
            let coarse = request(budget, view, &datasets, nodes).map_err(|_| Issue::CpuBudget)?;
            pending = build(budget, datasets, coarse, nodes, limits, Tier::Coarse, None)?;
            pending.retry_stamp = Some(stamp);
        }
        let state = self.states.get_mut(&registered.retained).unwrap();
        if pending.coarse()
            && state.active.as_ref().is_some_and(|active| {
                active.coarse()
                    && same_datasets(&active.datasets, &pending.datasets)
                    && active.request.view == pending.request.view
                    && active.request.placements.as_slice() == pending.request.placements.as_slice()
            })
        {
            state.active.as_mut().unwrap().retry_stamp = pending.retry_stamp;
            return Ok(());
        }
        tracing::debug!(
            target: "gearbox::draw::data::lod",
            view = registered.identity.id,
            full_pages = pending.pages.iter().filter(|page| page.tier == Tier::Full).count(),
            coarse_pages = pending.pages.iter().filter(|page| page.tier == Tier::Coarse).count(),
            selected = pending.history.statistics.selected,
            max_error_px = pending.history.statistics.max_error_px,
            planned_bytes = admission.plan.bytes,
            gpu_bytes = usage.bytes,
            auxiliary_bytes = usage.auxiliary,
            other_bytes = usage.other,
            "cloud cut candidate"
        );
        if pending.ready(prepared) {
            state.active = Some(pending);
        } else {
            state.pending = Some(pending);
        }
        state.reset_starved();
        Ok(())
    }

    /// The bundle a finished job selected, or `None` while one runs; a stale,
    /// failed or absent job is replaced by one for `request`.
    #[allow(clippy::too_many_arguments)]
    fn background(
        &mut self,
        budget: &CpuBudget,
        registered: &super::super::receipts::RegisteredView,
        datasets: &Arc<DatasetSet>,
        request: Request,
        stamp: u64,
        nodes: &Query<(Entity, &MainEntity, &Node)>,
        limits: lod::Limits,
        admission: &Admission,
    ) -> Result<Option<(Bundle, Tiering)>, Issue> {
        let state = self.states.get_mut(&registered.retained).unwrap();
        if state.job.as_ref().is_some_and(|job| !same_datasets(&job.datasets, datasets)) {
            tracing::debug!(target: "gearbox::draw::data::lod", view = registered.identity.id, "background cut superseded by new datasets");
            state.job = None;
        }
        if state
            .job
            .as_ref()
            .and_then(|job| job.task.as_ref())
            .is_some_and(|task| !task.is_finished())
        {
            return Ok(None);
        }
        if let Some(mut job) = state.job.take()
            && let (Some(task), Some(selected)) = (job.task.take(), job.request.take())
            && let Ok(cut) = block_on(task).inspect_err(|issue| {
                tracing::debug!(target: "gearbox::draw::data::lod", view = registered.identity.id, ?issue, "background cut failed")
            })
        {
            let (tiers, tiering) = (job.tiers.clone(), job.tiering);
            let admitted = |key| if tiering == Tiering::Coarse { Tier::Coarse } else { tiers.tier(key) };
            if let Ok(mut bundle) = assemble(
                budget,
                job.datasets.clone(),
                selected,
                nodes,
                limits,
                admitted,
                state.active.as_ref(),
                cut,
            ).inspect_err(|issue| {
                tracing::debug!(target: "gearbox::draw::data::lod", view = registered.identity.id, ?issue, "background cut not assembled")
            }) {
                bundle.retry_stamp = Some(job.stamp);
                return Ok(Some((bundle, tiering)));
            }
        }
        let tiers = Arc::new(Tiers::of(budget, admission)?);
        self.spawn(budget, registered, datasets.clone(), request, stamp, limits, tiers, Tiering::Admitted, nodes)?;
        Ok(None)
    }

    /// Starts selecting `request` on the async compute pool against the active cut's history.
    #[allow(clippy::too_many_arguments)]
    fn spawn(
        &mut self,
        budget: &CpuBudget,
        registered: &super::super::receipts::RegisteredView,
        datasets: Arc<DatasetSet>,
        request: Request,
        stamp: u64,
        limits: lod::Limits,
        tiers: Arc<Tiers>,
        tiering: Tiering,
        nodes: &Query<(Entity, &MainEntity, &Node)>,
    ) -> Result<(), Issue> {
        let sources = sources(budget, &request, nodes)?;
        let state = self.states.get_mut(&registered.retained).unwrap();
        let previous = state.active.as_ref().map(|bundle| bundle.history.clone());
        let cancel = Arc::new(AtomicBool::new(false));
        let (owned, view, flag, admitted) = (budget.clone(), request.view, cancel.clone(), tiers.clone());
        let task = AsyncComputeTaskPool::get_or_init(TaskPool::default).spawn(async move {
            select_cut(
                &owned,
                &sources,
                view,
                limits,
                previous.as_deref(),
                |key| if tiering == Tiering::Coarse { Tier::Coarse } else { admitted.tier(key) },
                || {
                    if flag.load(Ordering::Relaxed) {
                        Err(StorageError::Unavailable("superseded selection".into()))
                    } else {
                        Ok(())
                    }
                },
            )
        });
        state.job = Some(Job {
            datasets,
            request: Some(request),
            tiers,
            tiering,
            stamp,
            cancel,
            task: Some(task),
        });
        Ok(())
    }

    pub(super) fn upload(
        &mut self,
        key: upload::Key,
        item: u64,
        cache: &Cache,
        shared: &Shared,
        device: &RenderDevice,
        queue: &RenderQueue,
        pipeline: &Pipeline,
        pipelines: &PipelineCache,
        remaining: &mut u64,
        staging: &mut List<(GpuCharge, Charge)>,
    ) -> Result<(), Issue> {
        let upload::Key::Selection(id, epoch) = key else {
            return Err(Issue::Deferred);
        };
        let Some(page) = self
            .states
            .values_mut()
            .find(|state| state.identity == ViewIdentity { id, epoch })
            .and_then(|state| state.pending.as_mut())
            .and_then(|bundle| bundle.pages.get_mut(item as usize))
        else {
            return Err(Issue::Deferred);
        };
        if !page.dataset.lifetime.is_live() {
            return Err(Issue::Deferred);
        }
        let budget = shared.backend.budget();
        let ledger = shared.backend.gpu();
        if page.index.is_none() {
            let Some(values) = &page.values else {
                return Ok(());
            };
            if *remaining < 4 {
                return Err(Issue::Deferred);
            }
            let charge = super::super::worker::charge(budget, size_of::<Index>() + 16)
                .map_err(|_| Issue::CpuBudget)?;
            let buffer = allocate(
                device,
                queue,
                ledger,
                budget,
                (values.len() * 4) as u64,
                BufferUsages::STORAGE,
                "cloud selected ordinals",
            )?;
            page.index = Some(Arc::new(Index {
                values: page.values.take().unwrap(),
                buffer,
                _charge: charge,
            }));
        }
        let index = page.index.as_ref().unwrap();
        let bytes: &[u8] = bytemuck::cast_slice(&index.values);
        if page.written < bytes.len() as u64 {
            let count = (bytes.len() as u64 - page.written).min(*remaining) & !3;
            if count == 0 {
                return Err(Issue::Deferred);
            }
            write(
                queue,
                ledger,
                budget,
                remaining,
                &index.buffer.buffer,
                page.written,
                &bytes[page.written as usize..(page.written + count) as usize],
                staging,
            )?;
            page.written += count;
            #[cfg(feature = "draw-probes")]
            if let Some(probe) = &shared.probe {
                probe.uploaded(
                    match page.tier {
                        Tier::Full => super::super::probe::UploadKind::Indices,
                        Tier::Coarse => super::super::probe::UploadKind::CoarseIndices,
                    },
                    count as usize,
                );
            }
        }
        if page.written != bytes.len() as u64 {
            return Err(Issue::Deferred);
        }
        if page.group.is_none() {
            let source = cache
                .pages
                .get(&(page.entity, page.tier))
                .ok_or(Issue::Deferred)?;
            let geometry = cache
                .geometry
                .get(&source.geometry)
                .ok_or(Issue::Deferred)?;
            let appearance = cache
                .appearance
                .get(&source.appearance)
                .ok_or(Issue::Deferred)?;
            page.group = Some(device.create_bind_group(
                "selected cloud page",
                &pipelines.get_bind_group_layout(&pipeline.page_selected),
                &BindGroupEntries::sequential((
                    source.buffer.buffer.as_entire_binding(),
                    geometry.buffer.buffer.as_entire_binding(),
                    appearance.buffer.buffer.as_entire_binding(),
                    index.buffer.buffer.as_entire_binding(),
                )),
            ));
        }
        Ok(())
    }
}

fn request(
    budget: &CpuBudget,
    view: &ExtractedView,
    datasets: &DatasetSet,
    nodes: &Query<(Entity, &MainEntity, &Node)>,
) -> Result<Request, StorageError> {
    let data = datasets.datasets.read();
    let selected = |node: &Node| {
        data.binary_search_by_key(&(Arc::as_ptr(&node.dataset) as usize), |data| {
            Arc::as_ptr(data) as usize
        })
        .is_ok()
    };
    let count = nodes
        .iter()
        .filter(|(_, _, node)| node.placed && selected(node))
        .count();
    let mut placements = List::new(budget, count, MemoryClass::Metadata)?;
    for (entity, _, node) in nodes
        .iter()
        .filter(|(_, _, node)| node.placed && selected(node))
    {
        placements.push(Placement {
            entity,
            key: node_key(node),
            world: node.world_from_local,
            glyph: node_glyph(node),
        })?;
    }
    placements.sort_unstable_by_key(|placement| placement.key);
    Ok(Request {
        view: view_request(view),
        placements,
    })
}

fn view_request(view: &ExtractedView) -> lod::View {
    lod::View {
        clip_from_world: view
            .clip_from_world
            .unwrap_or_else(|| {
                view.clip_from_view * Mat4::from(view.world_from_view.affine().inverse())
            })
            .as_dmat4(),
        viewport: [view.viewport.z, view.viewport.w],
        error_px: 1.5,
        hysteresis: 0.15,
    }
}

#[cfg(feature = "draw-probes")]
fn request_stamp(view: lod::View) -> u64 {
    let mut stamp = std::collections::hash_map::DefaultHasher::new();
    view.clip_from_world
        .to_cols_array()
        .map(f64::to_bits)
        .hash(&mut stamp);
    view.viewport.hash(&mut stamp);
    stamp.finish()
}

#[cfg(feature = "draw-probes")]
pub(super) fn view_stamp(view: &ExtractedView) -> u64 {
    request_stamp(view_request(view))
}

fn node_key(node: &Node) -> lod::PageKey {
    let address = node.page.appearance.geometry.descriptor().address;
    lod::PageKey {
        incarnation: node.dataset.revision.incarnation,
        scan: address.scan_key,
        page: address.page_id,
    }
}

fn node_glyph(node: &Node) -> lod::Glyph {
    let (metres, pixels) = match node.dataset.generation.descriptor().presentation.size {
        gearbox_api::wire::draw_data::PointSize::Metres { radius } => (radius, 0.0),
        gearbox_api::wire::draw_data::PointSize::Pixels { radius } => (0.0, radius),
    };
    lod::Glyph {
        metres: f64::from(node.page.appearance.max_radius_m.max(metres)),
        pixels: f64::from(pixels),
    }
}

fn direct_range(indices: &[u32]) -> Option<Range<u32>> {
    let low = indices.iter().copied().min()?;
    let high = indices.iter().copied().max()?;
    (u64::from(high) - u64::from(low) + 1 == indices.len() as u64).then_some(low..high + 1)
}

fn validate_ordinals(indices: &mut [u32], source_count: usize) -> Result<(), Issue> {
    indices.sort_unstable();
    if indices
        .last()
        .is_some_and(|last| *last as usize >= source_count)
        || indices.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(Issue::InvalidSelection);
    }
    Ok(())
}

fn validate_cut(cut: &lod::Cut, limits: lod::Limits) -> Result<(), Issue> {
    let count = cut.indices.len() as u64;
    let mut cursor = 0u64;
    if count != cut.statistics.selected
        || count > u64::from(limits.points)
        || count.saturating_mul(u64::from(limits.bytes_per_point)) > limits.bytes
        || cut.pages.windows(2).any(|pair| pair[0].key >= pair[1].key)
    {
        return Err(Issue::InvalidSelection);
    }
    for page in &cut.pages {
        if u64::from(page.first) != cursor {
            return Err(Issue::InvalidSelection);
        }
        cursor += u64::from(page.count);
    }
    if cursor != count {
        return Err(Issue::InvalidSelection);
    }
    Ok(())
}

fn build(
    budget: &CpuBudget,
    datasets: Arc<DatasetSet>,
    request: Request,
    nodes: &Query<(Entity, &MainEntity, &Node)>,
    limits: lod::Limits,
    tier: Tier,
    previous: Option<&Bundle>,
) -> Result<Bundle, Issue> {
    build_admitted(budget, datasets, request, nodes, limits, |_| tier, previous)
}

/// Whether two frontiers hold the same datasets, whichever frame built them.
fn same_datasets(a: &Arc<DatasetSet>, b: &Arc<DatasetSet>) -> bool {
    Arc::ptr_eq(a, b) || {
        let (a, b) = (a.datasets.read(), b.datasets.read());
        a.len() == b.len() && a.iter().zip(b.iter()).all(|(a, b)| Arc::ptr_eq(a, b))
    }
}

/// One placed page a selection reads, owned so the selection can leave the render thread.
struct Source {
    key: lod::PageKey,
    geometry: Arc<Geometry>,
    world: DMat4,
    glyph: lod::Glyph,
}

fn sources(
    budget: &CpuBudget,
    request: &Request,
    nodes: &Query<(Entity, &MainEntity, &Node)>,
) -> Result<List<Source>, Issue> {
    let mut sources = List::new(budget, request.placements.len(), MemoryClass::Scratch)
        .map_err(|_| Issue::CpuBudget)?;
    for placement in &request.placements {
        let (_, _, node) = nodes.get(placement.entity).map_err(|_| Issue::Deferred)?;
        sources
            .push(Source {
                key: placement.key,
                geometry: node.page.appearance.geometry.clone(),
                world: placement.world.as_dmat4(),
                glyph: placement.glyph,
            })
            .map_err(|_| Issue::CpuBudget)?;
    }
    Ok(sources)
}

fn select_cut(
    budget: &CpuBudget,
    sources: &[Source],
    view: lod::View,
    limits: lod::Limits,
    previous: Option<&lod::Cut>,
    admission: impl Fn(lod::PageKey) -> Tier,
    check: impl Fn() -> Result<(), StorageError>,
) -> Result<lod::Cut, Issue> {
    let mut inputs =
        List::new(budget, sources.len(), MemoryClass::Scratch).map_err(|_| Issue::CpuBudget)?;
    for source in sources {
        inputs
            .push(lod::Input {
                key: source.key,
                geometry: Arc::downgrade(&source.geometry),
                metadata: &source.geometry.metadata,
                world_from_local: source.world,
                glyph: source.glyph,
            })
            .map_err(|_| Issue::CpuBudget)?;
    }
    lod::select(
        budget,
        &inputs,
        view,
        limits,
        previous,
        |samples| !samples.full || admission(samples.page) == Tier::Full,
        check,
    )
    .map_err(|error| match error {
        lod::Error::Storage(_) => Issue::CpuBudget,
        _ => Issue::Deferred,
    })
}

fn build_admitted(
    budget: &CpuBudget,
    datasets: Arc<DatasetSet>,
    request: Request,
    nodes: &Query<(Entity, &MainEntity, &Node)>,
    limits: lod::Limits,
    admission: impl Fn(lod::PageKey) -> Tier,
    previous: Option<&Bundle>,
) -> Result<Bundle, Issue> {
    let sources = sources(budget, &request, nodes)?;
    let cut = select_cut(
        budget,
        &sources,
        request.view,
        limits,
        previous.map(|bundle| &*bundle.history),
        &admission,
        || Ok(()),
    )?;
    assemble(budget, datasets, request, nodes, limits, admission, previous, cut)
}

/// The bundle a selected `cut` draws, reusing `previous` index buffers that still match.
#[allow(clippy::too_many_arguments)]
fn assemble(
    budget: &CpuBudget,
    datasets: Arc<DatasetSet>,
    request: Request,
    nodes: &Query<(Entity, &MainEntity, &Node)>,
    limits: lod::Limits,
    admission: impl Fn(lod::PageKey) -> Tier,
    previous: Option<&Bundle>,
    mut cut: lod::Cut,
) -> Result<Bundle, Issue> {
    validate_cut(&cut, limits)?;
    let charge =
        super::super::worker::charge(budget, size_of::<Bundle>()).map_err(|_| Issue::CpuBudget)?;
    let mut pages =
        List::new(budget, cut.pages.len(), MemoryClass::Metadata).map_err(|_| Issue::CpuBudget)?;
    for selected in &cut.pages {
        if selected.count == 0 {
            continue;
        }
        let tier = admission(selected.key);
        let placement = &request.placements[request
            .placements
            .binary_search_by_key(&selected.key, |placement| placement.key)
            .map_err(|_| Issue::InvalidSelection)?];
        let (_, _, node) = nodes.get(placement.entity).map_err(|_| Issue::Deferred)?;
        let end = selected
            .first
            .checked_add(selected.count)
            .ok_or(Issue::InvalidSelection)?;
        let ordinals = cut
            .indices
            .get_mut(selected.first as usize..end as usize)
            .ok_or(Issue::InvalidSelection)?;
        validate_ordinals(ordinals, node.page.appearance.geometry.len())?;
        let selected_count = ordinals
            .iter()
            .filter(|ordinal| {
                node.page
                    .clip
                    .accepts(&node.page.appearance.geometry, **ordinal)
            })
            .count() as u64;
        if tier == Tier::Coarse {
            for ordinal in ordinals.iter_mut() {
                *ordinal = node
                    .page
                    .appearance
                    .geometry
                    .coarse
                    .index(*ordinal)
                    .ok_or(Issue::InvalidSelection)?;
            }
        }
        let range = direct_range(ordinals);
        let mut values = None;
        let mut index = None;
        let mut group = None;
        let mut written = 0;
        if range.is_none() {
            let mut owned = List::new(budget, ordinals.len(), MemoryClass::Metadata)
                .map_err(|_| Issue::CpuBudget)?;
            for ordinal in ordinals {
                owned.push(*ordinal).map_err(|_| Issue::CpuBudget)?;
            }
            let old = previous.and_then(|bundle| {
                bundle.pages.iter().find(|old| {
                    old.key == selected.key
                        && old.tier == tier
                        && std::sync::Weak::ptr_eq(&old.geometry, &selected.geometry)
                        && old.values() == Some(owned.as_slice())
                        && old.ready()
                })
            });
            if let Some(old) = old {
                index = old.index.clone();
                written = (owned.len() * 4) as u64;
                if old.entity == placement.entity {
                    group = old.group.clone();
                }
            } else {
                values = Some(owned);
            }
        }
        pages
            .push(Page {
                entity: placement.entity,
                tier,
                dataset: node.dataset.clone(),
                key: selected.key,
                geometry: selected.geometry.clone(),
                appearance: Arc::downgrade(&node.page.appearance),
                range: range.unwrap_or(0..selected.count),
                selected: selected_count,
                values,
                index,
                written,
                group,
                _charge: super::super::worker::charge(budget, 256).map_err(|_| Issue::CpuBudget)?,
            })
            .map_err(|_| Issue::CpuBudget)?;
    }
    cut.indices = List::default();
    Ok(Bundle {
        datasets,
        request,
        history: Arc::new(cut),
        pages,
        retry_stamp: None,
        _charge: charge,
    })
}

fn demand_bytes<'a, I: Iterator<Item = &'a Page>>(pages: impl Fn() -> I) -> u64 {
    let mut total = 0u64;
    for (index, page) in pages().enumerate() {
        let count = page
            .geometry
            .upgrade()
            .map_or(0, |geometry| match page.tier {
                Tier::Full => geometry.positions.values.len(),
                Tier::Coarse => geometry.coarse.positions.values.len(),
            }) as u64;
        if !pages().take(index).any(|old| {
            old.tier == page.tier && std::sync::Weak::ptr_eq(&old.geometry, &page.geometry)
        }) {
            total += count * 16;
        }
        if !pages().take(index).any(|old| {
            old.tier == page.tier && std::sync::Weak::ptr_eq(&old.appearance, &page.appearance)
        }) {
            total += count * 32;
        }
        if !pages()
            .take(index)
            .any(|old| Arc::ptr_eq(&old.dataset, &page.dataset))
        {
            total += 16 * 1024;
        }
        total += 512;
        if page.indexed() {
            total += page.range.len() as u64 * 4;
        }
    }
    total
}
