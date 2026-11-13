//! One indexed triangle block per page (`PageEncoding::MeshBlockV1`), little-endian:
//! magic `GBM1`, vertex count, index count, flags (bit 0 normals, bit 1 RGBA8,
//! bit 2 confidence), then positions (3 × f32 per vertex), the optional arrays in
//! flag order, then u32 indices, three per triangle. Normals are the caller's own
//! authored normals and must be unit length; confidence is 0..1.

use serde::{Deserialize, Serialize};

pub const MESH_MAGIC: &[u8; 4] = b"GBM1";
pub const MAX_MESH_VERTICES: u32 = 65_536;
pub const MAX_MESH_TRIANGLES: u32 = 131_072;
const HEADER: usize = 16;
const NORMALS: u32 = 1;
const COLORS: u32 = 2;
const CONFIDENCE: u32 = 4;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MeshBlock {
    pub positions: Vec<[f32; 3]>,
    pub normals: Option<Vec<[f32; 3]>>,
    pub colors: Option<Vec<[u8; 4]>>,
    pub confidence: Option<Vec<f32>>,
    pub indices: Vec<u32>,
}

impl MeshBlock {
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }
}

/// Byte offsets of a validated block's arrays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeshLayout {
    pub vertices: u32,
    pub indices: u32,
    pub flags: u32,
    pub positions: usize,
    pub normals: Option<usize>,
    pub colors: Option<usize>,
    pub confidence: Option<usize>,
    pub index_offset: usize,
}

impl MeshLayout {
    pub fn triangles(&self) -> u32 {
        self.indices / 3
    }
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn f32_at(bytes: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

pub fn encode_mesh_block(block: &MeshBlock) -> Result<Vec<u8>, String> {
    let n = block.positions.len();
    let same = |len: Option<usize>| len.is_none_or(|len| len == n);
    if !same(block.normals.as_ref().map(Vec::len))
        || !same(block.colors.as_ref().map(Vec::len))
        || !same(block.confidence.as_ref().map(Vec::len))
    {
        return Err("mesh vertex arrays disagree in length".into());
    }
    let flags = u32::from(block.normals.is_some()) * NORMALS
        | u32::from(block.colors.is_some()) * COLORS
        | u32::from(block.confidence.is_some()) * CONFIDENCE;
    let mut bytes = Vec::with_capacity(HEADER + n * 32 + block.indices.len() * 4);
    bytes.extend_from_slice(MESH_MAGIC);
    bytes.extend((n as u32).to_le_bytes());
    bytes.extend((block.indices.len() as u32).to_le_bytes());
    bytes.extend(flags.to_le_bytes());
    bytes.extend(block.positions.iter().flatten().flat_map(|v| v.to_le_bytes()));
    if let Some(normals) = &block.normals {
        bytes.extend(normals.iter().flatten().flat_map(|v| v.to_le_bytes()));
    }
    if let Some(colors) = &block.colors {
        bytes.extend(colors.iter().flatten());
    }
    if let Some(confidence) = &block.confidence {
        bytes.extend(confidence.iter().flat_map(|v| v.to_le_bytes()));
    }
    bytes.extend(block.indices.iter().flat_map(|v| v.to_le_bytes()));
    mesh_layout(&bytes)?;
    Ok(bytes)
}

/// Validates an encoded block in place, returning where its arrays lie.
pub fn mesh_layout(bytes: &[u8]) -> Result<MeshLayout, String> {
    if bytes.len() < HEADER || &bytes[..4] != MESH_MAGIC {
        return Err("invalid mesh block header".into());
    }
    let (vertices, indices, flags) = (u32_at(bytes, 4), u32_at(bytes, 8), u32_at(bytes, 12));
    if !(3..=MAX_MESH_VERTICES).contains(&vertices)
        || indices == 0
        || indices % 3 != 0
        || indices / 3 > MAX_MESH_TRIANGLES
        || flags & !(NORMALS | COLORS | CONFIDENCE) != 0
    {
        return Err("mesh block counts or flags outside their limits".into());
    }
    let n = vertices as usize;
    let mut at = HEADER;
    let mut take = |present: bool, stride: usize| {
        present.then(|| {
            let start = at;
            at += n * stride;
            start
        })
    };
    let positions = take(true, 12).unwrap();
    let normals = take(flags & NORMALS != 0, 12);
    let colors = take(flags & COLORS != 0, 4);
    let confidence = take(flags & CONFIDENCE != 0, 4);
    let index_offset = at;
    if bytes.len() != index_offset + indices as usize * 4 {
        return Err("mesh block length disagrees with its counts".into());
    }
    for v in 0..n * 3 {
        if !f32_at(bytes, positions + v * 4).is_finite() {
            return Err("mesh vertex position is not finite".into());
        }
    }
    if let Some(normals) = normals {
        for v in 0..n {
            let c = [0, 1, 2].map(|k| f64::from(f32_at(bytes, normals + (v * 3 + k) * 4)));
            let length = c.iter().map(|x| x * x).sum::<f64>();
            if !length.is_finite() || (length - 1.0).abs() > 0.002 {
                return Err("authored mesh normals must be finite unit vectors".into());
            }
        }
    }
    if let Some(confidence) = confidence {
        for v in 0..n {
            let c = f32_at(bytes, confidence + v * 4);
            if !(0.0..=1.0).contains(&c) {
                return Err("mesh confidence must be in 0..1".into());
            }
        }
    }
    for i in 0..indices as usize {
        if u32_at(bytes, index_offset + i * 4) >= vertices {
            return Err("mesh index outside its block".into());
        }
    }
    Ok(MeshLayout { vertices, indices, flags, positions, normals, colors, confidence, index_offset })
}

pub fn decode_mesh_block(bytes: &[u8]) -> Result<MeshBlock, String> {
    let layout = mesh_layout(bytes)?;
    let n = layout.vertices as usize;
    let vec3 = |at: usize, v: usize| [0, 1, 2].map(|k| f32_at(bytes, at + (v * 3 + k) * 4));
    Ok(MeshBlock {
        positions: (0..n).map(|v| vec3(layout.positions, v)).collect(),
        normals: layout.normals.map(|at| (0..n).map(|v| vec3(at, v)).collect()),
        colors: layout.colors.map(|at| (0..n).map(|v| bytes[at + v * 4..at + v * 4 + 4].try_into().unwrap()).collect()),
        confidence: layout.confidence.map(|at| (0..n).map(|v| f32_at(bytes, at + v * 4)).collect()),
        indices: (0..layout.indices as usize).map(|i| u32_at(bytes, layout.index_offset + i * 4)).collect(),
    })
}
