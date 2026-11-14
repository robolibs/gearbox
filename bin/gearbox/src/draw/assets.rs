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
    let path = [PathBuf::from(asset), Path::new("assets").join(asset)]
        .into_iter()
        .find(|p| p.is_file())?;
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let cache = cache.get_or_insert_with(HashMap::new);
    if let Some(hit) = cache.get(&path) {
        return hit.clone();
    }
    let bytes = std::fs::read(&path).ok();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
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

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum ObjNormal {
    Authored(usize),
    Smooth(u64),
    Face(usize),
}

fn obj_index(text: &str, len: usize) -> Option<usize> {
    let index = text.parse::<i64>().ok()?;
    let index = if index > 0 {
        index - 1
    } else {
        i64::try_from(len).ok()?.checked_add(index)?
    };
    usize::try_from(index).ok().filter(|i| *i < len)
}

fn obj_vector<'a>(mut fields: impl Iterator<Item = &'a str>) -> Option<Vec3> {
    let v = Vec3::new(
        fields.next()?.parse().ok()?,
        fields.next()?.parse().ok()?,
        fields.next()?.parse().ok()?,
    );
    v.is_finite().then_some(v)
}

fn obj_triangles(face: &[u32], positions: &[Vec3]) -> Option<Vec<u32>> {
    if face.len() < 3 {
        return None;
    }
    if face.len() == 3 {
        return Some(face.to_vec());
    }
    let fan = || {
        (1..face.len() - 1)
            .flat_map(|k| [face[0], face[k], face[k + 1]])
            .collect()
    };
    let origin = positions[face[0] as usize].as_dvec3();
    let points: Vec<_> = face
        .iter()
        .map(|i| positions[*i as usize].as_dvec3() - origin)
        .collect();
    let normal = (0..points.len())
        .map(|i| points[i].cross(points[(i + 1) % points.len()]))
        .sum::<bevy::math::DVec3>();
    let n = normal.abs();
    let axis = if n.x >= n.y && n.x >= n.z {
        0
    } else if n.y >= n.z {
        1
    } else {
        2
    };
    if normal[axis] == 0.0 {
        return Some(fan());
    }
    let scale = points
        .iter()
        .map(|p| p.abs().max_element())
        .fold(0.0_f64, f64::max);
    let projected: Vec<_> = points
        .iter()
        .map(|p| {
            [
                (p[(axis + 1) % 3] / scale) as f32,
                (p[(axis + 2) % 3] / scale) as f32,
            ]
        })
        .collect();
    let turn = |a: usize, b: usize, c: usize| {
        let [a, b, c] = [a, b, c].map(|i| projected[i].map(f64::from));
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    };
    let sign = normal[axis].signum();
    if (0..face.len()).all(|i| turn(i, (i + 1) % face.len(), (i + 2) % face.len()) * sign >= 0.0) {
        return Some(fan());
    }
    let triangles = gearbox_api::wire::draw_geom::triangulate(&[&projected]);
    let expected = (1..face.len() - 1)
        .map(|i| turn(0, i, i + 1))
        .sum::<f64>()
        .abs();
    let area = triangles
        .iter()
        .map(|t| turn(t[0] as usize, t[1] as usize, t[2] as usize).abs())
        .sum::<f64>();
    if triangles.is_empty() || (area - expected).abs() > expected * 1e-5 {
        return None;
    }
    Some(
        triangles
            .into_iter()
            .flat_map(|mut t| {
                if turn(t[0] as usize, t[1] as usize, t[2] as usize) * sign < 0.0 {
                    t.swap(1, 2);
                }
                t.map(|i| face[i as usize])
            })
            .collect(),
    )
}

/// OBJ polygons triangulated in their face plane, split at authored normals and smoothing boundaries.
pub fn parse_obj(text: &str) -> Option<RawMesh> {
    let mut vertices = Vec::new();
    let mut normals = Vec::new();
    let mut corners = HashMap::new();
    let mut smoothing = 0;
    let mut m = RawMesh::default();
    for line in text.lines() {
        let mut it = line.split('#').next()?.split_whitespace();
        match it.next() {
            Some("v") => vertices.push(obj_vector(it)?),
            Some("vn") => normals.push(obj_vector(it)?.normalize_or_zero()),
            Some("s") => {
                smoothing = match it.next()? {
                    "off" | "0" => 0,
                    "on" => 1,
                    group => group.parse().ok()?,
                };
            }
            Some("f") => {
                let mut face = Vec::new();
                for token in it {
                    let mut fields = token.split('/');
                    let vertex = obj_index(fields.next()?, vertices.len())?;
                    let _texture = fields.next();
                    let normal = match fields.next().filter(|s| !s.is_empty()) {
                        Some(index) => Some(obj_index(index, normals.len())?),
                        None => None,
                    };
                    if fields.next().is_some() {
                        return None;
                    }
                    let source = match normal.filter(|i| normals[*i] != Vec3::ZERO) {
                        Some(i) => ObjNormal::Authored(i),
                        None if smoothing > 0 => ObjNormal::Smooth(smoothing),
                        None => ObjNormal::Face(m.indices.len()),
                    };
                    let next = u32::try_from(m.positions.len()).ok()?;
                    let index = *corners.entry((vertex, source)).or_insert_with(|| {
                        m.positions.push(vertices[vertex]);
                        m.normals.push(match source {
                            ObjNormal::Authored(i) => normals[i],
                            _ => Vec3::ZERO,
                        });
                        next
                    });
                    face.push(index);
                }
                m.indices.extend(obj_triangles(&face, &m.positions)?);
            }
            _ => {}
        }
    }
    (!m.indices.is_empty()).then_some(m)
}

/// Triangles of a binary or ASCII STL.
pub fn parse_stl(bytes: &[u8]) -> Option<RawMesh> {
    let mut m = RawMesh::default();
    let binary_len = bytes
        .get(80..84)
        .map(|n| u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize);
    if let Some(n) = binary_len.filter(|n| 84 + n * 50 == bytes.len()) {
        for t in 0..n {
            let at = 84 + t * 50 + 12;
            for k in 0..3 {
                let f = |o: usize| {
                    f32::from_le_bytes(
                        bytes[at + k * 12 + o..at + k * 12 + o + 4]
                            .try_into()
                            .unwrap(),
                    )
                };
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

/// Fill missing vertex normals from adjacent faces, retaining authored normals.
fn smooth_normals(m: &mut RawMesh) {
    let mut generated = vec![Vec3::ZERO; m.positions.len()];
    for t in m.indices.chunks_exact(3) {
        let (a, b, c) = (
            m.positions[t[0] as usize],
            m.positions[t[1] as usize],
            m.positions[t[2] as usize],
        );
        let n = (b - a).cross(c - a);
        for i in t {
            generated[*i as usize] += n;
        }
    }
    m.normals.resize(m.positions.len(), Vec3::ZERO);
    for (normal, generated) in m.normals.iter_mut().zip(generated) {
        if *normal == Vec3::ZERO {
            *normal = generated.normalize_or(Vec3::Z);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CORNER: &str = "v 0 0 0\nv 1 0 0\nv 0 1 0\nv 0 0 1\n";

    fn obj_normals(text: &str) -> RawMesh {
        let mut mesh = parse_obj(text).unwrap();
        smooth_normals(&mut mesh);
        mesh
    }

    #[test]
    fn obj_preserves_authored_corner_normals_over_smoothing_groups() {
        let mesh = obj_normals(&format!(
            "{CORNER}vn 0 0 4\nvn 0 3 0\ns 1\nf 1//1 2//1 3//1\nf 1//-1 4//-1 2//-1\n"
        ));
        for (triangle, expected) in mesh.indices.chunks_exact(3).zip([Vec3::Z, Vec3::Y]) {
            for i in triangle {
                assert_eq!(mesh.normals[*i as usize], expected);
            }
        }
        assert_eq!(mesh.positions.len(), 6);
    }

    #[test]
    fn obj_flat_default_and_smoothing_boundaries_do_not_blend_creases() {
        for (first, second) in [("", ""), ("s off\n", ""), ("s 0\n", ""), ("s 1\n", "s 2\n")] {
            let mesh = obj_normals(&format!("{CORNER}{first}f 1 2 3\n{second}f 1 4 2\n"));
            for (triangle, expected) in mesh.indices.chunks_exact(3).zip([Vec3::Z, Vec3::Y]) {
                for i in triangle {
                    assert_eq!(mesh.normals[*i as usize], expected, "{first}/{second}");
                }
            }
            assert_eq!(mesh.positions.len(), 6);
        }
    }

    #[test]
    fn obj_shared_smoothing_group_keeps_a_continuous_surface() {
        let mesh = obj_normals(&format!("{CORNER}s 7\nf 1 2 3\nf -4 -1 -3\n"));
        assert_eq!(mesh.positions.len(), 4);
        for (position, normal) in mesh.positions.iter().zip(&mesh.normals) {
            let expected = if *position == Vec3::Y {
                Vec3::Z
            } else if *position == Vec3::Z {
                Vec3::Y
            } else {
                (Vec3::Y + Vec3::Z).normalize()
            };
            assert!(normal.abs_diff_eq(expected, 1e-6));
        }
    }

    #[test]
    fn obj_comments_and_invalid_indices_cannot_change_face_topology() {
        let mesh = obj_normals(&format!("{CORNER}f 1 2 3 # 4 is not another corner\n"));
        assert_eq!(mesh.indices.len(), 3);
        for face in [
            "1 2 0 3",
            "1 2 99 3",
            "1 2 invalid 3",
            "1 2 -99 3",
            "1//1 2//1 3//1",
        ] {
            assert!(
                parse_obj(&format!("{CORNER}f {face}\n")).is_none(),
                "{face}"
            );
        }
        assert!(parse_obj("v NaN 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").is_none());
    }

    #[test]
    fn concave_obj_faces_preserve_area_winding_positions_and_normals() {
        use std::fmt::Write;
        let outline = [
            (0.0, 0.0),
            (3.0, 0.0),
            (3.0, 1.0),
            (1.0, 1.0),
            (1.0, 2.0),
            (3.0, 2.0),
            (3.0, 3.0),
            (0.0, 3.0),
        ];
        for rotation in [
            bevy::math::Quat::IDENTITY,
            bevy::math::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            bevy::math::Quat::from_rotation_x(0.7) * bevy::math::Quat::from_rotation_y(0.4),
        ] {
            for reverse in [false, true] {
                for authored in [false, true] {
                    let shift = Vec3::new(10.0, -4.0, 7.0);
                    let points: Vec<_> = outline
                        .iter()
                        .map(|&(x, y)| shift + rotation * Vec3::new(x, y, 0.0))
                        .collect();
                    let normal = rotation * Vec3::Z * if reverse { -1.0 } else { 1.0 };
                    let mut text = String::new();
                    for p in &points {
                        writeln!(text, "v {} {} {}", p.x, p.y, p.z).unwrap();
                    }
                    if authored {
                        writeln!(text, "vn {} {} {}", normal.x, normal.y, normal.z).unwrap();
                    }
                    text.push('f');
                    for i in 0..points.len() {
                        let index = if reverse { points.len() - i } else { i + 1 };
                        write!(text, " {index}{}", if authored { "//1" } else { "" }).unwrap();
                    }
                    let mesh = obj_normals(&text);
                    assert_eq!(mesh.indices.len(), 18);
                    assert_eq!(mesh.positions.len(), points.len());
                    assert!(points.iter().all(|p| mesh.positions.contains(p)));
                    let mut area = 0.0;
                    for tri in mesh.indices.chunks_exact(3) {
                        let [a, b, c] = std::array::from_fn(|i| mesh.positions[tri[i] as usize]);
                        let cross = (b - a).cross(c - a);
                        assert!(
                            cross.dot(normal) > 0.0,
                            "triangle reversed in concave face: {tri:?}"
                        );
                        area += 0.5 * cross.length();
                        let centre = rotation.inverse() * ((a + b + c) / 3.0 - shift);
                        assert!(
                            !(centre.x > 1.001 && centre.y > 1.001 && centre.y < 1.999),
                            "triangle fills notch: {centre:?}"
                        );
                    }
                    assert!((area - 7.0).abs() < 1e-4, "concave polygon area: {area}");
                    assert!(mesh.normals.iter().all(|n| n.abs_diff_eq(normal, 1e-5)));
                }
            }
        }
    }

    #[test]
    fn concave_obj_triangulation_is_scale_independent() {
        let points = [
            [0.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            [3.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 2.0, 0.0],
            [3.0, 2.0, 0.0],
            [3.0, 3.0, 0.0],
            [0.0, 3.0, 0.0],
        ];
        for scale in [1e-12_f32, 1.0, 1e18] {
            let positions: Vec<_> = points.map(|p| Vec3::from_array(p) * scale).into();
            let indices = obj_triangles(&[0, 1, 2, 3, 4, 5, 6, 7], &positions).unwrap();
            assert_eq!(indices.len(), 18);
            let area = indices
                .chunks_exact(3)
                .map(|t| {
                    let [a, b, c] = std::array::from_fn(|i| {
                        positions[t[i] as usize].as_dvec3() / f64::from(scale)
                    });
                    0.5 * (b - a).cross(c - a).z
                })
                .sum::<f64>();
            assert!((area - 7.0).abs() < 1e-5, "{scale}: {area}");
        }
    }

    #[test]
    fn degenerate_obj_faces_do_not_discard_other_faces_or_share_flat_normals() {
        let mesh = obj_normals("v 0 0 0\nv 1 0 0\nv 2 0 0\nv 3 0 0\nv 0 0 1\nf 1 2 3 4\nf 1 5 2\n");
        assert_eq!(mesh.indices, [0, 1, 2, 0, 2, 3, 4, 5, 6]);
        assert_eq!(mesh.normals[..4], [Vec3::Z; 4]);
        assert_eq!(mesh.normals[4..], [Vec3::Y; 3]);
    }

    #[test]
    fn obj_faces_fan_into_triangles_and_stl_reads_both_forms() {
        let obj = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvn 0 0 1\nf 1/1/1 2/2/1 3/3/1 4/4/1\n";
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
