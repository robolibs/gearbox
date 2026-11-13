use std::collections::{BTreeMap, HashMap};

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{MeshVertexAttribute, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use bevy::render::render_resource::VertexFormat;

pub(super) const BARYCENTRIC: MeshVertexAttribute =
    MeshVertexAttribute::new("GhostBarycentric", 0x6b1d_4e10, VertexFormat::Float32x3);
pub(super) const EDGE_MASK: MeshVertexAttribute =
    MeshVertexAttribute::new("GhostEdgeMask", 0x6b1d_4e11, VertexFormat::Float32x3);
pub(super) const EDGE_START: MeshVertexAttribute =
    MeshVertexAttribute::new("GhostEdgeStart", 0x6b1d_4e12, VertexFormat::Float32x3);
pub(super) const EDGE_END: MeshVertexAttribute =
    MeshVertexAttribute::new("GhostEdgeEnd", 0x6b1d_4e13, VertexFormat::Float32x3);
const MAX_TRIANGLES: usize = 200_000;
const CREASE_COS: f32 = 0.819_152;
type PositionKey = [u32; 3];
type EdgeKey = [PositionKey; 2];

#[derive(Default)]
pub(crate) struct Cache(HashMap<AssetId<Mesh>, Handle<Mesh>>);

impl Cache {
    pub(super) fn mesh(
        &mut self,
        source: &Handle<Mesh>,
        meshes: &mut Assets<Mesh>,
    ) -> Handle<Mesh> {
        if let Some(cached) = self.0.get(&source.id()) {
            return cached.clone();
        }
        let Some(derived) = meshes.get(source).and_then(derive) else {
            return source.clone();
        };
        let handle = meshes.add(derived);
        self.0.insert(source.id(), handle.clone());
        handle
    }

    pub(super) fn refresh(&mut self, event: &AssetEvent<Mesh>, meshes: &mut Assets<Mesh>) {
        match event {
            AssetEvent::Modified { id } => {
                let Some(handle) = self.0.get(id) else { return };
                let Some(mesh) = meshes
                    .get(*id)
                    .map(|source| derive(source).unwrap_or_else(|| source.clone()))
                else {
                    return;
                };
                if let Some(mut target) = meshes.get_mut(handle) {
                    *target = mesh;
                }
            }
            AssetEvent::Removed { id } => {
                self.0.remove(id);
            }
            _ => {}
        }
    }
}

fn edge_key(a: [f32; 3], b: [f32; 3]) -> EdgeKey {
    let key = |p: [f32; 3]| p.map(|v| if v == 0.0 { 0 } else { v.to_bits() });
    let (a, b) = (key(a), key(b));
    if a < b { [a, b] } else { [b, a] }
}

fn derive(source: &Mesh) -> Option<Mesh> {
    if source.primitive_topology() != PrimitiveTopology::TriangleList {
        return None;
    }
    let VertexAttributeValues::Float32x3(positions) = source.attribute(Mesh::ATTRIBUTE_POSITION)?
    else {
        return None;
    };
    let count = source
        .indices()
        .map_or(positions.len(), |indices| indices.len());
    if count == 0 || count % 3 != 0 || count / 3 > MAX_TRIANGLES {
        return None;
    }
    let indices: Vec<usize> = source
        .indices()
        .map_or_else(|| (0..count).collect(), |indices| indices.iter().collect());
    if indices.iter().any(|i| {
        positions
            .get(*i)
            .is_none_or(|p| !Vec3::from_array(*p).is_finite())
    }) {
        return None;
    }
    let source_normals = match source.attribute(Mesh::ATTRIBUTE_NORMAL) {
        Some(VertexAttributeValues::Float32x3(normals)) if normals.len() == positions.len() => {
            Some(normals)
        }
        _ => None,
    };
    let mut adjacency = BTreeMap::<EdgeKey, (Vec3, usize, bool, f32)>::new();
    let mut face_normals = Vec::with_capacity(count / 3);
    for triangle in indices.chunks_exact(3) {
        let p = [
            positions[triangle[0]],
            positions[triangle[1]],
            positions[triangle[2]],
        ];
        let [a, b, c] = p.map(Vec3::from_array);
        let normal = (b - a).cross(c - a).normalize_or_zero();
        face_normals.push(normal);
        if normal == Vec3::ZERO {
            continue;
        }
        for [a, b] in [[p[1], p[2]], [p[2], p[0]], [p[0], p[1]]] {
            let length = Vec3::from_array(a).distance_squared(Vec3::from_array(b));
            adjacency
                .entry(edge_key(a, b))
                .and_modify(|edge| {
                    edge.1 += 1;
                    edge.2 |= edge.0.dot(normal).abs() < CREASE_COS;
                })
                .or_insert((normal, 1, false, length));
        }
    }
    let mut features: Vec<_> = adjacency
        .into_iter()
        .filter_map(|(key, (_, count, crease, length))| {
            (count != 2 || crease).then_some((key, length))
        })
        .collect();
    features.sort_by(|a, b| b.1.total_cmp(&a.1));
    features.truncate(super::super::visual::FEATURE_EDGE_LIMIT);
    let feature_points: Vec<_> = features
        .iter()
        .map(|(key, _)| key.map(|p| Vec3::from_array(p.map(f32::from_bits))))
        .collect();
    let accents = super::super::structure::accents(&feature_points);
    let selected: BTreeMap<_, _> = features
        .into_iter()
        .zip(accents)
        .map(|((key, _), accent)| (key, accent))
        .collect();
    let mut points = Vec::with_capacity(count);
    let mut normals = Vec::with_capacity(count);
    let mut barycentric = Vec::with_capacity(count);
    let mut masks = Vec::with_capacity(count);
    let mut starts = Vec::with_capacity(count);
    let mut ends = Vec::with_capacity(count);
    for (face, triangle) in indices.chunks_exact(3).enumerate() {
        let p = [
            positions[triangle[0]],
            positions[triangle[1]],
            positions[triangle[2]],
        ];
        let data = [[p[1], p[2]], [p[2], p[0]], [p[0], p[1]]].map(|[a, b]| {
            let key = edge_key(a, b);
            let Some(accent) = selected.get(&key) else {
                return [0.0; 3];
            };
            let forward = Vec3::from_array(a) == Vec3::from_array(key[0].map(f32::from_bits));
            let [start, end] = if forward {
                accent.phase
            } else {
                [accent.phase[1], accent.phase[0]]
            };
            [f32::from(accent.mask | 1), start, end]
        });
        for corner in 0..3 {
            points.push(p[corner]);
            let normal = source_normals
                .map(|values| Vec3::from_array(values[triangle[corner]]))
                .filter(|v| v.is_finite() && v.length_squared() > 1e-12)
                .unwrap_or(face_normals[face]);
            normals.push(normal.to_array());
            barycentric.push([
                u8::from(corner == 0) as f32,
                u8::from(corner == 1) as f32,
                u8::from(corner == 2) as f32,
            ]);
            masks.push(data.map(|d| d[0]));
            starts.push(data.map(|d| d[1]));
            ends.push(data.map(|d| d[2]));
        }
    }
    Some(
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, points)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(BARYCENTRIC, barycentric)
        .with_inserted_attribute(EDGE_MASK, masks)
        .with_inserted_attribute(EDGE_START, starts)
        .with_inserted_attribute(EDGE_END, ends),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::Indices;

    fn quad() -> Mesh {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
        )
        .with_inserted_indices(Indices::U16(vec![0, 1, 2, 0, 2, 3]))
    }

    #[test]
    fn boundaries_survive_but_planar_triangulation_does_not() {
        let source = quad();
        let result = derive(&source).unwrap();
        let Some(VertexAttributeValues::Float32x3(masks)) = result.attribute(EDGE_MASK) else {
            panic!()
        };
        assert_eq!(
            masks,
            &vec![[7.0, 0.0, 7.0]; 3]
                .into_iter()
                .chain(vec![[7.0, 7.0, 0.0]; 3])
                .collect::<Vec<_>>()
        );
        assert!(!source.contains_attribute(BARYCENTRIC));
        assert!(!source.contains_attribute(Mesh::ATTRIBUTE_NORMAL));
        assert_eq!(source.indices().unwrap().len(), 6);
    }

    #[test]
    fn duplicated_seams_weld_and_real_creases_remain() {
        for (height, crease) in [(0.0, false), (2.0, true)] {
            let source = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::default(),
            )
            .with_inserted_attribute(
                Mesh::ATTRIBUTE_POSITION,
                vec![
                    [0.0, 0.0, 0.0],
                    [1.0, 0.0, 0.0],
                    [1.0, 1.0, 0.0],
                    [-0.0, 0.0, 0.0],
                    [1.0, 1.0, 0.0],
                    [0.0, 1.0, height],
                ],
            );
            let result = derive(&source).unwrap();
            let Some(VertexAttributeValues::Float32x3(masks)) = result.attribute(EDGE_MASK) else {
                panic!()
            };
            assert_eq!(masks[0][1] as u8 & 1, u8::from(crease));
            assert_eq!(masks[3][2] as u8 & 1, u8::from(crease));
        }
    }

    #[test]
    fn round_boundaries_have_four_stations_without_triangle_spokes() {
        let count = 64;
        let mut points = vec![[0.0; 3]];
        points.extend((0..count).map(|i| {
            let angle = std::f32::consts::TAU * i as f32 / count as f32;
            [angle.cos(), angle.sin(), 0.0]
        }));
        let indices: Vec<u32> = (0..count)
            .flat_map(|i| [0, i + 1, (i + 1) % count + 1])
            .collect();
        let source = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, points.clone())
        .with_inserted_indices(Indices::U32(indices.clone()));
        let derived = derive(&source).unwrap();
        let values = |attribute| {
            let Some(VertexAttributeValues::Float32x3(values)) = derived.attribute(attribute)
            else {
                panic!()
            };
            values
        };
        let (masks, starts, ends) = (values(EDGE_MASK), values(EDGE_START), values(EDGE_END));
        let mut periods = 0.0;
        for triangle in 0..count as usize {
            let at = triangle * 3;
            assert_eq!(masks[at], [15.0, 0.0, 0.0]);
            periods += (ends[at][0] - starts[at][0]).abs();
            for corner in 0..3 {
                assert_eq!(
                    values(Mesh::ATTRIBUTE_POSITION)[at + corner],
                    points[indices[at + corner] as usize]
                );
                assert_eq!(masks[at + corner], masks[at]);
                assert_eq!(starts[at + corner], starts[at]);
                assert_eq!(ends[at + corner], ends[at]);
            }
        }
        assert!((periods - 4.0).abs() < 1e-5);
        assert!(!source.contains_attribute(EDGE_START));
    }

    #[test]
    fn unsupported_and_invalid_meshes_keep_source_fallback() {
        let lines = Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::default());
        assert!(derive(&lines).is_none());
        let invalid = quad().with_inserted_indices(Indices::U32(vec![0, 1, 99]));
        assert!(derive(&invalid).is_none());
        let oversized =
            quad().with_inserted_indices(Indices::U32(vec![0; (MAX_TRIANGLES + 1) * 3]));
        assert!(derive(&oversized).is_none());
        let mut meshes = Assets::default();
        let handle = meshes.add(lines);
        assert_eq!(Cache::default().mesh(&handle, &mut meshes), handle);
    }

    #[test]
    fn cache_reuses_meshes_and_refreshes_without_source_mutation() {
        let mut meshes = Assets::default();
        let source = meshes.add(quad());
        let mut cache = Cache::default();
        let first = cache.mesh(&source, &mut meshes);
        assert_ne!(first, source);
        assert_eq!(cache.mesh(&source, &mut meshes), first);
        meshes.get_mut(&source).unwrap().insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
                [1.0, 1.0, 1.0],
                [0.0, 1.0, 1.0],
            ],
        );
        cache.refresh(&AssetEvent::Modified { id: source.id() }, &mut meshes);
        let Some(VertexAttributeValues::Float32x3(points)) = meshes
            .get(&first)
            .unwrap()
            .attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!()
        };
        assert_eq!(points[0][2], 1.0);
        assert!(!meshes.get(&source).unwrap().contains_attribute(BARYCENTRIC));
        cache.refresh(&AssetEvent::Removed { id: source.id() }, &mut meshes);
        assert!(cache.0.is_empty());
    }
}
