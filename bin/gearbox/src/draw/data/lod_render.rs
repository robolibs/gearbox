use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Weak;
use std::sync::atomic::{AtomicBool, Ordering};

use super::super::{lod, upload};
use super::gpu::{LodPipelines, ViewLod};
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

/// GPU bytes a page holds at full detail: positions, attributes, its LOD tree and order.
fn full_bytes(node: &Node) -> u64 {
    let geometry = &node.page.appearance.geometry;
    geometry.len() as u64 * size_of::<super::super::cloud::Position>() as u64
        + node.page.appearance.attributes.layout.bytes()
        + size_of_val(geometry.tree.values.as_slice()) as u64
        + size_of_val(geometry.hierarchy.refinement.order.values.as_slice()) as u64
        + 512
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
        let position = size_of::<super::super::cloud::Position>() as u64;
        let index = pages.len();
        styles
            .push(Arc::as_ptr(&node.dataset) as usize)
            .map_err(|_| Issue::CpuBudget)?;
        pages
            .push(residency::Page {
                key: *key,
                coarse_bytes: geometry.coarse.positions.values.len() as u64 * position
                    + node.page.appearance.coarse.attributes.layout.bytes()
                    + 512,
                full_bytes: full_bytes(node),
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
        views.len(),
        selected_nodes.len(),
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

pub(in crate::draw::data) struct Page {
    pub entity: Entity,
    pub tier: Tier,
    pub dataset: Arc<Dataset>,
    /// Coarse pages draw every coarse record; full pages draw what the view's LOD pass lists.
    pub range: Range<u32>,
    /// Clip-accepted coarse records; a full page's count is read back from the GPU.
    pub selected: u64,
    /// A full page's slot in the view's LOD buffers.
    pub slot: Option<u32>,
    /// A full page's draw bind group, and its LOD pass slot groups with and without the
    /// accept dispatch arguments.
    pub group: Option<BindGroup>,
    pub lod_group: Option<BindGroup>,
    pub accept_group: Option<BindGroup>,
    key: lod::PageKey,
    geometry: Weak<Geometry>,
    appearance: Weak<Appearance>,
    _charge: Charge,
}

impl Page {
    /// Whether the GPU level-of-detail pass selects what this page draws.
    pub fn indexed(&self) -> bool {
        self.slot.is_some()
    }

    fn ready(&self) -> bool {
        !self.indexed()
            || (self.group.is_some() && self.lod_group.is_some() && self.accept_group.is_some())
    }

    fn same(&self, other: &Self) -> bool {
        self.entity == other.entity
            && self.tier == other.tier
            && Arc::ptr_eq(&self.dataset, &other.dataset)
            && Weak::ptr_eq(&self.geometry, &other.geometry)
            && Weak::ptr_eq(&self.appearance, &other.appearance)
    }
}

/// One page as a view draws it: instances `range` of its `tier` records, placed by `world`.
pub(in crate::draw::data) struct Drawn<'a> {
    pub entity: Entity,
    pub key: lod::PageKey,
    pub tier: Tier,
    pub dataset: &'a Arc<Dataset>,
    pub range: Range<u32>,
    pub geometry: Arc<Geometry>,
    pub appearance: Arc<Appearance>,
    pub world: Mat4,
}

struct Bundle {
    datasets: Arc<DatasetSet>,
    request: Request,
    pages: List<Page>,
    _charge: Charge,
}

impl Bundle {
    fn coarse(&self) -> bool {
        !self.pages.is_empty() && self.pages.iter().all(|page| page.tier == Tier::Coarse)
    }

    fn prune(&mut self, offered: &Offered) {
        self.datasets.datasets.prune();
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

    fn slots(&self) -> u32 {
        self.pages.iter().filter(|page| page.indexed()).count() as u32
    }
}

struct State {
    identity: ViewIdentity,
    active: Option<Bundle>,
    pending: Option<Bundle>,
    lod: Option<ViewLod>,
    /// A page of `pending` was refused GPU memory since it was last checked.
    starved: AtomicBool,
    /// Most `pending` pages seen ready, and refused updates since that last grew.
    stalled: (usize, u32),
}

/// Refused updates without a new ready page after which the active bundle gives way.
const STALLED_UPDATES: u32 = 30;

impl State {
    /// A pending bundle refused GPU memory, and no longer progressing, while the active one
    /// holds it replaces the active bundle: pruning then frees the pages it does not share.
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
            tracing::debug!(target: "gearbox::draw::data::lod", view = self.identity.id, ready, "pending pages stalled on GPU memory; releasing the active bundle");
            self.active = None;
            self.stalled.1 = 0;
        }
    }

    fn reset_starved(&mut self) {
        self.starved.store(false, Ordering::Relaxed);
        self.stalled = (0, 0);
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

    #[cfg(feature = "draw-probes")]
    pub(super) fn demanded_position_bytes(&self) -> u64 {
        self.source_pages()
            .enumerate()
            .filter(|(index, page)| {
                !self.source_pages().take(*index).any(|old| {
                    old.tier == page.tier && Weak::ptr_eq(&old.geometry, &page.geometry)
                })
            })
            .filter_map(|(_, page)| page.geometry.upgrade().map(|geometry| (geometry, page.tier)))
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

    /// Per page of the active bundle: points it represents and selects, GPU pages as their
    /// view's LOD pass last read them back.
    #[cfg(feature = "draw-probes")]
    fn page_counts(state: &State) -> impl Iterator<Item = (u64, u64)> + '_ {
        let last = state.lod.as_ref().map(|lod| lod.last.lock().unwrap().clone()).unwrap_or_default();
        state.active.iter().flat_map(|bundle| bundle.pages.iter()).map(move |page| match page.slot {
            Some(slot) => last.get(slot as usize).map_or((0, 0), |head| {
                (u64::from(head.represented), u64::from(head.accepted))
            }),
            None => (
                page.geometry.upgrade().map_or(0, |geometry| geometry.len() as u64),
                page.selected,
            ),
        })
    }

    #[cfg(feature = "draw-probes")]
    pub(super) fn observe(&self, probe: &super::super::probe::Probe) {
        probe.clear_lod();
        for (view, state) in self.states.iter() {
            if let Some(active) = &state.active {
                let valid = active
                    .datasets
                    .datasets
                    .read()
                    .iter()
                    .map(|data| data.generation.counts().valid_samples)
                    .sum::<u64>();
                let (represented, selected) = Self::page_counts(state)
                    .fold((0, 0), |(r, s), (page_r, page_s)| (r + page_r, s + page_s));
                probe.lod(
                    *view,
                    super::super::probe::LodReport {
                        valid,
                        represented,
                        selected,
                        culled: valid.saturating_sub(represented),
                        pending_branches: 0,
                        budget_limited_branches: 0,
                        max_error_px: None,
                        pages: active.pages.len() as u64,
                        preparing: state.pending.is_some(),
                        coarse: active.coarse(),
                        full_pages: active.pages.iter().filter(|page| page.tier == Tier::Full).count() as u64,
                        coarse_pages: active.pages.iter().filter(|page| page.tier == Tier::Coarse).count() as u64,
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

    /// GPU bytes of the views' level-of-detail buffers.
    #[cfg(feature = "draw-probes")]
    pub(super) fn index_bytes(&self) -> u64 {
        self.states
            .values()
            .filter_map(|state| state.lod.as_ref())
            .map(|lod| lod.heads.buffer.size() + lod.tables.buffer.size())
            .sum()
    }

    #[cfg(feature = "draw-probes")]
    pub(super) fn selected_points(&self) -> u64 {
        self.states
            .values()
            .flat_map(Self::page_counts)
            .map(|(_, selected)| selected)
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

    /// Marks every view whose pending bundle draws `entity` as refused GPU memory.
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

    /// The view's LOD buffers and the active pages the GPU selects for it.
    pub(super) fn lod_pages(
        &self,
        key: RetainedViewEntity,
    ) -> Option<(&ViewLod, impl Iterator<Item = &Page>)> {
        let state = self.states.get(&key)?;
        let lod = state.lod.as_ref()?;
        let pages = state.active.as_ref()?.pages.iter().filter(|page| page.indexed());
        Some((lod, pages))
    }

    /// The heads buffer a view's full pages draw their indirect arguments from.
    pub(super) fn heads(&self, key: RetainedViewEntity) -> Option<&Buffer> {
        Some(&self.states.get(&key)?.lod.as_ref()?.heads.buffer)
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
                    let geometry = page.geometry.upgrade()?;
                    Some(Drawn {
                        entity: page.entity,
                        key: page.key,
                        tier: page.tier,
                        dataset: &page.dataset,
                        range: match page.tier {
                            Tier::Full => 0..geometry.len() as u32,
                            Tier::Coarse => page.range.clone(),
                        },
                        geometry,
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
    ) -> Option<(usize, u64, Vec<u32>)> {
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
        let pages = || bundle.pages.iter().filter(|page| Arc::as_ptr(&page.dataset) as usize == address);
        Some((
            pages().count(),
            pages().map(|page| page.selected).sum(),
            pages().filter_map(|page| page.slot).collect(),
        ))
    }

    /// The pages `view` draws: every placed page it can see, at the tier admission grants.
    /// A new page set waits as pending until its pages can draw, while the active one keeps
    /// drawing; the GPU LOD pass then selects within full pages every frame.
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
            lod: None,
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
        let request = request(budget, view, &datasets, nodes).map_err(|_| Issue::CpuBudget)?;
        let bundle = build(budget, datasets, request, nodes, |key| admission.tier(key))?;
        if let Some(active) = &mut state.active
            && same_datasets(&active.datasets, &bundle.datasets)
            && active.pages.len() == bundle.pages.len()
            && active.pages.iter().zip(bundle.pages.iter()).all(|(a, b)| a.same(b))
        {
            for (old, new) in active.pages.iter_mut().zip(bundle.pages.iter()) {
                old.selected = if old.indexed() { old.selected } else { new.selected };
            }
            active.request = bundle.request;
            return Ok(());
        }
        tracing::debug!(
            target: "gearbox::draw::data::lod",
            view = registered.identity.id,
            full_pages = bundle.slots(),
            coarse_pages = bundle.pages.len() as u32 - bundle.slots(),
            "cloud page set"
        );
        if bundle.ready(prepared) {
            state.active = Some(bundle);
        } else {
            state.pending = Some(bundle);
        }
        state.reset_starved();
        Ok(())
    }

    /// Binds one pending full page to its view's LOD slot once its buffers are resident,
    /// growing the view's LOD buffers first when the pending pages outnumber them.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn upload(
        &mut self,
        key: upload::Key,
        item: u64,
        cache: &Cache,
        shared: &Shared,
        device: &RenderDevice,
        queue: &RenderQueue,
        pipeline: &Pipeline,
        lod_pipelines: &LodPipelines,
        pipelines: &PipelineCache,
    ) -> Result<(), Issue> {
        let upload::Key::Selection(id, epoch) = key else {
            return Err(Issue::Deferred);
        };
        let Some(state) = self
            .states
            .values_mut()
            .find(|state| state.identity == ViewIdentity { id, epoch })
        else {
            return Err(Issue::Deferred);
        };
        let wanted = state.pending.as_ref().map_or(0, Bundle::slots);
        if state.lod.as_ref().is_none_or(|lod| lod.capacity < wanted) {
            state.lod = Some(ViewLod::new(device, queue, shared, lod_pipelines, pipelines, wanted)?);
            // Groups bound to the old buffers would draw stale tables.
            if let Some(active) = &mut state.active {
                for page in active.pages.iter_mut() {
                    page.group = None;
                    page.lod_group = None;
                    page.accept_group = None;
                }
            }
            state.active = None;
        }
        let lod = state.lod.as_ref().unwrap();
        let Some(page) = state
            .pending
            .as_mut()
            .and_then(|bundle| bundle.pages.get_mut(item as usize))
        else {
            return Err(Issue::Deferred);
        };
        if !page.dataset.lifetime.is_live() {
            return Err(Issue::Deferred);
        }
        let Some(slot) = page.slot else {
            return Ok(());
        };
        let source = cache
            .pages
            .get(&(page.entity, Tier::Full))
            .ok_or(Issue::Deferred)?;
        let geometry = cache.geometry.get(&source.geometry).ok_or(Issue::Deferred)?;
        let appearance = match &source.appearance {
            Some(key) => cache.appearance.get(key).ok_or(Issue::Deferred)?,
            None => geometry,
        };
        let order = source.order.and_then(|key| cache.orders.get(&key)).ok_or(Issue::Deferred)?;
        let [head, table, dispatch] = lod.slot(slot);
        page.group = Some(device.create_bind_group(
            "cloud lod page",
            &pipelines.get_bind_group_layout(&pipeline.page_selected),
            &BindGroupEntries::sequential((
                source.buffer.buffer.as_entire_binding(),
                geometry.buffer.buffer.as_entire_binding(),
                appearance.buffer.buffer.as_entire_binding(),
                order.buffer.buffer.as_entire_binding(),
                table.clone(),
            )),
        ));
        page.lod_group = Some(device.create_bind_group(
            "cloud lod slot",
            &pipelines.get_bind_group_layout(&lod_pipelines.slot),
            &BindGroupEntries::sequential((head.clone(), table.clone(), dispatch)),
        ));
        page.accept_group = Some(device.create_bind_group(
            "cloud lod accept slot",
            &pipelines.get_bind_group_layout(&lod_pipelines.accept_slot),
            &BindGroupEntries::sequential((head, table)),
        ));
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

/// Every placed page `request` sees, newest generation per key, at its admitted tier.
fn build(
    budget: &CpuBudget,
    datasets: Arc<DatasetSet>,
    request: Request,
    nodes: &Query<(Entity, &MainEntity, &Node)>,
    admitted: impl Fn(lod::PageKey) -> Tier,
) -> Result<Bundle, Issue> {
    let charge = super::super::worker::charge(budget, size_of::<Bundle>() + 16)
        .map_err(|_| Issue::CpuBudget)?;
    let mut visible = List::new(budget, request.placements.len(), MemoryClass::Scratch)
        .map_err(|_| Issue::CpuBudget)?;
    for placement in request.placements.iter() {
        let (_, _, node) = nodes.get(placement.entity).map_err(|_| Issue::Deferred)?;
        let metadata = &node.page.appearance.geometry.metadata;
        if request.view.page_error(metadata, placement.world.as_dmat4(), placement.glyph).is_some() {
            visible
                .push((placement.key, node.dataset.revision.generation, placement.entity))
                .map_err(|_| Issue::CpuBudget)?;
        }
    }
    newest_pages(&mut visible);
    let mut pages = List::new(budget, visible.len(), MemoryClass::Metadata).map_err(|_| Issue::CpuBudget)?;
    let mut slots = 0;
    for (key, _, entity) in &visible {
        let (_, _, node) = nodes.get(*entity).map_err(|_| Issue::Deferred)?;
        let geometry = &node.page.appearance.geometry;
        let full = admitted(*key) == Tier::Full && geometry.resident();
        let (tier, range, selected, slot) = if full {
            slots += 1;
            (Tier::Full, 0..0, 0, Some(slots - 1))
        } else {
            let ordinals = &geometry.coarse.ordinals.values;
            let accepted = ordinals
                .iter()
                .filter(|ordinal| node.page.clip.accepts(geometry, **ordinal))
                .count() as u64;
            (Tier::Coarse, 0..ordinals.len() as u32, accepted, None)
        };
        pages
            .push(Page {
                entity: *entity,
                tier,
                dataset: node.dataset.clone(),
                range,
                selected,
                slot,
                group: None,
                lod_group: None,
                accept_group: None,
                key: *key,
                geometry: Arc::downgrade(geometry),
                appearance: Arc::downgrade(&node.page.appearance),
                _charge: super::super::worker::charge(budget, size_of::<Page>() + 16)
                    .map_err(|_| Issue::CpuBudget)?,
            })
            .map_err(|_| Issue::CpuBudget)?;
    }
    Ok(Bundle {
        datasets,
        request,
        pages,
        _charge: charge,
    })
}

fn same_datasets(a: &Arc<DatasetSet>, b: &Arc<DatasetSet>) -> bool {
    if Arc::ptr_eq(a, b) {
        return true;
    }
    let (a, b) = (a.datasets.read(), b.datasets.read());
    a.len() == b.len() && a.iter().zip(b.iter()).all(|(a, b)| Arc::ptr_eq(a, b))
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

pub(super) fn node_glyph(node: &Node) -> lod::Glyph {
    let (metres, pixels) = match node.dataset.generation.descriptor().presentation.size {
        gearbox_api::wire::draw_data::PointSize::Metres { radius } => (radius, 0.0),
        gearbox_api::wire::draw_data::PointSize::Pixels { radius } => (0.0, radius),
    };
    lod::Glyph {
        metres: f64::from(node.page.appearance.max_radius_m.max(metres)),
        pixels: f64::from(pixels),
    }
}
