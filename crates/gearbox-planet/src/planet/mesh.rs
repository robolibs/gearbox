//! The single shared grid mesh instanced for every selected quadtree node.
//! Positions carry integer grid coordinates (i, j, skirt); the vertex shader
//! does all real positioning (CDLOD morph -> cube face -> sphere ->
//! displacement). A skirt ring duplicates the edge vertices with z=1; the
//! shader pulls those toward the planet center, hiding any residual boundary
//! gap (residency gating may transiently pair nodes >1 LOD apart, which the
//! morph alone does not cover).

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};

use crate::config::GRID_QUADS;

pub fn build_grid_mesh() -> Mesh {
    let n = GRID_QUADS + 1;
    let mut positions = Vec::with_capacity((n * n + 4 * n) as usize);
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    for j in 0..n {
        for i in 0..n {
            positions.push([i as f32, j as f32, 0.0]);
        }
    }
    let inner = |i: u32, j: u32| j * n + i;

    // Skirt ring: one duplicate per edge vertex, marked with z = 1.
    // Sides: [bottom j=0, top j=max, left i=0, right i=max].
    let mut skirt_base = [0u32; 4];
    let sides: [Box<dyn Fn(u32) -> (u32, u32)>; 4] = [
        Box::new(|k| (k, 0)),
        Box::new(|k| (k, GRID_QUADS)),
        Box::new(|k| (0, k)),
        Box::new(|k| (GRID_QUADS, k)),
    ];
    for (s, side) in sides.iter().enumerate() {
        skirt_base[s] = positions.len() as u32;
        for k in 0..n {
            let (i, j) = side(k);
            positions.push([i as f32, j as f32, 1.0]);
        }
    }

    for p in &positions {
        normals.push([0.0, 0.0, 1.0]);
        uvs.push([p[0] / GRID_QUADS as f32, p[1] / GRID_QUADS as f32]);
    }

    let mut indices = Vec::with_capacity((GRID_QUADS * GRID_QUADS * 6 + 4 * GRID_QUADS * 12) as usize);
    for j in 0..GRID_QUADS {
        for i in 0..GRID_QUADS {
            let a = inner(i, j);
            let b = a + 1;
            let c = a + n + 1;
            let d = a + n;
            // CCW seen from +z; face bases are right-handed so this is CCW
            // seen from outside the planet.
            indices.extend_from_slice(&[a, b, c, a, c, d]);
        }
    }
    // Skirt quads, both windings (visible whichever way the side faces).
    for (s, side) in sides.iter().enumerate() {
        for k in 0..GRID_QUADS {
            let (i0, j0) = side(k);
            let (i1, j1) = side(k + 1);
            let e0 = inner(i0, j0);
            let e1 = inner(i1, j1);
            let s0 = skirt_base[s] + k;
            let s1 = s0 + 1;
            indices.extend_from_slice(&[e0, e1, s1, e0, s1, s0]);
            indices.extend_from_slice(&[e1, e0, s0, e1, s0, s1]);
        }
    }

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}
