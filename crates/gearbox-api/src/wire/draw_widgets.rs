//! Composite draws: robotics visuals built from several entities under one
//! path (`ns/id`), so `clear("ns/id")` removes the whole widget. Every
//! builder returns the entities to send, in the frame given by
//! [`Place`]; coordinates follow the frame (Z-up, world x east, y north).

use std::f32::consts::{PI, TAU};

use super::draw::*;
use super::draw_geom::{covariance_axes, covariance_ellipse, pinhole_fov, quat_from_axes};

mod world;
pub use world::{TerrainGlyph, clearance_box, destination, field_volume, route_stations, terrain_marks};

/// Fraction of an edge occupied by each structural corner bracket.
pub const BRACKET_FRACTION: f32 = 0.18;
pub const STRUCTURE_RULE_PX: f32 = 1.0;
pub const STRUCTURE_CORNER_GAIN: f32 = 1.75;

/// Where a widget's entities live: frame, lifetime and whether z is height
/// above the terrain (world and odom frames only).
#[derive(Debug, Clone)]
pub struct Place {
    pub frame: String,
    pub lifetime: f64,
    pub drape: bool,
}

impl Default for Place {
    fn default() -> Self {
        Self { frame: "world".into(), lifetime: 0.0, drape: true }
    }
}

impl Place {
    pub fn frame(frame: &str) -> Self {
        Self { frame: frame.into(), ..Self::default() }
    }

    pub fn lifetime(mut self, seconds: f64) -> Self {
        self.lifetime = seconds;
        self
    }

    pub fn no_drape(mut self) -> Self {
        self.drape = false;
        self
    }

    /// `e` in this frame, draped when it may be.
    pub fn put(&self, e: DrawEntity) -> DrawEntity {
        let e = e.in_frame(&self.frame).with_lifetime(self.lifetime);
        if self.drape && DrawFrame::drapes(&self.frame) { e.with_flags(draw_flag::DRAPE) } else { e.without_flags(draw_flag::DRAPE) }
    }

    /// `e` in this frame, never draped.
    pub fn put_raw(&self, e: DrawEntity) -> DrawEntity {
        e.in_frame(&self.frame).with_lifetime(self.lifetime).without_flags(draw_flag::DRAPE)
    }
}

/// A pose: position and rotation w, x, y, z.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub position: [f64; 3],
    pub rotation: [f64; 4],
}

impl Default for Pose {
    fn default() -> Self {
        Self { position: [0.0; 3], rotation: [1.0, 0.0, 0.0, 0.0] }
    }
}

impl Pose {
    pub fn at(x: f64, y: f64, z: f64) -> Self {
        Self { position: [x, y, z], ..Self::default() }
    }

    pub fn yawed(x: f64, y: f64, z: f64, yaw: f64) -> Self {
        Self { position: [x, y, z], rotation: [(0.5 * yaw).cos(), 0.0, 0.0, (0.5 * yaw).sin()] }
    }

    pub fn yaw(&self) -> f64 {
        let [w, x, y, z] = self.rotation;
        (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z))
    }

    fn instance(&self, size: [f32; 3]) -> Instance {
        let p = self.position.map(|v| v as f32);
        Instance::at(p, size).turned(self.rotation.map(|v| v as f32))
    }

    fn f32(&self) -> [f32; 3] {
        self.position.map(|v| v as f32)
    }

    /// `v` from this pose's frame into its parent's.
    pub fn apply(&self, v: [f32; 3]) -> [f32; 3] {
        let r = super::draw_geom::rotate(self.rotation, v.map(|c| c as f64));
        [(r[0] + self.position[0]) as f32, (r[1] + self.position[1]) as f32, (r[2] + self.position[2]) as f32]
    }
}

/// `sub` of [`draw_kind::ARROWS`]: arrow (0) or chevron (1).
pub const ARROW_CHEVRON: u32 = 1;
/// `sub` of [`draw_kind::POINTS`] next to sphere and disc: square splats.
pub const POINT_SQUARE: u32 = 2;

fn path(ns: &str, id: &str) -> String {
    format!("{}/{}", ns.trim_end_matches('/'), id)
}

fn circle(c: [f32; 3], rx: f32, ry: f32, angle: f32, n: usize) -> Vec<[f32; 3]> {
    let (s, co) = angle.sin_cos();
    (0..n)
        .map(|k| {
            let t = k as f32 / n as f32 * TAU;
            let (x, y) = (rx * t.cos(), ry * t.sin());
            [c[0] + x * co - y * s, c[1] + x * s + y * co, c[2]]
        })
        .collect()
}

fn arc(c: [f32; 3], r: f32, from: f32, to: f32, n: usize) -> Vec<[f32; 3]> {
    (0..=n)
        .map(|k| {
            let t = from + (to - from) * k as f32 / n as f32;
            [c[0] + r * t.cos(), c[1] + r * t.sin(), c[2]]
        })
        .collect()
}

/// A frame triad (x vermillion, y bluish green, z blue), its name,
/// and a dashed hairline to `parent`.
pub fn frame(place: &Place, ns: &str, name: &str, pose: Pose, parent: Option<[f32; 3]>, length: f32, label: Option<&str>) -> Vec<DrawEntity> {
    let root = path(ns, name);
    let at = pose.f32();
    let mut out = vec![
        place
            .put(DrawEntity::axes(&root, "axes", length))
            .with_style(draw_style::ARROW_GLOW)
            .with_pose(pose.position, pose.rotation)
            .with_width(0.05 * length),
        place
            .put(DrawEntity::text(&root, "name", &[at], label.unwrap_or(name)))
            .with_color(color::WHITE)
            .with_width(-16.0)
            .with_prop("label_icon", "frame"),
    ];
    match parent {
        Some(p) => out.push(
            place
                .put(DrawEntity::lines(&root, "parent", &[at, p]))
                .with_color(color::with_alpha(color::WHITE, 0.55))
                .with_width(-1.2)
                .with_dash(0.15)
                .with_style(draw_style::PLAIN),
        ),
        None => out.push(DrawEntity::delete(&root, "parent")),
    }
    out
}

/// How [`pose_array`] draws each pose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Chevron,
    Arrow,
    Axes,
}

impl Glyph {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "chevron" | "flat" | "" => Some(Self::Chevron),
            "arrow" | "arrows" | "3d" => Some(Self::Arrow),
            "axes" | "axis" => Some(Self::Axes),
            _ => None,
        }
    }
}

/// Poses as flat chevrons, arrows or triads, `length` long.
pub fn pose_array(place: &Place, ns: &str, id: &str, poses: &[Pose], glyph: Glyph, length: f32) -> DrawEntity {
    let instances: Vec<Instance> = poses.iter().map(|p| p.instance([length, length, length])).collect();
    let e = match glyph {
        Glyph::Axes => DrawEntity::new(draw_kind::AXES, ns, id).with_instances(&instances).with_scale([length; 3]),
        Glyph::Arrow => {
            DrawEntity::new(draw_kind::ARROWS, ns, id).with_instances(&instances).with_scale([length; 3]).with_style(draw_style::ARROW_GLOW)
        }
        Glyph::Chevron => DrawEntity::new(draw_kind::ARROWS, ns, id)
            .with_sub(ARROW_CHEVRON)
            .with_instances(&instances)
            .with_scale([length; 3])
            .with_style(draw_style::ARROW_GLOW)
            .with_flags(draw_flag::FLAT | draw_flag::ADDITIVE),
    };
    place.put(e)
}

/// Numbered route stations, selected corner brackets, and a connecting ribbon.
/// Completed stations retain 35 % opacity; the current index selects one marker.
pub fn waypoints(place: &Place, ns: &str, id: &str, points: &[[f32; 3]], current: usize) -> Vec<DrawEntity> {
    waypoints_colored(place, ns, id, points, current, color::PLAN)
}

/// Route tint; completed/current stations retain their semantic RGB and share the tint's opacity.
pub fn waypoints_colored(place: &Place, ns: &str, id: &str, points: &[[f32; 3]], current: usize, tint: u32) -> Vec<DrawEntity> {
    let root = path(ns, id);
    if points.is_empty() {
        return ["route", "pillars", "stations", "current", "selection"].iter()
            .map(|id| place.put(DrawEntity::delete(&root, id))).collect();
    }
    let labels: Vec<String> = (1..=points.len()).map(|i| i.to_string()).collect();
    let plan = tint.max(1);
    let opacity = (tint >> 24) as f32 / 255.0;
    let selected = faded(color::GOAL, opacity);
    let colors: Vec<u32> =
        (0..points.len()).map(|i| if i < current { faded(color::WHITE, 0.35 * opacity) } else { plan }).collect();
    let pillar_colors: Vec<u32> = colors.iter().enumerate().map(|(i, c)| if i == current { 1 } else { *c }).collect();
    let pillar_labels: Vec<String> = labels.iter().enumerate().map(|(i, l)| if i == current { String::new() } else { l.clone() }).collect();
    let mut stations = Vec::with_capacity(points.len() * 6);
    let mut station_colors = Vec::with_capacity(points.len() * 6);
    for (i, p) in points.iter().enumerate() {
        let (a, b) = if i + 1 < points.len() { (*p, points[i + 1]) } else { (points[i.saturating_sub(1)], *p) };
        let delta = std::array::from_fn::<_, 3, _>(|axis| b[axis] as f64 - a[axis] as f64);
        let length = delta.iter().map(|v| v * v).sum::<f64>().sqrt();
        let tangent = if length > 1e-9 { delta.map(|v| v / length) } else { [1.0, 0.0, 0.0] };
        let start = stations.len();
        world::station_mark(&mut stations, p.map(|v| v as f64), tangent, 0.45);
        station_colors.extend(std::iter::repeat_n(if i == current { selected } else { colors[i] }, stations.len() - start));
    }
    let mut out = vec![
        place.put(DrawEntity::lines(&root, "route", points)).with_style(draw_style::RIBBON_FLOW).with_colors(&colors).with_width(0.22),
        place
            .put(DrawEntity::points(&root, "pillars", points))
            .with_sub(point_style::DISC)
            .with_style(draw_style::PLAIN)
            .with_width(-4.0)
            .with_colors(&pillar_colors)
            .labelled(&pillar_labels),
        world::segments(place, &root, "stations", &stations, plan, -1.25).with_colors(&station_colors),
    ];
    match points.get(current) {
        Some(p) => {
            out.push(place
                .put(DrawEntity::points(&root, "current", &[*p]))
                .with_sub(point_style::DISC)
                .with_style(draw_style::PLAIN)
                .with_width(-6.0)
                .with_color(selected)
                .labelled(&[labels[current].clone()])
                .with_prop("label_selected", "true").with_prop("label_icon", "destination"));
            let mut corners = Vec::with_capacity(16);
            let polygon = [[-0.32, -0.32], [0.32, -0.32], [0.32, 0.32], [-0.32, 0.32]]
                .map(|xy| [p[0] + xy[0], p[1] + xy[1], p[2]]);
            world::corner_outline(&mut corners, &polygon);
            out.push(world::segments(place, &root, "selection", &corners, selected, -STRUCTURE_RULE_PX * STRUCTURE_CORNER_GAIN));
        }
        None => {
            out.push(place.put(DrawEntity::delete(&root, "current")));
            out.push(place.put(DrawEntity::delete(&root, "selection")));
        }
    }
    out
}

/// Opacity of the ring at sigma rank `k` (0 the innermost).
fn sigma_alpha(k: usize) -> f32 {
    [1.0, 0.6, 0.35, 0.2].get(k).copied().unwrap_or(0.15)
}

fn faded(tint: u32, opacity: f32) -> u32 {
    color::with_alpha(tint, (tint >> 24) as f32 / 255.0 * opacity).max(1)
}

/// A covariance: 2×2 (row-major, 4 values) as nested draped rings, 3×3 (9)
/// as nested shield ellipsoids, 6×6 (36, x y z roll pitch yaw) as the
/// position ellipsoid plus a yaw wedge; `sigmas` the rings drawn.
pub fn covariance(place: &Place, ns: &str, id: &str, pose: Pose, cov: &[f64], sigmas: &[f32], tint: u32) -> Result<Vec<DrawEntity>, String> {
    let root = path(ns, id);
    let sigmas = if sigmas.is_empty() { &[1.0, 2.0, 3.0][..] } else { sigmas };
    let tint = if tint == 0 { color::CYAN } else { tint };
    let c = pose.f32();
    let mut out = Vec::new();
    match cov.len() {
        4 | 3 => {
            let (xx, xy, yy) = if cov.len() == 4 { (cov[0], cov[1], cov[3]) } else { (cov[0], cov[1], cov[2]) };
            let (a, b, angle) = covariance_ellipse(xx, xy, yy);
            let rings: Vec<Vec<[f32; 3]>> =
                sigmas.iter().map(|k| circle(c, k * a as f32, k * b as f32, angle as f32, 72)).collect();
            let colors: Vec<u32> =
                rings.iter().enumerate().flat_map(|(k, r)| std::iter::repeat_n(faded(tint, sigma_alpha(k)), r.len())).collect();
            out.push(
                place
                    .put(DrawEntity::strips(&root, "rings", &rings))
                    .with_sub(line_mode::LOOP)
                    .with_style(draw_style::DECAL)
                    .with_colors(&colors)
                    .with_width(0.04),
            );
            out.push(place.put(DrawEntity::points(&root, "mean", &[c])).with_color(tint).with_width(-4.0).with_style(draw_style::POINT_GLOW));
            out.push(DrawEntity::delete(&root, "ellipsoid"));
            out.push(DrawEntity::delete(&root, "yaw"));
        }
        9 | 36 => {
            let m = |r: usize, k: usize| if cov.len() == 9 { cov[3 * r + k] } else { cov[6 * r + k] };
            let c3 = [[m(0, 0), m(0, 1), m(0, 2)], [m(1, 0), m(1, 1), m(1, 2)], [m(2, 0), m(2, 1), m(2, 2)]];
            let (sd, q) = covariance_axes(c3);
            let instances: Vec<Instance> = sigmas
                .iter()
                .map(|k| {
                    Instance::at(c, [2.0 * k * sd[0] as f32, 2.0 * k * sd[1] as f32, 2.0 * k * sd[2] as f32]).turned(q.map(|v| v as f32))
                })
                .collect();
            let colors: Vec<u32> = (0..instances.len()).map(|k| faded(tint, sigma_alpha(k))).collect();
            out.push(
                place
                    .put(DrawEntity::shapes(&root, "ellipsoid", shape_kind::ELLIPSOID, &instances))
                    .with_style(draw_style::SHIELD)
                    .with_colors(&colors),
            );
            if cov.len() == 36 {
                let yaw_sd = cov[35].max(0.0).sqrt() as f32;
                let yaw = pose.yaw() as f32;
                let reach = (2.0 * sd[0] as f32).max(0.6) * 1.6;
                let wedges: Vec<Instance> = sigmas
                    .iter()
                    .map(|k| Instance::at(c, [reach, (2.0 * k * yaw_sd).min(TAU), 0.0]).yawed(yaw))
                    .collect();
                let colors: Vec<u32> = (0..wedges.len()).map(|k| faded(color::ORANGE, sigma_alpha(k) * (tint >> 24) as f32 / 255.0)).collect();
                out.push(
                    place
                        .put(DrawEntity::shapes(&root, "yaw", shape_kind::SECTOR, &wedges))
                        .with_style(draw_style::SWEEP)
                        .with_colors(&colors)
                        .with_pulse(-1.0),
                );
            } else {
                out.push(DrawEntity::delete(&root, "yaw"));
            }
            out.push(DrawEntity::delete(&root, "rings"));
            out.push(DrawEntity::delete(&root, "mean"));
        }
        n => return Err(format!("a covariance is 2×2, 3×3 or 6×6, not {n} values")),
    }
    Ok(out)
}

/// A ruler from `a` to `b`: ticks every `spacing` (0 picks one), end caps
/// and its length.
pub fn ruler(place: &Place, ns: &str, id: &str, a: [f32; 3], b: [f32; 3], spacing: f32) -> DrawEntity {
    place.put(DrawEntity::lines(ns, id, &[a, b])).with_style(draw_style::RULER).with_spacing(spacing)
}

/// One 3-D detection (vision_msgs `Detection3D`).
#[derive(Debug, Clone, PartialEq)]
pub struct Detection3d {
    pub pose: Pose,
    pub size: [f32; 3],
    pub class: String,
    pub score: f32,
    pub track: Option<u32>,
    pub velocity: Option<[f32; 3]>,
    /// Past centres of the track, oldest first.
    pub trail: Vec<[f32; 3]>,
}

/// How detections are drawn (vision_msgs_rviz_plugins).
#[derive(Debug, Clone)]
pub struct DetectionStyle {
    /// Colours by class id; the defaults cover car, person, cyclist,
    /// motorcycle, truck, bus and the rest.
    pub class_colors: Vec<(String, u32)>,
    /// Boxes scoring under it are dimmed.
    pub threshold: f32,
    pub only_edge: bool,
    pub line_width: f32,
    pub show_score: bool,
}

impl Default for DetectionStyle {
    fn default() -> Self {
        Self { class_colors: Vec::new(), threshold: 0.0, only_edge: false, line_width: 0.05, show_score: true }
    }
}

/// The plugin's class colours on the Okabe-Ito palette.
pub fn class_color(class: &str, overrides: &[(String, u32)]) -> u32 {
    if let Some((_, c)) = overrides.iter().find(|(k, _)| k.eq_ignore_ascii_case(class)) {
        return (*c).max(1);
    }
    match class.to_ascii_lowercase().as_str() {
        "car" | "vehicle" => color::ORANGE,
        "person" | "pedestrian" | "human" => color::BLUE,
        "cyclist" | "bicycle" | "bike" => color::YELLOW,
        "motorcycle" | "motorbike" => color::PURPLE,
        "truck" | "bus" => color::RED,
        "animal" | "cow" | "sheep" => color::GREEN,
        "" | "other" | "unknown" => color::GREY,
        other => color::OKABE[(other.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32)) % 7) as usize],
    }
}

fn identity_detail(identity: &str, detail: &str) -> String {
    serde_json::to_string(&[identity, detail]).expect("string pair serializes")
}

/// 3-D detections: class-coloured hologram boxes (edges only when asked)
/// with faces as bright as their score, identity and score/track labels,
/// velocity arrows and short fading track trails.
pub fn detections_3d(place: &Place, ns: &str, id: &str, dets: &[Detection3d], style: &DetectionStyle) -> Vec<DrawEntity> {
    let root = path(ns, id);
    if dets.is_empty() {
        return ["boxes", "tags", "brackets", "velocity", "trails"].iter()
            .map(|id| place.put(DrawEntity::delete(&root, id))).collect();
    }
    let mut boxes = Vec::new();
    let mut colors = Vec::new();
    let mut tags = Vec::new();
    let mut tag_text = Vec::new();
    let mut tag_colors = Vec::new();
    let mut arrows = Vec::new();
    let mut arrow_colors = Vec::new();
    let mut trails = Vec::new();
    let mut trail_colors = Vec::new();
    for d in dets {
        let base = class_color(&d.class, &style.class_colors);
        let strong = d.score >= style.threshold;
        let alpha = if strong { 0.35 + 0.65 * d.score.clamp(0.0, 1.0) } else { 0.22 };
        boxes.push(d.pose.instance(d.size));
        colors.push(faded(base, alpha));
        let top = d.pose.apply([0.0, 0.0, 0.5 * d.size[2]]);
        tags.push(top);
        let identity = if d.class.is_empty() { "object" } else { &d.class };
        let mut detail = String::new();
        if style.show_score {
            detail.push_str(&format!("score {:.2}", d.score));
        }
        if let Some(t) = d.track {
            if !detail.is_empty() { detail.push_str(" / "); }
            detail.push_str(&format!("track #{t}"));
        }
        tag_text.push(identity_detail(identity, &detail));
        tag_colors.push(faded(base, if strong { 1.0 } else { 0.5 }));
        if let Some(v) = d.velocity.filter(|v| v.iter().map(|c| c * c).sum::<f32>() > 1e-6) {
            let c = d.pose.f32();
            arrows.push((c, [c[0] + v[0], c[1] + v[1], c[2] + v[2]]));
            arrow_colors.push(base);
        }
        if d.trail.len() >= 2 {
            trail_colors.extend(std::iter::repeat_n(base, d.trail.len()));
            trails.push(d.trail.clone());
        }
    }
    let shapes = DrawEntity::shapes(&root, "boxes", shape_kind::BOX, &boxes).with_colors(&colors);
    let shapes = if style.only_edge {
        shapes.with_fill(draw_flag::FILL_WIRE).with_style(draw_style::PLAIN).with_width(style.line_width).with_prop("corner_frame", "true")
    } else {
        shapes.with_style(draw_style::HOLO).with_beam(-1.0).with_width(style.line_width)
    };
    let mut out = vec![
        place.put(shapes),
        place
            .put(DrawEntity::text(&root, "tags", &tags, ""))
            .with_colors(&tag_colors)
            .with_width(-16.0)
            .with_labels(&tag_text).with_prop("label_icon", "sensor")
            .with_prop("label_parts", "identity-detail-json").with_prop("label_neutral_identity", "true"),
        place.put(DrawEntity::delete(&root, "brackets")),
    ];
    out.push(if arrows.is_empty() {
        place.put(DrawEntity::delete(&root, "velocity"))
    } else {
        place.put(DrawEntity::arrows(&root, "velocity", &arrows)).with_colors(&arrow_colors).with_style(draw_style::ARROW_GLOW)
    });
    out.push(if trails.is_empty() {
        place.put(DrawEntity::delete(&root, "trails"))
    } else {
        place
            .put(DrawEntity::strips(&root, "trails", &trails))
            .with_colors(&trail_colors)
            .with_width(-2.0)
            .with_fade(0.02)
            .with_style(draw_style::PLAIN)
    });
    out
}

/// A point cloud as splats, coloured through `map` by `values` (or flat),
/// sized `size` (> 0 metres, < 0 pixels), sent as `quant` metre steps when
/// `quant` > 0.
pub fn cloud(place: &Place, ns: &str, id: &str, points: &[[f32; 3]], values: &[f32], map: u32, size: f32, quant: f32) -> DrawEntity {
    let e = if quant > 0.0 {
        DrawEntity::new(draw_kind::POINTS, ns, id).with_quantized_points(points, quant)
    } else {
        DrawEntity::points(ns, id, points)
    };
    let e = e.with_sub(super::draw::point_style::DISC).with_style(draw_style::SPLAT).with_width(if size != 0.0 { 2.0 * size } else { -3.0 });
    let e = if values.is_empty() {
        e
    } else if quant > 0.0 {
        let (lo, hi) = super::draw_geom::value_range(values.iter().copied(), 0.0, 0.0);
        e.with_byte_values(values, lo, hi).with_colormap(if map == 0 { colormap::TURBO } else { map }, lo, hi)
    } else {
        e.with_values(values).with_colormap(if map == 0 { colormap::TURBO } else { map }, 0.0, 0.0)
    };
    place.put(e)
}

/// A planar scan: its hits as splats over a faint sweeping fan.
#[allow(clippy::too_many_arguments)]
pub fn scan(place: &Place, ns: &str, id: &str, pose: Pose, ranges: &[f32], angle_min: f32, angle_inc: f32, range_max: f32) -> Vec<DrawEntity> {
    let root = path(ns, id);
    let reach = if range_max > 0.0 { range_max } else { ranges.iter().copied().filter(|r| r.is_finite()).fold(1.0, f32::max) };
    let span = (angle_inc * ranges.len().saturating_sub(1) as f32).abs().min(TAU);
    let mid = angle_min + 0.5 * angle_inc * ranges.len().saturating_sub(1) as f32;
    let hits = DrawEntity::scan(&root, "hits", ranges, angle_min, angle_inc)
        .with_pose(pose.position, pose.rotation)
        .with_prop("range_max", &reach.to_string())
        .with_color(color::SENSOR)
        .with_values(&[])
        .with_width(-3.0);
    let fan = DrawEntity::shapes(&root, "fan", shape_kind::SECTOR, &[Instance::at([0.0; 3], [reach, span, 0.0]).yawed(mid)])
        .with_pose(pose.position, pose.rotation)
        .with_style(draw_style::SWEEP)
        .with_color(color::with_alpha(color::SENSOR, 0.45))
        .with_spacing(reach / 4.0);
    vec![place.put(hits), place.put(fan)]
}

/// A range sensor's cone: a sweep wedge `fov` wide from `min` to `max`,
/// with the reading at `range` as a bright arc.
pub fn range_cone(place: &Place, ns: &str, id: &str, pose: Pose, fov: f32, min: f32, max: f32, range: f32) -> Vec<DrawEntity> {
    let root = path(ns, id);
    let wedge = DrawEntity::shapes(&root, "cone", shape_kind::SECTOR, &[Instance::at([0.0; 3], [max, fov, min])])
        .with_pose(pose.position, pose.rotation)
        .with_style(draw_style::SWEEP)
        .with_color(color::SENSOR);
    let mut out = vec![place.put(wedge)];
    out.push(if range.is_finite() && range >= min && range <= max {
        place
            .put(DrawEntity::lines(&root, "reading", &arc([0.0; 3], range, -0.5 * fov, 0.5 * fov, 24)))
            .with_pose(pose.position, pose.rotation)
            .with_color(color::ORANGE)
            .with_style(draw_style::TUBE_PULSE)
            .with_width(0.03)
            .with_fade(-1.0)
    } else {
        DrawEntity::delete(&root, "reading")
    });
    out
}

/// A camera's frustum along +x of `pose` out to `far`, glowing edges and a
/// faint image plane at `near`.
pub fn camera_frustum(place: &Place, ns: &str, id: &str, pose: Pose, hfov: f32, vfov: f32, near: f32, far: f32) -> DrawEntity {
    place
        .put(DrawEntity::shapes(ns, id, shape_kind::FRUSTUM, &[Instance::at([0.0; 3], [hfov, vfov, far])]))
        .with_pose(pose.position, pose.rotation)
        .with_style(draw_style::SWEEP)
        .with_color(color::SENSOR)
        .with_prop("near", &near.to_string())
}

/// Field of view of a pinhole camera (`K` = fx, fy, cx, cy over a width ×
/// height image) as horizontal and vertical angles.
pub fn camera_fov(fx: f64, fy: f64, width: f64, height: f64) -> (f32, f32) {
    let (h, v) = pinhole_fov(fx, fy, width, height);
    (h as f32, v as f32)
}

/// A particle cloud as additive flat chevrons, each between `min` and `max`
/// long by its weight (nav2 ParticleCloud).
pub fn particles(place: &Place, ns: &str, id: &str, poses: &[Pose], weights: &[f32], min: f32, max: f32, glyph: Glyph) -> DrawEntity {
    particles_colored(place, ns, id, poses, weights, min, max, glyph, color::CYAN)
}

/// Particle tint multiplied by weight opacity; axes keep their axis RGB convention.
#[allow(clippy::too_many_arguments)]
pub fn particles_colored(place: &Place, ns: &str, id: &str, poses: &[Pose], weights: &[f32], min: f32, max: f32, glyph: Glyph, tint: u32) -> DrawEntity {
    let top = weights.iter().copied().fold(0.0f32, f32::max).max(1e-9);
    let instances: Vec<Instance> = poses
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let w = weights.get(i).copied().unwrap_or(top) / top;
            let len = min + (max - min) * w.clamp(0.0, 1.0);
            p.instance([len, len, len])
        })
        .collect();
    let colors: Vec<u32> = (0..poses.len())
        .map(|i| faded(tint, 0.25 + 0.75 * (weights.get(i).copied().unwrap_or(top) / top).clamp(0.0, 1.0)))
        .collect();
    let e = match glyph {
        Glyph::Axes => DrawEntity::new(draw_kind::AXES, ns, id).with_instances(&instances),
        Glyph::Arrow => DrawEntity::new(draw_kind::ARROWS, ns, id).with_instances(&instances).with_style(draw_style::ARROW_GLOW),
        Glyph::Chevron => DrawEntity::new(draw_kind::ARROWS, ns, id)
            .with_sub(ARROW_CHEVRON)
            .with_instances(&instances)
            .with_style(draw_style::ARROW_GLOW)
            .with_flags(draw_flag::FLAT | draw_flag::ADDITIVE),
    };
    place.put(e.with_colors(&colors).with_width(if glyph == Glyph::Chevron { 0.02 } else { 0.0 }))
}

/// A field: its boundary as a ground decal fenced by a 0.3 m wall of light.
pub fn field(place: &Place, ns: &str, id: &str, boundary: &[[f32; 3]], holes: &[Vec<[f32; 3]>], tint: u32) -> Vec<DrawEntity> {
    let root = path(ns, id);
    let tint = if tint == 0 { color::GREEN } else { tint };
    let mut rings = vec![boundary.to_vec()];
    rings.extend(holes.iter().cloned());
    let mut fence = boundary.to_vec();
    if let Some(first) = boundary.first() {
        fence.push(*first);
    }
    vec![
        place.put(DrawEntity::polygon(&root, "area", &rings)).with_style(draw_style::DECAL).with_color(tint),
        place.put(DrawEntity::lines(&root, "fence", &fence)).with_style(draw_style::LIGHT_WALL).with_color(tint).with_beam(0.3).with_fade(-1.0),
    ]
}

/// A headland: the band between a field's boundary and its inner boundary.
pub fn headland(place: &Place, ns: &str, id: &str, outer: &[[f32; 3]], inner: &[[f32; 3]]) -> DrawEntity {
    place
        .put(DrawEntity::polygon(ns, id, &[outer.to_vec(), inner.to_vec()]))
        .with_style(draw_style::DECAL)
        .with_color(color::ORANGE)
}

/// Swaths `width` wide (the implement's coverage width) with chevrons in
/// the driving direction; the first `done` fade to the history colour.
pub fn swaths(place: &Place, ns: &str, id: &str, lines: &[Vec<[f32; 3]>], width: f32, done: usize) -> Vec<DrawEntity> {
    swaths_colored(place, ns, id, lines, width, done, color::PLAN)
}

/// Future swath tint; completed swaths retain history RGB and share the tint's opacity.
pub fn swaths_colored(place: &Place, ns: &str, id: &str, lines: &[Vec<[f32; 3]>], width: f32, done: usize, tint: u32) -> Vec<DrawEntity> {
    let root = path(ns, id);
    let (old, todo) = lines.split_at(done.min(lines.len()));
    let part = |name: &str, strips: &[Vec<[f32; 3]>], c: u32, flow: f32| {
        if strips.is_empty() {
            DrawEntity::delete(&root, name)
        } else {
            place
                .put(DrawEntity::strips(&root, name, strips))
                .with_style(draw_style::RIBBON_FLOW)
                .with_color(c)
                .with_width(width)
                .with_flow(flow, (width * 1.5).max(0.8))
        }
    };
    vec![part("todo", todo, tint.max(1), 1.2), part("done", old, faded(color::HISTORY, (tint >> 24) as f32 / 255.0), -1.0)]
}

/// Headland turns: narrower dashed ribbons.
pub fn turns(place: &Place, ns: &str, id: &str, paths: &[Vec<[f32; 3]>], width: f32) -> DrawEntity {
    place
        .put(DrawEntity::strips(ns, id, paths))
        .with_style(draw_style::RIBBON_FLOW)
        .with_color(color::CYAN)
        .with_width(0.5 * width.max(0.2))
        .with_dash((width * 1.5).max(0.8))
        .with_flow(1.0, (width * 1.5).max(0.8))
}

/// An AB line through `a` and `b`, run on `extend` metres both ways, dashed
/// and long, its ends named A and B.
pub fn ab_line(place: &Place, ns: &str, id: &str, a: [f32; 3], b: [f32; 3], extend: f32) -> Vec<DrawEntity> {
    let root = path(ns, id);
    let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
    let u = d.map(|c| c / len);
    let start = [a[0] - u[0] * extend, a[1] - u[1] * extend, a[2] - u[2] * extend];
    let end = [b[0] + u[0] * extend, b[1] + u[1] * extend, b[2] + u[2] * extend];
    vec![
        place
            .put(DrawEntity::lines(&root, "line", &[start, a, b, end]))
            .with_style(draw_style::PLAIN)
            .with_color(color::with_alpha(color::YELLOW, 0.85))
            .with_width(-2.0)
            .with_dash(2.0)
            .with_flags(draw_flag::FLAT_CAPS),
        place
            .put(DrawEntity::points(&root, "ends", &[a, b]))
            .with_color(color::YELLOW)
            .with_width(-6.0)
            .with_style(draw_style::POINT_GLOW)
            .labelled(&["A", "B"]),
    ]
}

/// Coverage: a pass count per cell, coloured from none to many passes.
pub fn coverage(place: &Place, ns: &str, id: &str, cols: usize, rows: usize, cell: f32, origin: Pose, counts: &[u8]) -> DrawEntity {
    place
        .put(DrawEntity::grid(ns, id, cols, rows, cell, GridCells::U8(counts.to_vec())))
        .with_pose(origin.position, origin.rotation)
        .with_colormap(colormap::VIRIDIS, 0.0, 4.0)
        .with_prop("unknown", "0")
}

/// An implement `width` wide and `depth` deep behind its hitch, its
/// sections lit when on (AgOpenGPS section control).
pub fn implement(place: &Place, ns: &str, id: &str, width: f32, depth: f32, sections_on: &[bool]) -> DrawEntity {
    let n = sections_on.len().max(1);
    let w = width / n as f32;
    let instances: Vec<Instance> = (0..n)
        .map(|k| Instance::at([-0.5 * depth, -0.5 * width + w * (k as f32 + 0.5), 0.02], [depth, w * 0.92, 0.0]))
        .collect();
    let colors: Vec<u32> = (0..n)
        .map(|k| if sections_on.get(k).copied().unwrap_or(false) { color::GREEN } else { color::with_alpha(color::GREY, 0.5) })
        .collect();
    place.put(DrawEntity::shapes(ns, id, shape_kind::RECT, &instances)).with_style(draw_style::DECAL).with_colors(&colors).with_width(0.03)
}

/// What a zone keeps out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Keepout,
    Slow,
    Stop,
}

impl Zone {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "keepout" | "keep_out" | "no_go" => Some(Self::Keepout),
            "slow" | "speed" => Some(Self::Slow),
            "stop" | "danger" => Some(Self::Stop),
            _ => None,
        }
    }
}

/// A zone on the ground: keep-out purple, slow orange, stop hazard stripes.
pub fn zone(place: &Place, ns: &str, id: &str, kind: Zone, rings: &[Vec<[f32; 3]>]) -> DrawEntity {
    let (style, c) = match kind {
        Zone::Keepout => (draw_style::DECAL, color::PURPLE),
        Zone::Slow => (draw_style::DECAL, color::ORANGE),
        Zone::Stop => (draw_style::ALERT, color::DANGER),
    };
    place.put(DrawEntity::polygon(ns, id, rings)).with_style(style).with_color(c)
}

/// A wrench at `pose`: the force an arrow, the torque an arc with an
/// arrowhead about its axis, `scale` metres per newton (newton-metre).
pub fn wrench(place: &Place, ns: &str, id: &str, pose: Pose, force: [f32; 3], torque: [f32; 3], scale: f32) -> Vec<DrawEntity> {
    let root = path(ns, id);
    let s = if scale > 0.0 { scale } else { 0.1 };
    let tip = force.map(|f| f * s);
    let mut out = vec![
        place
            .put(DrawEntity::arrows(&root, "force", &[([0.0; 3], tip)]))
            .with_pose(pose.position, pose.rotation)
            .with_color(color::RED)
            .with_style(draw_style::ARROW_GLOW)
            .with_width(0.03),
    ];
    let mag = (torque[0] * torque[0] + torque[1] * torque[1] + torque[2] * torque[2]).sqrt();
    out.push(if mag > 1e-6 {
        let axis = torque.map(|t| t / mag);
        let helper = if axis[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
        let u = normalize(cross(helper, axis));
        let v = cross(axis, u);
        let r = 0.25 + 0.05 * (mag * s).min(4.0);
        let sweep = (0.6 + 0.4 * (mag * s).min(1.0)) * 1.5 * PI;
        let pts: Vec<[f32; 3]> = (0..=36)
            .map(|k| {
                let t = sweep * k as f32 / 36.0;
                let (c, sn) = (t.cos() * r, t.sin() * r);
                [u[0] * c + v[0] * sn, u[1] * c + v[1] * sn, u[2] * c + v[2] * sn]
            })
            .collect();
        place
            .put(DrawEntity::lines(&root, "torque", &pts))
            .with_pose(pose.position, pose.rotation)
            .with_color(color::YELLOW)
            .with_style(draw_style::ARROW_GLOW)
            .with_flags(draw_flag::ARROW_TIP)
            .with_width(0.025)
    } else {
        DrawEntity::delete(&root, "torque")
    });
    out
}

/// A contact: a small orb at `point` and its normal force as an arrow.
pub fn contact(place: &Place, ns: &str, id: &str, point: [f32; 3], normal: [f32; 3], force: f32, scale: f32) -> Vec<DrawEntity> {
    let root = path(ns, id);
    let n = normalize(normal);
    let len = (force.abs() * if scale > 0.0 { scale } else { 0.05 }).max(0.15);
    vec![
        place.put(DrawEntity::points(&root, "point", &[point])).with_style(draw_style::ORB).with_color(color::ORANGE).with_width(0.16),
        place
            .put(DrawEntity::arrows(&root, "normal", &[(point, [point[0] + n[0] * len, point[1] + n[1] * len, point[2] + n[2] * len])]))
            .with_style(draw_style::ARROW_GLOW)
            .with_color(color::ORANGE)
            .with_width(0.02),
    ]
}

/// A graph (RRT, PRM): nodes as glowing points, edges as hairlines.
pub fn graph(place: &Place, ns: &str, id: &str, nodes: &[[f32; 3]], edges: &[(usize, usize)], labels: &[String]) -> Vec<DrawEntity> {
    let root = path(ns, id);
    let mut segs = Vec::with_capacity(edges.len() * 2);
    for (a, b) in edges {
        if let (Some(a), Some(b)) = (nodes.get(*a), nodes.get(*b)) {
            segs.push(*a);
            segs.push(*b);
        }
    }
    let mut points = place.put(DrawEntity::points(&root, "nodes", nodes)).with_style(draw_style::POINT_GLOW).with_color(color::CYAN).with_width(-4.0);
    if !labels.is_empty() {
        points = points.labelled(labels);
    }
    vec![
        points,
        place
            .put(DrawEntity::lines(&root, "edges", &segs))
            .with_sub(line_mode::LIST)
            .with_style(draw_style::HAIRLINE)
            .with_color(color::with_alpha(color::CYAN, 0.35))
            .with_width(-1.2),
    ]
}

/// Ghost copies of `machine` along a trajectory, every `step`-th state.
pub fn ghost_trail(place: &Place, ns: &str, id: &str, machine: &str, poses: &[Pose], joints: &[String], values: &[Vec<f32>], step: usize, tint: u32) -> DrawEntity {
    let step = step.max(1);
    let mut picked: Vec<usize> = (0..poses.len()).step_by(step).collect();
    if poses.len() > 1 && picked.last() != Some(&(poses.len() - 1)) {
        picked.push(poses.len() - 1);
    }
    let instances: Vec<Instance> = picked.iter().map(|i| poses[*i].instance([1.0; 3])).collect();
    let mut e = DrawEntity::ghost(ns, id, machine, &instances).with_color(if tint == 0 { color::CYAN } else { tint }).with_fade(0.15);
    if !joints.is_empty() {
        let flat: Vec<f32> =
            picked.iter().flat_map(|i| (0..joints.len()).map(|j| values.get(*i).and_then(|v| v.get(j)).copied().unwrap_or(0.0)).collect::<Vec<_>>()).collect();
        e = e.with_values(&flat).with_prop("joints", &joints.join(","));
    }
    place.put(e)
}

/// A panel at `anchor`: `kind` (sparkline, bars, gauge, chip, table).
pub fn panel(place: &Place, ns: &str, id: &str, anchor: [f32; 3], kind: u32, values: &[f32], labels: &[String], title: &str) -> DrawEntity {
    let mut e = DrawEntity::panel(ns, id, kind, values).with_pose([anchor[0] as f64, anchor[1] as f64, anchor[2] as f64], [1.0, 0.0, 0.0, 0.0]);
    if !labels.is_empty() {
        e = e.with_labels(labels);
    }
    if !title.is_empty() {
        e = e.with_prop("title", title);
    }
    place.put(e)
}

/// A gauge from `min` to `max` showing `value` in `unit`.
pub fn gauge(place: &Place, ns: &str, id: &str, anchor: [f32; 3], value: f32, min: f32, max: f32, title: &str, unit: &str) -> DrawEntity {
    panel(place, ns, id, anchor, panel_kind::GAUGE, &[value], &[], title)
        .with_prop("min", &min.to_string())
        .with_prop("max", &max.to_string())
        .with_prop("unit", unit)
}

/// A state chip naming `text` in the tone of `tint`.
pub fn chip(place: &Place, ns: &str, id: &str, anchor: [f32; 3], text: &str, tint: u32) -> DrawEntity {
    panel(place, ns, id, anchor, panel_kind::CHIP, &[], &[], "").with_prop("text", text).with_color(if tint == 0 { color::PLAN } else { tint })
}

/// A key/value table.
pub fn table(place: &Place, ns: &str, id: &str, anchor: [f32; 3], rows: &[(String, String)], title: &str) -> DrawEntity {
    let text: Vec<String> = rows.iter().map(|(k, v)| format!("{k}={v}")).collect();
    panel(place, ns, id, anchor, panel_kind::TABLE, &[], &[], title).with_prop("rows", &text.join("\n"))
}

/// A colormap legend in screen space: the ramp from `vmin` to `vmax`.
pub fn legend(ns: &str, id: &str, map: u32, vmin: f32, vmax: f32, title: &str, unit: &str) -> DrawEntity {
    DrawEntity::panel(ns, id, panel_kind::LEGEND, &[])
        .in_frame("world")
        .with_colormap(map, vmin, vmax)
        .with_prop("title", title)
        .with_prop("unit", unit)
}

/// A status badge `height` metres over a machine's base link.
pub fn badge(ns: &str, id: &str, machine: &str, text: &str, tint: u32, height: f32) -> DrawEntity {
    DrawEntity::panel(ns, id, panel_kind::CHIP, &[])
        .in_frame(machine)
        .with_pose([0.0, 0.0, height as f64], [1.0, 0.0, 0.0, 0.0])
        .with_prop("text", text)
        .with_prop("badge", "1")
        .with_color(if tint == 0 { color::PLAN } else { tint })
}

/// A compass rose `size` metres across at `at`, north in goal yellow.
pub fn compass(place: &Place, ns: &str, id: &str, at: [f32; 3], size: f32) -> Vec<DrawEntity> {
    compass_colored(place, ns, id, at, size, color::WHITE)
}

/// Compass tint; north retains goal RGB and shares the tint's opacity.
pub fn compass_colored(place: &Place, ns: &str, id: &str, at: [f32; 3], size: f32, tint: u32) -> Vec<DrawEntity> {
    let root = path(ns, id);
    let r = 0.5 * size;
    let ring = circle(at, r, r, 0.0, 72);
    let mut spokes = Vec::new();
    let mut tips = Vec::new();
    for (k, dir) in [[0.0f32, 1.0], [1.0, 0.0], [0.0, -1.0], [-1.0, 0.0]].iter().enumerate() {
        let inner = if k == 0 { 0.0 } else { 0.25 * r };
        spokes.push(vec![[at[0] + dir[0] * inner, at[1] + dir[1] * inner, at[2]], [at[0] + dir[0] * r * 1.15, at[1] + dir[1] * r * 1.15, at[2]]]);
        tips.push([at[0] + dir[0] * r * 1.45, at[1] + dir[1] * r * 1.45, at[2] + 0.05]);
    }
    let north = faded(color::GOAL, (tint >> 24) as f32 / 255.0);
    let tint = tint.max(1);
    let spoke_colors: Vec<u32> = (0..4).flat_map(|k| std::iter::repeat_n(if k == 0 { north } else { tint }, 2)).collect();
    let text_colors = [north, tint, tint, tint];
    vec![
        place.put(DrawEntity::lines(&root, "ring", &ring)).with_sub(line_mode::LOOP).with_style(draw_style::DECAL).with_color(tint).with_width(0.03),
        place.put(DrawEntity::strips(&root, "spokes", &spokes)).with_style(draw_style::DECAL).with_colors(&spoke_colors).with_width(0.05),
        place
            .put(DrawEntity::new(draw_kind::TEXT, &root, "letters").with_points(&tips))
            .with_labels(&["N", "E", "S", "W"])
            .with_colors(&text_colors)
            .with_width(-15.0),
    ]
}

/// A scale bar `length` metres long from `at` along x.
pub fn scale_bar(place: &Place, ns: &str, id: &str, at: [f32; 3], length: f32) -> DrawEntity {
    ruler(place, ns, id, at, [at[0] + length, at[1], at[2]], length / 5.0)
}

/// A callout: `text` at `anchor + offset` on a leader line to `anchor`.
pub fn callout(place: &Place, ns: &str, id: &str, anchor: [f32; 3], offset: [f32; 3], text: &str, tint: u32) -> Vec<DrawEntity> {
    let root = path(ns, id);
    let tint = if tint == 0 { color::PLAN } else { tint };
    let at = [anchor[0] + offset[0], anchor[1] + offset[1], anchor[2] + offset[2]];
    let mut marker = Vec::with_capacity(8);
    let diamond = [[0.0, 0.12], [0.12, 0.0], [0.0, -0.12], [-0.12, 0.0]]
        .map(|xy| [anchor[0] + xy[0], anchor[1] + xy[1], anchor[2]]);
    world::open_outline(&mut marker, &diamond);
    vec![
        place.put(DrawEntity::lines(&root, "leader", &[anchor, at])).with_color(tint).with_width(-1.5).with_style(draw_style::PLAIN),
        world::segments(place, &root, "anchor", &marker, tint, -1.25),
        place.put(DrawEntity::text(&root, "text", &[at], text)).with_width(-16.0).with_color(faded(color::WHITE, (tint >> 24) as f32 / 255.0)).with_color2(0).with_prop("accent", &format!("{tint}")),
    ]
}

/// An angle at `centre` from direction `from` to `to` (radians about z):
/// a faint wedge, its two rays and the angle in degrees.
pub fn angle(place: &Place, ns: &str, id: &str, centre: [f32; 3], from: f32, to: f32, radius: f32) -> Vec<DrawEntity> {
    angle_colored(place, ns, id, centre, from, to, radius, color::YELLOW)
}

/// An angle with caller-supplied geometry and label tint.
#[allow(clippy::too_many_arguments)]
pub fn angle_colored(place: &Place, ns: &str, id: &str, centre: [f32; 3], from: f32, to: f32, radius: f32, tint: u32) -> Vec<DrawEntity> {
    let root = path(ns, id);
    let span = to - from;
    let mid = from + 0.5 * span;
    let r = if radius > 0.0 { radius } else { 1.0 };
    let mut rays = arc(centre, r, from, to, 32);
    rays.insert(0, centre);
    rays.push(centre);
    let text_at = [centre[0] + 1.25 * r * mid.cos(), centre[1] + 1.25 * r * mid.sin(), centre[2] + 0.05];
    vec![
        place
            .put(DrawEntity::shapes(&root, "wedge", shape_kind::SECTOR, &[Instance::at(centre, [r, span.abs(), 0.0]).yawed(mid)]))
            .with_style(draw_style::SWEEP)
            .with_color(faded(tint, 0.6))
            .with_pulse(-1.0),
        place.put(DrawEntity::lines(&root, "rays", &rays)).with_style(draw_style::DECAL).with_color(tint.max(1)).with_width(0.025),
        place.put(DrawEntity::text(&root, "degrees", &[text_at], &format!("{:.1}°", span.to_degrees())))
            .with_color(tint.max(1)).with_width(-16.0),
    ]
}

/// Voxels: a box per centre `size` across, coloured through `map` by
/// `values` (or plain).
pub fn voxels(place: &Place, ns: &str, id: &str, centres: &[[f32; 3]], size: f32, values: &[f32], map: u32, holo: bool) -> DrawEntity {
    let instances: Vec<Instance> = centres.iter().map(|c| Instance::at(*c, [size; 3])).collect();
    let mut e = DrawEntity::shapes(ns, id, shape_kind::BOX, &instances).with_style(if holo { draw_style::HOLO } else { draw_style::PLAIN });
    if holo {
        e = e.with_beam(-1.0);
    } else {
        e = e.with_flags(draw_flag::SHADED);
    }
    if !values.is_empty() {
        e = e.with_values(values).with_colormap(if map == 0 { colormap::TURBO } else { map }, 0.0, 0.0);
    }
    place.put(e)
}

/// Odometry: a chevron at the pose, the velocity as an arrow, the position
/// covariance (2×2, 3×3 or 6×6) under it. Empty covariance removes its four retained components.
pub fn odometry(place: &Place, ns: &str, id: &str, pose: Pose, velocity: [f32; 3], cov: &[f64]) -> Vec<DrawEntity> {
    let root = path(ns, id);
    let c = pose.f32();
    let mut out = vec![pose_array(place, &root, "pose", &[pose], Glyph::Chevron, 0.8).with_color(color::PLAN)];
    let v = super::draw_geom::rotate(pose.rotation, velocity.map(|x| x as f64)).map(|x| x as f32);
    out.push(if v.iter().map(|x| x * x).sum::<f32>() > 1e-6 {
        place
            .put(DrawEntity::arrows(&root, "twist", &[(c, [c[0] + v[0], c[1] + v[1], c[2] + v[2]])]))
            .with_style(draw_style::ARROW_GLOW)
            .with_color(color::PREDICTION)
    } else {
        DrawEntity::delete(&root, "twist")
    });
    if cov.is_empty() {
        let cov_root = path(&root, "cov");
        out.extend(["rings", "mean", "ellipsoid", "yaw"].map(|id| DrawEntity::delete(&cov_root, id)));
    } else if let Ok(parts) = covariance(place, &root, "cov", pose, cov, &[1.0, 2.0], color::PLAN) {
        out.extend(parts);
    }
    out
}

/// A band: a path drawn as a pulsing tube as wide as `sigma` at each point.
pub fn band(place: &Place, ns: &str, id: &str, points: &[[f32; 3]], sigma: &[f32]) -> DrawEntity {
    place
        .put(DrawEntity::lines(ns, id, points))
        .with_style(draw_style::TUBE_PULSE)
        .with_radii(sigma)
        .with_color(color::with_alpha(color::PREDICTION, 0.6))
        .with_fade(-1.0)
}

/// Sampled rollouts tinted by cost (low green, high vermillion), the
/// colliding ones magenta; nav2 MPPI keeps every `stride`-th.
pub fn rollouts(place: &Place, ns: &str, id: &str, strips: &[Vec<[f32; 3]>], costs: &[f32], collisions: &[bool], stride: usize) -> Vec<DrawEntity> {
    let root = path(ns, id);
    let stride = stride.max(1);
    let mut free = Vec::new();
    let mut free_cost = Vec::new();
    let mut hit = Vec::new();
    for (k, s) in strips.iter().enumerate().step_by(stride) {
        if collisions.get(k).copied().unwrap_or(false) {
            hit.push(s.clone());
        } else {
            free.push(s.clone());
            free_cost.push(costs.get(k).copied().unwrap_or(0.0));
        }
    }
    let (lo, hi) = super::draw_geom::value_range(free_cost.iter().copied(), 0.0, 0.0);
    vec![
        if free.is_empty() {
            DrawEntity::delete(&root, "samples")
        } else {
            place
                .put(DrawEntity::strips(&root, "samples", &free))
                .with_style(draw_style::HAIRLINE)
                .with_values(&free_cost)
                .with_colormap(colormap::RDYLGN, hi, lo)
                .with_color(color::with_alpha(color::WHITE, 0.12))
        },
        if hit.is_empty() {
            DrawEntity::delete(&root, "collisions")
        } else {
            place.put(DrawEntity::strips(&root, "collisions", &hit)).with_style(draw_style::HAIRLINE).with_color(color::with_alpha(color::MAGENTA, 0.35))
        },
    ]
}

/// A path as a flowing ribbon with a pose marker (axes or arrows) every
/// `spacing` metres (RViz Path pose style).
pub fn path_with_markers(place: &Place, ns: &str, id: &str, points: &[[f32; 3]], spacing: f32, glyph: Glyph) -> Vec<DrawEntity> {
    let root = path(ns, id);
    let mut poses = Vec::new();
    let mut along = 0.0f32;
    let mut next = 0.0f32;
    for w in points.windows(2) {
        let d = [w[1][0] - w[0][0], w[1][1] - w[0][1], w[1][2] - w[0][2]];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        while spacing > 0.0 && next <= along + len {
            let t = if len > 1e-9 { (next - along) / len } else { 0.0 };
            let p = [w[0][0] + d[0] * t, w[0][1] + d[1] * t, w[0][2] + d[2] * t];
            poses.push(Pose::yawed(p[0] as f64, p[1] as f64, p[2] as f64, d[1].atan2(d[0]) as f64));
            next += spacing;
        }
        along += len;
    }
    vec![
        place.put(DrawEntity::lines(&root, "path", points)).with_style(draw_style::RIBBON_FLOW),
        if poses.is_empty() { DrawEntity::delete(&root, "poses") } else { pose_array(place, &root, "poses", &poses, glyph, 0.5) },
    ]
}

/// Boxes and keypoints of 2-D detections drawn on a camera's image plane
/// `depth` metres out, with a faint wedge from the camera to each box.
/// Boxes are centre x, y and size w, h in pixels of a `width` × `height`
/// image seen with fields of view `hfov` × `vfov`.
#[allow(clippy::too_many_arguments)]
pub fn detections_2d(
    place: &Place,
    ns: &str,
    id: &str,
    camera: Pose,
    hfov: f32,
    vfov: f32,
    image: [f32; 2],
    boxes: &[[f32; 4]],
    classes: &[String],
    scores: &[f32],
    depth: f32,
) -> Vec<DrawEntity> {
    let root = path(ns, id);
    let d = if depth > 0.0 { depth } else { 2.0 };
    let half_w = d * (0.5 * hfov).tan();
    let half_h = d * (0.5 * vfov).tan();
    let to_plane = |u: f32, v: f32| -> [f32; 3] {
        let y = half_w * (1.0 - 2.0 * u / image[0].max(1.0));
        let z = half_h * (1.0 - 2.0 * v / image[1].max(1.0));
        camera.apply([d, y, z])
    };
    let origin = camera.f32();
    let mut rects = Vec::new();
    let mut rect_colors = Vec::new();
    let mut wedge = Vec::new();
    let mut wedge_colors = Vec::new();
    let mut tags = Vec::new();
    let mut tag_text = Vec::new();
    let mut tag_colors = Vec::new();
    for (k, b) in boxes.iter().enumerate() {
        let class = classes.get(k).cloned().unwrap_or_default();
        let c = class_color(&class, &[]);
        let corners = [
            to_plane(b[0] - 0.5 * b[2], b[1] - 0.5 * b[3]),
            to_plane(b[0] + 0.5 * b[2], b[1] - 0.5 * b[3]),
            to_plane(b[0] + 0.5 * b[2], b[1] + 0.5 * b[3]),
            to_plane(b[0] - 0.5 * b[2], b[1] + 0.5 * b[3]),
        ];
        rects.push(corners.to_vec());
        rect_colors.extend([c; 4]);
        for i in 0..4 {
            wedge.extend([origin, corners[i], corners[(i + 1) % 4]]);
        }
        wedge_colors.extend(std::iter::repeat_n(color::with_alpha(c, 0.07), 12));
        tags.push(corners[0]);
        let detail = scores.get(k).map(|s| format!("score {s:.2}")).unwrap_or_default();
        tag_text.push(identity_detail(if class.is_empty() { "object" } else { &class }, &detail));
        tag_colors.push(c);
    }
    vec![
        place.put(DrawEntity::strips(&root, "boxes", &rects)).with_sub(line_mode::LOOP).with_colors(&rect_colors)
            .with_width(-STRUCTURE_RULE_PX).with_style(draw_style::PLAIN).with_prop("corner_frame", "true"),
        place.put(DrawEntity::triangles(&root, "wedges", &wedge)).with_colors(&wedge_colors),
        place.put(DrawEntity::text(&root, "tags", &tags, "")).with_colors(&tag_colors).with_width(-16.0).with_labels(&tag_text)
            .with_prop("label_icon", "sensor").with_prop("label_parts", "identity-detail-json").with_prop("label_neutral_identity", "true"),
    ]
}

/// The TF tree of `machine` switched on in the viewer while this lives.
pub fn tf_tree(ns: &str, id: &str, machine: &str) -> DrawEntity {
    DrawEntity::new(draw_kind::AXES, ns, id).with_prop("tf", machine).with_scale([0.0; 3])
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 1e-9 { v.map(|c| c / l) } else { [0.0, 0.0, 1.0] }
}

/// The rotation (w, x, y, z) of a frame whose x points along `dir`, z as
/// near up as it can.
pub fn look_along(dir: [f32; 3]) -> [f64; 4] {
    let x = normalize(dir);
    let up = if x[2].abs() > 0.95 { [1.0, 0.0, 0.0] } else { [0.0, 0.0, 1.0] };
    let y = normalize(cross(up, x));
    let z = cross(x, y);
    quat_from_axes(x.map(|v| v as f64), y.map(|v| v as f64), z.map(|v| v as f64))
}
