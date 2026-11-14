use std::sync::{Arc, Weak};

use bevy::math::DMat4;
use gearbox_api::draw_data::{CpuBudget, MemoryClass, StorageError};

use super::cloud::{Geometry, SpatialMetadata, invalid};
use super::owned::List;

#[path = "lod_projection.rs"]
mod projection;
use projection::Projection;
pub(super) use projection::{Glyph, View};

impl View {
    pub(super) fn page_error(
        self,
        metadata: &SpatialMetadata,
        world: DMat4,
        glyph: Glyph,
    ) -> Option<f64> {
        let root = metadata.hierarchy.nodes.values.first()?;
        let projection = self.project(
            world,
            metadata.rebase_m,
            root.bounds_m,
            metadata.max_error_m,
            glyph,
        );
        projection.visible.then(|| projection.error())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct PageKey {
    pub incarnation: u64,
    pub scan: u64,
    pub page: u64,
}

pub(super) struct Input<'a> {
    pub key: PageKey,
    pub geometry: Weak<Geometry>,
    pub metadata: &'a Arc<SpatialMetadata>,
    pub world_from_local: DMat4,
    pub glyph: Glyph,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Limits {
    pub points: u32,
    pub bytes: u64,
    pub bytes_per_point: u32,
}

#[derive(Debug)]
pub(super) enum Error {
    Storage(StorageError),
    PointBudget { minimum: usize, available: usize },
    UnavailableRoot(PageKey),
}

impl From<StorageError> for Error {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Samples {
    pub page: PageKey,
    pub node: u32,
    pub full: bool,
}

pub(super) struct Page {
    pub key: PageKey,
    pub geometry: Weak<Geometry>,
    metadata: Weak<SpatialMetadata>,
    pub first: u32,
    pub count: u32,
    statistics: Statistics,
    _identity_charge: gearbox_api::draw_data::Charge,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Piece {
    pub page: u32,
    pub node: u32,
    pub first: u32,
    pub count: u32,
    pub full: bool,
    pub selected: u32,
    pub dataset_root: bool,
    pub represented: u64,
    pub error_px: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Statistics {
    pub valid: u64,
    pub represented: u64,
    pub selected: u64,
    pub culled: u64,
    pub pending_branches: u64,
    pub budget_limited_branches: u64,
    pub max_error_px: f64,
}

pub(super) struct Cut {
    pub pages: List<Page>,
    pub pieces: List<Piece>,
    pub indices: List<u32>,
    pub statistics: Statistics,
}

impl Cut {
    pub fn retain(&mut self, live: impl Fn(u64) -> bool) {
        if self.pages.iter().all(|page| live(page.key.incarnation)) {
            return;
        }
        self.pieces
            .retain(|piece| live(self.pages[piece.page as usize].key.incarnation));
        let mut page_index = 0;
        let mut piece_index = 0;
        let mut ordinal = 0;
        let materialized = !self.indices.is_empty();
        let mut statistics = Statistics::default();
        for (old_index, page) in self.pages.iter_mut().enumerate() {
            if !live(page.key.incarnation) {
                continue;
            }
            while let Some(piece) = self.pieces.get_mut(piece_index)
                && piece.page as usize == old_index
            {
                piece.page = page_index;
                piece_index += 1;
            }
            if materialized {
                let first = page.first as usize;
                self.indices
                    .copy_within(first..first + page.count as usize, ordinal);
            }
            page.first = ordinal as u32;
            ordinal += page.count as usize;
            page_index += 1;
            let stats = page.statistics;
            statistics.valid += stats.valid;
            statistics.represented += stats.represented;
            statistics.selected += stats.selected;
            statistics.pending_branches += stats.pending_branches;
            statistics.budget_limited_branches += stats.budget_limited_branches;
            statistics.max_error_px = statistics.max_error_px.max(stats.max_error_px);
        }
        self.pages.retain(|page| live(page.key.incarnation));
        while self.indices.len() > ordinal {
            self.indices.pop();
        }
        statistics.culled = statistics.valid - statistics.represented;
        self.statistics = statistics;
        if self.pages.is_empty() {
            self.pages = List::default();
            self.pieces = List::default();
            self.indices = List::default();
        }
    }

    fn expanded_dataset(&self, input: &Input<'_>) -> bool {
        let Ok(page) = self.pages.binary_search_by_key(&input.key, |page| page.key) else {
            return false;
        };
        Weak::ptr_eq(&self.pages[page].metadata, &Arc::downgrade(input.metadata))
            && self
                .pieces
                .iter()
                .any(|piece| piece.page as usize == page && !piece.dataset_root)
    }

    fn refined(&self, input: &Input<'_>, node: u32, selected: u16) -> bool {
        let Ok(page) = self.pages.binary_search_by_key(&input.key, |page| page.key) else {
            return false;
        };
        if !Weak::ptr_eq(&self.pages[page].metadata, &Arc::downgrade(input.metadata)) {
            return false;
        }
        let bounds = input.metadata.hierarchy.nodes.values[node as usize];
        let first = self
            .pieces
            .partition_point(|piece| (piece.page, piece.first) < (page as u32, bounds.first));
        self.pieces.get(first).is_some_and(|piece| {
            !piece.dataset_root
                && piece.page == page as u32
                && piece.first >= bounds.first
                && piece.first + piece.count <= bounds.first + bounds.count
                && (piece.node != node || piece.selected > u32::from(selected))
        })
    }
}

#[derive(Clone, Copy)]
enum Branch {
    Dataset { start: usize, end: usize },
    Node { page: usize, node: u32 },
}

struct Prepared<'a, 'g> {
    input: &'a Input<'g>,
    clip_from_local: DMat4,
    root: Projection,
    statistics: Statistics,
}

#[derive(Clone, Copy)]
struct Entry {
    branch: Branch,
    representative: usize,
    active: bool,
    full: bool,
    selected: u16,
    samples: u64,
    error: f64,
    /// Sampling slope of a leaf, formed on its first refinement; NaN until then.
    slope: f64,
}

#[derive(Clone, Copy)]
struct Candidate {
    entry: usize,
    error: f64,
}

impl Candidate {
    fn higher(self, other: Self) -> bool {
        self.error
            .total_cmp(&other.error)
            .then_with(|| other.entry.cmp(&self.entry))
            .is_gt()
    }
}

fn push(heap: &mut List<Candidate>, value: Candidate) -> Result<(), StorageError> {
    heap.push(value)?;
    let mut index = heap.len() - 1;
    while index > 0 {
        let parent = (index - 1) / 2;
        if !heap[index].higher(heap[parent]) {
            break;
        }
        heap.swap(index, parent);
        index = parent;
    }
    Ok(())
}

fn pop(heap: &mut List<Candidate>) -> Option<Candidate> {
    let last = heap.pop()?;
    if heap.is_empty() {
        return Some(last);
    }
    let result = std::mem::replace(&mut heap[0], last);
    let mut index = 0;
    loop {
        let left = 2 * index + 1;
        if left >= heap.len() {
            break;
        }
        let right = left + 1;
        let child = if right < heap.len() && heap[right].higher(heap[left]) {
            right
        } else {
            left
        };
        if !heap[child].higher(heap[index]) {
            break;
        }
        heap.swap(child, index);
        index = child;
    }
    Some(result)
}

struct Builder<'a, 'g, 'p> {
    pages: List<Prepared<'a, 'g>>,
    entries: List<Entry>,
    heap: List<Candidate>,
    previous: Option<&'p Cut>,
    view: View,
}

impl Builder<'_, '_, '_> {
    fn projection(&self, page: usize, node: u32) -> Projection {
        let input = self.pages[page].input;
        self.view.project_local(
            self.pages[page].clip_from_local,
            input.world_from_local,
            input.metadata.rebase_m,
            input.metadata.hierarchy.nodes.values[node as usize].bounds_m,
            input.metadata.max_error_m,
            input.glyph,
        )
    }

    fn add(&mut self, entry: Entry) -> Result<(), StorageError> {
        let index = self.entries.len();
        self.entries.push(entry)?;
        self.queue(index)
    }

    fn queue(&mut self, index: usize) -> Result<(), StorageError> {
        let entry = self.entries[index];
        let refined = self.previous.is_some_and(|previous| match entry.branch {
            Branch::Dataset { start, end } => self.pages[start..end]
                .iter()
                .any(|page| previous.expanded_dataset(page.input)),
            Branch::Node { page, node } => {
                previous.refined(self.pages[page].input, node, entry.selected)
            }
        });
        let threshold = self.view.error_px
            * if refined {
                1.0 - self.view.hysteresis
            } else {
                1.0 + self.view.hysteresis
            };
        if entry.error > threshold {
            push(
                &mut self.heap,
                Candidate {
                    entry: index,
                    error: entry.error,
                },
            )?;
        }
        Ok(())
    }

    fn node(&self, page: usize, node: u32) -> Entry {
        let geometry = self.pages[page].input.metadata;
        let source = geometry.hierarchy.nodes.values[node as usize];
        Entry {
            branch: Branch::Node { page, node },
            representative: page,
            active: true,
            full: false,
            selected: 1,
            samples: u64::from(source.count),
            error: if source.count == 1 {
                0.0
            } else {
                self.projection(page, node).error()
            },
            slope: f64::NAN,
        }
    }

    fn children(&self, entry: Entry, mut visit: impl FnMut(usize, u32)) {
        match entry.branch {
            Branch::Dataset { start, end } => {
                for page in start..end {
                    if self.pages[page].root.visible {
                        visit(page, 0);
                    }
                }
            }
            Branch::Node { page, node } => {
                if let Some(children) =
                    self.pages[page].input.metadata.hierarchy.nodes.values[node as usize].children
                {
                    for child in children {
                        if self.projection(page, child).visible {
                            visit(page, child);
                        }
                    }
                }
            }
        }
    }
}

pub(super) fn select(
    budget: &CpuBudget,
    inputs: &[Input<'_>],
    view: View,
    limits: Limits,
    previous: Option<&Cut>,
    ready: impl Fn(Samples) -> bool,
    check: impl Fn() -> Result<(), StorageError>,
) -> Result<Cut, Error> {
    check()?;
    if !view.valid() || limits.bytes_per_point == 0 {
        return Err(invalid("invalid LOD view or byte cost").into());
    }
    let cap =
        u64::from(limits.points).min(limits.bytes / u64::from(limits.bytes_per_point)) as usize;
    let mut pages = List::new(budget, inputs.len(), MemoryClass::Scratch)?;
    let mut capacity = inputs.len();
    let mut statistics = Statistics::default();
    for input in inputs {
        check()?;
        if !input.world_from_local.is_finite()
            || input.world_from_local.x_axis.w != 0.0
            || input.world_from_local.y_axis.w != 0.0
            || input.world_from_local.z_axis.w != 0.0
            || input.world_from_local.w_axis.w != 1.0
            || !input.glyph.metres.is_finite()
            || !input.glyph.pixels.is_finite()
            || input.glyph.metres < 0.0
            || input.glyph.pixels < 0.0
        {
            return Err(invalid("invalid LOD placement or glyph extent").into());
        }
        capacity = capacity
            .checked_add(input.metadata.hierarchy.nodes.values.len())
            .ok_or(StorageError::Allocation)?;
        statistics.valid = statistics
            .valid
            .checked_add(input.metadata.hierarchy.order.values.len() as u64)
            .ok_or(StorageError::Allocation)?;
        let clip_from_local = view.clip_from_world * input.world_from_local;
        let root = input
            .metadata
            .bounds_m
            .map_or(Projection::empty(), |bounds| {
                view.project_local(
                    clip_from_local,
                    input.world_from_local,
                    input.metadata.rebase_m,
                    bounds,
                    input.metadata.max_error_m,
                    input.glyph,
                )
            });
        pages.push(Prepared {
            input,
            clip_from_local,
            root,
            statistics: Statistics {
                valid: input.metadata.hierarchy.order.values.len() as u64,
                ..Default::default()
            },
        })?;
    }
    pages.sort_unstable_by_key(|page| page.input.key);
    if pages
        .windows(2)
        .any(|pair| pair[0].input.key == pair[1].input.key)
    {
        return Err(invalid("duplicate LOD page identity").into());
    }
    let mut builder = Builder {
        pages,
        entries: List::new(budget, capacity, MemoryClass::Scratch)?,
        heap: List::new(budget, capacity, MemoryClass::Scratch)?,
        previous,
        view,
    };
    let mut selected = 0usize;
    let mut start = 0;
    while start < builder.pages.len() {
        check()?;
        let incarnation = builder.pages[start].input.key.incarnation;
        let end = start
            + builder.pages[start..]
                .partition_point(|page| page.input.key.incarnation == incarnation);
        let mut projection = Projection::empty();
        let mut representative = None;
        let mut samples = 0u64;
        for (offset, page) in builder.pages[start..end].iter().enumerate() {
            if page.root.visible {
                representative.get_or_insert(start + offset);
                projection = projection.union(page.root);
                samples += page.input.metadata.hierarchy.order.values.len() as u64;
            }
        }
        if let Some(representative) = representative {
            let key = builder.pages[representative].input.key;
            if !ready(Samples {
                page: key,
                node: 0,
                full: false,
            }) {
                return Err(Error::UnavailableRoot(key));
            }
            builder.add(Entry {
                branch: Branch::Dataset { start, end },
                representative,
                active: true,
                full: false,
                samples,
                selected: 1,
                error: if samples == 1 {
                    0.0
                } else {
                    projection.error()
                },
                slope: f64::NAN,
            })?;
            selected += 1;
        }
        start = end;
    }
    if selected > cap {
        return Err(Error::PointBudget {
            minimum: selected,
            available: cap,
        });
    }
    while let Some(candidate) = pop(&mut builder.heap) {
        check()?;
        let entry = builder.entries[candidate.entry];
        let leaf = match entry.branch {
            Branch::Node { page, node }
                if builder.pages[page].input.metadata.hierarchy.nodes.values[node as usize]
                    .children
                    .is_none() =>
            {
                Some((page, node))
            }
            _ => None,
        };
        let mut replacement = 0usize;
        let mut available = true;
        if let Some((page, node)) = leaf {
            replacement = (usize::from(entry.selected) * 2).min(entry.samples as usize);
            available = ready(Samples {
                page: builder.pages[page].input.key,
                node,
                full: true,
            });
        } else {
            builder.children(entry, |page, node| {
                replacement += 1;
                available &= ready(Samples {
                    page: builder.pages[page].input.key,
                    node,
                    full: false,
                });
            });
        }
        if !available {
            statistics.pending_branches += 1;
            builder.pages[entry.representative]
                .statistics
                .pending_branches += 1;
            continue;
        }
        if replacement > cap - selected + usize::from(entry.selected) {
            statistics.budget_limited_branches += 1;
            builder.pages[entry.representative]
                .statistics
                .budget_limited_branches += 1;
            continue;
        }
        selected = selected - usize::from(entry.selected) + replacement;
        if let Some((page, node)) = leaf {
            let input = builder.pages[page].input;
            let bounds = input.metadata.hierarchy.nodes.values[node as usize];
            let full = replacement as u64 == entry.samples;
            let error = if full {
                0.0
            } else {
                let level = replacement.ilog2() as usize;
                let slope = if entry.slope.is_nan() {
                    builder.view.sampling_slope(
                        builder.pages[page].clip_from_local,
                        input.metadata.rebase_m,
                        bounds.bounds_m,
                        input.metadata.max_error_m,
                    )
                } else {
                    entry.slope
                };
                builder.entries[candidate.entry].slope = slope;
                projection::sampled(
                    slope,
                    input.metadata.max_error_m,
                    input.metadata.hierarchy.refinement.errors_m.values[node as usize][level],
                )
                .min(entry.error)
            };
            let target = &mut builder.entries[candidate.entry];
            target.full = full;
            target.selected = replacement as u16;
            target.error = error;
            builder.queue(candidate.entry)?;
        } else {
            builder.entries[candidate.entry].active = false;
            match entry.branch {
                Branch::Dataset { start, end } => {
                    for page in start..end {
                        if builder.pages[page].root.visible {
                            builder.add(builder.node(page, 0))?;
                        }
                    }
                }
                Branch::Node { page, node } => {
                    for child in builder.pages[page].input.metadata.hierarchy.nodes.values
                        [node as usize]
                        .children
                        .unwrap()
                    {
                        if builder.projection(page, child).visible {
                            builder.add(builder.node(page, child))?;
                        }
                    }
                }
            }
        }
    }
    let count = builder.entries.iter().filter(|entry| entry.active).count();
    let mut pieces = List::new(budget, count, MemoryClass::Metadata)?;
    for entry in builder.entries.iter().filter(|entry| entry.active) {
        check()?;
        let (page, node, dataset_root) = match entry.branch {
            Branch::Dataset { .. } => (entry.representative, 0, true),
            Branch::Node { page, node } => (page, node, false),
        };
        let source = builder.pages[page].input.metadata.hierarchy.nodes.values[node as usize];
        pieces.push(Piece {
            page: page as u32,
            node,
            first: source.first,
            count: source.count,
            full: entry.full,
            selected: u32::from(entry.selected),
            dataset_root,
            represented: entry.samples,
            error_px: entry.error,
        })?;
        statistics.represented += entry.samples;
        statistics.max_error_px = statistics.max_error_px.max(entry.error);
        let stats = &mut builder.pages[page].statistics;
        stats.represented += entry.samples;
        stats.max_error_px = stats.max_error_px.max(entry.error);
    }
    pieces.sort_unstable_by_key(|piece| (piece.page, piece.first));
    let mut output = List::new(budget, inputs.len(), MemoryClass::Metadata)?;
    let mut indices = List::new(budget, selected, MemoryClass::Metadata)?;
    let mut cursor = 0;
    for (page, prepared) in builder.pages.iter().enumerate() {
        check()?;
        let first = indices.len();
        let tree = &prepared.input.metadata.hierarchy;
        while cursor < pieces.len() && pieces[cursor].page as usize == page {
            check()?;
            let piece = pieces[cursor];
            if piece.selected > 1 {
                for &index in &tree.refinement.order.values
                    [piece.first as usize..(piece.first + piece.selected) as usize]
                {
                    indices.push(index)?;
                }
            } else {
                indices.push(tree.nodes.values[piece.node as usize].representative)?;
            }
            cursor += 1;
        }
        output.push(Page {
            key: prepared.input.key,
            geometry: prepared.input.geometry.clone(),
            metadata: Arc::downgrade(prepared.input.metadata),
            first: first as u32,
            count: (indices.len() - first) as u32,
            statistics: Statistics {
                selected: (indices.len() - first) as u64,
                ..prepared.statistics
            },
            _identity_charge: super::worker::charge(
                budget,
                size_of::<Geometry>() + size_of::<SpatialMetadata>() + 32,
            )?,
        })?;
    }
    statistics.selected = indices.len() as u64;
    statistics.culled = statistics.valid - statistics.represented;
    check()?;
    Ok(Cut {
        pages: output,
        pieces,
        indices,
        statistics,
    })
}

#[cfg(test)]
#[path = "lod_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "lod_quality_tests.rs"]
mod quality_tests;
