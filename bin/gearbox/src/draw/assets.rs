//! Mesh files a draw names (props `asset`): Wavefront OBJ and STL (binary
//! or ASCII), read once and kept by path. Coordinates are taken as metres
//! in the entity's frame.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bevy::math::Vec3;

/// Triangles of a mesh file with a normal per vertex.
#[derive(Debug, Default)]
pub struct RawMesh {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub indices: Vec<u32>,
}

static CACHE: Mutex<Option<HashMap<PathBuf, Option<Arc<RawMesh>>>>> = Mutex::new(None);

/// The mesh at `asset`, looked up as given and under `assets/`.
pub fn load(asset: &str) -> Option<Arc<RawMesh>> {
    let path = [PathBuf::from(asset), Path::new("assets").join(asset)].into_iter().find(|p| p.is_file())?;
    let mut cache = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let cache = cache.get_or_insert_with(HashMap::new);
    if let Some(hit) = cache.get(&path) {
        return hit.clone();
    }
    let bytes = std::fs::read(&path).ok();
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    let mesh = bytes.and_then(|b| match ext.as_str() {
        "obj" => parse_obj(&String::from_utf8_lossy(&b)),
        "stl" => parse_stl(&b),
        _ => None,
    });
    let mesh = mesh.map(|mut m| {
        smooth_normals(&mut m);
        Arc::new(m)
    });
    if mesh.is_none() {
        bevy::log::warn!("draw: cannot read mesh {}", path.display());
    }
    cache.insert(path, mesh.clone());
    mesh
}

/// Vertices and faces of an OBJ, polygons fanned into triangles.
pub fn parse_obj(text: &str) -> Option<RawMesh> {
    let mut vertices = Vec::new();
    let mut m = RawMesh::default();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("v") => {
                let v: Vec<f32> = it.take(3).filter_map(|t| t.parse().ok()).collect();
                if v.len() == 3 {
                    vertices.push(Vec3::new(v[0], v[1], v[2]));
                }
            }
            Some("f") => {
                let face: Vec<usize> = it
                    .filter_map(|t| t.split('/').next()?.parse::<i64>().ok())
                    .filter_map(|i| if i > 0 { Some(i as usize - 1) } else { (vertices.len() as i64 + i).try_into().ok() })
                    .filter(|i| *i < vertices.len())
                    .collect();
                for k in 1..face.len().saturating_sub(1) {
                    for i in [face[0], face[k], face[k + 1]] {
                        m.indices.push(i as u32);
                    }
                }
            }
            _ => {}
        }
    }
    m.positions = vertices;
    (!m.indices.is_empty()).then_some(m)
}

/// Triangles of a binary or ASCII STL.
pub fn parse_stl(bytes: &[u8]) -> Option<RawMesh> {
    let mut m = RawMesh::default();
    let binary_len = bytes.get(80..84).map(|n| u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize);
    if let Some(n) = binary_len.filter(|n| 84 + n * 50 == bytes.len()) {
        for t in 0..n {
            let at = 84 + t * 50 + 12;
            for k in 0..3 {
                let f = |o: usize| f32::from_le_bytes(bytes[at + k * 12 + o..at + k * 12 + o + 4].try_into().unwrap());
                m.indices.push(m.positions.len() as u32);
                m.positions.push(Vec3::new(f(0), f(4), f(8)));
            }
        }
    } else {
        for line in String::from_utf8_lossy(bytes).lines() {
            let mut it = line.split_whitespace();
            if it.next() == Some("vertex") {
                let v: Vec<f32> = it.take(3).filter_map(|t| t.parse().ok()).collect();
                if v.len() == 3 {
                    m.indices.push(m.positions.len() as u32);
                    m.positions.push(Vec3::new(v[0], v[1], v[2]));
                }
            }
        }
    }
    (m.indices.len() >= 3).then_some(m)
}

/// A normal per vertex from the faces around it.
fn smooth_normals(m: &mut RawMesh) {
    m.normals = vec![Vec3::ZERO; m.positions.len()];
    for t in m.indices.chunks_exact(3) {
        let (a, b, c) = (m.positions[t[0] as usize], m.positions[t[1] as usize], m.positions[t[2] as usize]);
        let n = (b - a).cross(c - a);
        for i in t {
            m.normals[*i as usize] += n;
        }
    }
    for n in &mut m.normals {
        *n = n.normalize_or(Vec3::Z);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obj_faces_fan_into_triangles_and_stl_reads_both_forms() {
        let obj = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1/1/1 2/2/2 3/3/3 4/4/4\n";
        let m = parse_obj(obj).unwrap();
        assert_eq!(m.indices, vec![0, 1, 2, 0, 2, 3]);
        let ascii = "solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid t\n";
        assert_eq!(parse_stl(ascii.as_bytes()).unwrap().positions.len(), 3);
        let mut bin = vec![0u8; 80];
        bin.extend(1u32.to_le_bytes());
        bin.extend([0u8; 12]);
        for v in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            for c in v {
                bin.extend(c.to_le_bytes());
            }
        }
        bin.extend([0u8; 2]);
        let m = parse_stl(&bin).unwrap();
        assert_eq!(m.positions[1], Vec3::X);
    }
}
