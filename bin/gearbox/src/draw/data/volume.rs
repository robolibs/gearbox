//! Sparse volumes drawn as slices for the compact instance renderer. Every
//! slice cell samples the field at its centre with validity-aware
//! interpolation and becomes two unlit triangles. Unknown and unallocated
//! support draws as a two-tone checker, never as a palette colour; contour
//! cells mark where neighbouring cells straddle a level. Cells are grouped by
//! the block holding their centre, and a group whose records did not change
//! keeps its previous buffer, so a block delta uploads only what it moved.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use bevy::math::DVec3;
use gearbox_api::draw_data::{CpuBudget, DatasetLifetime, DatasetStorage, StorageError};
use gearbox_api::wire::draw_data::volume::{BlockView, Channel, Support, Volume};
use gearbox_api::wire::draw_data::{
    ColorMode, DataKey, DataShape, DatasetRevision, PageAddress, Presentation, VolumeField, VolumeSlice, VolumeView,
    volume_page_id,
};
use gearbox_api::wire::draw_geom::colormap_rgba;

use super::cloud::{MAX_LOCAL_ERROR_M, Records, invalid};
use super::octree::{Cell, CellBuffer, Counts, PageCells, Prepared, Shape};
use super::surface::ARROW;

#[cfg(test)]
#[path = "volume_tests.rs"]
mod tests;

/// A slice cell: unlit, and pickable for its field value.
pub(super) const SLICE: u32 = 1 << 20;
/// A slice cell whose support is unknown or unallocated.
pub(super) const UNKNOWN: u32 = 1 << 21;
/// A slice border quad: `quad.x` runs along its edge, drawn with corner brackets.
pub(super) const FRAME: u32 = 1 << 4;
/// Scan keys of slice pages: slice `s` uses `SLICE_SCAN - s`; its border uses page id `u64::MAX`.
pub(super) const SLICE_SCAN: u64 = u64::MAX - 1;
const BORDER_PAGE: u64 = u64::MAX;

/// One sampled slice cell.
#[derive(Clone, Copy)]
struct Sample {
    value: Result<f64, Support>,
    color: Option<[u8; 4]>,
}

fn channel(presentation: &Presentation) -> Channel {
    match &presentation.color {
        ColorMode::Scalar { name, .. } if name == "weight" => Channel::Weight,
        _ => Channel::Value,
    }
}

fn darker(rgba: [u8; 4]) -> [u8; 4] {
    [0, 1, 2, 3].map(|k| if k == 3 { rgba[3] } else { (f32::from(rgba[k]) * 0.55) as u8 })
}

/// The colour of a valid sample under the dataset's colour mode.
fn colour(presentation: &Presentation, value: f64, voxel: Option<[u8; 4]>) -> Result<[u8; 4], StorageError> {
    match &presentation.color {
        ColorMode::Constant { rgba } => Ok(*rgba),
        ColorMode::Scalar { colormap, range, .. } => {
            Ok(colormap_rgba(*colormap, value as f32, range[0] as f32, range[1] as f32))
        }
        ColorMode::Attribute { .. } => Ok(voxel.unwrap_or([128, 128, 128, 255])),
        _ => Err(invalid("volume slices take constant, scalar or per-voxel colours")),
    }
}

fn sample_slice(volume: &Volume<'_>, view: &VolumeView, slice: &VolumeSlice, channel: Channel, colors: bool) -> Vec<Sample> {
    let [nx, ny] = slice.cells();
    let mut samples = Vec::with_capacity((nx * ny) as usize);
    for j in 0..ny {
        for i in 0..nx {
            let p = slice.point(i, j);
            let value = volume.at(p, channel, view.interpolation);
            let color = colors.then(|| volume.voxel_color(volume.voxel_of(p))).flatten();
            samples.push(Sample { value, color });
        }
    }
    samples
}

/// Whether cell `(i, j)` and a valid neighbour straddle one of `levels`.
fn on_contour(samples: &[Sample], nx: u64, ny: u64, i: u64, j: u64, levels: &[f64]) -> bool {
    let Ok(a) = samples[(j * nx + i) as usize].value else {
        return false;
    };
    let mut neighbours = [(i + 1, j), (i, j + 1)].into_iter().filter(|(x, y)| *x < nx && *y < ny);
    neighbours.any(|(x, y)| {
        samples[(y * nx + x) as usize]
            .value
            .is_ok_and(|b| a != b && levels.iter().any(|level| (a - level) * (b - level) <= 0.0))
    })
}

fn record(at: DVec3, rebase: DVec3, rgba: [u8; 4], opacity: f32, code: u32, value: f32) -> Result<Cell, StorageError> {
    let local = at - rebase;
    let xyz = local.as_vec3();
    if (xyz.as_dvec3() - local).length() > MAX_LOCAL_ERROR_M {
        return Err(invalid("slice group exceeds one-millimetre local GPU precision"));
    }
    Ok(Cell { center: xyz.to_array(), half: 0.0, rgba: super::octree::rgba(rgba, opacity), code, probability: value, spare: 0 })
}

/// Two triangles covering the cell centred at `at`.
fn quad(cells: &mut Vec<Cell>, slice: &VolumeSlice, at: DVec3, rebase: DVec3, rgba: [u8; 4], opacity: f32, code: u32, value: f32) -> Result<(), StorageError> {
    let (u, v) = (DVec3::from_array(slice.u) * slice.cell_m * 0.5, DVec3::from_array(slice.v) * slice.cell_m * 0.5);
    let corners = [at - u - v, at + u - v, at + u + v, at - u + v];
    for k in [0, 1, 2, 0, 2, 3] {
        cells.push(record(corners[k], rebase, rgba, opacity, code, value)?);
    }
    Ok(())
}

/// Records grouped by the block containing each cell centre, in a stable order.
type Groups = Vec<(u64, Vec<Cell>, u64, DVec3)>;

fn slice_groups(
    field: &VolumeField,
    presentation: &Presentation,
    view: &VolumeView,
    slice: &VolumeSlice,
    samples: &[Sample],
) -> Result<Groups, StorageError> {
    let [nx, ny] = slice.cells();
    let block_m = field.voxel_m * f64::from(field.block_side);
    let mut groups: HashMap<u64, (Vec<Cell>, Option<DVec3>, u64)> = HashMap::new();
    for j in 0..ny {
        for i in 0..nx {
            let sample = samples[(j * nx + i) as usize];
            let at = DVec3::from_array(slice.point(i, j));
            if let Some(clip) = &presentation.clip
                && !super::octree::kept(at, 0.0, clip)
            {
                continue;
            }
            let key = (at / block_m).floor().as_ivec3().to_array();
            let page = volume_page_id(key).unwrap_or(BORDER_PAGE - 1);
            let (rgba, code, value) = match sample.value {
                Ok(value) if view.band.is_some_and(|band| value < band[0] || value > band[1]) => continue,
                Ok(value) => {
                    let rgba = if !view.contours.is_empty() && on_contour(samples, nx, ny, i, j, &view.contours) {
                        view.contour_rgba
                    } else {
                        colour(presentation, value, sample.color)?
                    };
                    (rgba, ARROW | SLICE, value as f32)
                }
                Err(_) => {
                    let rgba = if (i + j) % 2 == 0 { view.unknown_rgba } else { darker(view.unknown_rgba) };
                    (rgba, ARROW | SLICE | UNKNOWN, f32::NAN)
                }
            };
            let (cells, rebase, count) = groups.entry(page).or_insert_with(|| (Vec::new(), None, 0));
            let rebase = *rebase.get_or_insert(at);
            quad(cells, slice, at, rebase, rgba, presentation.opacity, code, value)?;
            *count += 1;
        }
    }
    let mut groups: Groups = groups
        .into_iter()
        .map(|(page, (cells, rebase, count))| (page, cells, count, rebase.unwrap_or_default()))
        .collect();
    groups.sort_unstable_by_key(|(page, ..)| *page);
    Ok(groups)
}

/// Thin quads along the slice's four edges.
fn border(slice: &VolumeSlice, rgba: [u8; 4]) -> Result<Vec<Cell>, StorageError> {
    let center = DVec3::from_array(slice.center_m);
    let (u, v) = (DVec3::from_array(slice.u), DVec3::from_array(slice.v));
    let (a, b) = (slice.size_m[0] / 2.0, slice.size_m[1] / 2.0);
    let width = slice.size_m[0].min(slice.size_m[1]) * 0.03;
    let corners = [center - u * a - v * b, center + u * a - v * b, center + u * a + v * b, center - u * a + v * b];
    let mut cells = Vec::with_capacity(24);
    for k in 0..4 {
        let (p, q) = (corners[k], corners[(k + 1) % 4]);
        let inward = (center - (p + q) / 2.0).normalize_or_zero() * width;
        let quad = [p, q, q + inward, p + inward];
        for n in [0, 1, 2, 0, 2, 3] {
            cells.push(record(quad[n], center, rgba, 1.0, ARROW | FRAME, -1.0)?);
        }
    }
    Ok(cells)
}

fn digest(cells: &[Cell], rebase: DVec3) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    rebase.to_array().map(f64::to_bits).hash(&mut hasher);
    for cell in cells {
        (cell.center.map(f32::to_bits), cell.rgba, cell.code, cell.probability.to_bits()).hash(&mut hasher);
    }
    hasher.finish()
}

/// One page of slice records, or the previous page when its records are identical.
fn page(
    budget: &CpuBudget,
    previous: &HashMap<PageAddress, &Arc<PageCells>>,
    address: PageAddress,
    cells: Vec<Cell>,
    rebase: DVec3,
    drawn: u64,
    opaque: bool,
) -> Result<Arc<PageCells>, StorageError> {
    let basis = digest(&cells, rebase);
    if let Some(old) = previous.get(&address).filter(|old| old.basis == basis) {
        return Ok((*old).clone());
    }
    let mut records = Records::<Cell>::new(budget, cells.len())?;
    records.values.copy_from_slice(&cells);
    Ok(Arc::new(PageCells {
        address,
        source: None,
        cells: CellBuffer::new(budget, records)?,
        rebase_m: rebase.to_array(),
        counts: Counts { cells: drawn, occupied: drawn, ..Counts::default() },
        opaque,
        shape: Shape::Triangles,
        ids: None,
        basis,
        colors: None,
    }))
}

/// Slice records for one volume generation, reusing unchanged groups of `previous`.
pub(super) fn prepare(
    budget: &CpuBudget,
    key: DataKey,
    revision: DatasetRevision,
    lifetime: DatasetLifetime,
    storage: Arc<DatasetStorage>,
    previous: Option<&Prepared>,
) -> Result<Arc<Prepared>, StorageError> {
    let descriptor = storage.descriptor().clone();
    let DataShape::Volume { field, .. } = &descriptor.shape else {
        return Err(invalid("not a volume dataset"));
    };
    let presentation = &descriptor.presentation;
    let style = super::octree::style_key(presentation);
    let stored = super::worker::acquire_pages(&storage, &lifetime)?;
    let blocks = stored.iter().filter_map(|page| page.volume_layout().map(|layout| BlockView::trusted(page.bytes(), layout)));
    let volume = Volume::new(field, blocks);
    let previous: HashMap<PageAddress, &Arc<PageCells>> = previous
        .filter(|previous| previous.style == style)
        .map(|previous| previous.pages.iter().map(|page| (page.address, page)).collect())
        .unwrap_or_default();
    let mut pages = Vec::new();
    if let Some(view) = &presentation.volume {
        let channel = channel(presentation);
        let colors = matches!(presentation.color, ColorMode::Attribute { .. });
        let opaque = presentation.opacity >= 1.0 && presentation.blur <= 0.0;
        for (s, slice) in view.slices.iter().enumerate() {
            let scan_key = SLICE_SCAN - s as u64;
            let samples = sample_slice(&volume, view, slice, channel, colors);
            for (page_id, cells, drawn, rebase) in slice_groups(field, presentation, view, slice, &samples)? {
                let opaque = opaque && cells.iter().all(|cell| cell.rgba >> 24 == 255);
                pages.push(page(budget, &previous, PageAddress { scan_key, page_id }, cells, rebase, drawn, opaque)?);
            }
            if let Some(rgba) = view.frame_rgba {
                let cells = border(slice, rgba)?;
                let center = DVec3::from_array(slice.center_m);
                pages.push(page(budget, &previous, PageAddress { scan_key, page_id: BORDER_PAGE }, cells, center, 0, rgba[3] == 255)?);
            }
        }
    }
    let source: u64 = stored.iter().map(|page| u64::from(page.descriptor().items)).sum();
    let valid: u64 = stored.iter().filter_map(|page| page.volume_layout()).map(|layout| u64::from(layout.valid_count)).sum();
    let mut prepared = super::octree::assemble(budget, key, revision, lifetime, storage, pages, presentation, style)?;
    // Receipts describe the retained samples the slices view, not the slice cells.
    if let Some(prepared) = Arc::get_mut(&mut prepared) {
        prepared.counts.leaves = source;
        prepared.counts.cells = valid;
    }
    Ok(prepared)
}
