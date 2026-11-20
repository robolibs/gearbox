use super::*;

const MAX_POINTS: usize = 65_536;
const MAX_STATIONS: usize = 4_096;

/// Outline geometry for a caller-classified terrain sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerrainGlyph {
    Square,
    Cross,
    Triangle,
    Diamond,
}

impl TerrainGlyph {
    pub fn name(self) -> &'static str {
        match self {
            Self::Square => "square",
            Self::Cross => "cross",
            Self::Triangle => "triangle",
            Self::Diamond => "diamond",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "square" => Some(Self::Square),
            "cross" => Some(Self::Cross),
            "triangle" => Some(Self::Triangle),
            "diamond" => Some(Self::Diamond),
            _ => None,
        }
    }
}

fn positive(value: f32) -> bool {
    value.is_finite() && value > 0.0
}

/// Encodes transparent black without the wire's zero/AUTO sentinel.
fn explicit_color(color: u32) -> u32 {
    if color == 0 { rgba(1, 0, 0, 0) } else { color }
}

fn finite(point: [f32; 3]) -> bool {
    point.iter().all(|v| v.is_finite())
}

fn valid_pose(pose: Pose) -> Option<Pose> {
    if !pose.position.iter().chain(pose.rotation.iter()).all(|v| v.is_finite()) {
        return None;
    }
    let scale = pose.rotation.iter().fold(0.0_f64, |a, b| a.max(b.abs()));
    if scale == 0.0 {
        return Some(Pose { rotation: [1.0, 0.0, 0.0, 0.0], ..pose });
    }
    let q = pose.rotation.map(|v| v / scale);
    let norm = q.iter().map(|v| v * v).sum::<f64>().sqrt();
    Some(Pose { rotation: q.map(|v| v / norm), ..pose })
}

pub(super) fn segment(points: &mut Vec<[f32; 3]>, a: [f32; 3], b: [f32; 3]) {
    if finite(a) && finite(b) && a != b {
        points.extend([a, b]);
    }
}

pub(super) fn open_outline(points: &mut Vec<[f32; 3]>, polygon: &[[f32; 3]]) {
    for i in 0..polygon.len() {
        let (a, b) = (polygon[i], polygon[(i + 1) % polygon.len()]);
        let lerp = |t: f32| std::array::from_fn(|j| a[j] * (1.0 - t) + b[j] * t);
        segment(points, lerp(0.12), lerp(0.88));
    }
}

pub(super) fn corner_outline(points: &mut Vec<[f32; 3]>, polygon: &[[f32; 3]]) {
    for i in 0..polygon.len() {
        let p = polygon[i];
        for j in [(i + polygon.len() - 1) % polygon.len(), (i + 1) % polygon.len()] {
            segment(points, p, std::array::from_fn(|axis| p[axis] + BRACKET_FRACTION * (polygon[j][axis] - p[axis])));
        }
    }
}

pub(super) fn box_brackets(size: [f32; 3]) -> Vec<[f32; 3]> {
    let mut points = Vec::with_capacity(48);
    if size.into_iter().all(positive) {
        for corner in 0..8 {
            let p = std::array::from_fn(|i| size[i] * if corner & (1 << i) == 0 { -0.5 } else { 0.5 });
            for axis in 0..3 {
                let mut q = p;
                q[axis] -= p[axis].signum() * size[axis] * BRACKET_FRACTION;
                segment(&mut points, p, q);
            }
        }
    }
    points
}

pub(super) fn station_mark(points: &mut Vec<[f32; 3]>, centre: [f64; 3], tangent: [f64; 3], size: f32) {
    let xy = tangent[0].hypot(tangent[1]);
    let side = if xy > 1e-9 { [-tangent[1] / xy, tangent[0] / xy, 0.0] } else { [1.0, 0.0, 0.0] };
    let at = |across: f64, along: f64| std::array::from_fn(|i| (centre[i] + size as f64 * (across * side[i] + along * tangent[i])) as f32);
    segment(points, at(-0.5, 0.0), at(0.5, 0.0));
    segment(points, at(-0.3, -0.25), at(0.0, 0.25));
    segment(points, at(0.0, 0.25), at(0.3, -0.25));
}

pub(super) fn segments(place: &Place, ns: &str, id: &str, points: &[[f32; 3]], tint: u32, width: f32) -> DrawEntity {
    let e = if points.is_empty() {
        DrawEntity::delete(ns, id)
    } else {
        DrawEntity::lines(ns, id, points).with_sub(line_mode::LIST).with_style(draw_style::PLAIN)
            .with_width(width).with_color(explicit_color(tint))
    };
    place.put(e)
}

fn cleared(place: &Place, ns: &str, ids: &[&str]) -> Vec<DrawEntity> {
    ids.iter().map(|id| place.put(DrawEntity::delete(ns, id))).collect()
}

fn world_label(place: &Place, ns: &str, pose: Pose, at: [f32; 3], label: &str, tint: u32) -> DrawEntity {
    let e = if label.is_empty() || !finite(at) {
        DrawEntity::delete(ns, "label")
    } else {
        DrawEntity::text(ns, "label", &[at], label).with_width(-16.0).with_color(explicit_color(tint))
            .with_pose(pose.position, pose.rotation).with_prop("drape_anchor", "true").with_prop("drape_pose_height", "true")
    };
    place.put(e)
}

/// Batched XY glyphs of full span `size` metres at the first 65,536 samples.
/// Nonfinite samples are skipped; invalid size clears the marks child.
pub fn terrain_marks(place: &Place, ns: &str, id: &str, points: &[[f32; 3]], glyph: TerrainGlyph, size: f32, color: u32) -> Vec<DrawEntity> {
    let root = path(ns, id);
    let mut vertices = Vec::new();
    if positive(size) {
        let h = 0.5 * size;
        let polygon = match glyph {
            TerrainGlyph::Square => vec![[-h, -h], [h, -h], [h, h], [-h, h]],
            TerrainGlyph::Triangle => vec![[0.0, h], [-h, -h], [h, -h]],
            TerrainGlyph::Diamond => vec![[0.0, h], [-h, 0.0], [0.0, -h], [h, 0.0]],
            TerrainGlyph::Cross => Vec::new(),
        };
        vertices.reserve(points.len().min(MAX_POINTS) * 8);
        for p in points.iter().take(MAX_POINTS).copied().filter(|p| finite(*p)) {
            let at = |xy: [f32; 2]| [p[0] + xy[0], p[1] + xy[1], p[2]];
            if glyph == TerrainGlyph::Cross {
                segment(&mut vertices, at([-h, -h]), at([h, h]));
                segment(&mut vertices, at([-h, h]), at([h, -h]));
            } else {
                for i in 0..polygon.len() {
                    segment(&mut vertices, at(polygon[i]), at(polygon[(i + 1) % polygon.len()]));
                }
            }
        }
    }
    vec![segments(place, &root, "marks", &vertices, color, -1.25)]
}

/// A thin 3-D route and equally spaced transverse bars with forward chevrons.
/// Processes 65,536 input points and at most 4,096 stations; invalid points
/// break the route and reset station spacing. Invalid spacing/size clears stations.
pub fn route_stations(place: &Place, ns: &str, id: &str, points: &[[f32; 3]], spacing: f32, size: f32, color: u32) -> Vec<DrawEntity> {
    let root = path(ns, id);
    let mut route = Vec::new();
    let mut marks = Vec::new();
    let mut remaining = 0.0_f64;
    let mut count = 0;
    for pair in points[..points.len().min(MAX_POINTS)].windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if !finite(a) || !finite(b) {
            remaining = 0.0;
            continue;
        }
        segment(&mut route, a, b);
        let delta = std::array::from_fn::<_, 3, _>(|i| b[i] as f64 - a[i] as f64);
        let length = delta.iter().map(|v| v * v).sum::<f64>().sqrt();
        if length <= 1e-9 || !positive(spacing) || !positive(size) || count >= MAX_STATIONS {
            continue;
        }
        let tangent = delta.map(|v| v / length);
        while remaining <= length && count < MAX_STATIONS {
            let centre = std::array::from_fn::<_, 3, _>(|i| a[i] as f64 + remaining * tangent[i]);
            station_mark(&mut marks, centre, tangent, size);
            remaining += spacing as f64;
            count += 1;
        }
        remaining -= length;
    }
    vec![segments(place, &root, "route", &route, color, -1.0), segments(place, &root, "stations", &marks, color, -1.25)]
}

/// A broken XY ring and open XZ diamond at the supplied pose.
/// `radius` is the outer ring radius in metres; the marker centre is at local z=2r.
pub fn destination(place: &Place, ns: &str, id: &str, pose: Pose, radius: f32, label: &str, color: u32) -> Vec<DrawEntity> {
    let root = path(ns, id);
    let Some(pose) = valid_pose(pose).filter(|_| positive(radius) && (radius * 2.5).is_finite()) else {
        return cleared(place, &root, &["rings", "marker", "leader", "label"]);
    };
    let mut rings = Vec::new();
    for quadrant in 0..4 {
        for i in 0..12 {
            let point = |k: i32| {
                let angle = quadrant as f32 * 0.5 * PI + 0.14 + k as f32 / 12.0 * (0.5 * PI - 0.28);
                [radius * angle.cos(), radius * angle.sin(), 0.0]
            };
            segment(&mut rings, point(i), point(i + 1));
        }
    }
    let mut marker = Vec::new();
    let h = radius * 0.5;
    let diamond = [[0.0, 0.0, 2.0 * radius + h], [h, 0.0, 2.0 * radius], [0.0, 0.0, 2.0 * radius - h], [-h, 0.0, 2.0 * radius]];
    open_outline(&mut marker, &diamond);
    let mut leader = vec![[0.0; 3], [0.0, 0.0, 1.35 * radius]];
    if !label.is_empty() {
        segment(&mut leader, [0.58 * radius, 0.0, 2.0 * radius], [0.7 * radius, 0.0, 2.0 * radius]);
    }
    let mut out: Vec<_> = [("rings", rings), ("marker", marker), ("leader", leader)].into_iter()
        .map(|(id, points)| {
            let e = segments(place, &root, id, &points, color, -1.25).with_pose(pose.position, pose.rotation).with_prop("drape_pose_height", "true");
            if id == "rings" { e } else { e.with_prop("drape_anchor", "true") }
        }).collect();
    out.push(world_label(place, &root, pose, [0.75 * radius, 0.0, 2.0 * radius], label, color));
    out
}

/// Broken corner brackets at the supplied centre and full XYZ extents.
pub fn clearance_box(place: &Place, ns: &str, id: &str, pose: Pose, size: [f32; 3], color: u32, label: &str) -> Vec<DrawEntity> {
    let root = path(ns, id);
    let Some(pose) = valid_pose(pose).filter(|_| size.into_iter().all(positive)) else {
        return cleared(place, &root, &["brackets", "label"]);
    };
    let points = box_brackets(size);
    vec![
        segments(place, &root, "brackets", &points, color, -STRUCTURE_RULE_PX * STRUCTURE_CORNER_GAIN).with_pose(pose.position, pose.rotation)
            .with_prop("drape_anchor", "true").with_prop("drape_pose_height", "true"),
        world_label(place, &root, pose, [0.0, 0.0, size[2] * 0.5], label, color),
    ]
}

/// A depth-tested field box with full XYZ extents and major-grid pitch `spacing` metres.
pub fn field_volume(place: &Place, ns: &str, id: &str, pose: Pose, size: [f32; 3], spacing: f32, color: u32) -> Vec<DrawEntity> {
    let root = path(ns, id);
    let Some(pose) = valid_pose(pose).filter(|_| size.into_iter().all(positive) && positive(spacing)) else {
        return cleared(place, &root, &["volume"]);
    };
    vec![place.put(DrawEntity::shapes(&root, "volume", shape_kind::BOX, &[Instance::at([0.0; 3], size)]))
        .with_pose(pose.position, pose.rotation).with_prop("drape_pose_height", "true").with_style(draw_style::FIELD)
        .with_fill(draw_flag::FILL_SOLID).with_dash(spacing).with_color(explicit_color(color))]
}

/// What a machine ring says about its machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RingMark {
    /// The pointer is over it.
    Hover,
    Selected,
    /// The camera follows it.
    Following,
}

impl RingMark {
    pub fn name(self) -> &'static str {
        match self {
            Self::Hover => "hover",
            Self::Selected => "selected",
            Self::Following => "following",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "hover" | "hovered" => Some(Self::Hover),
            "selected" | "select" => Some(Self::Selected),
            "following" | "follow" => Some(Self::Following),
            _ => None,
        }
    }

    /// The tint a ring takes when given none.
    pub fn tint(self) -> u32 {
        match self {
            Self::Hover => rgba(220, 232, 236, 80),
            Self::Selected | Self::Following => rgba(220, 232, 236, 140),
        }
    }
}

/// Smallest outer radius of a machine ring, metres.
pub const RING_MIN_RADIUS: f32 = 0.5;

/// A machine's ring on the ground, its rim `radius` metres out from
/// `centre`; `centre` z is its height, above the terrain when draped. One
/// [`draw_style::RING`] entity: a soft off-white band in four arcs with faint
/// gaps, fainter when hovered, closed with a thin outer line while followed.
/// A zero `tint` takes the mark's own.
pub fn machine_ring(place: &Place, ns: &str, id: &str, centre: [f32; 3], radius: f32, mark: RingMark, tint: u32) -> Vec<DrawEntity> {
    let root = path(ns, id);
    if !finite(centre) || !radius.is_finite() || radius < RING_MIN_RADIUS {
        return cleared(place, &root, &["ring"]);
    }
    let tint = if tint == 0 { mark.tint() } else { tint };
    vec![place.put(DrawEntity::points(&root, "ring", &[centre]))
        .with_style(draw_style::RING)
        .with_width(2.0 * radius)
        .with_color(explicit_color(tint))
        .with_prop("mark", mark.name())]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::DrawStore;

    fn p() -> Place {
        Place::frame("world").no_drape()
    }

    fn near(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-5, "{a} != {b}");
    }

    #[test]
    fn every_terrain_glyph_has_exact_span_and_no_status_inference() {
        for (glyph, segments) in [(TerrainGlyph::Square, 4), (TerrainGlyph::Cross, 2), (TerrainGlyph::Triangle, 3), (TerrainGlyph::Diamond, 4)] {
            assert_eq!(TerrainGlyph::parse(glyph.name()), Some(glyph));
            let e = terrain_marks(&p(), "n", "terrain", &[[2.0, 3.0, 4.0]], glyph, 2.0, color::MAGENTA).remove(0);
            assert_eq!(e.point_count(), segments * 2);
            assert_eq!(e.color, color::MAGENTA);
            for axis in 0..2 {
                let values: Vec<_> = e.point_list().iter().map(|p| p[axis]).collect();
                near(values.iter().copied().fold(f32::INFINITY, f32::min), [1.0, 2.0][axis]);
                near(values.iter().copied().fold(f32::NEG_INFINITY, f32::max), [3.0, 4.0][axis]);
            }
            assert!(e.point_list().iter().all(|p| p[2] == 4.0));
        }
        assert_eq!(TerrainGlyph::parse("Square"), None);
        assert_eq!(TerrainGlyph::parse("safe"), None);
    }

    #[test]
    fn route_stations_follow_three_dimensional_arc_length() {
        let out = route_stations(&p(), "n", "r", &[[0.0; 3], [0.0, 0.0, 3.0], [4.0, 0.0, 3.0]], 2.0, 0.4, color::CYAN);
        let stations = &out[1];
        assert_eq!(stations.point_count(), 4 * 6);
        let points = stations.point_list();
        for (k, centre) in [[0.0; 3], [0.0, 0.0, 2.0], [1.0, 0.0, 3.0], [3.0, 0.0, 3.0]].into_iter().enumerate() {
            for axis in 0..3 {
                near((points[6 * k][axis] + points[6 * k + 1][axis]) * 0.5, centre[axis]);
            }
        }
        assert_eq!(out[0].point_list(), vec![[0.0; 3], [0.0, 0.0, 3.0], [0.0, 0.0, 3.0], [4.0, 0.0, 3.0]]);
    }

    #[test]
    fn invalid_points_break_routes_and_generation_is_bounded() {
        let points = [[0.0; 3], [1.0, 0.0, 0.0], [f32::NAN; 3], [10.0, 0.0, 0.0], [11.0, 0.0, 0.0]];
        let out = route_stations(&p(), "n", "r", &points, 1.0, 0.2, color::WHITE);
        assert_eq!(out[0].point_count(), 4);
        assert_eq!(out[1].point_count(), 24);
        let dense = route_stations(&p(), "n", "r", &[[0.0; 3], [f32::MAX, 0.0, 0.0]], f32::MIN_POSITIVE, 0.2, color::WHITE);
        assert!(dense[1].point_count() <= MAX_STATIONS * 6);
        for spacing in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let out = route_stations(&p(), "n", "r", &points, spacing, 0.2, color::WHITE);
            assert_eq!(out[0].op, draw_op::UPSERT);
            assert_eq!(out[1].op, draw_op::DELETE);
        }
        assert!(route_stations(&p(), "n", "r", &[[0.0; 3]; 4], 1.0, 0.2, color::WHITE).iter().all(|e| e.op == draw_op::DELETE));
    }

    #[test]
    fn pose_and_extents_remain_geometric_not_screen_space() {
        let pose = Pose::yawed(10.0, 20.0, 30.0, 0.5);
        let goal = destination(&p(), "n", "goal", pose, 2.0, "dock", color::CYAN);
        assert_eq!(goal.iter().map(DrawEntity::id).collect::<Vec<_>>(), ["rings", "marker", "leader", "label"]);
        assert_eq!(goal[0].point_count(), 96);
        assert_eq!(goal[1].point_count(), 8);
        assert!(goal[0].point_list().iter().all(|p| (p[0].hypot(p[1]) - 2.0).abs() < 1e-5));
        assert!(goal[0].point_list().iter().all(|p| p[2] == 0.0 && p[0].hypot(p[1]) <= 2.00001));
        assert!(goal[1].point_list().iter().all(|p| p[1] == 0.0 && p[0].abs() <= 1.0 && (3.0..=5.0).contains(&p[2])));
        for e in &goal {
            assert_eq!([e.px, e.py, e.pz], pose.position);
            e.validate().unwrap();
        }
        let clearance = clearance_box(&p(), "n", "box", pose, [2.0, 4.0, 6.0], color::YELLOW, "2m");
        assert_eq!(clearance[0].point_count(), 48);
        assert!(clearance[0].point_list().iter().all(|p| p[0].abs() <= 1.0 && p[1].abs() <= 2.0 && p[2].abs() <= 3.0));
        let volume = field_volume(&p(), "n", "field", pose, [2.0, 4.0, 6.0], 0.5, color::GREEN).remove(0);
        assert_eq!(volume.instance_list()[0].size, [2.0, 4.0, 6.0]);
        assert_eq!(volume.style, draw_style::FIELD);
        assert_eq!(volume.dash, 0.5);
    }

    #[test]
    fn all_helpers_preserve_place_depth_and_explicit_alpha() {
        for frame in ["world", "robot/odom", "robot/link"] {
            let place = Place::frame(frame).lifetime(7.0);
            let tint = color::with_alpha(color::CYAN, 0.0);
            let mut out = terrain_marks(&place, "n", "a", &[[0.0; 3]], TerrainGlyph::Square, 1.0, tint);
            out.extend(route_stations(&place, "n", "b", &[[0.0; 3], [2.0, 0.0, 0.0]], 1.0, 0.2, tint));
            out.extend(destination(&place, "n", "c", Pose::default(), 1.0, "goal", tint));
            out.extend(clearance_box(&place, "n", "d", Pose::default(), [1.0; 3], tint, "box"));
            out.extend(field_volume(&place, "n", "e", Pose::default(), [1.0; 3], 0.2, tint));
            for e in out {
                e.validate().unwrap();
                assert_eq!(e.props().get("frame").as_deref(), Some(frame));
                assert_eq!(e.lifetime_s, 7.0);
                assert_eq!(e.flags & draw_flag::ON_TOP, 0);
                assert_eq!(e.flags & draw_flag::DRAPE != 0, DrawFrame::drapes(frame));
                assert_eq!(e.color, tint);
                if e.kind == draw_kind::LINES {
                    assert_eq!(e.style, draw_style::PLAIN);
                    assert_eq!(e.glow, 0.0);
                    assert_eq!((e.pulse_hz, e.flow_speed), (0.0, 0.0));
                    for glow in [-1.0, 0.0, 0.9] {
                        assert_eq!(e.clone().with_glow(glow).glow, glow);
                    }
                } else {
                    assert_eq!(e.glow, 0.0);
                }
            }
        }
    }

    #[test]
    fn rigid_annotations_anchor_once_and_zero_color_stays_transparent() {
        let place = Place::default();
        let goal = destination(&place, "n", "goal", Pose::default(), 1.0, "goal", 0);
        for e in &goal {
            assert_eq!(e.color, 1);
            assert_eq!(e.color >> 24, 0);
            assert_eq!(e.props().get("drape_pose_height").as_deref(), Some("true"));
            assert_eq!(e.props().get("drape_anchor").as_deref(), if e.id() == "rings" { None } else { Some("true") });
        }
        for e in clearance_box(&place, "n", "box", Pose::default(), [1.0; 3], 0, "box") {
            assert_eq!(e.props().get("drape_anchor").as_deref(), Some("true"));
            assert_eq!(e.props().get("drape_pose_height").as_deref(), Some("true"));
            assert_eq!(e.color, 1);
        }
        let field = field_volume(&place, "n", "field", Pose::default(), [1.0; 3], 0.2, 0).remove(0);
        assert_eq!(field.color, 1);
        assert_eq!(field.props().get("drape_pose_height").as_deref(), Some("true"));
        assert_eq!(terrain_marks(&place, "n", "terrain", &[[0.0; 3]], TerrainGlyph::Square, 1.0, 0)[0].color, 1);
    }

    #[test]
    fn invalid_geometry_and_empty_labels_clear_stable_children() {
        let mut store = DrawStore::default();
        for e in destination(&p(), "n", "goal", Pose::default(), 1.0, "label", color::WHITE) {
            store.apply(e, 0.0).unwrap();
        }
        for e in destination(&p(), "n", "goal", Pose::default(), 2.0, "", color::WHITE) {
            store.apply(e, 1.0).unwrap();
        }
        assert_eq!(store.len(), 3);
        assert!(store.get("n/goal", "label").is_none());
        for e in destination(&p(), "n", "goal", Pose::default(), f32::NAN, "label", color::WHITE) {
            store.apply(e, 2.0).unwrap();
        }
        assert!(store.is_empty());
        for e in clearance_box(&p(), "n", "box", Pose::default(), [2.0; 3], color::WHITE, "old") {
            store.apply(e, 3.0).unwrap();
        }
        for e in clearance_box(&p(), "n", "box", Pose::default(), [3.0; 3], color::WHITE, "") {
            store.apply(e, 4.0).unwrap();
        }
        assert_eq!(store.len(), 1);
        assert!(store.get("n/box", "label").is_none());
        for e in clearance_box(&p(), "n", "box", Pose::default(), [0.0; 3], color::WHITE, "old") {
            store.apply(e, 5.0).unwrap();
        }
        assert!(store.is_empty());
        for invalid in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(terrain_marks(&p(), "n", "a", &[[0.0; 3]], TerrainGlyph::Diamond, invalid, color::WHITE)[0].op, draw_op::DELETE);
            assert!(clearance_box(&p(), "n", "b", Pose::default(), [invalid; 3], color::WHITE, "").iter().all(|e| e.op == draw_op::DELETE));
            assert_eq!(field_volume(&p(), "n", "c", Pose::default(), [1.0; 3], invalid, color::WHITE)[0].op, draw_op::DELETE);
        }
        assert!(destination(&p(), "n", "a", Pose::at(f64::NAN, 0.0, 0.0), 1.0, "", color::WHITE).iter().all(|e| e.op == draw_op::DELETE));
        let input = vec![[0.0; 3]; MAX_POINTS + 1];
        let marks = terrain_marks(&p(), "n", "limit", &input, TerrainGlyph::Cross, 1.0, color::WHITE);
        assert_eq!(marks[0].point_count(), MAX_POINTS * 4);
    }

    #[test]
    fn dense_glyphs_roundtrip_through_native_chunks() {
        let points: Vec<_> = (0..20_000).map(|i| [(i % 200) as f32, (i / 200) as f32, 0.0]).collect();
        let e = terrain_marks(&p(), "n", "dense", &points, TerrainGlyph::Square, 0.5, color::CYAN).remove(0);
        assert_eq!(e.point_count(), 160_000);
        let mut store = DrawStore::default();
        let chunks = e.chunks(DRAW_CHUNK_BYTES);
        assert!(chunks.len() > 1);
        for c in chunks {
            c.validate().unwrap();
            store.apply(c, 0.0).unwrap();
        }
        let restored = &store.get("n/dense", "marks").unwrap().entity;
        assert_eq!(restored.points, e.points);
        assert_eq!(restored.sub, line_mode::LIST);
        assert_eq!(restored.color, e.color);
        store.apply(DrawEntity::clear("n/dense"), 1.0).unwrap();
        assert!(store.is_empty());
    }

    #[test]
    fn a_machine_ring_is_one_ring_entity_carrying_its_mark() {
        for mark in [RingMark::Hover, RingMark::Selected, RingMark::Following] {
            assert_eq!(RingMark::parse(mark.name()), Some(mark));
            let [ring] = <[DrawEntity; 1]>::try_from(machine_ring(&p(), "n", "m", [10.0, -4.0, 0.1], 3.0, mark, 0)).unwrap();
            assert_eq!((ring.ns(), ring.id()), ("n/m".to_string(), "ring".to_string()));
            assert_eq!((ring.kind, ring.style, ring.width), (draw_kind::POINTS, draw_style::RING, 6.0));
            assert_eq!(ring.point_list(), [[10.0, -4.0, 0.1]]);
            assert_eq!((ring.color, ring.props().get("mark").as_deref()), (mark.tint(), Some(mark.name())));
        }
        let red = machine_ring(&p(), "n", "m", [0.0; 3], 3.0, RingMark::Following, color::RED);
        assert_eq!(red[0].color, color::RED);
        assert_ne!(machine_ring(&Place::default(), "n", "m", [0.0; 3], 3.0, RingMark::Hover, 0)[0].flags & draw_flag::DRAPE, 0);
    }

    #[test]
    fn machine_rings_clear_when_they_cannot_be_drawn() {
        for (centre, radius) in [([f32::NAN, 0.0, 0.0], 3.0), ([0.0; 3], RING_MIN_RADIUS - 0.01), ([0.0; 3], f32::INFINITY)] {
            let parts = machine_ring(&p(), "n", "m", centre, radius, RingMark::Following, 0);
            assert_eq!(parts.iter().map(|e| (e.op, e.id())).collect::<Vec<_>>(), [(draw_op::DELETE, "ring".to_string())]);
        }
    }
}
