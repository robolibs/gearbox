//! Layered elevation maps for the compact instance renderer. Each tile's
//! elevation becomes two triangles per quad of four valid neighbouring
//! samples, reaching into the +x, +y and diagonal tiles at its edges; a missing
//! elevation is a hole, never zero height. Colour comes from a separate buffer,
//! so switching the colour layer or palette re-uploads colours alone and a
//! layer-only update keeps every tile's geometry.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use bevy::math::DVec3;
use gearbox_api::draw_data::{CpuBudget, DatasetLifetime, DatasetStorage, StorageError, StoredPage};
use gearbox_api::wire::draw_data::elevation::ElevationLayout;
use gearbox_api::wire::draw_data::{
    ColorMode, DataKey, DataShape, DatasetRevision, ElevationGrid, PageAddress, Presentation, elevation_page_id,
};
use gearbox_api::wire::draw_geom::colormap_rgba;

use super::cloud::{MAX_LOCAL_ERROR_M, Records, invalid};
use super::octree::{Cell, CellBuffer, ColorBuffer, Counts, PageCells, Prepared, Shape};
use super::surface::NORMAL_MISSING;

#[cfg(test)]
#[path = "elevation_tests.rs"]
mod tests;

/// A terrain quad's vertex record; `spare` holds the quad's cell within its tile.
pub(super) const TERRAIN: u32 = 1 << 22;
/// Vertex records a dataset may draw; the elevation profile's data bound.
pub(super) const MAX_ELEVATION_RECORDS: usize = 2_000_000;

/// Every stored tile of one generation, by tile key and layer.
struct Tiles<'a> {
    grid: &'a ElevationGrid,
    pages: HashMap<([i32; 2], u8), (&'a Arc<StoredPage>, ElevationLayout)>,
}

impl<'a> Tiles<'a> {
    fn new(grid: &'a ElevationGrid, stored: &'a [Arc<StoredPage>]) -> Self {
        let pages = stored
            .iter()
            .filter_map(|page| page.elevation_layout().map(|layout| ((layout.key, layout.layer), (page, layout))))
            .collect();
        Self { grid, pages }
    }

    /// `layer` at grid cell `cell`, when stored and valid.
    fn get(&self, layer: u8, cell: [i64; 2]) -> Option<f32> {
        let (key, index) = self.grid.locate(cell);
        let (page, layout) = self.pages.get(&(key, layer))?;
        let bytes = page.bytes();
        if bytes[layout.validity + index / 8] >> (index % 8) & 1 == 0 {
            return None;
        }
        let at = layout.values + index * 4;
        Some(f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()))
    }

    fn page(&self, key: [i32; 2], layer: u8) -> usize {
        self.pages.get(&(key, layer)).map_or(0, |(page, _)| Arc::as_ptr(page) as usize)
    }

    /// Quads whose lowest corner lies in tile `key`, all four corner elevations valid,
    /// as the lowest corner's cell, in row order.
    fn quads(&self, key: [i32; 2]) -> Vec<[i64; 2]> {
        let side = i64::from(self.grid.tile_side);
        let origin = key.map(|k| i64::from(k) * side);
        let mut out = Vec::new();
        for j in 0..side {
            for i in 0..side {
                let c = [origin[0] + i, origin[1] + j];
                if [[0, 0], [1, 0], [1, 1], [0, 1]].iter().all(|d| self.get(0, [c[0] + d[0], c[1] + d[1]]).is_some()) {
                    out.push(c);
                }
            }
        }
        out
    }
}

const CORNERS: [[i64; 2]; 6] = [[0, 0], [1, 0], [1, 1], [0, 0], [1, 1], [0, 1]];

fn position(grid: &ElevationGrid, cell: [i64; 2], z: f32) -> DVec3 {
    DVec3::new((cell[0] as f64 + 0.5) * grid.cell_m, (cell[1] as f64 + 0.5) * grid.cell_m, f64::from(z))
}

/// Six vertex records per quad of tile `key`, rebased to the tile's first sample.
fn geometry(tiles: &Tiles<'_>, key: [i32; 2], quads: &[[i64; 2]]) -> Result<(Vec<Cell>, DVec3), StorageError> {
    let side = i64::from(tiles.grid.tile_side);
    let rebase = position(tiles.grid, key.map(|k| i64::from(k) * side), 0.0);
    let mut cells = Vec::with_capacity(quads.len() * 6);
    for c in quads {
        let corners = CORNERS.map(|d| [c[0] + d[0], c[1] + d[1]]);
        let heights = corners.map(|corner| tiles.get(0, corner).unwrap_or_default());
        let mean = [heights[0], heights[1], heights[2], heights[5]].iter().sum::<f32>() / 4.0;
        let local = [c[0] - i64::from(key[0]) * side, c[1] - i64::from(key[1]) * side];
        for k in 0..6 {
            let at = position(tiles.grid, corners[k], heights[k]) - rebase;
            let xyz = at.as_vec3();
            if (xyz.as_dvec3() - at).length() > MAX_LOCAL_ERROR_M {
                return Err(invalid("elevation tile exceeds one-millimetre local GPU precision"));
            }
            cells.push(Cell {
                center: xyz.to_array(),
                half: 0.0,
                rgba: 0xff80_8080,
                code: TERRAIN | NORMAL_MISSING,
                probability: mean,
                spare: (local[0] as u32) | (local[1] as u32) << 16,
            });
        }
    }
    Ok((cells, rebase))
}

/// One packed colour per vertex record: the quad's mean layer value, or the missing colour.
fn colours(tiles: &Tiles<'_>, presentation: &Presentation, quads: &[[i64; 2]]) -> Result<Vec<u32>, StorageError> {
    let pack = |rgba: [u8; 4]| super::octree::rgba(rgba, presentation.opacity);
    let per_quad: Vec<u32> = match &presentation.color {
        ColorMode::Constant { rgba } => vec![pack(*rgba); quads.len()],
        ColorMode::Scalar { name, colormap, range, missing } => {
            let layer = tiles.grid.layer(name).ok_or_else(|| invalid("unknown elevation layer"))?;
            quads
                .iter()
                .map(|c| {
                    let values: Option<Vec<f32>> = [[0, 0], [1, 0], [1, 1], [0, 1]]
                        .iter()
                        .map(|d| tiles.get(layer, [c[0] + d[0], c[1] + d[1]]))
                        .collect();
                    let rgba = values.map_or(*missing, |v| {
                        colormap_rgba(*colormap, v.iter().sum::<f32>() / 4.0, range[0] as f32, range[1] as f32)
                    });
                    pack(rgba)
                })
                .collect()
        }
        _ => return Err(invalid("elevation maps take constant or layer colours")),
    };
    Ok(per_quad.iter().flat_map(|c| [*c; 6]).collect())
}

fn digest<T: Hash>(value: T) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// Tile records for one elevation generation, reusing unchanged geometry and colours of `previous`.
pub(super) fn prepare(
    budget: &CpuBudget,
    key: DataKey,
    revision: DatasetRevision,
    lifetime: DatasetLifetime,
    storage: Arc<DatasetStorage>,
    previous: Option<&Prepared>,
) -> Result<Arc<Prepared>, StorageError> {
    let descriptor = storage.descriptor().clone();
    let DataShape::Elevation { grid, .. } = &descriptor.shape else {
        return Err(invalid("not an elevation dataset"));
    };
    let presentation = &descriptor.presentation;
    let style = super::octree::style_key(presentation);
    let stored = super::worker::acquire_pages(&storage, &lifetime)?;
    let tiles = Tiles::new(grid, &stored);
    let previous: HashMap<PageAddress, &Arc<PageCells>> = previous
        .map(|previous| previous.pages.iter().map(|page| (page.address, page)).collect())
        .unwrap_or_default();
    let mut keys: Vec<[i32; 2]> = tiles.pages.keys().filter(|(_, layer)| *layer == 0).map(|(key, _)| *key).collect();
    keys.sort_unstable();
    let mut pages = Vec::with_capacity(keys.len());
    let mut records = 0usize;
    for tile in keys {
        let address = PageAddress { scan_key: 0, page_id: elevation_page_id(tile, 0).ok_or_else(|| invalid("tile key out of range"))? };
        let shape = [[0, 0], [1, 0], [0, 1], [1, 1]].map(|d| tiles.page([tile[0] + d[0], tile[1] + d[1]], 0));
        let basis = digest(shape);
        let quads = tiles.quads(tile);
        let old = previous.get(&address).copied();
        let (cells, rebase) = match old.filter(|old| old.basis == basis) {
            Some(old) => (old.cells.clone(), old.rebase_m),
            None => {
                let (cells, rebase) = geometry(&tiles, tile, &quads)?;
                let mut buffer = Records::<Cell>::new(budget, cells.len())?;
                buffer.values.copy_from_slice(&cells);
                (CellBuffer::new(budget, buffer)?, rebase.to_array())
            }
        };
        let values = colours(&tiles, presentation, &quads)?;
        let colors = match old.and_then(|old| old.colors.clone()).filter(|old| old.values.values[..] == values[..]) {
            Some(colors) => colors,
            None => {
                let mut buffer = Records::<u32>::new(budget, values.len())?;
                buffer.values.copy_from_slice(&values);
                ColorBuffer::new(budget, buffer)?
            }
        };
        records += cells.records.values.len();
        if records > MAX_ELEVATION_RECORDS {
            return Err(invalid("elevation map exceeds its drawn-record bound"));
        }
        let opaque = presentation.opacity >= 1.0 && presentation.blur <= 0.0;
        pages.push(Arc::new(PageCells {
            address,
            source: None,
            cells,
            rebase_m: rebase,
            counts: Counts { cells: quads.len() as u64, occupied: quads.len() as u64, ..Counts::default() },
            opaque,
            shape: Shape::Triangles,
            ids: None,
            basis,
            colors: Some(colors),
        }));
    }
    let source: u64 = stored.iter().map(|page| u64::from(page.descriptor().items)).sum();
    let valid: u64 = stored.iter().filter_map(|page| page.elevation_layout()).map(|layout| u64::from(layout.valid_count)).sum();
    let mut prepared = super::octree::assemble(budget, key, revision, lifetime, storage, pages, presentation, style)?;
    if let Some(prepared) = Arc::get_mut(&mut prepared) {
        prepared.counts.leaves = source;
        prepared.counts.cells = valid;
        prepared.uniform.extra.y = 1.0;
    }
    Ok(prepared)
}
