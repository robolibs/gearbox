//! Poly Haven plant clumps (CC0), packed by `scripts/pack_vegetation.py` and
//! drawn as instanced vegetation layers with their own albedo.

use super::profile::VegetationLayer;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use std::ops::Range;

const SHADER: &str = "embedded://gearbox_fields/clumps/shaders/clumps.wgsl";

pub struct ClumpsPlugin;

impl Plugin for ClumpsPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "shaders/clumps.wgsl");
        bevy::asset::embedded_asset!(app, "textures/grass_bermuda_01.png");
        bevy::asset::embedded_asset!(app, "textures/grass_medium_01.png");
        bevy::asset::embedded_asset!(app, "textures/shrub_sorrel_01.png");
        bevy::asset::embedded_asset!(app, "textures/celandine_01.png");
        bevy::asset::embedded_asset!(app, "textures/weed_plant_02.png");
        bevy::asset::embedded_asset!(app, "textures/dandelion_01.png");
        bevy::asset::embedded_asset!(app, "textures/nettle_plant.png");
    }
}

// One layer constructor per packed model: density per m² and fade distance.
macro_rules! pack {
    ($name:ident, $model:literal) => {
        pub fn $name(density: f32, fade_end: f32) -> VegetationLayer {
            fn template() -> Mesh {
                clump_mesh(include_bytes!(concat!("meshes/", $model, ".bin")))
            }
            VegetationLayer {
                shader: SHADER,
                template,
                density,
                fade_start: fade_end * 0.5,
                fade_end,
                inverse_square_thinning: true,
                follow_grass: 0.0,
                albedo: Some(concat!(
                    "embedded://gearbox_fields/clumps/textures/",
                    $model,
                    ".png"
                )),
                lod_band: [0.0, f32::MAX],
            }
        }
    };
}

pack!(bermuda, "grass_bermuda_01");
pack!(meadow_tufts, "grass_medium_01");
pack!(sorrel, "shrub_sorrel_01");
pack!(celandine, "celandine_01");
pack!(flat_weeds, "weed_plant_02");
pack!(dandelion, "dandelion_01");
pack!(nettle, "nettle_plant");

/// Mesh from a VEG1 pack: position, normal, uv, colour per vertex, u32 indices.
fn clump_mesh(bytes: &[u8]) -> Mesh {
    assert_eq!(&bytes[..4], b"VEG1", "not a clump pack");
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
    let (vertices, count) = (word(4), word(8));
    let floats: Vec<f32> = bytes[12..12 + vertices * 48]
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    let rows = floats.chunks_exact(12);
    let start = 12 + vertices * 48;
    let indices: Vec<u32> = bytes[start..start + count * 4]
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, rows.clone().map(|r| [r[0], r[1], r[2]]).collect::<Vec<_>>())
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, rows.clone().map(|r| [r[3], r[4], r[5]]).collect::<Vec<_>>())
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, rows.clone().map(|r| [r[6], r[7]]).collect::<Vec<_>>())
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, rows.map(|r| [r[8], r[9], r[10], r[11]]).collect::<Vec<_>>())
        .with_inserted_indices(Indices::U32(indices))
}

/// Index ranges of a clump mesh's variants, which the pack stores in order;
/// empty for procedural templates.
pub fn variant_ranges(mesh: &Mesh) -> Vec<Range<u32>> {
    let (Some(VertexAttributeValues::Float32x4(colours)), Some(Indices::U32(indices))) =
        (mesh.attribute(Mesh::ATTRIBUTE_COLOR), mesh.indices())
    else {
        return Vec::new();
    };
    let mut ranges: Vec<Range<u32>> = Vec::new();
    for (i, triangle) in indices.chunks_exact(3).enumerate() {
        let at = (i * 3) as u32;
        match ranges.get_mut(colours[triangle[0] as usize][0] as usize) {
            Some(range) => range.end = at + 3,
            None => ranges.push(at..at + 3),
        }
    }
    debug_assert!(ranges.len() <= 15, "the shader decodes at most 15 variants");
    ranges
}
