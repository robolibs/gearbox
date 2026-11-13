//! Layered elevation maps: one `ElevationTileV1` page per tile and layer. Layer
//! 0 is elevation in metres along the frame's z, and its validity is topology: a
//! missing elevation is a hole, never zero height. Named layers colour the same
//! samples; a missing layer value is a missing colour on present geometry.
//! Sample `(i, j)` of tile `(tx, ty)` sits at `((tx·side + i) + 0.5)·cell_m`,
//! `((ty·side + j) + 0.5)·cell_m`, x fastest.
//!
//! Layout, little-endian: magic `GBE1`, tile key (2 × i32), layer (u32), side
//! (u32), f32 values, then a validity bitmask, least significant bit first.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use super::frames::bounded_text;

pub const ELEVATION_MAGIC: &[u8; 4] = b"GBE1";
const HEADER: usize = 20;
/// Largest tile key magnitude; keys pack into page ids as two 24-bit fields.
pub const MAX_TILE_KEY: i32 = (1 << 23) - 1;
pub const MAX_LAYERS: usize = 15;
pub const ELEVATION: &str = "elevation";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElevationLayer {
    pub name: String,
    pub units: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElevationGrid {
    pub cell_m: f64,
    /// Samples along each tile edge: 8 to 128.
    pub tile_side: u16,
    /// Named layers after elevation, in layer-index order from 1.
    pub layers: Vec<ElevationLayer>,
}

impl ElevationGrid {
    pub fn validate(&self) -> Result<(), String> {
        if !self.cell_m.is_finite() || self.cell_m <= 0.0 || self.cell_m > 1000.0 || !(8..=128).contains(&self.tile_side) {
            return Err("an elevation map needs a 0..1000 m cell and 8..128-sample tiles".into());
        }
        if self.layers.len() > MAX_LAYERS {
            return Err("an elevation map holds at most 15 named layers".into());
        }
        for (k, layer) in self.layers.iter().enumerate() {
            bounded_text(&layer.name, 63, false)?;
            bounded_text(&layer.units, 31, true)?;
            if layer.name == ELEVATION || self.layers[..k].iter().any(|other| other.name == layer.name) {
                return Err("layer names must be unique and not `elevation`".into());
            }
        }
        Ok(())
    }

    pub fn samples(&self) -> usize {
        usize::from(self.tile_side).pow(2)
    }

    /// The layer index of `name`: 0 for elevation.
    pub fn layer(&self, name: &str) -> Option<u8> {
        if name == ELEVATION {
            return Some(0);
        }
        self.layers.iter().position(|layer| layer.name == name).map(|k| k as u8 + 1)
    }

    /// The tile holding grid cell `cell` and the cell's sample index in it.
    pub fn locate(&self, cell: [i64; 2]) -> ([i32; 2], usize) {
        let side = i64::from(self.tile_side);
        let key = cell.map(|c| c.div_euclid(side) as i32);
        let local = cell.map(|c| c.rem_euclid(side) as usize);
        (key, local[0] + local[1] * side as usize)
    }
}

/// The page id of `layer` of tile `key`: x in the low 24 bits, then y, then the layer.
pub fn elevation_page_id(key: [i32; 2], layer: u8) -> Option<u64> {
    if key.iter().any(|k| !(-MAX_TILE_KEY - 1..=MAX_TILE_KEY).contains(k)) || usize::from(layer) > MAX_LAYERS {
        return None;
    }
    Some((key[0] as u64 & 0xff_ffff) | (key[1] as u64 & 0xff_ffff) << 24 | u64::from(layer) << 48)
}

pub fn elevation_tile_of(page_id: u64) -> ([i32; 2], u8) {
    let field = |shift: u32| (((page_id >> shift) & 0xff_ffff) as i32) << 8 >> 8;
    ([field(0), field(24)], (page_id >> 48) as u8)
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ElevationTile {
    pub key: [i32; 2],
    pub layer: u8,
    pub values: Vec<f32>,
    pub valid: Vec<bool>,
}

/// Where a validated tile's arrays lie.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ElevationLayout {
    pub key: [i32; 2],
    pub layer: u8,
    pub side: u16,
    pub values: usize,
    pub validity: usize,
    pub valid_count: u32,
}

pub fn encode_elevation_tile(grid: &ElevationGrid, tile: &ElevationTile) -> Result<Vec<u8>, String> {
    grid.validate()?;
    let n = grid.samples();
    if tile.values.len() != n || tile.valid.len() != n {
        return Err("an elevation tile holds side² samples".into());
    }
    let mut bytes = Vec::with_capacity(HEADER + n * 4 + n.div_ceil(8));
    bytes.extend_from_slice(ELEVATION_MAGIC);
    tile.key.iter().for_each(|k| bytes.extend(k.to_le_bytes()));
    bytes.extend(u32::from(tile.layer).to_le_bytes());
    bytes.extend(u32::from(grid.tile_side).to_le_bytes());
    for (value, valid) in tile.values.iter().zip(&tile.valid) {
        bytes.extend(if *valid { *value } else { 0.0 }.to_le_bytes());
    }
    let mut mask = vec![0u8; n.div_ceil(8)];
    for (i, valid) in tile.valid.iter().enumerate() {
        mask[i / 8] |= u8::from(*valid) << (i % 8);
    }
    bytes.extend(mask);
    elevation_layout(grid, &bytes)?;
    Ok(bytes)
}

pub fn elevation_layout(grid: &ElevationGrid, bytes: &[u8]) -> Result<ElevationLayout, String> {
    grid.validate()?;
    if bytes.len() < HEADER || &bytes[..4] != ELEVATION_MAGIC {
        return Err("invalid elevation tile header".into());
    }
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let key = [word(4) as i32, word(8) as i32];
    let (layer, side) = (word(12), word(16));
    if side != u32::from(grid.tile_side) || layer as usize > grid.layers.len() {
        return Err("elevation tile side or layer disagrees with its map".into());
    }
    let n = grid.samples();
    let validity = HEADER + n * 4;
    if bytes.len() != validity + n.div_ceil(8) {
        return Err("elevation tile length disagrees with its map".into());
    }
    let mut valid_count = 0;
    for i in 0..n {
        let value = f32::from_le_bytes(bytes[HEADER + i * 4..HEADER + i * 4 + 4].try_into().unwrap());
        if bytes[validity + i / 8] >> (i % 8) & 1 == 1 {
            if !value.is_finite() {
                return Err("valid elevation sample is not finite".into());
            }
            valid_count += 1;
        } else if value != 0.0 {
            return Err("missing elevation samples must hold canonical zeros".into());
        }
    }
    if n % 8 != 0 && bytes[validity + n.div_ceil(8) - 1] >> (n % 8) != 0 {
        return Err("elevation validity padding bits must be zero".into());
    }
    Ok(ElevationLayout { key, layer: layer as u8, side: grid.tile_side, values: HEADER, validity, valid_count })
}

pub fn decode_elevation_tile(grid: &ElevationGrid, bytes: &[u8]) -> Result<ElevationTile, String> {
    let layout = elevation_layout(grid, bytes)?;
    let n = grid.samples();
    Ok(ElevationTile {
        key: layout.key,
        layer: layout.layer,
        values: (0..n).map(|i| f32::from_le_bytes(bytes[HEADER + i * 4..HEADER + i * 4 + 4].try_into().unwrap())).collect(),
        valid: (0..n).map(|i| bytes[layout.validity + i / 8] >> (i % 8) & 1 == 1).collect(),
    })
}

/// The layers an elevation map can be coloured by, for presentation validation.
pub fn elevation_schema(grid: &ElevationGrid) -> super::CloudSchema {
    use super::{Attribute, AttributeSemantic, Normalization, ScalarType};
    let attribute = |name: &str, units: &str| Attribute {
        name: name.into(),
        scalar: ScalarType::F32,
        components: 1,
        semantic: AttributeSemantic::Custom,
        units: units.into(),
        normalization: Normalization::None,
        nullable: true,
    };
    let mut attributes = vec![
        Attribute {
            name: "position".into(),
            scalar: ScalarType::F32,
            components: 3,
            semantic: AttributeSemantic::Position,
            units: "m".into(),
            normalization: Normalization::None,
            nullable: false,
        },
        attribute(ELEVATION, "m"),
    ];
    attributes.extend(grid.layers.iter().map(|layer| attribute(&layer.name, &layer.units)));
    super::CloudSchema { revision: super::SCHEMA_REVISION, attributes }
}

/// A rectangle of grid cells: `origin` is its lowest cell, `size` its extent in cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElevationWindow {
    pub origin: [i64; 2],
    pub size: [u32; 2],
}

impl ElevationWindow {
    pub fn contains(&self, cell: [i64; 2]) -> bool {
        (0..2).all(|a| cell[a] >= self.origin[a] && cell[a] < self.origin[a] + i64::from(self.size[a]))
    }

    /// Keys of every tile the window touches.
    pub fn tiles(&self, side: u16) -> Vec<[i32; 2]> {
        let side = i64::from(side);
        let low = self.origin.map(|c| c.div_euclid(side));
        let high = [0, 1].map(|a| (self.origin[a] + i64::from(self.size[a]) - 1).div_euclid(side));
        let mut keys = Vec::new();
        for y in low[1]..=high[1] {
            for x in low[0]..=high[0] {
                keys.push([x as i32, y as i32]);
            }
        }
        keys
    }
}

/// A writer's rolling map: every layer's samples inside a window, keyed by cell.
/// Shifting keeps the overlap and leaves newly exposed cells missing; cells that
/// leave the window are dropped, so tiles carry only what the window holds.
#[derive(Clone, Debug)]
pub struct RollingMap {
    pub grid: ElevationGrid,
    pub window: ElevationWindow,
    layers: Vec<HashMap<[i64; 2], f32>>,
    /// Page ids sent and the blake3 digest of their bytes.
    sent: BTreeMap<u64, [u8; 32]>,
}

/// A rolling map's whole state, for writers that keep it between runs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollingState {
    pub grid: ElevationGrid,
    pub window: ElevationWindow,
    /// Every cell holding a value: its index and each layer, missing as null.
    pub cells: Vec<([i64; 2], Vec<Option<f32>>)>,
    /// Page ids sent and the hex blake3 digest of their bytes.
    pub sent: Vec<(u64, String)>,
}

/// The pages a shift or an edit changes: tiles to (re)send and page ids to remove.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ElevationDelta {
    pub tiles: Vec<ElevationTile>,
    pub removed: Vec<u64>,
}

impl RollingMap {
    pub fn new(grid: ElevationGrid, window: ElevationWindow) -> Result<Self, String> {
        grid.validate()?;
        let layers = vec![HashMap::new(); grid.layers.len() + 1];
        Ok(Self { grid, window, layers, sent: BTreeMap::new() })
    }

    /// Sets `layer` at `cell`; `None` marks it missing. Cells outside the window are refused.
    pub fn set(&mut self, layer: u8, cell: [i64; 2], value: Option<f32>) -> Result<(), String> {
        if !self.window.contains(cell) {
            return Err("cell lies outside the rolling window".into());
        }
        let map = self.layers.get_mut(usize::from(layer)).ok_or("unknown layer")?;
        match value {
            Some(v) if v.is_finite() => {
                map.insert(cell, v);
            }
            Some(_) => return Err("layer values must be finite".into()),
            None => {
                map.remove(&cell);
            }
        }
        Ok(())
    }

    pub fn get(&self, layer: u8, cell: [i64; 2]) -> Option<f32> {
        self.layers.get(usize::from(layer))?.get(&cell).copied()
    }

    /// Moves the window by whole cells, keeping the overlap and dropping what left it.
    pub fn shift(&mut self, origin: [i64; 2]) {
        self.window.origin = origin;
        let window = self.window;
        for layer in &mut self.layers {
            layer.retain(|cell, _| window.contains(*cell));
        }
    }

    /// Every non-empty tile of the window as it stands, sent or not.
    pub fn tiles(&self) -> Vec<ElevationTile> {
        let side = usize::from(self.grid.tile_side);
        let mut out = Vec::new();
        for key in self.window.tiles(self.grid.tile_side) {
            for (layer, map) in self.layers.iter().enumerate() {
                let mut tile = ElevationTile { key, layer: layer as u8, values: vec![0.0; side * side], valid: vec![false; side * side] };
                for j in 0..side {
                    for i in 0..side {
                        let cell = [i64::from(key[0]) * side as i64 + i as i64, i64::from(key[1]) * side as i64 + j as i64];
                        if let Some(value) = map.get(&cell) {
                            tile.values[i + j * side] = *value;
                            tile.valid[i + j * side] = true;
                        }
                    }
                }
                if tile.valid.iter().any(|v| *v) {
                    out.push(tile);
                }
            }
        }
        out
    }

    /// What changed since the last delta: tiles whose bytes differ and tiles now empty.
    pub fn delta(&mut self) -> Result<ElevationDelta, String> {
        let mut now = BTreeMap::new();
        let mut delta = ElevationDelta::default();
        for tile in self.tiles() {
            let id = elevation_page_id(tile.key, tile.layer).ok_or("tile key out of range")?;
            let digest = *blake3::hash(&encode_elevation_tile(&self.grid, &tile)?).as_bytes();
            if self.sent.get(&id) != Some(&digest) {
                delta.tiles.push(tile);
            }
            now.insert(id, digest);
        }
        delta.removed = self.sent.keys().filter(|id| !now.contains_key(id)).copied().collect();
        self.sent = now;
        Ok(delta)
    }
}

impl RollingMap {
    pub fn state(&self) -> RollingState {
        let mut cells: BTreeMap<[i64; 2], Vec<Option<f32>>> = BTreeMap::new();
        for (layer, map) in self.layers.iter().enumerate() {
            for (cell, value) in map {
                cells.entry(*cell).or_insert_with(|| vec![None; self.layers.len()])[layer] = Some(*value);
            }
        }
        RollingState {
            grid: self.grid.clone(),
            window: self.window,
            cells: cells.into_iter().collect(),
            sent: self.sent.iter().map(|(id, digest)| (*id, digest.iter().map(|b| format!("{b:02x}")).collect())).collect(),
        }
    }

    pub fn from_state(state: RollingState) -> Result<Self, String> {
        let mut map = Self::new(state.grid, state.window)?;
        for (cell, values) in state.cells {
            if values.len() != map.layers.len() {
                return Err("rolling state cell has the wrong layer count".into());
            }
            for (layer, value) in values.into_iter().enumerate() {
                if value.is_some() {
                    map.set(layer as u8, cell, value)?;
                }
            }
        }
        for (id, hex) in state.sent {
            let bytes: Vec<u8> = (0..hex.len())
                .step_by(2)
                .map(|i| hex.get(i..i + 2).and_then(|pair| u8::from_str_radix(pair, 16).ok()))
                .collect::<Option<_>>()
                .ok_or("rolling state digest is not hex")?;
            map.sent.insert(id, bytes.try_into().map_err(|_| "rolling state digest is not 32 bytes")?);
        }
        Ok(map)
    }
}
