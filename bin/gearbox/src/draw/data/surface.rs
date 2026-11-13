//! Surfels and mesh blocks as records for the compact instance renderer: a
//! surfel is one disc record (centre, radius, colour, confidence, octahedral
//! normal), a mesh triangle three vertex records drawn without instancing.
//! Missing normals are flagged, never invented: a surfel without one faces the
//! camera, a triangle without vertex normals shades by its own plane.

use std::sync::Arc;

use bevy::math::{DVec3, Vec3};
use gearbox_api::draw_data::{DatasetLifetime, DatasetStorage, StorageError, StoredPage};
use gearbox_api::wire::draw_data::{
    AttributeSemantic, CloudSchema, ColorMode, ColumnView, DataKey, DataShape, DatasetRevision,
    Presentation, ScalarType,
};
use gearbox_api::wire::draw_geom::colormap_rgba;

use super::cloud::{MAX_LOCAL_ERROR_M, Records, invalid, position, valid};
use super::octree::{Cell, CellBuffer, Counts, PageCells, Prepared, Shape};
use super::worker::charge;

#[cfg(test)]
#[path = "surface_tests.rs"]
mod tests;

/// The normal of this record is unknown.
pub(super) const NORMAL_MISSING: u32 = 1 << 18;
/// Records a dataset may draw; the surface performance profile's data bound.
pub(super) const MAX_SURFACE_RECORDS: usize = 2_000_000;

/// Octahedral 16+16-bit encoding of a unit vector.
pub(super) fn oct_encode(n: Vec3) -> u32 {
    let n = n / (n.x.abs() + n.y.abs() + n.z.abs()).max(1e-20);
    let (mut x, mut y) = (n.x, n.y);
    if n.z < 0.0 {
        (x, y) = ((1.0 - y.abs()) * x.signum(), (1.0 - x.abs()) * y.signum());
    }
    let q = |v: f32| ((v.clamp(-1.0, 1.0) * 0.5 + 0.5) * 65535.0).round() as u32;
    q(x) | q(y) << 16
}

pub(super) fn oct_decode(value: u32) -> Vec3 {
    let f = |v: u32| (v as f32 / 65535.0) * 2.0 - 1.0;
    let (x, y) = (f(value & 0xffff), f(value >> 16));
    let z = 1.0 - x.abs() - y.abs();
    let t = (-z).max(0.0);
    Vec3::new(x - t * x.signum(), y - t * y.signum(), z).normalize()
}

fn number(column: ColumnView<'_>, scalar: ScalarType, components: usize, row: usize, part: usize) -> Option<f64> {
    if !valid(column, row) {
        return None;
    }
    let size = scalar.size();
    let at = (row * components + part) * size;
    let raw = column.values.get(at..at + size)?;
    let mut wide = [0u8; 8];
    wide[..size].copy_from_slice(raw);
    let value = match scalar {
        ScalarType::F32 => f64::from(f32::from_le_bytes(raw.try_into().ok()?)),
        ScalarType::F64 => f64::from_le_bytes(wide),
        ScalarType::I16 => f64::from(i16::from_le_bytes(raw.try_into().ok()?)),
        ScalarType::I32 => f64::from(i32::from_le_bytes(raw.try_into().ok()?)),
        _ => u64::from_le_bytes(wide) as f64,
    };
    value.is_finite().then_some(value)
}

/// One sample's colour from its own columns, or `None` for a mode surfaces lack.
struct Colors<'a> {
    presentation: &'a Presentation,
    schema: &'a CloudSchema,
}

impl Colors<'_> {
    fn column<'p>(&self, page: &'p StoredPage, name: &str) -> Option<(usize, ColumnView<'p>)> {
        let index = self.schema.attributes.iter().position(|a| a.name == name)?;
        Some((index, page.column(index)?))
    }

    fn of(&self, page: &StoredPage, row: usize, at: DVec3) -> Result<[u8; 4], StorageError> {
        match &self.presentation.color {
            ColorMode::Constant { rgba } => Ok(*rgba),
            ColorMode::Attribute { name, .. } => {
                let (_, column) = self.column(page, name).ok_or_else(|| invalid("colour attribute missing"))?;
                Ok(if valid(column, row) {
                    column.values[row * 4..row * 4 + 4].try_into().unwrap()
                } else {
                    [128, 128, 128, 255]
                })
            }
            ColorMode::Scalar { name, colormap, range, missing } => {
                let (index, column) = self.column(page, name).ok_or_else(|| invalid("scalar attribute missing"))?;
                let field = &self.schema.attributes[index];
                Ok(number(column, field.scalar, field.components as usize, row, 0)
                    .map_or(*missing, |v| colormap_rgba(*colormap, v as f32, range[0] as f32, range[1] as f32)))
            }
            ColorMode::Axis { axis, colormap, range } => {
                Ok(colormap_rgba(*colormap, at[usize::from(*axis).min(2)] as f32, range[0] as f32, range[1] as f32))
            }
            _ => Err(invalid("surfaces take constant, attribute, scalar or axis colours")),
        }
    }
}

fn record(center: DVec3, rebase: DVec3, half: f32, rgba: u32, code: u32, probability: f32, spare: u32) -> Result<Cell, StorageError> {
    let local = center - rebase;
    let xyz = local.as_vec3();
    if (xyz.as_dvec3() - local).length() > MAX_LOCAL_ERROR_M {
        return Err(invalid("surface page exceeds one-millimetre local GPU precision"));
    }
    Ok(Cell { center: xyz.to_array(), half, rgba, code, probability, spare })
}

fn semantic(schema: &CloudSchema, semantic: AttributeSemantic) -> Option<usize> {
    schema.attributes.iter().position(|a| a.semantic == semantic)
}

/// Disc records of one surfel page, rebased to its first drawn surfel.
fn surfel_page(
    budget: &gearbox_api::draw_data::CpuBudget,
    presentation: &Presentation,
    schema: &CloudSchema,
    page: &Arc<StoredPage>,
) -> Result<PageCells, StorageError> {
    let colors = Colors { presentation, schema };
    let index = |s| semantic(schema, s).ok_or_else(|| invalid("surfel attribute missing"));
    let (p, n, r) = (index(AttributeSemantic::Position)?, index(AttributeSemantic::Normal)?, index(AttributeSemantic::Radius)?);
    let id = index(AttributeSemantic::SourceId)?;
    let confidence = semantic(schema, AttributeSemantic::Confidence);
    let column = |i: usize| page.column(i).ok_or_else(|| invalid("surfel column missing"));
    let (positions, normals, radii, ids) = (column(p)?, column(n)?, column(r)?, column(id)?);
    let origin = DVec3::from_array(page.descriptor().origin_m);
    let count = page.descriptor().items as usize;
    let mut cells = Vec::with_capacity(count);
    let mut sources = Vec::with_capacity(count);
    let mut rebase = None;
    let mut counts = Counts { leaves: count as u64, ..Counts::default() };
    for row in 0..count {
        if !valid(positions, row) {
            continue;
        }
        let Some(radius) = number(radii, schema.attributes[r].scalar, 1, row, 0).filter(|v| *v > 0.0) else {
            continue;
        };
        let at = origin + DVec3::from_array(position(positions, schema.attributes[p].scalar, row)?);
        if let Some(clip) = &presentation.clip
            && !super::octree::kept(at, radius, clip)
        {
            continue;
        }
        let rebase = *rebase.get_or_insert(at);
        let normal: Option<Vec3> = (0..3)
            .map(|k| number(normals, schema.attributes[n].scalar, 3, row, k).map(|v| v as f32))
            .collect::<Option<Vec<f32>>>()
            .map(|v| Vec3::new(v[0], v[1], v[2]))
            .filter(|v| v.is_finite() && v.length() > 0.5);
        let weight = confidence
            .and_then(|c| number(page.column(c)?, schema.attributes[c].scalar, 1, row, 0))
            .map_or(-1.0, |v| v as f32);
        let code = if normal.is_some() { 0 } else { NORMAL_MISSING };
        let rgba = super::octree::rgba(colors.of(page, row, at)?, presentation.opacity);
        cells.push(record(at, rebase, radius as f32, rgba, code, weight, normal.map_or(0, oct_encode))?);
        sources.push(u64::from_le_bytes(ids.values[row * 8..row * 8 + 8].try_into().unwrap()));
    }
    counts.cells = cells.len() as u64;
    counts.occupied = counts.cells;
    finish(budget, presentation, page, Shape::Discs, cells, Some(sources), rebase, counts)
}

/// Three vertex records per triangle of one mesh block.
fn mesh_page(
    budget: &gearbox_api::draw_data::CpuBudget,
    presentation: &Presentation,
    page: &Arc<StoredPage>,
) -> Result<PageCells, StorageError> {
    let layout = page.mesh_layout().ok_or_else(|| invalid("page is not a mesh block"))?;
    let bytes = page.bytes();
    let f32_at = |at: usize| f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let origin = DVec3::from_array(page.descriptor().origin_m);
    let vertex = |v: usize| origin + DVec3::new(f32_at(layout.positions + v * 12).into(), f32_at(layout.positions + v * 12 + 4).into(), f32_at(layout.positions + v * 12 + 8).into());
    let normal = |v: usize| layout.normals.map(|at| Vec3::new(f32_at(at + v * 12), f32_at(at + v * 12 + 4), f32_at(at + v * 12 + 8)));
    let confidence = |v: usize| layout.confidence.map(|at| f32_at(at + v * 4));
    let color = |v: usize, at: DVec3| -> Result<[u8; 4], StorageError> {
        match &presentation.color {
            ColorMode::Constant { rgba } => Ok(*rgba),
            ColorMode::Attribute { .. } => Ok(layout.colors.map_or([180, 180, 180, 255], |c| bytes[c + v * 4..c + v * 4 + 4].try_into().unwrap())),
            ColorMode::Scalar { colormap, range, missing, .. } => Ok(confidence(v).map_or(*missing, |c| colormap_rgba(*colormap, c, range[0] as f32, range[1] as f32))),
            ColorMode::Axis { axis, colormap, range } => Ok(colormap_rgba(*colormap, at[usize::from(*axis).min(2)] as f32, range[0] as f32, range[1] as f32)),
            _ => Err(invalid("mesh blocks take constant, attribute, confidence or axis colours")),
        }
    };
    let triangles = layout.triangles() as usize;
    let mut cells = Vec::with_capacity(triangles * 3);
    let mut rebase = None;
    for t in 0..triangles {
        let corners = [0, 1, 2].map(|k| u32_at(layout.index_offset + (t * 3 + k) * 4) as usize);
        let points = corners.map(vertex);
        if let Some(clip) = &presentation.clip {
            let centroid = (points[0] + points[1] + points[2]) / 3.0;
            if !super::octree::kept(centroid, 0.0, clip) {
                continue;
            }
        }
        let rebase = *rebase.get_or_insert(points[0]);
        for (k, &v) in corners.iter().enumerate() {
            let n = normal(v);
            let code = if n.is_some() { 0 } else { NORMAL_MISSING };
            let rgba = super::octree::rgba(color(v, points[k])?, presentation.opacity);
            cells.push(record(points[k], rebase, 0.0, rgba, code, confidence(v).unwrap_or(-1.0), n.map_or(0, oct_encode))?);
        }
    }
    let counts = Counts { leaves: triangles as u64, cells: (cells.len() / 3) as u64, occupied: (cells.len() / 3) as u64, ..Counts::default() };
    finish(budget, presentation, page, Shape::Triangles, cells, None, rebase, counts)
}

#[allow(clippy::too_many_arguments)]
fn finish(
    budget: &gearbox_api::draw_data::CpuBudget,
    presentation: &Presentation,
    page: &Arc<StoredPage>,
    shape: Shape,
    cells: Vec<Cell>,
    sources: Option<Vec<u64>>,
    rebase: Option<DVec3>,
    counts: Counts,
) -> Result<PageCells, StorageError> {
    let mut records = Records::<Cell>::new(budget, cells.len())?;
    records.values.copy_from_slice(&cells);
    let ids = sources
        .map(|sources| -> Result<_, StorageError> {
            let mut ids = Records::<u64>::new(budget, sources.len())?;
            ids.values.copy_from_slice(&sources);
            Ok(Arc::new(ids))
        })
        .transpose()?;
    let display = presentation.octree.clone().unwrap_or_default();
    let opaque = presentation.opacity >= 1.0
        && presentation.blur <= 0.0
        && display.style != gearbox_api::wire::draw_data::VoxelStyle::Wire
        && cells.iter().all(|cell| cell.rgba >> 24 == 255);
    Ok(PageCells {
        address: page.descriptor().address,
        source: Some(page.clone()),
        cells: CellBuffer::new(budget, records)?,
        rebase_m: rebase.unwrap_or_default().to_array(),
        counts,
        opaque,
        shape,
        ids,
        basis: 0,
        colors: None,
    })
}

/// Records for one surfel or mesh generation, reusing unchanged pages of `previous`.
pub(super) fn prepare(
    budget: &gearbox_api::draw_data::CpuBudget,
    key: DataKey,
    revision: DatasetRevision,
    lifetime: DatasetLifetime,
    storage: Arc<DatasetStorage>,
    previous: Option<&Prepared>,
) -> Result<Arc<Prepared>, StorageError> {
    let descriptor = storage.descriptor().clone();
    let presentation = &descriptor.presentation;
    let style = super::octree::style_key(presentation);
    let previous = previous.filter(|previous| previous.style == style);
    let mut pages = Vec::with_capacity(storage.page_len());
    let mut records = 0usize;
    for page in super::worker::acquire_pages(&storage, &lifetime)? {
        let reused = previous.and_then(|previous| {
            previous.pages.iter().find(|old| old.source.as_ref().is_some_and(|old| Arc::ptr_eq(old, &page)))
        });
        let cells = match (reused, &descriptor.shape) {
            (Some(old), _) => old.clone(),
            (None, DataShape::Surfels { schema, .. }) => Arc::new(surfel_page(budget, presentation, schema, &page)?),
            (None, DataShape::MeshBlocks { .. }) => Arc::new(mesh_page(budget, presentation, &page)?),
            _ => return Err(invalid("not a surface dataset")),
        };
        records += cells.cells.records.values.len();
        if records > MAX_SURFACE_RECORDS {
            return Err(invalid("surface dataset exceeds its drawn-record bound"));
        }
        pages.push(cells);
    }
    if let Some(arrows) = presentation.normals {
        let arrows = normal_arrows(budget, &arrows, &pages)?;
        if !arrows.cells.records.values.is_empty() {
            pages.push(Arc::new(arrows));
        }
    }
    let _ = charge;
    super::octree::assemble(budget, key, revision, lifetime, storage, pages, presentation, style)
}

/// Unlit normal-arrow triangles.
pub(super) const ARROW: u32 = 1 << 19;
/// The synthetic page holding a dataset's normal arrows.
pub(super) const ARROW_PAGE: gearbox_api::wire::draw_data::PageAddress =
    gearbox_api::wire::draw_data::PageAddress { scan_key: u64::MAX, page_id: u64::MAX - 1 };

/// Two crossed thin quads along every `every`-th supplied normal; none where it is missing.
fn normal_arrows(
    budget: &gearbox_api::draw_data::CpuBudget,
    arrows: &gearbox_api::wire::draw_data::NormalArrows,
    pages: &[Arc<PageCells>],
) -> Result<PageCells, StorageError> {
    let rgba = u32::from_le_bytes(arrows.rgba);
    let mut cells = Vec::new();
    let mut rebase = None;
    let mut seen = 0u64;
    for page in pages {
        let origin = DVec3::from_array(page.rebase_m);
        for cell in page.cells.records.values.iter().filter(|c| c.code & NORMAL_MISSING == 0) {
            seen += 1;
            if (seen - 1) % u64::from(arrows.every) != 0 {
                continue;
            }
            let base = origin + DVec3::from_array(cell.center.map(f64::from));
            let n = oct_decode(cell.spare).as_dvec3();
            let side = if n.x.abs() < 0.9 { n.cross(DVec3::X) } else { n.cross(DVec3::Y) }.normalize();
            let tip = base + n * f64::from(arrows.length_m);
            let width = f64::from(arrows.length_m) * 0.04;
            let rebase = *rebase.get_or_insert(base);
            for across in [side, n.cross(side)] {
                let (a, b) = (base - across * width, base + across * width);
                let (c, d) = (tip - across * width * 0.3, tip + across * width * 0.3);
                for corner in [a, b, d, a, d, c] {
                    cells.push(record(corner, rebase, 0.0, rgba, ARROW, -1.0, 0)?);
                }
            }
            if cells.len() > MAX_SURFACE_RECORDS {
                return Err(invalid("normal arrows exceed the drawn-record bound"));
            }
        }
    }
    let mut records = Records::<Cell>::new(budget, cells.len())?;
    records.values.copy_from_slice(&cells);
    let triangles = (cells.len() / 3) as u64;
    Ok(PageCells {
        address: ARROW_PAGE,
        source: None,
        cells: CellBuffer::new(budget, records)?,
        rebase_m: rebase.unwrap_or_default().to_array(),
        counts: Counts { cells: triangles, ..Counts::default() },
        opaque: arrows.rgba[3] == 255,
        shape: Shape::Triangles,
        ids: None,
        basis: 0,
        colors: None,
    })
}
