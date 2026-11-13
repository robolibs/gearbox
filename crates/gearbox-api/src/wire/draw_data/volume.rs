//! Sparse field volumes: one cube of `side`³ samples per page (`VolumeBlockV1`)
//! at an integer block key. Sample `(i, j, k)` of block `b` sits at the voxel
//! centre `((b * side + (i, j, k)) + 0.5) * voxel_m`; samples are x-fastest.
//! Unknown is a validity state: an invalid sample is never a distance, and
//! interpolation needs every supporting corner valid.
//!
//! Layout, little-endian: magic `GBV1`, key (3 × i32), side, flags (bit 0
//! weights, bit 1 RGBA8), then values (f32), the validity mask (one bit per
//! sample, least significant first), weights (f32) and colours in flag order.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub const VOLUME_MAGIC: &[u8; 4] = b"GBV1";
const HEADER: usize = 24;
const WEIGHTS: u32 = 1;
const COLORS: u32 = 2;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "field", rename_all = "snake_case", deny_unknown_fields)]
pub enum FieldKind {
    /// Signed distance in metres, clamped to ± `truncation_m`; weights required.
    Tsdf { truncation_m: f32 },
    /// Metric clearance; `signed` fields are negative inside obstacles.
    Esdf { signed: bool },
    /// A generic scalar in `units`; no collision meaning.
    Scalar { units: String, range: [f64; 2] },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VolumeField {
    pub kind: FieldKind,
    pub voxel_m: f64,
    /// Samples along each block edge: 8 or 16.
    pub block_side: u8,
    /// Per-sample RGBA8 accompanies the field.
    pub colors: bool,
}

impl VolumeField {
    pub fn validate(&self) -> Result<(), String> {
        if !self.voxel_m.is_finite() || self.voxel_m <= 0.0 || self.voxel_m > 100.0 || !matches!(self.block_side, 8 | 16) {
            return Err("a volume needs a 0..100 m voxel and 8- or 16-sample blocks".into());
        }
        match &self.kind {
            FieldKind::Tsdf { truncation_m } if !truncation_m.is_finite() || *truncation_m <= 0.0 => {
                Err("TSDF truncation must be positive metres".into())
            }
            FieldKind::Scalar { units, range } if units.len() > 63 || !range.iter().all(|v| v.is_finite()) || range[0] >= range[1] => {
                Err("scalar fields need short units and a finite increasing range".into())
            }
            _ => Ok(()),
        }
    }

    pub fn weights(&self) -> bool {
        matches!(self.kind, FieldKind::Tsdf { .. })
    }

    pub fn samples(&self) -> usize {
        usize::from(self.block_side).pow(3)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VolumeBlock {
    pub key: [i32; 3],
    pub values: Vec<f32>,
    pub valid: Vec<bool>,
    pub weights: Option<Vec<f32>>,
    pub colors: Option<Vec<[u8; 4]>>,
}

/// Where a validated block's arrays lie.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VolumeLayout {
    pub key: [i32; 3],
    pub side: u8,
    pub values: usize,
    pub validity: usize,
    pub weights: Option<usize>,
    pub colors: Option<usize>,
    pub valid_count: u32,
}

fn f32_at(bytes: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

pub fn encode_volume_block(field: &VolumeField, block: &VolumeBlock) -> Result<Vec<u8>, String> {
    field.validate()?;
    let n = field.samples();
    if block.values.len() != n || block.valid.len() != n {
        return Err("volume block arrays must hold side³ samples".into());
    }
    if block.weights.is_some() != field.weights() || block.colors.is_some() != field.colors {
        return Err("volume block carries weights/colours its field does not declare".into());
    }
    let flags = u32::from(block.weights.is_some()) * WEIGHTS | u32::from(block.colors.is_some()) * COLORS;
    let mut bytes = Vec::with_capacity(HEADER + n * 9);
    bytes.extend_from_slice(VOLUME_MAGIC);
    block.key.iter().for_each(|k| bytes.extend(k.to_le_bytes()));
    bytes.extend(u32::from(field.block_side).to_le_bytes());
    bytes.extend(flags.to_le_bytes());
    for (value, valid) in block.values.iter().zip(&block.valid) {
        bytes.extend(if *valid { *value } else { 0.0 }.to_le_bytes());
    }
    let mut mask = vec![0u8; n.div_ceil(8)];
    for (i, valid) in block.valid.iter().enumerate() {
        mask[i / 8] |= u8::from(*valid) << (i % 8);
    }
    bytes.extend(mask);
    if let Some(weights) = &block.weights {
        for (weight, valid) in weights.iter().zip(&block.valid) {
            bytes.extend(if *valid { *weight } else { 0.0 }.to_le_bytes());
        }
    }
    if let Some(colors) = &block.colors {
        colors.iter().zip(&block.valid).for_each(|(c, valid)| bytes.extend(if *valid { *c } else { [0; 4] }));
    }
    volume_layout(field, &bytes)?;
    Ok(bytes)
}

/// Validates an encoded block against its field.
pub fn volume_layout(field: &VolumeField, bytes: &[u8]) -> Result<VolumeLayout, String> {
    field.validate()?;
    if bytes.len() < HEADER || &bytes[..4] != VOLUME_MAGIC {
        return Err("invalid volume block header".into());
    }
    let int = |at: usize| i32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let key = [int(4), int(8), int(12)];
    let (side, flags) = (u32::from_le_bytes(bytes[16..20].try_into().unwrap()), u32::from_le_bytes(bytes[20..24].try_into().unwrap()));
    if side != u32::from(field.block_side) || flags != u32::from(field.weights()) * WEIGHTS | u32::from(field.colors) * COLORS {
        return Err("volume block side or arrays disagree with its field".into());
    }
    let n = field.samples();
    let values = HEADER;
    let validity = values + n * 4;
    let weights = field.weights().then_some(validity + n.div_ceil(8));
    let colors = field.colors.then(|| validity + n.div_ceil(8) + if field.weights() { n * 4 } else { 0 });
    let end = validity + n.div_ceil(8) + if field.weights() { n * 4 } else { 0 } + if field.colors { n * 4 } else { 0 };
    if bytes.len() != end {
        return Err("volume block length disagrees with its field".into());
    }
    let mut valid_count = 0u32;
    for i in 0..n {
        let valid = bytes[validity + i / 8] >> (i % 8) & 1 == 1;
        let value = f32_at(bytes, values + i * 4);
        let weight = weights.map(|at| f32_at(bytes, at + i * 4));
        if !valid {
            if value != 0.0 || weight.is_some_and(|w| w != 0.0) || colors.is_some_and(|at| bytes[at + i * 4..at + i * 4 + 4] != [0; 4]) {
                return Err("unknown volume samples must hold canonical zeros".into());
            }
            continue;
        }
        valid_count += 1;
        if !value.is_finite() {
            return Err("valid volume sample is not finite".into());
        }
        match &field.kind {
            FieldKind::Tsdf { truncation_m } if value.abs() > *truncation_m * (1.0 + 1e-6) => {
                return Err("TSDF sample exceeds its truncation".into());
            }
            FieldKind::Esdf { signed: false } if value < 0.0 => {
                return Err("unsigned ESDF sample is negative".into());
            }
            _ => {}
        }
        if weight.is_some_and(|w| !w.is_finite() || w <= 0.0) {
            return Err("a valid TSDF sample needs a positive weight; zero weight is unknown".into());
        }
    }
    let used = n % 8;
    if used != 0 && bytes[validity + n.div_ceil(8) - 1] >> used != 0 {
        return Err("volume validity padding bits must be zero".into());
    }
    Ok(VolumeLayout { key, side: field.block_side, values, validity, weights, colors, valid_count })
}

pub fn decode_volume_block(field: &VolumeField, bytes: &[u8]) -> Result<VolumeBlock, String> {
    let layout = volume_layout(field, bytes)?;
    let n = field.samples();
    Ok(VolumeBlock {
        key: layout.key,
        values: (0..n).map(|i| f32_at(bytes, layout.values + i * 4)).collect(),
        valid: (0..n).map(|i| bytes[layout.validity + i / 8] >> (i % 8) & 1 == 1).collect(),
        weights: layout.weights.map(|at| (0..n).map(|i| f32_at(bytes, at + i * 4)).collect()),
        colors: layout.colors.map(|at| (0..n).map(|i| bytes[at + i * 4..at + i * 4 + 4].try_into().unwrap()).collect()),
    })
}


/// Largest block key magnitude; keys pack into page ids as three 21-bit fields.
pub const MAX_BLOCK_KEY: i32 = (1 << 20) - 1;
pub const MAX_SLICES: usize = 4;
/// Slice cells across all slices of one volume.
pub const MAX_SLICE_CELLS: u64 = 65_536;
pub const MAX_CONTOURS: usize = 8;
pub const MAX_FIELD_POINTS: usize = 64;

/// The page id of block `key`: x in the low 21 bits, then y, then z, two's complement.
pub fn volume_page_id(key: [i32; 3]) -> Option<u64> {
    if key.iter().any(|k| !(-MAX_BLOCK_KEY - 1..=MAX_BLOCK_KEY).contains(k)) {
        return None;
    }
    Some((0..3).fold(0, |id, a| id | (key[a] as u64 & 0x1f_ffff) << (21 * a)))
}

pub fn volume_block_key(page_id: u64) -> [i32; 3] {
    [0, 1, 2].map(|a| (((page_id >> (21 * a)) & 0x1f_ffff) as i32) << 11 >> 11)
}

/// The sample arrays a volume can be read and coloured by, for presentation validation.
pub fn volume_schema(field: &VolumeField) -> super::CloudSchema {
    use super::{Attribute, AttributeSemantic, Normalization, ScalarType};
    let attribute = |name: &str, scalar, components, semantic, units: &str, normalization| Attribute {
        name: name.into(),
        scalar,
        components,
        semantic,
        units: units.into(),
        normalization,
        nullable: true,
    };
    let units = match &field.kind {
        FieldKind::Scalar { units, .. } => units.as_str(),
        _ => "m",
    };
    let mut attributes = vec![
        Attribute { nullable: false, ..attribute("position", ScalarType::F32, 3, AttributeSemantic::Position, "m", Normalization::None) },
        attribute("value", ScalarType::F32, 1, AttributeSemantic::Custom, units, Normalization::None),
    ];
    if field.weights() {
        attributes.push(attribute("weight", ScalarType::F32, 1, AttributeSemantic::Confidence, "", Normalization::None));
    }
    if field.colors {
        attributes.push(attribute("color", ScalarType::U8, 4, AttributeSemantic::Color, "", Normalization::Unorm));
    }
    super::CloudSchema { revision: super::SCHEMA_REVISION, attributes }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Value,
    /// TSDF integration weight.
    Weight,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interpolation {
    /// The sample of the voxel containing the point.
    Nearest,
    /// Eight voxel centres around the point, all of them valid.
    #[default]
    Trilinear,
}

/// Why a read has no value, or that it has one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Support {
    Valid,
    /// An allocated block marks a supporting sample unknown.
    Unknown,
    /// A supporting sample lies in a block that was never sent.
    Unallocated,
}

/// One validated encoded block, read in place.
#[derive(Clone, Copy, Debug)]
pub struct BlockView<'a> {
    bytes: &'a [u8],
    layout: VolumeLayout,
}

impl<'a> BlockView<'a> {
    pub fn new(field: &VolumeField, bytes: &'a [u8]) -> Result<Self, String> {
        Ok(Self { bytes, layout: volume_layout(field, bytes)? })
    }

    /// A block whose layout was validated when it was stored.
    pub fn trusted(bytes: &'a [u8], layout: VolumeLayout) -> Self {
        Self { bytes, layout }
    }

    pub fn key(&self) -> [i32; 3] {
        self.layout.key
    }

    pub fn valid(&self, i: usize) -> bool {
        self.bytes[self.layout.validity + i / 8] >> (i % 8) & 1 == 1
    }

    pub fn get(&self, i: usize, channel: Channel) -> Option<f64> {
        if !self.valid(i) {
            return None;
        }
        let at = match channel {
            Channel::Value => self.layout.values,
            Channel::Weight => self.layout.weights?,
        };
        Some(f64::from(f32_at(self.bytes, at + i * 4)))
    }

    pub fn color(&self, i: usize) -> Option<[u8; 4]> {
        let at = self.layout.colors? + i * 4;
        self.valid(i).then(|| self.bytes[at..at + 4].try_into().unwrap())
    }
}

/// A volume's blocks by key, read with validity-aware interpolation.
pub struct Volume<'a> {
    pub field: &'a VolumeField,
    blocks: HashMap<[i32; 3], BlockView<'a>>,
}

/// One field read: no value without valid support, never a guess.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldSample {
    pub value: Option<f64>,
    pub support: Support,
    /// Central difference one voxel apart, only on valid support.
    pub gradient: Option<[f64; 3]>,
}

impl<'a> Volume<'a> {
    pub fn new(field: &'a VolumeField, blocks: impl IntoIterator<Item = BlockView<'a>>) -> Self {
        Self { field, blocks: blocks.into_iter().map(|b| (b.key(), b)).collect() }
    }

    pub fn blocks(&self) -> impl Iterator<Item = &[i32; 3]> {
        self.blocks.keys()
    }

    fn locate(&self, v: [i64; 3]) -> Result<(&BlockView<'a>, usize), Support> {
        let side = i64::from(self.field.block_side);
        let key = v.map(|c| c.div_euclid(side));
        if key.iter().any(|k| k.unsigned_abs() > MAX_BLOCK_KEY as u64) {
            return Err(Support::Unallocated);
        }
        let block = self.blocks.get(&key.map(|k| k as i32)).ok_or(Support::Unallocated)?;
        let (s, local) = (side as usize, v.map(|c| c.rem_euclid(side) as usize));
        Ok((block, local[0] + s * (local[1] + s * local[2])))
    }

    /// The sample of integer voxel `v`.
    pub fn voxel(&self, v: [i64; 3], channel: Channel) -> Result<f64, Support> {
        let (block, i) = self.locate(v)?;
        block.get(i, channel).ok_or(Support::Unknown)
    }

    pub fn voxel_color(&self, v: [i64; 3]) -> Option<[u8; 4]> {
        let (block, i) = self.locate(v).ok()?;
        block.color(i)
    }

    /// The voxel containing `p` (volume frame, metres).
    pub fn voxel_of(&self, p: [f64; 3]) -> [i64; 3] {
        p.map(|c| (c / self.field.voxel_m).floor() as i64)
    }

    pub fn at(&self, p: [f64; 3], channel: Channel, interpolation: Interpolation) -> Result<f64, Support> {
        if interpolation == Interpolation::Nearest {
            return self.voxel(self.voxel_of(p), channel);
        }
        let f = p.map(|c| c / self.field.voxel_m - 0.5);
        let base = f.map(|c| c.floor() as i64);
        let t = [0, 1, 2].map(|a| f[a] - base[a] as f64);
        let (mut sum, mut missing) = (0.0, None);
        for corner in 0..8 {
            let offset = [corner & 1, corner >> 1 & 1, corner >> 2 & 1];
            match self.voxel([0, 1, 2].map(|a| base[a] + offset[a] as i64), channel) {
                Ok(value) => sum += value * (0..3).map(|a| if offset[a] == 1 { t[a] } else { 1.0 - t[a] }).product::<f64>(),
                Err(support) => missing = missing.max(Some(support)),
            }
        }
        missing.map_or(Ok(sum), Err)
    }

    pub fn sample(&self, p: [f64; 3], channel: Channel, interpolation: Interpolation, gradient: bool) -> FieldSample {
        let h = self.field.voxel_m;
        let value = self.at(p, channel, interpolation);
        let gradient = gradient
            .then(|| {
                let mut g = [0.0; 3];
                for a in 0..3 {
                    let (mut lo, mut hi) = (p, p);
                    lo[a] -= h;
                    hi[a] += h;
                    g[a] = (self.at(hi, channel, interpolation).ok()? - self.at(lo, channel, interpolation).ok()?) / (2.0 * h);
                }
                Some(g)
            })
            .flatten();
        FieldSample { value: value.ok(), support: value.err().unwrap_or(Support::Valid), gradient }
    }
}

/// A rectangle sampled through the field, in the volume frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VolumeSlice {
    pub center_m: [f64; 3],
    /// Orthogonal unit axes spanning the plane; the normal is u × v.
    pub u: [f64; 3],
    pub v: [f64; 3],
    pub size_m: [f64; 2],
    /// Edge of one sampled cell.
    pub cell_m: f64,
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

impl VolumeSlice {
    /// A slice across `axis` (0 x, 1 y, 2 z) through `center_m`.
    pub fn across(axis: usize, center_m: [f64; 3], size_m: [f64; 2], cell_m: f64) -> Self {
        let unit = |a: usize| {
            let mut e = [0.0; 3];
            e[a % 3] = 1.0;
            e
        };
        Self { center_m, u: unit(axis + 1), v: unit(axis + 2), size_m, cell_m }
    }

    pub fn normal(&self) -> [f64; 3] {
        let (u, v) = (self.u, self.v);
        [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]]
    }

    pub fn cells(&self) -> [u64; 2] {
        self.size_m.map(|s| (s / self.cell_m).ceil() as u64)
    }

    /// The centre of cell `(i, j)`.
    pub fn point(&self, i: u64, j: u64) -> [f64; 3] {
        let a = (i as f64 + 0.5) * self.cell_m - self.size_m[0] / 2.0;
        let b = (j as f64 + 0.5) * self.cell_m - self.size_m[1] / 2.0;
        [0, 1, 2].map(|k| self.center_m[k] + a * self.u[k] + b * self.v[k])
    }

    pub fn validate(&self) -> Result<(), String> {
        let finite = self.center_m.iter().chain(&self.u).chain(&self.v).chain(&self.size_m).all(|v| v.is_finite());
        if !finite
            || (dot(self.u, self.u) - 1.0).abs() > 1e-6
            || (dot(self.v, self.v) - 1.0).abs() > 1e-6
            || dot(self.u, self.v).abs() > 1e-6
        {
            return Err("slice axes must be finite orthogonal unit vectors".into());
        }
        if self.size_m.iter().any(|s| *s <= 0.0 || *s > 10_000.0) || !self.cell_m.is_finite() || self.cell_m <= 0.0 {
            return Err("slices need a positive extent and cell".into());
        }
        Ok(())
    }
}

fn contour_white() -> [u8; 4] {
    [255, 255, 255, 255]
}

/// How a volume is shown: planes sampled through it, coloured by the dataset's color mode.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VolumeView {
    pub slices: Vec<VolumeSlice>,
    #[serde(default)]
    pub interpolation: Interpolation,
    /// Cells whose value lies outside this band are left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub band: Option<[f64; 2]>,
    /// Levels marked where neighbouring cells straddle them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contours: Vec<f64>,
    #[serde(default = "contour_white")]
    pub contour_rgba: [u8; 4],
    /// Unknown and unallocated cells: a checker of this colour and a darker one.
    pub unknown_rgba: [u8; 4],
    /// The slice border; none draws no border.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_rgba: Option<[u8; 4]>,
}

impl VolumeView {
    pub fn validate_intrinsic(&self) -> Result<(), String> {
        if self.slices.len() > MAX_SLICES || self.contours.len() > MAX_CONTOURS {
            return Err("a volume view holds at most 4 slices and 8 contours".into());
        }
        let mut cells = 0u64;
        for slice in &self.slices {
            slice.validate()?;
            cells = cells.saturating_add(slice.cells()[0].saturating_mul(slice.cells()[1]));
        }
        if cells > MAX_SLICE_CELLS {
            return Err("slices exceed 65536 cells".into());
        }
        if self.band.is_some_and(|b| !b.iter().all(|v| v.is_finite()) || b[0] > b[1]) || !self.contours.iter().all(|v| v.is_finite()) {
            return Err("band and contour levels must be finite and ordered".into());
        }
        Ok(())
    }

    pub fn validate(&self, field: &VolumeField) -> Result<(), String> {
        self.validate_intrinsic()?;
        if self.slices.iter().any(|slice| slice.cell_m < field.voxel_m / 4.0) {
            return Err("slice cells must be at least a quarter voxel".into());
        }
        Ok(())
    }
}

/// Reads of one volume at volume-frame points.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldQuery {
    pub points: Vec<[f64; 3]>,
    pub channel: Channel,
    pub interpolation: Interpolation,
    pub gradient: bool,
    /// The generation to read; the current one when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
}

impl FieldQuery {
    pub fn validate(&self) -> Result<(), String> {
        if self.points.is_empty() || self.points.len() > MAX_FIELD_POINTS || !self.points.iter().flatten().all(|v| v.is_finite()) {
            return Err("field queries take 1..64 finite points".into());
        }
        if self.generation == Some(0) {
            return Err("generation zero is not retained data".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldAnswer {
    pub revision: super::DatasetRevision,
    pub field: VolumeField,
    /// The voxel spacing values and gradients were derived from.
    pub spacing_m: f64,
    pub interpolation: Interpolation,
    pub samples: Vec<FieldSample>,
}
