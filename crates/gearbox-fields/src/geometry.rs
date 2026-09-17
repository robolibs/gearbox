//! Field-boundary clipping of existing terrain triangles without height offsets.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipped_sloping_surface_preserves_height_and_area() {
        let source = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![
                [0.0, 0.0, 0.0],
                [4.0, 4.0, 0.0],
                [0.0, 8.0, 4.0],
                [4.0, 12.0, 4.0],
            ],
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; 4])
        .with_inserted_indices(Indices::U32(vec![0, 2, 1, 1, 2, 3]));
        let mut total_area = 0.0;
        for (min_x, max_x) in [(0.0, 1.5), (1.5, 4.0)] {
            let bounds = FieldBounds {
                min: Vec2::new(min_x, 0.0),
                max: Vec2::new(max_x, 4.0),
            };
            let clipped = clip_mesh(&source, bounds).unwrap();
            let VertexAttributeValues::Float32x3(vertices) =
                clipped.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
            else {
                panic!("positions")
            };
            for p in vertices {
                assert!(bounds.contains(Vec2::new(p[0], p[2])));
                assert!((p[1] - p[0] - 2.0 * p[2]).abs() < 1e-5);
            }
            for tri in vertices.chunks_exact(3) {
                let a = Vec3::from_array(tri[0]);
                let b = Vec3::from_array(tri[1]);
                let c = Vec3::from_array(tri[2]);
                total_area += (b - a).cross(c - a).y.abs() * 0.5;
            }
        }
        assert!((total_area - 16.0).abs() < 1e-5);
    }
}

use super::layout::FieldBounds;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;

#[derive(Clone, Copy)]
struct Vertex {
    position: Vec3,
    normal: Vec3,
    uv: Vec2,
}

impl Vertex {
    fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            position: self.position.lerp(other.position, t),
            normal: self.normal.lerp(other.normal, t),
            uv: self.uv.lerp(other.uv, t),
        }
    }
}

fn clip_plane(polygon: Vec<Vertex>, axis: usize, edge: f32, greater: bool) -> Vec<Vertex> {
    let mut clipped = Vec::new();
    let Some(&last) = polygon.last() else {
        return clipped;
    };
    let mut previous = last;
    let inside = |v: Vertex| {
        if greater {
            v.position[axis] >= edge
        } else {
            v.position[axis] <= edge
        }
    };
    for current in polygon {
        if inside(previous) != inside(current) {
            let t = (edge - previous.position[axis])
                / (current.position[axis] - previous.position[axis]);
            let mut intersection = previous.lerp(current, t);
            intersection.position[axis] = edge;
            clipped.push(intersection);
        }
        if inside(current) {
            clipped.push(current);
        }
        previous = current;
    }
    clipped
}

pub fn mesh_bounds(mesh: &Mesh) -> Option<FieldBounds> {
    let VertexAttributeValues::Float32x3(positions) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)?
    else {
        return None;
    };
    let mut min = Vec2::splat(f32::INFINITY);
    let mut max = Vec2::splat(f32::NEG_INFINITY);
    for p in positions {
        let point = Vec2::new(p[0], p[2]);
        min = min.min(point);
        max = max.max(point);
    }
    Some(FieldBounds { min, max })
}

pub fn clip_mesh(mesh: &Mesh, bounds: FieldBounds) -> Option<Mesh> {
    let VertexAttributeValues::Float32x3(positions) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)?
    else {
        return None;
    };
    let VertexAttributeValues::Float32x3(normals) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL)? else {
        return None;
    };
    let uvs = match mesh.attribute(Mesh::ATTRIBUTE_UV_0) {
        Some(VertexAttributeValues::Float32x2(uvs)) => Some(uvs),
        _ => None,
    };
    let indices: Vec<usize> = match mesh.indices() {
        Some(indices) => indices.iter().collect(),
        None => (0..positions.len()).collect(),
    };
    let mut out_positions = Vec::new();
    let mut out_normals = Vec::new();
    let mut out_uvs = Vec::new();
    let mut out_indices = Vec::new();
    for triangle in indices.chunks_exact(3) {
        let mut polygon = triangle
            .iter()
            .map(|&i| Vertex {
                position: Vec3::from_array(positions[i]),
                normal: Vec3::from_array(normals[i]),
                uv: uvs
                    .map(|uvs| Vec2::from_array(uvs[i]))
                    .unwrap_or(Vec2::ZERO),
            })
            .collect();
        for (axis, edge, greater) in [
            (0, bounds.min.x, true),
            (0, bounds.max.x, false),
            (2, bounds.min.y, true),
            (2, bounds.max.y, false),
        ] {
            polygon = clip_plane(polygon, axis, edge, greater);
        }
        if polygon.len() < 3 {
            continue;
        }
        for i in 1..polygon.len() - 1 {
            let vertices = [polygon[0], polygon[i], polygon[i + 1]];
            if (vertices[1].position - vertices[0].position)
                .cross(vertices[2].position - vertices[0].position)
                .length_squared()
                < 1e-14
            {
                continue;
            }
            for vertex in vertices {
                out_indices.push(out_positions.len() as u32);
                out_positions.push(vertex.position.to_array());
                out_normals.push(vertex.normal.to_array());
                out_uvs.push(vertex.uv.to_array());
            }
        }
    }
    if out_indices.is_empty() {
        return None;
    }
    Some(
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, out_positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, out_normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, out_uvs)
        .with_inserted_indices(Indices::U32(out_indices)),
    )
}
