//! Terrain draping: a draped entity's z is height above the ground under
//! each point, and its lines and polygon rings are split so they bend over
//! the ground.

use bevy::prelude::*;
use gearbox_api::wire::draw::{INSTANCE_FLOATS, rgba, unpack_rgba};
use gearbox_api::{DrawEntity, draw_kind, line_mode};

/// Longest line segment left whole, metres.
pub const STEP_M: f32 = 0.5;

/// Height of the terrain over the entity's z = 0 at entity coordinates x,
/// y, `place` taking entity coordinates (Z up) into the home site's (Y up).
pub fn rise_fn(place: Transform) -> impl Fn(f32, f32) -> f32 {
    move |x: f32, y: f32| -> f32 {
        let s = place.transform_point(Vec3::new(x, y, 0.0));
        crate::globe::ground_height_at_physics(s.x as f64, s.z as f64) as f32 - s.y
    }
}

/// `e` raised onto the ground, `place` taking entity coordinates (Z up) into
/// the home site's (Y up).
pub fn drape(e: &DrawEntity, place: &Transform) -> DrawEntity {
    let mut out = e.clone();
    if matches!(e.kind, draw_kind::LINES | draw_kind::POLYGON) {
        split_lines(&mut out);
    }
    let rise = rise_fn(*place);
    if out.instanced() {
        for i in out.instances.chunks_exact_mut(INSTANCE_FLOATS) {
            i[2] += rise(i[0], i[1]);
        }
    } else if !matches!(e.kind, draw_kind::GRID | draw_kind::PANEL) {
        for p in out.points.chunks_exact_mut(3) {
            p[2] += rise(p[0], p[1]);
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
            let (ca, cb) = (self.e.colors.get(i).copied().unwrap_or(0), self.e.colors.get(j).copied().unwrap_or(0));
            self.colors.push(if ca == 0 || cb == 0 { if t < 0.5 { ca } else { cb } } else { mix_rgba(ca, cb, t) });
        }
        if !self.e.radii.is_empty() {
            let (ra, rb) = (self.e.radii.get(i).copied().unwrap_or(0.0), self.e.radii.get(j).copied().unwrap_or(0.0));
            self.radii.push(if ra == 0.0 || rb == 0.0 || ra.signum() != rb.signum() { if t < 0.5 { ra } else { rb } } else { ra + (rb - ra) * t });
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
    let per_point_values = e.values.len() == e.point_count() && !polygon;
    let mut s = Splitter { e: &*e, per_point: !polygon, points: Vec::new(), colors: Vec::new(), radii: Vec::new(), values: Vec::new() };
    let mut offsets = Vec::new();
    if e.sub == line_mode::LIST && !polygon {
        for i in (0..e.point_count() / 2 * 2).step_by(2) {
            let a = Vec3::from_array(e.point_at(i));
            let b = Vec3::from_array(e.point_at(i + 1));
            let n = ((a.truncate().distance(b.truncate()) / STEP_M).ceil() as usize).max(1);
            for k in 0..n {
                s.push(i, i + 1, k as f32 / n as f32);
                s.push(i, i + 1, (k + 1) as f32 / n as f32);
            }
        }
    } else {
        let closed = polygon || e.sub == line_mode::LOOP;
        for r in e.strip_ranges() {
            offsets.push(s.count());
            for i in r.start..r.end - 1 {
                s.segment(i, i + 1);
            }
            if closed && r.len() > 2 {
                s.segment(r.end - 1, r.start);
            } else {
                s.push(r.end - 1, r.end - 1, 0.0);
            }
        }
    }
    let Splitter { points, colors, radii, values, .. } = s;
    e.points = points;
    if !polygon {
        e.colors = colors;
        e.radii = radii;
    }
    if per_point_values {
        e.values = values;
    }
    if !e.offsets.is_empty() || polygon {
        e.offsets = if offsets.len() > 1 { offsets } else { Vec::new() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_segments_split_and_keep_their_ends() {
        let mut e = DrawEntity::lines("t", "l", &[[0.0, 0.0, 0.1], [2.0, 0.0, 0.1], [2.0, 0.2, 0.1]]);
        e.colors = vec![rgba(0, 0, 0, 255), rgba(200, 0, 0, 255), rgba(200, 0, 0, 255)];
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
        let mut e = DrawEntity::lines("t", "l", &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]]);
        e.sub = line_mode::LOOP;
        split_lines(&mut e);
        let pts = e.point_list();
        assert_eq!(pts.len(), 8);
        assert_eq!(pts[7], [0.0, 0.5, 0.0]);
    }

    #[test]
    fn several_strips_keep_their_starts() {
        let mut e = DrawEntity::strips("t", "l", &[vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]], vec![[0.0, 1.0, 0.0], [0.2, 1.0, 0.0]]]);
        split_lines(&mut e);
        assert_eq!(e.offsets, vec![0, 3]);
        assert_eq!(e.point_count(), 5);
    }

    #[test]
    fn polygon_rings_split_as_loops_and_keep_their_rings() {
        let square = |s: f32| vec![[0.0, 0.0, 0.0], [s, 0.0, 0.0], [s, s, 0.0], [0.0, s, 0.0]];
        let mut e = DrawEntity::polygons("t", "p", &[vec![square(2.0)], vec![square(1.0)]]).with_colors(&[1, 2]);
        split_lines(&mut e);
        assert_eq!(e.offsets, vec![0, 16]);
        assert_eq!(e.point_count(), 24);
        assert_eq!(e.rings, vec![1, 1]);
        assert_eq!(e.colors, vec![1, 2], "polygon colours stay one per polygon");
    }
}
