//! Typed occupancy octrees: one compact instanced box per displayed cell.

use std::collections::BTreeMap;
use std::sync::{Arc, mpsc};

use bevy::camera::visibility::RenderLayers;
use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::render_resource::ShaderType;
use bevy::render::sync_component::SyncComponent;
use gearbox_api::DrawStoreRes;
use gearbox_api::draw_data::{
    Charge, CpuBudget, DatasetLifetime, DatasetStorage, DrawingBackend, StorageError,
    StoredPage,
};
use gearbox_api::wire::draw_data::{
    ClipBounds, ColorMode, DataKey, DataShape, DatasetKind, DatasetRevision, OccupancyLeaf,
    OccupancyState, OctreeConfig, OctreeDisplay, PageAddress, Presentation, VoxelKey, VoxelStyle,
};
use gearbox_api::wire::draw_geom::colormap_rgba;

use super::cloud::{MAX_LOCAL_ERROR_M, Records, invalid};
use super::worker::charge;

#[path = "octree_render.rs"]
pub(super) mod render;
#[cfg(test)]
#[path = "octree_tests.rs"]
mod tests;

/// Display state of one drawn cell; `Mixed` only for renderer-side aggregates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Shown {
    Unknown = 0,
    Free = 1,
    Uncertain = 2,
    Occupied = 3,
    Mixed = 4,
}

/// One drawn cell: 32 bytes, rebased to the dataset origin.
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub(super) struct Cell {
    pub center: [f32; 3],
    pub half: f32,
    pub rgba: u32,
    /// Bits 0..3 state, 8..15 depth, 16 aggregate, 17 implicit unknown.
    pub code: u32,
    pub probability: f32,
    pub spare: u32,
}

pub(super) const AGGREGATE: u32 = 1 << 16;
pub(super) const IMPLICIT: u32 = 1 << 17;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Counts {
    pub leaves: u64,
    pub cells: u64,
    pub occupied: u64,
    pub free: u64,
    pub uncertain: u64,
    pub unknown: u64,
    pub mixed: u64,
    pub aggregated: u64,
}

pub(super) struct CellBuffer {
    pub records: Records<Cell>,
    _charge: Charge,
}

impl CellBuffer {
    pub fn new(budget: &CpuBudget, records: Records<Cell>) -> Result<Arc<Self>, StorageError> {
        Ok(Arc::new(Self { records, _charge: charge(budget, size_of::<Self>() + 16)? }))
    }
}

/// Per-record packed RGBA that overrides the records' own colour, so a recolour
/// uploads 4 bytes a record and leaves the geometry buffer alone.
pub(super) struct ColorBuffer {
    pub values: Records<u32>,
    _charge: Charge,
}

impl ColorBuffer {
    pub fn new(budget: &CpuBudget, values: Records<u32>) -> Result<Arc<Self>, StorageError> {
        Ok(Arc::new(Self { values, _charge: charge(budget, size_of::<Self>() + 16)? }))
    }
}

/// What one page's records draw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(super) enum Shape {
    /// An occupancy cell: a cube per record.
    #[default]
    Boxes,
    /// A surfel: an oriented disc per record.
    Discs,
    /// A mesh: three vertex records per triangle.
    Triangles,
}

#[derive(Clone, ShaderType)]
pub(super) struct Uniform {
    pub world_from_local: Mat4,
    /// x opacity, y glow, z style (0 solid, 1 surface, 2 wire), w edge width.
    pub params: Vec4,
    pub extra: Vec4,
}

/// Drawn cells of one source page, or of the synthetic unknown-ROI page.
pub(super) struct PageCells {
    pub address: PageAddress,
    pub source: Option<Arc<StoredPage>>,
    pub cells: Arc<CellBuffer>,
    pub rebase_m: [f64; 3],
    pub counts: Counts,
    pub opaque: bool,
    pub shape: Shape,
    /// Source IDs of disc records, in record order.
    pub ids: Option<Arc<Records<u64>>>,
    /// Digest of derived records, for reuse across generations; zero for stored pages.
    pub basis: u64,
    /// Colours drawn instead of the records' own, one per record.
    pub colors: Option<Arc<ColorBuffer>>,
}

pub(super) struct Prepared {
    pub key: DataKey,
    pub revision: DatasetRevision,
    pub lifetime: DatasetLifetime,
    pub storage: Arc<DatasetStorage>,
    pub pages: Vec<Arc<PageCells>>,
    pub counts: Counts,
    pub style: u64,
    pub uniform: Uniform,
    _charge: Charge,
}

impl Prepared {
    pub fn cells(&self) -> impl Iterator<Item = &Cell> + '_ {
        self.pages
            .iter()
            .flat_map(|page| page.cells.records.values.iter())
    }
}

/// Address of the synthetic page holding implicit unknown ROI cells.
const UNKNOWN_PAGE: PageAddress = PageAddress {
    scan_key: u64::MAX,
    page_id: 0,
};

pub(super) fn rgba(color: [u8; 4], opacity: f32) -> u32 {
    let alpha = (f32::from(color[3]) * opacity.clamp(0.0, 1.0)).round() as u8;
    u32::from_le_bytes([color[0], color[1], color[2], alpha])
}

fn shown(state: OccupancyState) -> Shown {
    match state {
        OccupancyState::Unknown => Shown::Unknown,
        OccupancyState::Free => Shown::Free,
        OccupancyState::Uncertain => Shown::Uncertain,
        OccupancyState::Occupied => Shown::Occupied,
    }
}

fn visible(display: &OctreeDisplay, state: Shown) -> bool {
    match state {
        Shown::Occupied => display.show_occupied,
        Shown::Free => display.show_free,
        Shown::Uncertain => display.show_uncertain,
        Shown::Unknown => display.show_unknown,
        Shown::Mixed => display.show_occupied || display.show_uncertain,
    }
}

fn color(
    presentation: &Presentation,
    config: &OctreeConfig,
    state: Shown,
    probability: Option<f32>,
    depth: u8,
) -> [u8; 4] {
    match &presentation.color {
        ColorMode::Occupancy {
            occupied,
            free,
            uncertain,
            unknown,
            mixed,
        } => match state {
            Shown::Occupied => *occupied,
            Shown::Free => *free,
            Shown::Uncertain => *uncertain,
            Shown::Unknown => *unknown,
            Shown::Mixed => *mixed,
        },
        ColorMode::OccupancyProbability { colormap, unknown } => match probability {
            Some(p) => colormap_rgba(*colormap, p, 0.0, 1.0),
            None => *unknown,
        },
        ColorMode::OctreeDepth { colormap } => colormap_rgba(
            *colormap,
            f32::from(depth) / f32::from(config.max_depth.max(1)),
            0.0,
            1.0,
        ),
        ColorMode::Constant { rgba } => *rgba,
        _ => [220, 220, 220, 255],
    }
}

/// Tagged aggregate of leaves below the display depth: occupied if any is,
/// free only when every finest cell is observed free, mixed otherwise.
#[derive(Default)]
struct Aggregate {
    occupied: bool,
    other: bool,
    free_cells: u128,
    probability: Option<f32>,
}

fn ancestor(key: &VoxelKey, depth: u8) -> VoxelKey {
    VoxelKey {
        tile: key.tile,
        depth,
        morton: key.morton >> (3 * u32::from(key.depth - depth)),
    }
}

fn cells_below(config: &OctreeConfig, depth: u8) -> u128 {
    1u128 << (3 * u32::from(config.max_depth - depth))
}

/// Cell centre and half size in octree-frame metres.
pub(super) fn geometry(config: &OctreeConfig, key: &VoxelKey) -> Result<(DVec3, f64), StorageError> {
    let bounds = key.grid_bounds(config).map_err(invalid)?;
    let size = bounds.size as f64 * config.leaf_resolution_m;
    let min = DVec3::new(
        bounds.min[0] as f64,
        bounds.min[1] as f64,
        bounds.min[2] as f64,
    ) * config.leaf_resolution_m;
    Ok((min + DVec3::splat(size * 0.5), size * 0.5))
}

/// Key of the depth-`depth` cell containing integer leaf-unit coordinate `coord`.
fn key_at(config: &OctreeConfig, depth: u8, coord: [i128; 3]) -> VoxelKey {
    let root = 1i128 << config.max_depth;
    let size = 1i128 << (config.max_depth - depth);
    let mut tile = [0i64; 3];
    let mut local = [0u64; 3];
    for axis in 0..3 {
        tile[axis] = coord[axis].div_euclid(root) as i64;
        local[axis] = (coord[axis].rem_euclid(root) / size) as u64;
    }
    let mut morton = 0u64;
    for level in (0..u32::from(depth)).rev() {
        let child =
            (local[0] >> level & 1) | (local[1] >> level & 1) << 1 | (local[2] >> level & 1) << 2;
        morton = morton << 3 | child;
    }
    VoxelKey {
        tile,
        depth,
        morton,
    }
}

struct Spans(Vec<Span>);

/// Finest-depth interval of a key inside its tile.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Span {
    tile: [i64; 3],
    lo: u64,
    hi: u64,
}

impl Span {
    fn of(config: &OctreeConfig, key: &VoxelKey) -> Self {
        let shift = 3 * u32::from(config.max_depth - key.depth);
        let lo = key.morton << shift;
        Self {
            tile: key.tile,
            lo,
            hi: lo | ((1u64 << shift) - 1),
        }
    }
}

impl Spans {
    fn covers(&self, span: &Span) -> bool {
        let index = self
            .0
            .partition_point(|known| (known.tile, known.lo) <= (span.tile, span.hi));
        index > 0 && {
            let known = &self.0[index - 1];
            known.tile == span.tile && known.hi >= span.lo
        }
    }
}

type Drawn = (VoxelKey, Shown, Option<f32>, u32);

/// Drawn cells of one page's leaves: tagged aggregates below `limit`, then filters.
fn page_cells(
    config: &OctreeConfig,
    limit: u8,
    leaves: impl Iterator<Item = OccupancyLeaf>,
    counts: &mut Counts,
) -> Result<Vec<Drawn>, StorageError> {
    let mut drawn = Vec::new();
    let mut aggregates: BTreeMap<VoxelKey, Aggregate> = BTreeMap::new();
    for leaf in leaves {
        counts.leaves += 1;
        let state = shown(leaf.value.state(config).map_err(invalid)?);
        if leaf.key.depth <= limit {
            drawn.push((leaf.key, state, leaf.value.probability, 0));
            continue;
        }
        let entry = aggregates.entry(ancestor(&leaf.key, limit)).or_default();
        match state {
            Shown::Occupied => entry.occupied = true,
            Shown::Free => entry.free_cells += cells_below(config, leaf.key.depth),
            _ => entry.other = true,
        }
        if let Some(p) = leaf.value.probability {
            entry.probability = Some(entry.probability.map_or(p, |q: f32| q.max(p)));
        }
        counts.aggregated += 1;
    }
    for (key, aggregate) in aggregates {
        let state = if aggregate.occupied {
            Shown::Occupied
        } else if !aggregate.other && aggregate.free_cells == cells_below(config, key.depth) {
            Shown::Free
        } else {
            Shown::Mixed
        };
        drawn.push((key, state, aggregate.probability, AGGREGATE));
    }
    Ok(drawn)
}

/// Whether `clip` keeps a cell: it overlaps the box, or with `invert` is not wholly inside it.
pub(super) fn kept(center: DVec3, half: f64, clip: &ClipBounds) -> bool {
    let to_box = clip
        .rotation_wxyz
        .map_or(bevy::math::DQuat::IDENTITY, |q| bevy::math::DQuat::from_xyzw(q[1], q[2], q[3], q[0]).inverse());
    let at = to_box * center;
    let turn = bevy::math::DMat3::from_quat(to_box);
    let extent = DVec3::from_array(std::array::from_fn(|axis| {
        half * (0..3).map(|column| turn.col(column)[axis].abs()).sum::<f64>()
    }));
    let (low, high) = (DVec3::from_array(clip.min_m), DVec3::from_array(clip.max_m));
    let overlaps = (at + extent).cmpge(low).all() && (at - extent).cmple(high).all();
    if !clip.invert {
        return overlaps;
    }
    !((at - extent).cmpge(low).all() && (at + extent).cmple(high).all())
}

/// Faces covered by a same-state neighbour, one bit per face from here in `code`.
pub(super) const HIDDEN_SHIFT: u32 = 23;
/// Scan-key bit of the translucent part of a page whose other cells are opaque.
pub(super) const TRANSLUCENT_PART: u64 = 1 << 62;

/// Per drawn cell, the faces a drawn cell of the same state covers entirely: a
/// same-depth or coarser neighbour across that face. A finer neighbour covers only part
/// of a face, so that face stays.
fn hidden_faces(budget: &CpuBudget, config: &OctreeConfig, drawn: &[Drawn]) -> Result<Vec<u32>, StorageError> {
    let _scratch = charge(budget, drawn.len().saturating_mul(64))?;
    let states: std::collections::HashMap<VoxelKey, Shown> = drawn.iter().map(|(key, state, ..)| (*key, *state)).collect();
    let mut depths: Vec<u8> = drawn.iter().map(|(key, ..)| key.depth).collect();
    depths.sort_unstable();
    depths.dedup();
    drawn
        .iter()
        .map(|(key, state, ..)| {
            let bounds = key.grid_bounds(config).map_err(invalid)?;
            let mut mask = 0u32;
            for face in 0..6u32 {
                let axis = (face / 2) as usize;
                let mut probe = bounds.min;
                probe[axis] += if face & 1 == 0 { i128::from(bounds.size) } else { -1 };
                let covered = depths
                    .iter()
                    .take_while(|depth| **depth <= key.depth)
                    .any(|depth| states.get(&key_at(config, *depth, probe)) == Some(state));
                mask |= u32::from(covered) << face;
            }
            Ok(mask << HIDDEN_SHIFT)
        })
        .collect()
}

/// Records for drawn cells, rebased to the page's first cell. Opaque cells of a page
/// that also holds translucent ones form a second part, so they keep depth.
fn build(
    budget: &CpuBudget,
    config: &OctreeConfig,
    presentation: &Presentation,
    display: &OctreeDisplay,
    address: PageAddress,
    source: Option<Arc<StoredPage>>,
    mut drawn: Vec<Drawn>,
    mut counts: Counts,
) -> Result<Vec<Arc<PageCells>>, StorageError> {
    drawn.retain(|(_, state, _, _)| visible(display, *state));
    if let Some(clip) = &presentation.clip {
        let mut kept = Vec::with_capacity(drawn.len());
        for cell in drawn {
            let (center, half) = geometry(config, &cell.0)?;
            if self::kept(center, half, clip) {
                kept.push(cell);
            }
        }
        drawn = kept;
    }
    let hidden = match display.style {
        VoxelStyle::Wire => vec![0; drawn.len()],
        _ => hidden_faces(budget, config, &drawn)?,
    };
    let mut cells = Records::<Cell>::new(budget, drawn.len())?;
    let mut rebase_m = [0.0; 3];
    for (index, (key, state, probability, flags)) in drawn.iter().enumerate() {
        let (center, half) = geometry(config, key)?;
        if index == 0 {
            rebase_m = center.to_array();
        }
        let local = center - DVec3::from_array(rebase_m);
        let xyz = local.as_vec3();
        if (xyz.as_dvec3() - local).length() > MAX_LOCAL_ERROR_M {
            return Err(invalid(
                "octree page exceeds one-millimetre local GPU precision",
            ));
        }
        cells.values[index] = Cell {
            center: xyz.to_array(),
            half: half as f32,
            rgba: rgba(
                color(presentation, config, *state, *probability, key.depth),
                presentation.opacity,
            ),
            code: *state as u32 | u32::from(key.depth) << 8 | flags | hidden[index],
            probability: probability.unwrap_or(-1.0),
            spare: 0,
        };
        counts.cells += 1;
        match state {
            Shown::Occupied => counts.occupied += 1,
            Shown::Free => counts.free += 1,
            Shown::Uncertain => counts.uncertain += 1,
            Shown::Unknown => counts.unknown += 1,
            Shown::Mixed => counts.mixed += 1,
        }
    }
    let solid = presentation.opacity >= 1.0 && presentation.blur <= 0.0 && display.style != VoxelStyle::Wire;
    let opaque = |cell: &Cell| solid && cell.rgba >> 24 == 255;
    let part = |records: Records<Cell>, address: PageAddress, counts: Counts, opaque: bool| -> Result<Arc<PageCells>, StorageError> {
        Ok(Arc::new(PageCells {
            address,
            source: source.clone(),
            cells: Arc::new(CellBuffer {
                records,
                _charge: charge(budget, size_of::<CellBuffer>() + 16)?,
            }),
            rebase_m,
            counts,
            opaque,
            shape: Shape::Boxes,
            ids: None,
            basis: 0,
            colors: None,
        }))
    };
    let solid_cells = cells.values.iter().filter(|cell| opaque(cell)).count();
    if solid_cells == 0 || solid_cells == cells.values.len() {
        let whole = solid && solid_cells == cells.values.len();
        return Ok(vec![part(cells, address, counts, whole)?]);
    }
    let mut front = Records::<Cell>::new(budget, solid_cells)?;
    let mut back = Records::<Cell>::new(budget, cells.values.len() - solid_cells)?;
    let (mut f, mut b) = (0, 0);
    for cell in cells.values.iter() {
        if opaque(cell) {
            front.values[f] = *cell;
            f += 1;
        } else {
            back.values[b] = *cell;
            b += 1;
        }
    }
    drop(cells);
    Ok(vec![
        part(front, address, counts, true)?,
        part(back, PageAddress { scan_key: address.scan_key ^ TRANSLUCENT_PART, ..address }, Counts::default(), false)?,
    ])
}

pub(super) fn style_key(presentation: &Presentation) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(presentation)
        .unwrap_or_default()
        .hash(&mut hash);
    hash.finish()
}

/// Builds drawn cells for one octree generation, reusing unchanged pages of `previous`.
pub(super) fn prepare(
    budget: &CpuBudget,
    key: DataKey,
    revision: DatasetRevision,
    lifetime: DatasetLifetime,
    storage: Arc<DatasetStorage>,
    previous: Option<&Prepared>,
) -> Result<Arc<Prepared>, StorageError> {
    let descriptor = storage.descriptor().clone();
    if matches!(descriptor.shape, DataShape::Surfels { .. } | DataShape::MeshBlocks { .. }) {
        return super::surface::prepare(budget, key, revision, lifetime, storage, previous);
    }
    if matches!(descriptor.shape, DataShape::Volume { .. }) {
        return super::volume::prepare(budget, key, revision, lifetime, storage, previous);
    }
    if matches!(descriptor.shape, DataShape::Elevation { .. }) {
        return super::elevation::prepare(budget, key, revision, lifetime, storage, previous);
    }
    let DataShape::Octree { config, .. } = &descriptor.shape else {
        return Err(invalid("not an octree"));
    };
    let presentation = &descriptor.presentation;
    let display = presentation.octree.clone().unwrap_or_default();
    let limit = display
        .depth_limit
        .unwrap_or(config.max_depth)
        .min(config.max_depth);
    let style = style_key(presentation);
    let previous = previous.filter(|previous| previous.style == style);
    let mut pages = Vec::with_capacity(storage.page_len() + 1);
    let mut changed = previous.is_none();
    for page in super::worker::acquire_pages(&storage, &lifetime)? {
        let address = page.descriptor().address;
        let reused: Vec<Arc<PageCells>> = previous
            .iter()
            .flat_map(|previous| previous.pages.iter())
            .filter(|old| {
                old.address.page_id == address.page_id
                    && old.source.as_ref().is_some_and(|old| Arc::ptr_eq(old, &page))
            })
            .cloned()
            .collect();
        if !reused.is_empty() {
            pages.extend(reused);
            continue;
        }
        changed = true;
        let mut counts = Counts::default();
        let leaves = page
            .occupancy_leaves()
            .ok_or_else(|| invalid("octree page is not occupancy"))?;
        let _scratch = charge(budget, leaves.len().saturating_mul(64))?;
        let drawn = page_cells(config, limit, leaves, &mut counts)?;
        pages.extend(build(
            budget,
            config,
            presentation,
            &display,
            address,
            Some(page.clone()),
            drawn,
            counts,
        )?);
    }
    changed |= previous.is_some_and(|previous| {
        previous
            .pages
            .iter()
            .filter(|page| page.source.is_some())
            .count()
            != pages.len()
    });
    if display.show_unknown
        && let Some(roi) = display.roi
    {
        let unknown = |page: &&Arc<PageCells>| {
            page.address.page_id == UNKNOWN_PAGE.page_id && page.address.scan_key | TRANSLUCENT_PART == UNKNOWN_PAGE.scan_key
        };
        let reused: Vec<Arc<PageCells>> = previous
            .filter(|_| !changed)
            .iter()
            .flat_map(|p| p.pages.iter().filter(unknown))
            .cloned()
            .collect();
        if !reused.is_empty() {
            pages.extend(reused);
        } else {
            let mut spans: Vec<Span> = Vec::new();
            for page in pages.iter().filter(|page| page.address.scan_key & TRANSLUCENT_PART == 0) {
                if let Some(keys) = page.source.as_ref().and_then(|source| source.voxel_keys()) {
                    spans.extend(keys.map(|key| Span::of(config, &key)));
                }
            }
            spans.sort_unstable();
            let _scratch = charge(budget, spans.len().saturating_mul(size_of::<Span>()))?;
            let mut drawn = Vec::new();
            unknown_cells(
                config,
                &roi,
                limit,
                display.unknown_cell_budget,
                &Spans(spans),
                &mut drawn,
            )?;
            pages.extend(build(
                budget,
                config,
                presentation,
                &display,
                UNKNOWN_PAGE,
                None,
                drawn,
                Counts::default(),
            )?);
        }
    }
    assemble(budget, key, revision, lifetime, storage, pages, presentation, style)
}

/// One generation's prepared pages with their summed counts and style uniform.
#[allow(clippy::too_many_arguments)]
pub(super) fn assemble(
    budget: &CpuBudget,
    key: DataKey,
    revision: DatasetRevision,
    lifetime: DatasetLifetime,
    storage: Arc<DatasetStorage>,
    pages: Vec<Arc<PageCells>>,
    presentation: &Presentation,
    style: u64,
) -> Result<Arc<Prepared>, StorageError> {
    let display = presentation.octree.clone().unwrap_or_default();
    let mut counts = Counts::default();
    for page in &pages {
        let c = page.counts;
        counts.leaves += c.leaves;
        counts.cells += c.cells;
        counts.occupied += c.occupied;
        counts.free += c.free;
        counts.uncertain += c.uncertain;
        counts.unknown += c.unknown;
        counts.mixed += c.mixed;
        counts.aggregated += c.aggregated;
    }
    let style_code = match display.style {
        VoxelStyle::Solid => 0.0,
        VoxelStyle::Surface => 1.0,
        VoxelStyle::Wire => 2.0,
    };
    Ok(Arc::new(Prepared {
        key,
        revision,
        lifetime,
        storage,
        pages,
        counts,
        style,
        uniform: Uniform {
            world_from_local: Mat4::IDENTITY,
            params: Vec4::new(presentation.opacity, presentation.glow, style_code, 1.5),
            extra: Vec4::new(presentation.blur, 0.0, 0.0, 0.0),
        },
        _charge: charge(budget, size_of::<Prepared>() + 64)?,
    }))
}
/// Adds cells of the ROI that no leaf covers, at the coarsest depth within budget.
fn unknown_cells(
    config: &OctreeConfig,
    roi: &ClipBounds,
    limit: u8,
    budget: u32,
    known: &Spans,
    drawn: &mut Vec<(VoxelKey, Shown, Option<f32>, u32)>,
) -> Result<(), StorageError> {
    roi.validate().map_err(invalid)?;
    let mut depth = limit;
    let range = |depth: u8| {
        let size = config.leaf_resolution_m * f64::from(1u32 << (config.max_depth - depth).min(31));
        let lo: [i128; 3] = std::array::from_fn(|axis| (roi.min_m[axis] / size).floor() as i128);
        let hi: [i128; 3] = std::array::from_fn(|axis| (roi.max_m[axis] / size).ceil() as i128);
        (lo, hi)
    };
    loop {
        let (lo, hi) = range(depth);
        let count = (0..3)
            .map(|axis| (hi[axis] - lo[axis]).max(0) as u128)
            .product::<u128>();
        if count <= u128::from(budget) {
            let size = 1i128 << (config.max_depth - depth);
            for x in lo[0]..hi[0] {
                for y in lo[1]..hi[1] {
                    for z in lo[2]..hi[2] {
                        let key = key_at(config, depth, [x * size, y * size, z * size]);
                        if !known.covers(&Span::of(config, &key)) {
                            drawn.push((key, Shown::Unknown, None, IMPLICIT));
                        }
                    }
                }
            }
            return Ok(());
        }
        if depth == 0 {
            return Ok(());
        }
        depth -= 1;
    }
}

/// Octree generations the renderer currently shows, for visibility receipts.
#[derive(Resource, Clone, Default, bevy::render::extract_resource::ExtractResource)]
pub(super) struct Offered(pub Arc<Vec<Arc<Identity>>>);

/// Which generation a node belongs to, for visibility receipts.
pub(super) struct Identity {
    pub key: DataKey,
    pub revision: DatasetRevision,
    pub leaves: u64,
    pub cells: u64,
    pub pages: usize,
}

#[derive(Component, Clone)]
pub(super) struct Node {
    pub page: Arc<PageCells>,
    pub dataset: Arc<Identity>,
    pub frame: gearbox_api::wire::draw_data::DataFrame,
    /// The octree lattice; `None` for surfels and mesh blocks.
    pub config: Option<OctreeConfig>,
    pub uniform: Uniform,
    pub visible: bool,
    pub placed: bool,
}

impl SyncComponent for Node {
    type Target = Self;
}

impl ExtractComponent for Node {
    type QueryData = (
        &'static Self,
        &'static GlobalTransform,
        &'static InheritedVisibility,
    );
    type QueryFilter = ();
    type Out = Self;
    fn extract_component(
        (node, transform, visibility): bevy::ecs::query::QueryItem<'_, '_, Self::QueryData>,
    ) -> Option<Self> {
        let mut out = node.clone();
        out.uniform.world_from_local = transform.to_matrix();
        out.visible = visibility.get() && node.placed;
        Some(out)
    }
}

impl super::extract::Budgeted for Node {
    fn denied(_: bevy::ecs::query::QueryItem<'_, '_, Self::QueryData>, _: &DrawingBackend) {
        tracing::warn!("octree extraction: CpuBudget");
    }
}

type Entry = (
    DataKey,
    DatasetRevision,
    DatasetLifetime,
    Arc<DatasetStorage>,
);
type Job = (u64, Vec<Entry>);
type Done = (
    u64,
    Vec<Result<Arc<Prepared>, (DataKey, DatasetRevision, StorageError)>>,
);
type Slot = (u64, String, String, PageAddress);

#[derive(Resource)]
struct Runtime {
    backend: DrawingBackend,
    seen: u64,
    sequence: u64,
    jobs: mpsc::Sender<Job>,
    done: std::sync::Mutex<mpsc::Receiver<Done>>,
    nodes: BTreeMap<Slot, Entity>,
    /// Identity of the octree entries last sent; unrelated registry changes send nothing.
    sent: Vec<(DataKey, DatasetRevision, usize)>,
    applied: u64,
}

fn start(mut commands: Commands, store: Res<DrawStoreRes>) {
    let backend = store.backend();
    let (jobs, inbox) = mpsc::channel::<Job>();
    let (outbox, done) = mpsc::channel::<Done>();
    let worker = backend.clone();
    let spawned = std::thread::Builder::new()
        .name("gearbox-octree-prepare".into())
        .spawn(move || {
            let mut cache: Vec<Arc<Prepared>> = Vec::new();
            while let Ok(mut job) = inbox.recv() {
                while let Ok(newer) = inbox.try_recv() {
                    job = newer;
                }
                let (number, entries) = job;
                let results = entries
                    .into_iter()
                    .map(|(key, revision, lifetime, storage)| {
                        let previous = cache.iter().find(|old| {
                            old.key == key && old.revision.incarnation == revision.incarnation
                        });
                        if let Some(old) = previous.filter(|old| {
                            old.revision == revision && Arc::ptr_eq(&old.storage, &storage)
                        }) {
                            return Ok(old.clone());
                        }
                        prepare(
                            worker.budget(),
                            key.clone(),
                            revision,
                            lifetime,
                            storage,
                            previous.map(Arc::as_ref),
                        )
                        .map_err(|error| (key, revision, error))
                    })
                    .collect::<Vec<_>>();
                cache = results
                    .iter()
                    .filter_map(|result| result.as_ref().ok().cloned())
                    .collect();
                if outbox.send((number, results)).is_err() {
                    return;
                }
            }
        });
    if let Err(error) = spawned {
        error!("octree worker: {error}");
        return;
    }
    commands.insert_resource(Runtime {
        backend,
        seen: u64::MAX,
        sequence: 0,
        jobs,
        done: std::sync::Mutex::new(done),
        nodes: BTreeMap::new(),
        sent: Vec::new(),
        applied: 0,
    });
}

fn sync(mut commands: Commands, runtime: Option<ResMut<Runtime>>) {
    let Some(mut runtime) = runtime else { return };
    let registry = runtime.backend.registry();
    let entries = {
        let Ok(registry) = registry.try_lock() else {
            return receive(&mut commands, &mut runtime);
        };
        if registry.revision() == runtime.seen {
            drop(registry);
            return receive(&mut commands, &mut runtime);
        }
        runtime.seen = registry.revision();
        registry
            .datasets()
            .filter(|(_, entry)| matches!(entry.kind, DatasetKind::Octree | DatasetKind::Surfels | DatasetKind::MeshBlocks | DatasetKind::Volume | DatasetKind::Elevation))
            .map(|(_, entry)| {
                (
                    entry.payload.descriptor().key.clone(),
                    entry.revision,
                    entry.lifetime.clone(),
                    entry.payload.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    let identity: Vec<_> = entries
        .iter()
        .map(|(key, revision, _, storage)| (key.clone(), *revision, Arc::as_ptr(storage) as usize))
        .collect();
    if identity != runtime.sent {
        runtime.sent = identity;
        runtime.sequence += 1;
        let job = (runtime.sequence, entries);
        let _ = runtime.jobs.send(job);
    }
    receive(&mut commands, &mut runtime);
}

fn receive(commands: &mut Commands, runtime: &mut Runtime) {
    let latest = {
        let done = runtime.done.lock().unwrap_or_else(|p| p.into_inner());
        let mut latest = None;
        while let Ok(result) = done.try_recv() {
            latest = Some(result);
        }
        latest
    };
    let Some((number, results)) = latest else {
        return;
    };
    if number <= runtime.applied {
        return;
    }
    runtime.applied = number;
    let mut keep: Vec<Slot> = Vec::new();
    let mut offered: Vec<Arc<Identity>> = Vec::new();
    let mut failed: Vec<(u64, String, String)> = Vec::new();
    for result in results {
        match result {
            Ok(prepared) => {
                info!(
                    "octree {}/{} generation={} pages={} cells={}",
                    prepared.key.namespace,
                    prepared.key.id,
                    prepared.revision.generation,
                    prepared.pages.len(),
                    prepared.counts.cells
                );
                let shape = &prepared.storage.descriptor().shape;
                let Some(frame) = shape.frame() else {
                    continue;
                };
                let config = match shape {
                    DataShape::Octree { config, .. } => Some(*config),
                    _ => None,
                };
                let identity = Arc::new(Identity {
                    key: prepared.key.clone(),
                    revision: prepared.revision,
                    leaves: prepared.counts.leaves,
                    cells: prepared.counts.cells,
                    pages: prepared
                        .pages
                        .iter()
                        .filter(|page| !page.cells.records.values.is_empty())
                        .count(),
                });
                offered.push(identity.clone());
                for page in &prepared.pages {
                    if page.cells.records.values.is_empty() {
                        continue;
                    }
                    let slot = (
                        prepared.revision.incarnation,
                        prepared.key.namespace.clone(),
                        prepared.key.id.clone(),
                        page.address,
                    );
                    let node = Node {
                        page: page.clone(),
                        dataset: identity.clone(),
                        frame: frame.clone(),
                        config,
                        uniform: prepared.uniform.clone(),
                        visible: false,
                        placed: false,
                    };
                    if let Some(entity) = runtime.nodes.get(&slot) {
                        commands.entity(*entity).insert(node);
                    } else {
                        let entity = commands
                            .spawn((
                                node,
                                Transform::IDENTITY,
                                Visibility::Inherited,
                                RenderLayers::layer(super::super::DRAW_LAYER),
                            ))
                            .id();
                        runtime.nodes.insert(slot.clone(), entity);
                    }
                    keep.push(slot);
                }
            }
            Err((key, revision, error)) => {
                warn!("octree {}/{}: {error}", key.namespace, key.id);
                let reason = super::retirement::Text::new(&error);
                if let Ok(diagnostic) = gearbox_api::draw_data::RenderDiagnostic::new(
                    gearbox_api::wire::draw_data::DataPhase::Preparing,
                    gearbox_api::wire::draw_data::data_code::INVALID,
                    reason.as_str(),
                ) {
                    let _ = runtime
                        .backend
                        .report_renderer_diagnostic(&key, revision, diagnostic);
                }
                failed.push((revision.incarnation, key.namespace, key.id));
            }
        }
    }
    commands.insert_resource(Offered(Arc::new(offered)));
    runtime.nodes.retain(|slot, entity| {
        let live =
            keep.contains(slot) || failed.contains(&(slot.0, slot.1.clone(), slot.2.clone()));
        if !live {
            commands.entity(*entity).try_despawn();
        }
        live
    });
}

fn place(
    mut commands: Commands,
    sites: Option<Res<crate::globe::Sites>>,
    grids: Query<&big_space::prelude::Grid>,
    toggles: Res<crate::viewer::overlays::DisplayToggles>,
    inventory: Res<crate::controller::ControllerInventory>,
    index: Res<crate::prim_index::PrimIndex>,
    globals: Query<&GlobalTransform>,
    mut nodes: Query<(Entity, &mut Node, &mut Transform, Option<&ChildOf>)>,
) {
    let Some(sites) = sites else { return };
    let home = sites.home();
    for (entity, mut node, mut transform, parent) in &mut nodes {
        node.placed = false;
        if !toggles.show_draws {
            continue;
        }
        let Some((position, rotation)) =
            super::runtime::site_pose(&node.frame, [0.0; 3], node.page.rebase_m, home)
        else {
            if let Some((target, local)) =
                super::runtime::link_placement(&node.frame, [0.0; 3], node.page.rebase_m, &inventory, &index, &globals)
            {
                *transform = local;
                commands.entity(entity).remove::<big_space::prelude::CellCoord>();
                if parent.is_none_or(|parent| parent.parent() != target) {
                    commands.entity(entity).insert(ChildOf(target));
                }
                node.placed = true;
            }
            continue;
        };
        let Ok(grid) = grids.get(home.entity) else {
            continue;
        };
        if !position.is_finite() {
            continue;
        }
        let (cell, local) = grid.translation_to_grid(position);
        *transform = Transform::from_translation(local).with_rotation(rotation.as_quat());
        commands.entity(entity).insert(cell);
        if parent.is_none_or(|parent| parent.parent() != home.entity) {
            commands.entity(entity).insert(ChildOf(home.entity));
        }
        node.placed = true;
    }
}

pub(super) fn install(app: &mut App) {
    app.init_resource::<Offered>()
        .add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
            Offered,
        >::default())
        .add_systems(Startup, start)
        .add_systems(
            Update,
            (sync, place)
                .chain()
                .in_set(super::DataSystems)
                .after(gearbox_api::DrawSystems::Serve),
        );
    super::extract::install::<Node>(app);
    render::install(app);
}
