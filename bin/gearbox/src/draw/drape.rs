//! Terrain draping: a draped entity's z is height above the ground under
//! each point, and its lines and polygon rings are split so they bend over
//! the ground.

use bevy::prelude::*;
use gearbox_api::wire::draw::{INSTANCE_FLOATS, rgba, unpack_rgba};
use gearbox_api::{DrawEntity, draw_kind, line_mode};

/// Longest line segment left whole, metres.
pub const STEP_M: f32 = 0.5;
pub(super) const CORNER_SOURCES: &str = "__gearbox_corner_sources";

/// Height of the terrain over the entity's z = 0 at entity coordinates x,
/// y, `place` taking entity coordinates (Z up) into the home site's (Y up).
pub fn rise_fn(e: &DrawEntity, place: Transform) -> impl Fn(f32, f32) -> f32 + use<> {
    let anchored = e.props().get("drape_anchor").as_deref() == Some("true");
    let pose_height = if e.props().get("drape_pose_height").as_deref() == Some("true") {
        e.pz as f32
    } else {
        0.0
    };
    rise_from(place, anchored, pose_height, |x, y| {
        crate::globe::ground_height_at_physics(x as f64, y as f64) as f32
    })
}

fn rise_from(
    place: Transform,
    anchored: bool,
    pose_height: f32,
    ground: impl Fn(f32, f32) -> f32,
) -> impl Fn(f32, f32) -> f32 {
    move |x: f32, y: f32| -> f32 {
        let (x, y) = if anchored { (0.0, 0.0) } else { (x, y) };
        let s = place.transform_point(Vec3::new(x, y, 0.0));
        ground(s.x, s.z) - s.y + pose_height
    }
}

/// `e` raised onto the ground, `place` taking entity coordinates (Z up) into
/// the home site's (Y up).
pub fn drape(e: &DrawEntity, place: &Transform) -> DrawEntity {
    let up = if e.props().get("drape_pose_height").as_deref() == Some("true") {
        place.compute_affine().inverse().transform_vector3(Vec3::Y)
    } else {
        Vec3::Z
    };
    drape_with(e, rise_fn(e, *place), up)
}

fn drape_with(e: &DrawEntity, rise: impl Fn(f32, f32) -> f32, up: Vec3) -> DrawEntity {
    let mut out = e.clone();
    if out.instances.is_empty()
        && matches!(
            out.kind,
            draw_kind::AXES | draw_kind::SHAPES | draw_kind::MESH
        )
    {
        out = out.with_instances(&[gearbox_api::Instance::default()]);
    }
    let anchored = e.props().get("drape_anchor").as_deref() == Some("true");
    if !anchored && matches!(e.kind, draw_kind::LINES | draw_kind::POLYGON) {
        split_lines(&mut out);
    }
    let sample = |x, y| if anchored { rise(0.0, 0.0) } else { rise(x, y) };
    if out.instanced() {
        for i in out.instances.chunks_exact_mut(INSTANCE_FLOATS) {
            let offset = up * sample(i[0], i[1]);
            for k in 0..3 {
                i[k] += offset[k];
            }
        }
    } else if !matches!(e.kind, draw_kind::GRID | draw_kind::PANEL) {
        for p in out.points.chunks_exact_mut(3) {
            let offset = up * sample(p[0], p[1]);
            for k in 0..3 {
                p[k] += offset[k];
            }
        }
    }
    out
}

/// Per-point colours, radii and values of a line, carried through its splits.
struct Splitter<'a> {
    e: &'a DrawEntity,
    per_point: bool,
    points: Vec<f32>,
    colors: Vec<u32>,
    radii: Vec<f32>,
    values: Vec<f32>,
}

impl Splitter<'_> {
    fn push(&mut self, i: usize, j: usize, t: f32) {
        let (a, b) = (self.e.point_at(i), self.e.point_at(j));
        self.points.extend((0..3).map(|k| a[k] + (b[k] - a[k]) * t));
        if !self.per_point {
            return;
        }
        if !self.e.colors.is_empty() {
            let (ca, cb) = (
                self.e.colors.get(i).copied().unwrap_or(0),
                self.e.colors.get(j).copied().unwrap_or(0),
            );
            self.colors.push(if ca == 0 || cb == 0 {
                if t < 0.5 { ca } else { cb }
            } else {
                mix_rgba(ca, cb, t)
            });
        }
        if !self.e.radii.is_empty() {
            let (ra, rb) = (
                self.e.radii.get(i).copied().unwrap_or(0.0),
                self.e.radii.get(j).copied().unwrap_or(0.0),
            );
            self.radii
                .push(if ra == 0.0 || rb == 0.0 || ra.signum() != rb.signum() {
                    if t < 0.5 { ra } else { rb }
                } else {
                    ra + (rb - ra) * t
                });
        }
        if self.e.values.len() == self.e.point_count() {
            let (va, vb) = (self.e.values[i], self.e.values[j]);
            self.values.push(va + (vb - va) * t);
        }
    }

    /// Point `i`, then the splits of the segment from `i` to `j`.
    fn segment(&mut self, i: usize, j: usize) {
        self.push(i, i, 0.0);
        let a = Vec3::from_array(self.e.point_at(i));
        let b = Vec3::from_array(self.e.point_at(j));
        let n = ((a.truncate().distance(b.truncate()) / STEP_M).ceil() as usize).max(1);
        for k in 1..n {
            self.push(i, j, k as f32 / n as f32);
        }
    }

    fn count(&self) -> u32 {
        (self.points.len() / 3) as u32
    }
}

fn mix_rgba(a: u32, b: u32, t: f32) -> u32 {
    let (a, b) = (unpack_rgba(a), unpack_rgba(b));
    let m = |k: usize| (a[k] as f32 + (b[k] as f32 - a[k] as f32) * t).round() as u8;
    rgba(m(0), m(1), m(2), m(3))
}

/// Splits every line segment (polygon edge) longer than [`STEP_M`].
fn split_lines(e: &mut DrawEntity) {
    let polygon = e.kind == draw_kind::POLYGON;
    let list = e.sub == line_mode::LIST && !polygon;
    let per_point_values = e.values.len() == e.point_count() && !polygon;
    let mut s = Splitter {
        e: &*e,
        per_point: !polygon,
        points: Vec::new(),
        colors: Vec::new(),
        radii: Vec::new(),
        values: Vec::new(),
    };
    let mut offsets = Vec::new();
    let framed = e.strip_ranges().len() <= super::visual::STRUCTURE_LIMIT
        && (polygon
            || (e.kind == draw_kind::LINES
                && e.sub == line_mode::LOOP
                && e.props().get("corner_frame").as_deref() == Some("true")));
    let mut corners = Vec::new();
    if list {
        for i in (0..e.point_count() / 2 * 2).step_by(2) {
            offsets.push(s.count());
            s.segment(i, i + 1);
            s.push(i + 1, i + 1, 0.0);
        }
    } else {
        let closed = polygon || e.sub == line_mode::LOOP;
        for r in e.strip_ranges() {
            let framed = framed && r.len() <= super::visual::STRUCTURE_LIMIT + 1;
            offsets.push(s.count());
            for i in r.start..r.end - 1 {
                if framed {
                    corners.push(s.count());
                }
                s.segment(i, i + 1);
            }
            if closed && r.len() > 2 {
                if framed {
                    corners.push(s.count());
                }
                s.segment(r.end - 1, r.start);
            } else {
                s.push(r.end - 1, r.end - 1, 0.0);
            }
        }
    }
    let Splitter {
        points,
        colors,
        radii,
        values,
        ..
    } = s;
    e.points = points;
    if !polygon {
        e.colors = colors;
        e.radii = radii;
    }
    if per_point_values {
        e.values = values;
    }
    if list {
        e.sub = line_mode::STRIP;
    }
    if list || !e.offsets.is_empty() || polygon {
        e.offsets = if offsets.len() > 1 {
            offsets
        } else {
            Vec::new()
        };
    }
    if framed {
        *e = e
            .clone()
            .with_prop(CORNER_SOURCES, &serde_json::to_string(&corners).unwrap());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implicit_single_instances_drape_like_explicit_origin_poses() {
        for e in [
            DrawEntity::axes("t", "axes", 2.0),
            DrawEntity::mesh("t", "mesh", "marker.obj"),
            DrawEntity::shapes("t", "shape", gearbox_api::shape_kind::BOX, &[]),
        ] {
            let implicit = drape_with(&e, |x, y| 7.0 + x + y, Vec3::Z);
            let explicit = drape_with(
                &e.clone()
                    .with_instances(&[gearbox_api::Instance::default()]),
                |x, y| 7.0 + x + y,
                Vec3::Z,
            );
            assert_eq!(
                implicit.instances, explicit.instances,
                "kind {} omitted terrain elevation",
                e.kind
            );
            assert!(e.instances.is_empty());
        }
    }

    #[test]
    fn draping_keeps_empty_ghosts_empty_and_preserves_explicit_instances() {
        let ghost = DrawEntity::ghost("t", "ghost", "robot", &[]);
        assert!(drape_with(&ghost, |_, _| 7.0, Vec3::Z).instances.is_empty());
        let poses = [
            gearbox_api::Instance::at([2.0, 3.0, 0.4], [4.0, 5.0, 6.0]).yawed(0.7),
            gearbox_api::Instance::at([-2.0, -3.0, 0.8], [2.0, 1.0, 3.0]),
        ];
        let e = DrawEntity::shapes("t", "shape", gearbox_api::shape_kind::BOX, &poses);
        for anchored in [false, true] {
            let e = e
                .clone()
                .with_prop("drape_anchor", if anchored { "true" } else { "false" });
            let draped = drape_with(&e, |x, y| 7.0 + x + y, Vec3::Z).instance_list();
            assert_eq!(draped.len(), poses.len());
            for (before, after) in poses.iter().zip(draped) {
                assert_eq!(after.size, before.size);
                assert_eq!(after.rotation, before.rotation);
                assert_eq!(&after.position[..2], &before.position[..2]);
                let rise = if anchored {
                    7.0
                } else {
                    7.0 + before.position[0] + before.position[1]
                };
                assert_eq!(after.position[2], before.position[2] + rise);
            }
        }
    }

    #[test]
    fn anchored_geometry_stays_rigid_over_a_slope() {
        let e = DrawEntity::lines("t", "brackets", &[[-2.0, -1.0, 0.0], [2.0, 1.0, 3.0]])
            .with_prop("drape_anchor", "true");
        let anchored = drape_with(&e, |x, y| 7.0 + x + y, Vec3::Z);
        assert_eq!(
            anchored.point_list(),
            vec![[-2.0, -1.0, 7.0], [2.0, 1.0, 10.0]]
        );
        assert_eq!(anchored.point_count(), e.point_count());
    }

    #[test]
    fn corner_frames_retain_authored_vertices_through_terrain_subdivision() {
        let e = DrawEntity::lines(
            "t",
            "frame",
            &[
                [0.0, 0.0, 0.2],
                [10.0, 0.0, 0.2],
                [10.0, 10.0, 0.2],
                [0.0, 10.0, 0.2],
            ],
        )
        .with_sub(line_mode::LOOP)
        .with_style(gearbox_api::draw_style::PLAIN)
        .with_width(-2.0)
        .with_prop("corner_frame", "true");
        let draped = drape_with(&e, |x, y| 7.0 + 0.02 * (x * x + y * y), Vec3::Z);
        assert!(e.props().get(CORNER_SOURCES).is_none());
        assert_eq!(draped.point_count(), 80);
        let corners: Vec<usize> =
            serde_json::from_str(&draped.props().get(CORNER_SOURCES).unwrap()).unwrap();
        assert_eq!(corners, [0, 20, 40, 60]);
        let mesh = super::super::mesh::build(&draped, "t", "frame")
            .strokes
            .into_mesh();
        let bevy::mesh::VertexAttributeValues::Float32x2(radii) =
            mesh.attribute(super::super::material::ATTR_RADII).unwrap()
        else {
            panic!("radii")
        };
        let bevy::mesh::VertexAttributeValues::Float32x3(a) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
        else {
            panic!("positions")
        };
        let bevy::mesh::VertexAttributeValues::Float32x3(b) =
            mesh.attribute(super::super::material::ATTR_END).unwrap()
        else {
            panic!("ends")
        };
        let mut total = 0.0;
        let mut bold = 0.0;
        let mut transitions = 0;
        for i in (0..a.len()).step_by(4) {
            let len = Vec3::from_array(a[i]).distance(Vec3::from_array(b[i]));
            total += len;
            if radii[i][0] < -1.0 {
                bold += len;
            }
            if radii[i] != radii[(i + 4) % a.len()] {
                transitions += 1;
            }
        }
        assert_eq!(
            transitions, 8,
            "terrain samples must not become new corners"
        );
        assert!(
            (bold / total - 2.0 * gearbox_api::wire::draw_widgets::BRACKET_FRACTION).abs() < 1e-5
        );
        for point in draped.point_list() {
            assert!(
                (point[2] - 7.2 - 0.02 * (point[0] * point[0] + point[1] * point[1])).abs() < 1e-5
            );
        }
    }

    #[test]
    fn polygon_accents_stay_on_authored_boundaries_after_ground_sampling() {
        let e = DrawEntity::polygon(
            "t",
            "region",
            &[
                vec![
                    [0.0, 0.0, 0.2],
                    [10.0, 0.0, 0.2],
                    [10.0, 10.0, 0.2],
                    [0.0, 10.0, 0.2],
                ],
                vec![
                    [3.0, 3.0, 0.2],
                    [7.0, 3.0, 0.2],
                    [7.0, 7.0, 0.2],
                    [3.0, 7.0, 0.2],
                ],
            ],
        )
        .with_style(gearbox_api::draw_style::DECAL)
        .with_fill(gearbox_api::draw_flag::FILL_BOTH)
        .with_width(-2.0);
        let draped = drape_with(&e, |x, y| 7.0 + 0.02 * (x * x + y * y), Vec3::Z);
        let corners: Vec<usize> =
            serde_json::from_str(&draped.props().get(CORNER_SOURCES).unwrap()).unwrap();
        assert_eq!(corners.len(), 8);
        assert_eq!(draped.point_count(), 112);
        assert_eq!(draped.polygon_rings()[0].len(), 2);
        let mesh = super::super::mesh::build(&draped, "t", "region")
            .strokes
            .into_mesh();
        let bevy::mesh::VertexAttributeValues::Float32x2(radii) =
            mesh.attribute(super::super::material::ATTR_RADII).unwrap()
        else {
            panic!("radii")
        };
        let transitions = (0..radii.len())
            .step_by(4)
            .filter(|i| radii[*i] != radii[(*i + 4) % radii.len()])
            .count();
        assert_eq!(transitions, 16);
    }

    #[test]
    fn ordinary_geometry_still_follows_sampled_ground() {
        let e = DrawEntity::lines("t", "route", &[[-2.0, 0.0, 0.2], [2.0, 0.0, 0.2]]);
        let draped = drape_with(&e, |x, _| 7.0 + x, Vec3::Z);
        assert!(draped.point_count() > e.point_count());
        for p in draped.point_list() {
            assert!((p[2] - p[0] - 7.2).abs() < 1e-5);
        }
    }

    #[test]
    fn posed_world_helpers_keep_the_requested_height_over_ground() {
        let place = Transform::from_xyz(10.0, 1.55, 20.0)
            .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2));
        let rise = rise_from(place, true, 1.55, |x, y| 0.01 * x + 0.02 * y);
        assert!((rise(0.0, 0.0) - 0.5).abs() < 1e-6);
        assert_eq!(rise(-2.0, 1.0), rise(2.0, -1.0));
        let prior = rise_from(place, false, 0.0, |_, _| 0.0);
        assert!((prior(0.0, 0.0) + 1.55).abs() < 1e-6);
    }

    #[test]
    fn pitched_rigid_helpers_rise_vertically_without_sideways_drift() {
        let e = DrawEntity::lines("t", "brackets", &[[-2.0, -1.0, 0.0], [2.0, 1.0, 3.0]])
            .with_prop("drape_anchor", "true");
        let place = Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -1.0, 0.3, 0.4));
        let up = place.compute_affine().inverse().transform_vector3(Vec3::Y);
        let draped = drape_with(&e, |_, _| 5.0, up);
        for (before, after) in e.point_list().into_iter().zip(draped.point_list()) {
            let movement = place.transform_point(Vec3::from_array(after))
                - place.transform_point(Vec3::from_array(before));
            assert!(movement.distance(Vec3::Y * 5.0) < 1e-5);
        }
    }

    #[test]
    fn long_segments_split_and_keep_their_ends() {
        let mut e = DrawEntity::lines(
            "t",
            "l",
            &[[0.0, 0.0, 0.1], [2.0, 0.0, 0.1], [2.0, 0.2, 0.1]],
        );
        e.colors = vec![
            rgba(0, 0, 0, 255),
            rgba(200, 0, 0, 255),
            rgba(200, 0, 0, 255),
        ];
        split_lines(&mut e);
        let pts = e.point_list();
        assert_eq!(pts.len(), 6);
        assert_eq!(pts[0], [0.0, 0.0, 0.1]);
        assert_eq!(pts[4], [2.0, 0.0, 0.1]);
        assert_eq!(pts[5], [2.0, 0.2, 0.1]);
        assert_eq!(e.colors.len(), 6);
        assert_eq!(unpack_rgba(e.colors[2])[0], 100);
    }

    #[test]
    fn a_loop_splits_its_closing_segment_without_repeating_the_start() {
        let mut e = DrawEntity::lines(
            "t",
            "l",
            &[
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
        );
        e.sub = line_mode::LOOP;
        split_lines(&mut e);
        let pts = e.point_list();
        assert_eq!(pts.len(), 8);
        assert_eq!(pts[7], [0.0, 0.5, 0.0]);
    }

    #[test]
    fn several_strips_keep_their_starts() {
        let mut e = DrawEntity::strips(
            "t",
            "l",
            &[
                vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
                vec![[0.0, 1.0, 0.0], [0.2, 1.0, 0.0]],
            ],
        );
        split_lines(&mut e);
        assert_eq!(e.offsets, vec![0, 3]);
        assert_eq!(e.point_count(), 5);
    }

    #[test]
    fn draped_line_lists_keep_one_continuous_strip_per_original_segment() {
        let input = DrawEntity::lines(
            "t",
            "list",
            &[
                [0.0, 0.0, 0.1],
                [2.0, 0.0, 0.1],
                [10.0, 1.0, 0.2],
                [12.0, 1.0, 0.2],
            ],
        )
        .with_sub(line_mode::LIST)
        .with_colors(&[
            rgba(20, 80, 160, 255),
            rgba(100, 160, 240, 255),
            rgba(200, 40, 20, 128),
            rgba(200, 40, 20, 128),
        ])
        .with_radii(&[0.1, 0.3, -1.0, -2.0]);
        let draped = drape_with(&input, |x, y| 0.3 * x + 0.2 * y, Vec3::Z);
        assert_eq!(input.sub, line_mode::LIST);
        assert_eq!(draped.sub, line_mode::STRIP);
        assert_eq!(draped.offsets, [0, 5]);
        assert_eq!(draped.point_count(), 10);
        assert_eq!(draped.colors.len(), 10);
        assert_eq!(draped.radii.len(), 10);
        assert!((draped.radii[2] - 0.2).abs() < 1e-6);
        assert_eq!(unpack_rgba(draped.colors[2]), [60, 120, 200, 255]);
        for (strip, offset) in draped.strip_ranges().into_iter().zip([0.1, 0.2]) {
            for i in strip.clone() {
                let [x, y, z] = draped.point_at(i);
                assert!((z - (0.3 * x + 0.2 * y + offset)).abs() < 1e-5);
            }
            assert_eq!(strip.len(), 5);
        }
    }

    #[test]
    fn polygon_rings_split_as_loops_and_keep_their_rings() {
        let square = |s: f32| vec![[0.0, 0.0, 0.0], [s, 0.0, 0.0], [s, s, 0.0], [0.0, s, 0.0]];
        let mut e = DrawEntity::polygons("t", "p", &[vec![square(2.0)], vec![square(1.0)]])
            .with_colors(&[1, 2]);
        split_lines(&mut e);
        assert_eq!(e.offsets, vec![0, 16]);
        assert_eq!(e.point_count(), 24);
        assert_eq!(e.rings, vec![1, 1]);
        assert_eq!(e.colors, vec![1, 2], "polygon colours stay one per polygon");
    }
}
