//! Geometry of one draw entity in its own Z-up frame: strokes (segments and
//! points the stroke shader turns into capsules), surfaces (filled shapes,
//! triangles, ground rings, beams, walls of light and decals), and the
//! labels shown at its items, all in the look of the entity's style.

use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, TAU};

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use gearbox_api::draw::default_color;
use gearbox_api::wire::draw::unpack_rgba;
use gearbox_api::wire::draw_geom::{colormap_rgba, triangulate, value_range};
use gearbox_api::wire::draw_widgets::{ARROW_CHEVRON, POINT_SQUARE};
use gearbox_api::{DrawEntity, Instance, colormap, draw_flag, draw_kind, draw_style, line_mode, point_style, shape_kind};

/// Terrain height over the entity's z = 0 at entity x, y.
pub type Ground<'a> = Option<&'a dyn Fn(f32, f32) -> f32>;

use super::material::{
    ATTR_CODE, ATTR_COLOR_A, ATTR_COLOR_B, ATTR_DIST, ATTR_END, ATTR_FILL_COLOR, ATTR_FX, ATTR_FX2, ATTR_LOCAL, ATTR_NEXT,
    ATTR_PREV, ATTR_RADII, ATTR_SURFACE_FX, ATTR_SURFACE_FX2,
};

pub const FLAG_TUBE: u32 = 1;
pub const FLAG_SPHERE: u32 = 2;
pub const FLAG_FLAT_START: u32 = 4;
pub const FLAG_FLAT_END: u32 = 8;
pub const FLAG_DASHED: u32 = 16;
pub const FLAG_FLAT: u32 = 32;
pub const FLAG_ADD: u32 = 64;
pub const FLAG_HEAD: u32 = 128;
pub const FLAG_SQUARE: u32 = 256;

/// Looks of the surface shader.
const LOOK_PLAIN: f32 = 0.0;
const LOOK_WALL: f32 = 1.0;
const LOOK_RING: f32 = 2.0;
const LOOK_HOLO: f32 = 3.0;
const LOOK_SHIELD: f32 = 4.0;
const LOOK_ALERT: f32 = 5.0;
const LOOK_DECAL: f32 = 6.0;
const LOOK_BEAM: f32 = 7.0;
const LOOK_FOOTPRINT: f32 = 8.0;
const LOOK_SWEEP: f32 = 9.0;
const LOOK_GHOST: f32 = 10.0;
const LOOK_PLANE: f32 = 11.0;
const LOOK_ZONE: f32 = 12.0;

/// Role colours (Okabe-Ito) auto styles draw in when no colour is given.
const PLAN: [u8; 4] = [0x56, 0xB4, 0xE9, 255];
const PREDICTION: [u8; 4] = [0x00, 0x9E, 0x73, 255];
const LOOKAHEAD: [u8; 4] = [0xE6, 0x9F, 0x00, 255];
const GOAL: [u8; 4] = [0xF0, 0xE4, 0x42, 255];
const OBSTACLE: [u8; 4] = [0xCC, 0x79, 0xA7, 255];
const DANGER: [u8; 4] = [0xD5, 0x5E, 0x00, 255];
const HISTORY: [u8; 4] = [255, 255, 255, 140];
const SENSOR: [u8; 4] = [0x00, 0x72, 0xB2, 255];

/// Frame axes: x vermillion, y bluish green, z blue.
const AXIS_COLORS: [[u8; 4]; 3] = [[0xD5, 0x5E, 0x00, 255], [0x00, 0x9E, 0x73, 255], [0x00, 0x72, 0xB2, 255]];
const LABEL_BACKGROUND: [u8; 4] = [0, 0, 0, 153];
/// Lift of ribbons, rings and decals off the plane they lie in, metres.
const GROUND_LIFT: f32 = 0.03;
/// Longest edge left whole where a surface lies on the ground, metres.
const HUG_STEP: f32 = 0.5;
/// Distance from a polygon's edge its decal fill fades over, metres.
const DECAL_FADE: f32 = 0.4;

/// The style an entity is drawn in: its own, or for [`draw_style::AUTO`]
/// one picked by kind and by role words in its path.
pub fn resolve_style(e: &DrawEntity, ns: &str, id: &str) -> u32 {
    if e.style != draw_style::AUTO {
        return e.style;
    }
    let path = format!("{ns}/{id}").to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| path.contains(w));
    let alert = has(&["alert", "collision", "danger", "warn"]);
    match e.kind {
        draw_kind::TEXT => draw_style::LABEL,
        draw_kind::GRID => draw_style::HEAT,
        draw_kind::GHOST => draw_style::GHOST,
        draw_kind::POLYGON if alert => draw_style::ALERT,
        draw_kind::POLYGON => draw_style::DECAL,
        draw_kind::LINES if has(&["ruler", "measure"]) => draw_style::RULER,
        draw_kind::POINTS if has(&["cloud", "scan", "lidar", "splat"]) => draw_style::SPLAT,
        draw_kind::SHAPES if matches!(e.sub, shape_kind::SECTOR | shape_kind::FRUSTUM) => draw_style::SWEEP,
        draw_kind::LINES if alert => draw_style::ALERT,
        draw_kind::LINES if has(&["rollout", "sample"]) => draw_style::HAIRLINE,
        draw_kind::LINES if has(&["horizon", "predict", "mpc"]) => draw_style::TUBE_PULSE,
        draw_kind::LINES if has(&["trace", "history", "driven", "trail"]) => draw_style::LIGHT_WALL,
        draw_kind::LINES if has(&["footprint"]) => draw_style::PLAIN,
        draw_kind::LINES if has(&["path", "plan", "route"]) => draw_style::RIBBON_FLOW,
        draw_kind::POINTS if has(&["lookahead", "carrot"]) => draw_style::ORB,
        draw_kind::POINTS | draw_kind::ARROWS | draw_kind::SHAPES if has(&["goal"]) => draw_style::BEACON,
        draw_kind::POINTS => draw_style::POINT_GLOW,
        draw_kind::ARROWS => draw_style::ARROW_GLOW,
        draw_kind::SHAPES if alert => draw_style::ALERT,
        draw_kind::SHAPES if has(&["obstacle", "pred", "shield"]) => draw_style::SHIELD,
        draw_kind::SHAPES if has(&["waypoint", "holo"]) => draw_style::HOLO,
        draw_kind::SHAPES if has(&["footprint"]) => draw_style::HOLO,
        _ => draw_style::PLAIN,
    }
}

fn role_color(style: u32) -> Option<[u8; 4]> {
    Some(match style {
        draw_style::RIBBON_FLOW | draw_style::HOLO => PLAN,
        draw_style::TUBE_PULSE | draw_style::HAIRLINE => PREDICTION,
        draw_style::LIGHT_WALL => HISTORY,
        draw_style::BEACON => GOAL,
        draw_style::SHIELD => OBSTACLE,
        draw_style::ALERT => DANGER,
        draw_style::ORB => LOOKAHEAD,
        draw_style::DECAL => [255, 255, 255, 255],
        draw_style::RULER => GOAL,
        draw_style::SWEEP => SENSOR,
        draw_style::SPLAT => PLAN,
        draw_style::GHOST => PLAN,
        _ => return None,
    })
}

/// Seconds an entity's animations run ahead by, from its path.
fn path_phase(ns: &str, id: &str) -> f32 {
    let mut hash: u32 = 0x811c_9dc5;
    for b in ns.bytes().chain([b'/']).chain(id.bytes()) {
        hash = (hash ^ b as u32).wrapping_mul(0x0100_0193);
    }
    (hash % 6000) as f32 * 0.01
}

/// Seconds item `i` runs ahead of the entity.
fn item_phase(i: usize) -> f32 {
    (i as f32 * 0.618_034) % 1.0 * 7.0
}

/// An entity's effect values with the style's defaults filled in.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Fx {
    pub style: u32,
    pub glow: f32,
    pub pulse_hz: f32,
    pub flow: f32,
    pub spacing: f32,
    pub beam: f32,
    pub fade: f32,
    pub urgency: f32,
}

impl Fx {
    pub fn of(e: &DrawEntity, style: u32) -> Self {
        let pick = |given: f32, default: f32| if given > 0.0 { given } else if given < 0.0 { 0.0 } else { default };
        let urgency = e.urgency.clamp(0.0, 1.0);
        Self {
            style,
            glow: pick(
                e.glow,
                match style {
                    draw_style::BEACON => 4.0,
                    draw_style::HAIRLINE | draw_style::DECAL => 0.8,
                    draw_style::LIGHT_WALL => 0.6,
                    draw_style::POINT_GLOW => 0.5,
                    draw_style::RULER => 0.6,
                    draw_style::PLAIN | draw_style::LABEL | draw_style::AUTO | draw_style::SPLAT => 0.0,
                    _ => 1.0,
                },
            ),
            pulse_hz: pick(
                e.pulse_hz,
                match style {
                    draw_style::TUBE_PULSE => 1.0,
                    draw_style::BEACON => 0.5,
                    draw_style::HOLO => 0.3,
                    draw_style::SHIELD => 0.3 + 1.2 * urgency,
                    draw_style::ALERT => 2.0,
                    draw_style::ORB => 1.5,
                    draw_style::SWEEP => 0.5,
                    draw_style::GHOST => 0.25,
                    _ => 0.0,
                },
            )
            .min(if style == draw_style::SWEEP { 1.0 } else { 3.0 }),
            flow: pick(e.flow_speed, if style == draw_style::RIBBON_FLOW { 1.5 } else { 0.0 }),
            spacing: pick(
                e.dash,
                match style {
                    draw_style::RIBBON_FLOW => 1.2,
                    draw_style::RULER | draw_style::SWEEP | draw_style::HEAT => 0.0,
                    _ => 0.4,
                },
            ),
            beam: pick(
                e.beam_height,
                match style {
                    draw_style::BEACON => 12.0,
                    draw_style::LIGHT_WALL => 0.25,
                    _ => 2.0,
                },
            ),
            fade: if e.fade < 0.0 {
                1.0
            } else {
                pick(
                    e.fade,
                    match style {
                        draw_style::TUBE_PULSE | draw_style::GHOST => 0.15,
                        draw_style::LIGHT_WALL => 0.0,
                        _ => 1.0,
                    },
                )
                .min(1.0)
            },
            urgency,
        }
    }
}

/// Segments and points for the stroke material, with the effect values
/// every new one is drawn with.
#[derive(Default)]
pub struct Strokes {
    a: Vec<[f32; 3]>,
    b: Vec<[f32; 3]>,
    color_a: Vec<[u8; 4]>,
    color_b: Vec<[u8; 4]>,
    radii: Vec<[f32; 2]>,
    dist: Vec<[f32; 2]>,
    code: Vec<u32>,
    fx: Vec<[f32; 4]>,
    fx2: Vec<[f32; 4]>,
    prev: Vec<[f32; 3]>,
    next: Vec<[f32; 3]>,
    indices: Vec<u32>,
    /// Effects of what is added next.
    pub pen: Fx,
    /// Seconds what is added next runs ahead by.
    pub phase: f32,
    strip_len: f32,
}

impl Strokes {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// Room for `primitives` more segments or points.
    pub fn reserve(&mut self, primitives: usize) {
        let corners = 4 * primitives;
        self.a.reserve(corners);
        self.b.reserve(corners);
        self.color_a.reserve(corners);
        self.color_b.reserve(corners);
        self.radii.reserve(corners);
        self.dist.reserve(corners);
        self.code.reserve(corners);
        self.fx.reserve(corners);
        self.fx2.reserve(corners);
        self.prev.reserve(corners);
        self.next.reserve(corners);
        self.indices.reserve(6 * primitives);
    }

    /// A capsule from `a` to `b` joined to `prev` and `next` (equal to `a`
    /// and `b` where nothing joins); radii > 0 metres, < 0 pixels.
    #[allow(clippy::too_many_arguments)]
    pub fn segment(&mut self, a: Vec3, b: Vec3, ca: [u8; 4], cb: [u8; 4], ra: f32, rb: f32, dist: [f32; 2], flags: u32, prev: Vec3, next: Vec3) {
        let base = self.a.len() as u32;
        let p = self.pen;
        let fx = [p.glow, p.pulse_hz, p.flow, p.spacing];
        let fx2 = [p.fade, self.strip_len.max(dist[1]), p.urgency, self.phase];
        for corner in 0..4u32 {
            self.a.push(a.into());
            self.b.push(b.into());
            self.color_a.push(ca);
            self.color_b.push(cb);
            self.radii.push([ra, rb]);
            self.dist.push(dist);
            self.code.push(corner | (flags & 0xfff) << 2 | (p.style & 0x3f) << 14);
            self.fx.push(fx);
            self.fx2.push(fx2);
            self.prev.push(prev.into());
            self.next.push(next.into());
        }
        self.indices.extend([base, base + 1, base + 3, base, base + 3, base + 2]);
    }

    pub fn point(&mut self, p: Vec3, color: [u8; 4], radius: f32, flags: u32) {
        self.strip_len = 0.0;
        self.segment(p, p, color, color, radius, radius, [0.0; 2], flags, p, p);
    }

    /// A polyline, closed when `closed`; `flat_caps` squares its two ends.
    #[allow(clippy::too_many_arguments)]
    pub fn polyline(&mut self, points: &[Vec3], colors: &[[u8; 4]], radii: &[f32], closed: bool, flags: u32, flat_caps: bool) {
        let n = points.len();
        match n {
            0 => {}
            1 => self.point(points[0], colors[0], radii[0], flags & (FLAG_FLAT | FLAG_ADD) | if flags & FLAG_TUBE != 0 { FLAG_SPHERE } else { 0 }),
            _ => {
                let closed = closed && n > 2;
                let count = if closed { n } else { n - 1 };
                self.strip_len = (0..count).map(|i| points[i].distance(points[(i + 1) % n])).sum();
                let mut along = 0.0;
                for i in 0..count {
                    let j = (i + 1) % n;
                    let step = points[i].distance(points[j]);
                    let prev = if i > 0 { points[i - 1] } else if closed { points[n - 1] } else { points[i] };
                    let next = if j + 1 < n { points[j + 1] } else if closed { points[(j + 1) % n] } else { points[j] };
                    let mut f = flags;
                    if flat_caps && !closed && i == 0 {
                        f |= FLAG_FLAT_START;
                    }
                    if flat_caps && !closed && i + 1 == count {
                        f |= FLAG_FLAT_END;
                    }
                    self.segment(points[i], points[j], colors[i], colors[j], radii[i], radii[j], [along, along + step], f, prev, next);
                    along += step;
                }
            }
        }
    }

    /// An arrow from `tail` to `tip` with a head of its own colour.
    pub fn arrow(&mut self, tail: Vec3, tip: Vec3, color: [u8; 4], radius: f32, flags: u32) {
        let span = tip - tail;
        let len = span.length();
        if len < 1e-6 {
            self.point(tail, color, radius, flags & FLAG_FLAT);
            return;
        }
        let dir = span / len;
        let head_len = if radius > 0.0 { (radius * 6.5).min(0.45 * len) } else { 0.24 * len };
        let neck = tip - dir * head_len;
        self.strip_len = len;
        self.segment(tail, neck, color, color, radius, radius, [0.0, len - head_len], flags, tail, tip);
        let head = flags & (FLAG_FLAT | FLAG_ADD) | FLAG_FLAT_START | FLAG_HEAD;
        self.segment(neck, tip, color, color, radius * 2.7, radius * 0.3, [len - head_len, len], head, tail, tip);
    }

    /// An arrowhead on the end of `strip`, along its last segment.
    pub fn tip(&mut self, strip: &[Vec3], color: [u8; 4], radius: f32, flags: u32) {
        let n = strip.len();
        if n < 2 {
            return;
        }
        let tip = strip[n - 1];
        let Some(from) = strip[..n - 1].iter().rev().find(|p| p.distance(tip) > 1e-5) else {
            return;
        };
        let total: f32 = strip.windows(2).map(|w| w[0].distance(w[1])).sum();
        let dir = (tip - *from).normalize();
        let head_len = if radius > 0.0 { (radius * 6.5).min(0.45 * total) } else { (0.08 * total).clamp(0.05, 0.6) };
        let saved = self.pen.style;
        if matches!(saved, draw_style::RIBBON_FLOW | draw_style::HAIRLINE | draw_style::TUBE_PULSE | draw_style::LIGHT_WALL) {
            self.pen.style = draw_style::ARROW_GLOW;
        }
        self.strip_len = total;
        let neck = tip - dir * head_len;
        let head = flags & (FLAG_FLAT | FLAG_ADD) | FLAG_FLAT_START | FLAG_HEAD;
        self.segment(neck, tip, color, color, radius * 2.7, radius * 0.3, [total - head_len, total], head, neck, tip);
        self.pen.style = saved;
    }

    /// A chevron `length` long pointing along `dir` from `centre`: two
    /// strokes from its wings brightening into its tip.
    pub fn chevron(&mut self, centre: Vec3, dir: Vec3, side: Vec3, length: f32, color: [u8; 4], radius: f32, flags: u32) {
        let tip = centre + dir * (0.5 * length);
        let back = centre - dir * (0.3 * length);
        for wing in [back + side * (0.42 * length), back - side * (0.42 * length)] {
            self.strip_len = wing.distance(tip);
            self.segment(wing, tip, color, color, radius, radius, [0.0, self.strip_len], flags, wing, tip);
        }
    }

    pub fn into_mesh(self) -> Mesh {
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.a)
            .with_inserted_attribute(ATTR_END, self.b)
            .with_inserted_attribute(ATTR_COLOR_A, VertexAttributeValues::Unorm8x4(self.color_a))
            .with_inserted_attribute(ATTR_COLOR_B, VertexAttributeValues::Unorm8x4(self.color_b))
            .with_inserted_attribute(ATTR_RADII, self.radii)
            .with_inserted_attribute(ATTR_DIST, self.dist)
            .with_inserted_attribute(ATTR_CODE, self.code)
            .with_inserted_attribute(ATTR_FX, self.fx)
            .with_inserted_attribute(ATTR_FX2, self.fx2)
            .with_inserted_attribute(ATTR_PREV, self.prev)
            .with_inserted_attribute(ATTR_NEXT, self.next)
            .with_inserted_indices(Indices::U32(self.indices))
    }
}

/// Triangles with a colour per vertex for the surface material, with the
/// look every new one is drawn with.
#[derive(Default)]
pub struct Surfaces {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    colors: Vec<[u8; 4]>,
    local: Vec<[f32; 4]>,
    fx: Vec<[f32; 4]>,
    fx2: Vec<[f32; 4]>,
    indices: Vec<u32>,
    /// Look, glow, pulse Hz and phase of what is added next.
    pub look: [f32; 4],
    /// Urgency, size, length or spacing, and fade of what is added next.
    pub look2: [f32; 4],
    /// Spans of vertices and indices lying on the ground about an anchor.
    grounded: Vec<(std::ops::Range<usize>, std::ops::Range<usize>, Vec2)>,
}

impl Surfaces {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// Whether anything in it is see-through or light.
    pub fn translucent(&self) -> bool {
        self.colors.iter().any(|c| c[3] < 255) || self.fx.iter().any(|f| f[0] != LOOK_PLAIN)
    }

    fn vertex(&mut self, p: Vec3, n: Vec3, c: [u8; 4], local: [f32; 4]) -> u32 {
        self.positions.push(p.into());
        self.normals.push(n.into());
        self.colors.push(c);
        self.local.push(local);
        self.fx.push(self.look);
        self.fx2.push(self.look2);
        self.positions.len() as u32 - 1
    }

    /// Unit geometry placed by `pose` and stretched by `size`.
    fn place(&mut self, unit: &UnitMesh, pose: &Instance, size: Vec3, color: [u8; 4]) {
        let (rot, pos) = pose_of(pose);
        let inv = Vec3::new(inv_or_one(size.x), inv_or_one(size.y), inv_or_one(size.z));
        let base = self.positions.len() as u32;
        for (p, n) in unit.positions.iter().zip(&unit.normals) {
            let local = *p * unit.to_unit;
            self.vertex(pos + rot * (*p * size), (rot * (*n * inv)).normalize_or(Vec3::Z), color, [local.x, local.y, local.z, unit.kind]);
        }
        self.indices.extend(unit.indices.iter().map(|i| base + i));
    }

    /// A flat disc `radius` across in the XY plane at `centre`.
    fn ground_disc(&mut self, centre: Vec3, radius: f32, color: [u8; 4]) {
        self.look2[1] = radius;
        let at = Instance::at((centre + Vec3::Z * GROUND_LIFT).into(), [1.0; 3]);
        let mark = self.mark();
        self.place(&UnitMesh::disc(72), &at, Vec3::new(2.0 * radius, 2.0 * radius, 1.0), color);
        self.on_ground(mark, centre.truncate());
    }

    /// An open cylinder `radius` wide and `height` tall standing on `base`.
    fn column(&mut self, base: Vec3, radius: f32, height: f32, color: [u8; 4]) {
        let segments = 40u32;
        let first = self.positions.len() as u32;
        for k in 0..=segments {
            let theta = k as f32 / segments as f32 * TAU;
            let n = Vec3::new(theta.cos(), theta.sin(), 0.0);
            self.vertex(base + n * radius, n, color, [k as f32 / segments as f32, 0.0, 0.0, 0.0]);
            self.vertex(base + n * radius + Vec3::Z * height, n, color, [k as f32 / segments as f32, 1.0, 0.0, 0.0]);
        }
        for k in 0..segments {
            let i = first + 2 * k;
            self.indices.extend([i, i + 2, i + 1, i + 1, i + 2, i + 3]);
        }
    }

    /// A wall `height` tall standing on the polyline.
    fn wall(&mut self, points: &[Vec3], height: f32, color: [u8; 4]) {
        let mut along = 0.0;
        for pair in points.windows(2) {
            let step = pair[0].distance(pair[1]);
            let n = (pair[1] - pair[0]).cross(Vec3::Z).normalize_or(Vec3::X);
            let up = Vec3::Z * height;
            let i = self.vertex(pair[0], n, color, [along, 0.0, 0.0, 0.0]);
            self.vertex(pair[1], n, color, [along + step, 0.0, 0.0, 0.0]);
            self.vertex(pair[1] + up, n, color, [along + step, 1.0, 0.0, 0.0]);
            self.vertex(pair[0] + up, n, color, [along, 1.0, 0.0, 0.0]);
            self.indices.extend([i, i + 1, i + 2, i, i + 2, i + 3]);
            along += step;
        }
    }

    /// A fill of the closed polygon `ring` fading in from its edge, laid on
    /// the ground when `ground` is given.
    fn decal(&mut self, ring: &[Vec3], color: [u8; 4], ground: Ground) {
        if ring.len() < 3 {
            return;
        }
        let mut centre = ring.iter().copied().sum::<Vec3>() / ring.len() as f32;
        if let Some(g) = ground {
            let h = ring.iter().map(|p| p.z - g(p.x, p.y)).sum::<f32>() / ring.len() as f32;
            centre.z = g(centre.x, centre.y) + h;
        }
        let mark = self.mark();
        let inner = (0..ring.len())
            .map(|i| {
                let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                let ab = b - a;
                let t = ((centre - a).dot(ab) / ab.length_squared().max(1e-9)).clamp(0.0, 1.0);
                centre.distance(a + ab * t)
            })
            .fold(f32::MAX, f32::min);
        for i in 0..ring.len() {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            let c = self.vertex(centre, Vec3::Z, color, [inner, 0.0, 0.0, 0.0]);
            self.vertex(a, Vec3::Z, color, [0.0; 4]);
            self.vertex(b, Vec3::Z, color, [0.0; 4]);
            self.indices.extend([c, c + 1, c + 2]);
        }
        if ground.is_some() {
            self.hug(mark, ground, None, HUG_STEP);
        }
    }

    /// Polygons with holes filled as a decal (or in the current look):
    /// triangulated, split fine enough to fade in from the edges and to lie
    /// on the ground, each vertex carrying its distance from the nearest edge.
    fn polygon(&mut self, rings: &[Vec<Vec3>], color: [u8; 4], ground: Ground, decal: bool) {
        let flat: Vec<Vec<[f32; 2]>> = rings.iter().map(|r| r.iter().map(|p| [p.x, p.y]).collect()).collect();
        let refs: Vec<&[[f32; 2]]> = flat.iter().map(|r| r.as_slice()).collect();
        let tris = triangulate(&refs);
        if tris.is_empty() {
            return;
        }
        let all: Vec<Vec3> = rings.iter().flatten().copied().collect();
        let area = gearbox_api::wire::draw_geom::ring_area(&flat[0]).abs();
        let step = if ground.is_some() { HUG_STEP } else { (area.sqrt() / 60.0).clamp(0.6, 4.0) };
        let step = step.max((area / 150_000.0).sqrt() * 1.5);
        let mark = self.mark();
        let base = self.positions.len() as u32;
        for p in &all {
            self.vertex(*p, Vec3::Z, color, [0.0; 4]);
        }
        for t in &tris {
            self.indices.extend([base + t[0], base + t[1], base + t[2]]);
        }
        self.hug(mark, ground, None, step);
        if decal {
            let edges = EdgeIndex::new(&flat, DECAL_FADE);
            for k in mark.0..self.positions.len() {
                let p = self.positions[k];
                self.local[k] = [edges.distance([p[0], p[1]]), p[0], p[1], 0.0];
            }
        } else {
            for k in mark.0..self.positions.len() {
                let p = self.positions[k];
                self.local[k] = [p[0], p[1], 0.0, 0.0];
            }
        }
    }

    fn triangle(&mut self, a: Vec3, b: Vec3, c: Vec3, colors: [[u8; 4]; 3]) {
        let n = (b - a).cross(c - a).normalize_or(Vec3::Z);
        let i = self.vertex(a, n, colors[0], [0.0; 4]);
        self.vertex(b, n, colors[1], [0.0; 4]);
        self.vertex(c, n, colors[2], [0.0; 4]);
        self.indices.extend([i, i + 1, i + 2]);
    }

    /// Where the next geometry starts: vertex and index counts.
    fn mark(&self) -> (usize, usize) {
        (self.positions.len(), self.indices.len())
    }

    /// Lays what was added since `from` on the ground: triangles split
    /// until no edge is longer than `step`, then every vertex set at its
    /// height above the terrain under it. Heights are taken where the
    /// geometry was raised from: `anchor`, or each vertex's own place.
    fn hug(&mut self, from: (usize, usize), ground: Ground, anchor: Option<Vec2>, step: f32) {
        let (v, i) = (self.positions.len(), self.indices.len());
        self.hug_span(from.0..v, from.1..i, ground, anchor, step);
    }

    /// Notes that what was added since `from` lies on the ground about
    /// `anchor`, for [`Self::lay`].
    fn on_ground(&mut self, from: (usize, usize), anchor: Vec2) {
        self.grounded.push((from.0..self.positions.len(), from.1..self.indices.len(), anchor));
    }

    /// Lays everything noted with [`Self::on_ground`] on the terrain.
    pub fn lay(&mut self, ground: Ground) {
        let spans = std::mem::take(&mut self.grounded);
        if ground.is_none() {
            return;
        }
        for (verts, tris, anchor) in spans.into_iter().rev() {
            self.hug_span(verts, tris, ground, Some(anchor), HUG_STEP);
        }
    }

    /// [`Self::hug`] over the vertices `verts` and the triangles in `tris`.
    fn hug_span(&mut self, verts: std::ops::Range<usize>, tris: std::ops::Range<usize>, ground: Ground, anchor: Option<Vec2>, step: f32) {
        let rise = |x: f32, y: f32| ground.map_or(0.0, |g| g(x, y));
        let anchor_rise = anchor.map(|a| rise(a.x, a.y));
        let mut heights: HashMap<u32, f32> = verts
            .clone()
            .map(|k| {
                let p = self.positions[k];
                (k as u32, p[2] - anchor_rise.unwrap_or_else(|| rise(p[0], p[1])))
            })
            .collect();
        let mut work: Vec<[u32; 3]> = self.indices[tris.clone()].chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect();
        self.indices.drain(tris);
        let mut mids: HashMap<(u32, u32), u32> = HashMap::new();
        let step2 = step * step;
        let budget = 6_000_000usize;
        while let Some(t) = work.pop() {
            let xy = |i: u32| Vec2::new(self.positions[i as usize][0], self.positions[i as usize][1]);
            let lens = [xy(t[0]).distance_squared(xy(t[1])), xy(t[1]).distance_squared(xy(t[2])), xy(t[2]).distance_squared(xy(t[0]))];
            let (k, longest) = lens.iter().copied().enumerate().fold((0, 0.0f32), |best, (k, l)| if l > best.1 { (k, l) } else { best });
            if !(longest > step2) || self.indices.len() + 3 * work.len() > budget {
                self.indices.extend(t);
                continue;
            }
            let (a, b, c) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
            let key = (a.min(b), a.max(b));
            let m = match mids.get(&key) {
                Some(m) => *m,
                None => {
                    let h = 0.5 * (heights.get(&a).copied().unwrap_or(0.0) + heights.get(&b).copied().unwrap_or(0.0));
                    let m = self.midpoint(a, b);
                    heights.insert(m, h);
                    mids.insert(key, m);
                    m
                }
            };
            work.push([a, m, c]);
            work.push([m, b, c]);
        }
        if ground.is_some() {
            for (k, h) in heights {
                let p = &mut self.positions[k as usize];
                p[2] = rise(p[0], p[1]) + h;
            }
        }
    }

    /// A vertex halfway between `a` and `b`, its attributes averaged.
    fn midpoint(&mut self, a: u32, b: u32) -> u32 {
        let (a, b) = (a as usize, b as usize);
        let mix3 = |u: [f32; 3], v: [f32; 3]| [0.5 * (u[0] + v[0]), 0.5 * (u[1] + v[1]), 0.5 * (u[2] + v[2])];
        let mix4 = |u: [f32; 4], v: [f32; 4]| [0.5 * (u[0] + v[0]), 0.5 * (u[1] + v[1]), 0.5 * (u[2] + v[2]), 0.5 * (u[3] + v[3])];
        let (ca, cb) = (self.colors[a], self.colors[b]);
        let color = [0, 1, 2, 3].map(|k| ((ca[k] as u16 + cb[k] as u16) / 2) as u8);
        self.positions.push(mix3(self.positions[a], self.positions[b]));
        self.normals.push(Vec3::from_array(mix3(self.normals[a], self.normals[b])).normalize_or(Vec3::Z).into());
        self.colors.push(color);
        self.local.push(mix4(self.local[a], self.local[b]));
        self.fx.push(self.fx[a]);
        self.fx2.push(self.fx2[a]);
        self.positions.len() as u32 - 1
    }

    pub fn into_mesh(self) -> Mesh {
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
            .with_inserted_attribute(ATTR_FILL_COLOR, VertexAttributeValues::Unorm8x4(self.colors))
            .with_inserted_attribute(ATTR_LOCAL, self.local)
            .with_inserted_attribute(ATTR_SURFACE_FX, self.fx)
            .with_inserted_attribute(ATTR_SURFACE_FX2, self.fx2)
            .with_inserted_indices(Indices::U32(self.indices))
    }
}

/// Text shown at a point of the entity.
#[derive(Debug, Clone)]
pub struct LabelSpec {
    pub at: Vec3,
    pub text: String,
    pub color: [u8; 4],
    pub background: [u8; 4],
    /// Colour of the bar on the plate's left edge.
    pub accent: [u8; 4],
    /// > 0 metres tall, < 0 pixels.
    pub size: f32,
    /// Centred on the point rather than beside it on a leader line.
    pub centred: bool,
}

/// Everything one entity draws.
#[derive(Default)]
pub struct Built {
    pub strokes: Strokes,
    pub fill: Surfaces,
    pub labels: Vec<LabelSpec>,
    pub shaded: bool,
    pub double_sided: bool,
    /// Drawn over everything.
    pub on_top: bool,
    /// Strokes are opaque splats.
    pub splat: bool,
}

/// Share of every drawing's opacity kept: `GEARBOX_DRAW_OPACITY`, 0..1,
/// default 1.
fn opacity() -> f32 {
    static OPACITY: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *OPACITY.get_or_init(|| {
        std::env::var("GEARBOX_DRAW_OPACITY")
            .ok()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|v| v.is_finite())
            .map_or(1.0, |v| v.clamp(0.0, 1.0))
    })
}

/// `c` with its alpha scaled by [`opacity`].
fn faded(mut c: [u8; 4]) -> [u8; 4] {
    c[3] = (c[3] as f32 * opacity()).round() as u8;
    c
}

/// Reads an entity's per-item values with the entity defaults behind them.
struct Look<'a> {
    e: &'a DrawEntity,
    base: [u8; 4],
    radius: f32,
    /// A value per colour unit (per point for lines), when a colormap reads them.
    values: Vec<f32>,
    range: (f32, f32),
}

impl<'a> Look<'a> {
    fn new(e: &'a DrawEntity, base: [u8; 4], radius: f32) -> Self {
        let mapped = e.colormap != colormap::NONE && !e.values.is_empty();
        let values = if !mapped {
            Vec::new()
        } else if e.kind == draw_kind::LINES && e.values.len() != e.point_count() && e.values.len() > 1 {
            let mut per_point = vec![0.0; e.point_count()];
            for (s, r) in e.strip_ranges().into_iter().enumerate() {
                let v = e.values.get(s).copied().unwrap_or(0.0);
                per_point[r].fill(v);
            }
            per_point
        } else {
            e.values.clone()
        };
        let range = value_range(values.iter().copied(), e.vmin, e.vmax);
        Self { e, base, radius, values, range }
    }

    fn color(&self, i: usize) -> [u8; 4] {
        match self.e.colors.get(i) {
            Some(c) if *c != 0 => faded(unpack_rgba(*c)),
            _ if !self.values.is_empty() => {
                let v = if self.values.len() == 1 { self.values[0] } else { self.values.get(i).copied().unwrap_or(0.0) };
                let mut c = colormap_rgba(self.e.colormap, v, self.range.0, self.range.1);
                c[3] = ((c[3] as u16 * self.base[3] as u16) / 255) as u8;
                c
            }
            _ => self.base,
        }
    }

    fn radius(&self, i: usize) -> f32 {
        match self.e.radii.get(i) {
            Some(r) if *r != 0.0 && r.is_finite() => *r,
            _ => self.radius,
        }
    }
}

fn default_radius(kind: u32, style: u32) -> f32 {
    match (style, kind) {
        (draw_style::RIBBON_FLOW, _) => 0.175,
        (draw_style::TUBE_PULSE, _) => 0.04,
        (draw_style::HAIRLINE, _) => -0.75,
        (draw_style::LIGHT_WALL, _) => -1.0,
        (draw_style::DECAL, _) => 0.025,
        (draw_style::ALERT, draw_kind::LINES) => 0.15,
        (draw_style::ALERT, draw_kind::POLYGON) => 0.08,
        (draw_style::ORB, _) | (draw_style::BEACON, draw_kind::POINTS) => 0.25,
        (draw_style::ARROW_GLOW, _) => -2.0,
        (draw_style::RULER, _) => -1.6,
        (draw_style::SPLAT, _) => -2.0,
        (draw_style::SWEEP, _) => -1.25,
        (_, draw_kind::POINTS) => -4.5,
        (_, draw_kind::ARROWS) => -1.6,
        (_, draw_kind::SHAPES) => -1.0,
        _ => -1.25,
    }
}

fn inv_or_one(v: f32) -> f32 {
    if v.abs() > 1e-6 { 1.0 / v } else { 1.0 }
}

fn pose_of(i: &Instance) -> (Quat, Vec3) {
    let [w, x, y, z] = i.rotation;
    let q = Quat::from_xyzw(x, y, z, w);
    let q = if q.length_squared() > 1e-8 { q.normalize() } else { Quat::IDENTITY };
    (q, Vec3::from_array(i.position))
}

/// Metres of a radius, or `fallback` for one in pixels.
fn metres(radius: f32, fallback: f32) -> f32 {
    if radius > 0.0 { radius } else { fallback }
}

/// A circle of `radius` in the XY plane around `centre`.
fn circle(centre: Vec3, rx: f32, ry: f32, segments: usize) -> Vec<Vec3> {
    (0..segments)
        .map(|k| {
            let theta = k as f32 / segments as f32 * TAU;
            centre + Vec3::new(rx * theta.cos(), ry * theta.sin(), 0.0)
        })
        .collect()
}

/// Ring edges bucketed on a grid, for distances out to `reach`.
struct EdgeIndex {
    cell: f32,
    reach: f32,
    buckets: HashMap<(i32, i32), Vec<[f32; 4]>>,
}

impl EdgeIndex {
    fn new(rings: &[Vec<[f32; 2]>], reach: f32) -> Self {
        let cell = (2.0 * reach).max(0.2);
        let mut buckets: HashMap<(i32, i32), Vec<[f32; 4]>> = HashMap::new();
        for ring in rings {
            for i in 0..ring.len() {
                let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                let lo = [a[0].min(b[0]) - reach, a[1].min(b[1]) - reach];
                let hi = [a[0].max(b[0]) + reach, a[1].max(b[1]) + reach];
                let (x0, y0) = ((lo[0] / cell).floor() as i32, (lo[1] / cell).floor() as i32);
                let (x1, y1) = ((hi[0] / cell).floor() as i32, (hi[1] / cell).floor() as i32);
                if (x1 - x0) as i64 * (y1 - y0) as i64 > 1_000_000 {
                    continue;
                }
                for x in x0..=x1 {
                    for y in y0..=y1 {
                        buckets.entry((x, y)).or_default().push([a[0], a[1], b[0], b[1]]);
                    }
                }
            }
        }
        Self { cell, reach, buckets }
    }

    /// Distance from `p` to the nearest edge, at most `reach`.
    fn distance(&self, p: [f32; 2]) -> f32 {
        let key = ((p[0] / self.cell).floor() as i32, (p[1] / self.cell).floor() as i32);
        let Some(edges) = self.buckets.get(&key) else {
            return self.reach;
        };
        let p = Vec2::from_array(p);
        edges
            .iter()
            .map(|e| {
                let (a, b) = (Vec2::new(e[0], e[1]), Vec2::new(e[2], e[3]));
                let ab = b - a;
                let t = ((p - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
                p.distance(a + ab * t)
            })
            .fold(self.reach, f32::min)
    }
}

/// A closed `ring` raised from `anchor` laid on the ground instead: split
/// into pieces no longer than [`HUG_STEP`], each point at its height above
/// the terrain under it.
fn drape_ring(ring: &[Vec3], ground: Ground, anchor: Vec2) -> Vec<Vec3> {
    let Some(g) = ground else {
        return ring.to_vec();
    };
    let base = g(anchor.x, anchor.y);
    let mut out = Vec::with_capacity(ring.len() * 2);
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
        let n = ((a.truncate().distance(b.truncate()) / HUG_STEP).ceil() as usize).max(1);
        for k in 0..n {
            let p = a.lerp(b, k as f32 / n as f32);
            out.push(Vec3::new(p.x, p.y, g(p.x, p.y) + p.z - base));
        }
    }
    out
}

/// A round number of metres near `rough`: 1, 2 or 5 times a power of ten.
fn nice_step(rough: f32) -> f32 {
    if !(rough > 0.0) {
        return 1.0;
    }
    let p = 10f32.powf(rough.log10().floor());
    let f = rough / p;
    p * if f < 1.5 { 1.0 } else if f < 3.5 { 2.0 } else if f < 7.5 { 5.0 } else { 10.0 }
}

/// A length for people: centimetres under a metre, kilometres past one.
fn length_text(m: f32) -> String {
    if m < 1.0 {
        format!("{:.1} cm", m * 100.0)
    } else if m < 1000.0 {
        format!("{m:.2} m")
    } else {
        format!("{:.3} km", m / 1000.0)
    }
}

/// A ruler's ticks every `spacing` along `strip`, longer every fifth, its
/// end caps and its length written at the middle.
fn ruler_marks(out: &mut Built, fx: &Fx, strip: &[Vec3], color: [u8; 4], radius: f32) {
    let lengths: Vec<f32> = strip.windows(2).map(|w| w[0].distance(w[1])).collect();
    let total: f32 = lengths.iter().sum();
    if total < 1e-6 {
        return;
    }
    let spacing = if fx.spacing > 0.0 { fx.spacing } else { nice_step(total / 10.0) };
    let count = (total / spacing).floor() as usize;
    let at = |d: f32| -> (Vec3, Vec3) {
        let mut left = d;
        for (k, l) in lengths.iter().enumerate() {
            if left <= *l || k + 1 == lengths.len() {
                let dir = (strip[k + 1] - strip[k]).normalize_or(Vec3::X);
                return (strip[k] + dir * left.min(*l), dir);
            }
            left -= l;
        }
        (strip[0], Vec3::X)
    };
    let across = |dir: Vec3| -> Vec3 {
        let side = dir.cross(Vec3::Z);
        if side.length_squared() > 1e-6 { side.normalize() } else { dir.cross(Vec3::X).normalize_or(Vec3::Y) }
    };
    let tick_r = if radius < 0.0 { radius * 0.8 } else { radius * 0.7 };
    let half = (0.12 * spacing).clamp(0.01, 0.4);
    out.strokes.pen.style = draw_style::RULER;
    for k in 1..=count {
        let d = k as f32 * spacing;
        if d >= total - 1e-4 {
            break;
        }
        let (p, dir) = at(d);
        let side = across(dir);
        let len = if k % 5 == 0 { 2.0 * half } else { half };
        out.strokes.polyline(&[p - side * len, p + side * len], &[color; 2], &[tick_r; 2], false, FLAG_TUBE, true);
    }
    for (p, dir) in [at(0.0), at(total)] {
        let side = across(dir);
        let cap = 3.0 * half;
        out.strokes.polyline(&[p - side * cap, p + side * cap], &[color; 2], &[radius; 2], false, FLAG_TUBE, true);
        out.strokes.point(p, color, if radius < 0.0 { radius * 2.2 } else { radius * 2.2 }, FLAG_SPHERE);
    }
    let (mid, _) = at(0.5 * total);
    out.labels.push(LabelSpec {
        at: mid,
        text: length_text(total),
        color: [235, 238, 242, 255],
        background: LABEL_BACKGROUND,
        accent: [color[0], color[1], color[2], 255],
        size: -13.0,
        centred: false,
    });
}

/// A flat wedge about +x of `inst`: size is its radius, opening angle and
/// inner radius. Sweep style fills it with the sensor look; others draw its
/// outline (and a decal or plain fill).
#[allow(clippy::too_many_arguments)]
fn sector(out: &mut Built, fx: &Fx, style: u32, inst: &Instance, size: Vec3, color: [u8; 4], phase: f32, ground: Ground, radius: f32, fill: u32) {
    let (rot, pos) = pose_of(inst);
    let outer = size.x.abs().max(1e-3);
    let angle = size.y.abs().clamp(1e-3, TAU);
    let inner = size.z.abs().min(0.999 * outer);
    let half = 0.5 * angle;
    let segments = ((angle / TAU) * 96.0).ceil().max(6.0) as usize;
    let lift = Vec3::Z * GROUND_LIFT;
    let at = |r: f32, theta: f32| pos + rot * (Vec3::new(r * theta.cos(), r * theta.sin(), 0.0) + lift);
    if style == draw_style::SWEEP {
        let steps = 10usize;
        out.fill.look = [LOOK_SWEEP, fx.glow, fx.pulse_hz, phase];
        out.fill.look2 = [fx.urgency, outer, fx.spacing, fx.fade];
        let mark = out.fill.mark();
        let normal = rot * Vec3::Z;
        let base = out.fill.positions.len() as u32;
        for s in 0..=steps {
            let r = inner + (outer - inner) * s as f32 / steps as f32;
            for k in 0..=segments {
                let theta = -half + angle * k as f32 / segments as f32;
                out.fill.vertex(at(r, theta), normal, color, [r / outer, theta / half, half, 1.0]);
            }
        }
        let row = segments as u32 + 1;
        for s in 0..steps as u32 {
            for k in 0..segments as u32 {
                let (a, b) = (base + s * row + k, base + (s + 1) * row + k);
                out.fill.indices.extend([a, b, a + 1, a + 1, b, b + 1]);
            }
        }
        out.fill.on_ground(mark, pos.truncate());
        return;
    }
    let mut ring: Vec<Vec3> = (0..=segments).map(|k| at(outer, -half + angle * k as f32 / segments as f32)).collect();
    if inner > 1e-4 {
        ring.extend((0..=segments).rev().map(|k| at(inner, -half + angle * k as f32 / segments as f32)));
    } else if angle < TAU - 1e-3 {
        ring.push(pos + rot * lift);
    }
    let ring = drape_ring(&ring, ground, pos.truncate());
    out.strokes.phase = phase;
    let flags = if style == draw_style::DECAL { FLAG_FLAT | FLAG_ADD } else { FLAG_FLAT };
    if fill != draw_flag::FILL_SOLID || style == draw_style::DECAL {
        out.strokes.polyline(&ring, &vec![color; ring.len()], &vec![radius; ring.len()], true, flags, false);
    }
    if style == draw_style::DECAL {
        out.fill.look = [LOOK_DECAL, fx.glow, 0.0, phase];
        out.fill.decal(&ring, color, ground);
    } else if fill != draw_flag::FILL_WIRE {
        out.fill.look = [LOOK_PLAIN, fx.glow, fx.pulse_hz, phase];
        out.fill.decal(&ring, color, ground);
    }
}

/// A view frustum from `inst` along +x: size is the horizontal and vertical
/// field of view and the range. Faint faces with a scan running out, glowing
/// edges, and a faint image plane `near` metres out.
#[allow(clippy::too_many_arguments)]
fn frustum(out: &mut Built, fx: &Fx, inst: &Instance, size: Vec3, near: f32, color: [u8; 4], phase: f32, radius: f32) {
    let (rot, pos) = pose_of(inst);
    let hfov = size.x.abs().clamp(0.01, 3.1);
    let vfov = size.y.abs().clamp(0.01, 3.1);
    let range = size.z.abs().max(0.01);
    let (ty, tz) = ((0.5 * hfov).tan(), (0.5 * vfov).tan());
    let corner = |d: f32, sy: f32, sz: f32| pos + rot * Vec3::new(d, sy * d * ty, sz * d * tz);
    let signs = [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)];
    let far: Vec<Vec3> = signs.iter().map(|(y, z)| corner(range, *y, *z)).collect();
    out.fill.look = [LOOK_SWEEP, fx.glow * 0.8, fx.pulse_hz, phase];
    out.fill.look2 = [fx.urgency, range, fx.spacing, fx.fade];
    for k in 0..4 {
        let (a, b) = (far[k], far[(k + 1) % 4]);
        let n = (a - pos).cross(b - pos).normalize_or(Vec3::Z);
        let i = out.fill.vertex(pos, n, color, [0.0, 0.0, 10.0, 0.0]);
        out.fill.vertex(a, n, color, [1.0, 0.0, 10.0, 0.0]);
        out.fill.vertex(b, n, color, [1.0, 0.0, 10.0, 0.0]);
        out.fill.indices.extend([i, i + 1, i + 2]);
    }
    let near = if near > 0.0 { near.min(range) } else { (0.15 * range).min(1.0) };
    let plane: Vec<Vec3> = signs.iter().map(|(y, z)| corner(near, *y, *z)).collect();
    out.fill.look = [LOOK_PLANE, fx.glow, 0.0, phase];
    let normal = rot * Vec3::X;
    let uv = [[1.0, 0.0], [0.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
    let i = out.fill.positions.len() as u32;
    for (p, t) in plane.iter().zip(uv) {
        out.fill.vertex(*p, normal, color, [t[0], t[1], 0.0, 0.0]);
    }
    out.fill.indices.extend([i, i + 1, i + 2, i, i + 2, i + 3]);
    let saved = out.strokes.pen;
    out.strokes.pen.style = draw_style::ARROW_GLOW;
    out.strokes.phase = phase;
    for c in &far {
        out.strokes.polyline(&[pos, *c], &[color; 2], &[radius; 2], false, FLAG_ADD, false);
    }
    out.strokes.polyline(&far, &[color; 4], &[radius; 4], true, FLAG_ADD, false);
    out.strokes.pen.style = draw_style::PLAIN;
    out.strokes.polyline(&plane, &[color; 4], &[radius * 0.8; 4], true, FLAG_ADD, false);
    out.strokes.pen = saved;
}

/// A goal's ground ring of `radius` at `base` and its beam of light.
fn beacon(out: &mut Built, fx: &Fx, base: Vec3, radius: f32, color: [u8; 4], phase: f32) {
    let fill = &mut out.fill;
    fill.look = [LOOK_RING, fx.glow * 0.4, fx.pulse_hz, phase];
    fill.ground_disc(base, radius, color);
    fill.look = [LOOK_BEAM, fx.glow, fx.pulse_hz, phase];
    fill.look2 = [fx.urgency, 0.25, fx.beam, 1.0];
    fill.column(base, 0.25, fx.beam, color);
    fill.look = [LOOK_BEAM, fx.glow * 0.3, fx.pulse_hz, phase + 0.5];
    fill.column(base, 0.6, fx.beam * 0.8, color);
}

/// A short beam of light rising from `base`.
fn short_beam(out: &mut Built, fx: &Fx, base: Vec3, radius: f32, color: [u8; 4], phase: f32) {
    out.fill.look = [LOOK_BEAM, fx.glow * 0.8, fx.pulse_hz.max(0.3), phase];
    out.fill.look2 = [fx.urgency, radius, fx.beam, 1.0];
    out.fill.column(base, radius, fx.beam, color);
}

/// A shield ring where an obstacle meets the ground.
fn footprint(out: &mut Built, fx: &Fx, base: Vec3, radius: f32, color: [u8; 4], phase: f32) {
    out.fill.look = [LOOK_FOOTPRINT, fx.glow, fx.pulse_hz, phase];
    out.fill.look2 = [fx.urgency, radius, 0.0, 1.0];
    out.fill.ground_disc(base, radius, color);
}

/// [`build_on`] off the ground.
#[cfg(test)]
pub fn build(e: &DrawEntity, ns: &str, id: &str) -> Built {
    build_on(e, ns, id, None)
}

/// Everything entity `e` draws; ground looks lie on `ground` when given.
pub fn build_on(e: &DrawEntity, ns: &str, id: &str, ground: Ground) -> Built {
    let style = resolve_style(e, ns, id);
    let fx = Fx::of(e, style);
    let base = faded(if e.color != 0 {
        unpack_rgba(e.color)
    } else {
        role_color(style).unwrap_or_else(|| unpack_rgba(default_color(ns, id)))
    });
    let radius = if e.width != 0.0 && e.width.is_finite() { 0.5 * e.width } else { default_radius(e.kind, style) };
    let look = Look::new(e, base, radius);
    let phase0 = path_phase(ns, id);
    let mut out = Built { on_top: e.flags & draw_flag::ON_TOP != 0 || style == draw_style::ALERT, ..default() };
    out.strokes.pen = fx;
    let flags = e.flags;
    let shaded = flags & draw_flag::SHADED != 0;
    let additive = flags & draw_flag::ADDITIVE != 0;
    let mut stroke_flags = if shaded { FLAG_TUBE } else { 0 }
        | if flags & draw_flag::DASHED != 0 { FLAG_DASHED } else { 0 }
        | if flags & draw_flag::FLAT != 0 { FLAG_FLAT } else { 0 }
        | if additive { FLAG_ADD } else { 0 };
    let points: Vec<Vec3> = e.point_list().into_iter().map(Vec3::from_array).collect();
    out.strokes.reserve(match e.kind {
        draw_kind::LINES | draw_kind::POINTS | draw_kind::ARROWS => points.len() + e.strip_ranges().len(),
        draw_kind::AXES => 6 * e.instance_count().max(1),
        _ => 0,
    });
    let surface_look = match style {
        draw_style::HOLO => LOOK_HOLO,
        draw_style::SHIELD => LOOK_SHIELD,
        draw_style::ALERT => LOOK_ALERT,
        draw_style::GHOST => LOOK_GHOST,
        _ => LOOK_PLAIN,
    };
    out.fill.look = [surface_look, fx.glow, fx.pulse_hz, phase0];
    out.fill.look2 = [fx.urgency, 1.0, fx.spacing, fx.fade];
    let labels = e.label_list();
    let show_labels = flags & draw_flag::LABELS != 0 && !labels.is_empty();
    let label = |out: &mut Built, i: usize, at: Vec3, color: [u8; 4]| {
        if show_labels && let Some(text) = labels.get(i).filter(|t| !t.is_empty()) {
            out.labels.push(LabelSpec {
                at,
                text: text.clone(),
                color: [235, 238, 242, 255],
                background: LABEL_BACKGROUND,
                accent: [color[0], color[1], color[2], 255],
                size: -13.0,
                centred: false,
            });
        }
    };
    match e.kind {
        draw_kind::LINES => {
            let flat_caps = flags & draw_flag::FLAT_CAPS != 0;
            let lift = match style {
                draw_style::RIBBON_FLOW | draw_style::ALERT => {
                    stroke_flags |= FLAG_FLAT;
                    Vec3::Z * GROUND_LIFT
                }
                draw_style::DECAL => {
                    stroke_flags |= FLAG_FLAT | FLAG_ADD;
                    Vec3::Z * GROUND_LIFT
                }
                draw_style::TUBE_PULSE => {
                    stroke_flags |= FLAG_TUBE;
                    Vec3::ZERO
                }
                draw_style::LIGHT_WALL => {
                    stroke_flags |= FLAG_ADD;
                    Vec3::ZERO
                }
                draw_style::RULER => {
                    stroke_flags |= FLAG_TUBE;
                    Vec3::Z * if e.flags & draw_flag::DRAPE != 0 { GROUND_LIFT } else { 0.0 }
                }
                _ => Vec3::ZERO,
            };
            let points: Vec<Vec3> = points.iter().map(|p| *p + lift).collect();
            let arrow_tip = flags & draw_flag::ARROW_TIP != 0 && e.sub != line_mode::LOOP;
            if e.sub == line_mode::LIST {
                for i in (0..points.len() / 2 * 2).step_by(2) {
                    let pair = [points[i], points[i + 1]];
                    out.strokes.phase = phase0 + item_phase(i);
                    out.strokes.polyline(&pair, &[look.color(i), look.color(i + 1)], &[look.radius(i), look.radius(i + 1)], false, stroke_flags, flat_caps);
                    if arrow_tip {
                        out.strokes.tip(&pair, look.color(i + 1), look.radius(i + 1), stroke_flags);
                    }
                }
            } else {
                for (s, r) in e.strip_ranges().into_iter().enumerate() {
                    let colors: Vec<[u8; 4]> = r.clone().map(|i| look.color(i)).collect();
                    let radii: Vec<f32> = r.clone().map(|i| look.radius(i)).collect();
                    let strip = &points[r.clone()];
                    let closed = e.sub == line_mode::LOOP || (style == draw_style::DECAL && strip.len() > 3 && strip[0].distance(strip[strip.len() - 1]) < 1e-4);
                    let ring: Vec<Vec3> = if closed && strip.len() > 3 && strip[0].distance(strip[strip.len() - 1]) < 1e-4 { strip[..strip.len() - 1].to_vec() } else { strip.to_vec() };
                    out.strokes.phase = phase0 + item_phase(s);
                    out.strokes.polyline(&ring, &colors, &radii, closed, stroke_flags, flat_caps);
                    if arrow_tip && !closed {
                        out.strokes.tip(strip, *colors.last().unwrap(), *radii.last().unwrap(), stroke_flags);
                    }
                    if style == draw_style::RULER {
                        ruler_marks(&mut out, &fx, strip, colors[0], radii[0]);
                        continue;
                    }
                    if style == draw_style::LIGHT_WALL {
                        let length: f32 = strip.windows(2).map(|w| w[0].distance(w[1])).sum();
                        out.fill.look = [LOOK_WALL, fx.glow, fx.pulse_hz, phase0];
                        out.fill.look2 = [fx.urgency, fx.beam, length, fx.fade];
                        out.fill.wall(strip, fx.beam, colors[0]);
                    }
                    if style == draw_style::DECAL && closed {
                        out.fill.look = [LOOK_DECAL, fx.glow, 0.0, phase0];
                        out.fill.decal(&ring, colors[0], ground);
                    }
                    let mid = r.start + (r.end - r.start) / 2;
                    label(&mut out, s, points[mid], look.color(mid));
                }
            }
        }
        draw_kind::POINTS => {
            let disc = e.sub == point_style::DISC || flags & draw_flag::FLAT != 0;
            for (i, p) in points.iter().enumerate() {
                let color = look.color(i);
                let r = look.radius(i);
                let phase = phase0 + item_phase(i);
                out.strokes.phase = phase;
                match style {
                    draw_style::SPLAT => {
                        out.splat = true;
                        let shape = if e.sub == POINT_SQUARE { FLAG_SQUARE } else { 0 };
                        out.strokes.point(*p, color, r, shape | stroke_flags & FLAG_FLAT);
                    }
                    draw_style::ORB => out.strokes.point(*p, color, r, 0),
                    draw_style::BEACON => {
                        out.strokes.pen.style = draw_style::ORB;
                        out.strokes.point(*p, color, metres(r, 0.25) * 0.6, 0);
                        beacon(&mut out, &fx, *p, metres(r, 0.25) * 4.0, color, phase);
                    }
                    draw_style::HOLO => {
                        out.strokes.point(*p, color, r, FLAG_SPHERE);
                        short_beam(&mut out, &fx, *p, metres(r, 0.1).max(0.08), color, phase);
                    }
                    draw_style::SHIELD => {
                        let reach = metres(r, 0.5);
                        out.fill.look[3] = phase;
                        out.fill.place(&UnitMesh::dome(), &Instance::at((*p).into(), [1.0; 3]), Vec3::new(2.0 * reach, 2.0 * reach, 2.0 * 0.6 * reach), color);
                        footprint(&mut out, &fx, *p, reach, color, phase);
                        out.fill.look = [LOOK_SHIELD, fx.glow, fx.pulse_hz, phase0];
                        out.fill.look2 = [fx.urgency, 1.0, fx.spacing, fx.fade];
                    }
                    draw_style::DECAL => {
                        let reach = metres(r, 0.5);
                        let ring = drape_ring(&circle(*p + Vec3::Z * GROUND_LIFT, reach, reach, 64), ground, p.truncate());
                        out.strokes.polyline(&ring, &vec![color; ring.len()], &vec![0.025; ring.len()], true, FLAG_FLAT | FLAG_ADD, false);
                        out.fill.look = [LOOK_DECAL, fx.glow, 0.0, phase];
                        out.fill.decal(&ring, color, ground);
                    }
                    draw_style::ALERT => {
                        let reach = metres(r, 0.5);
                        out.fill.look[3] = phase;
                        out.fill.look2 = [fx.urgency, 2.0 * reach, fx.spacing, 1.0];
                        out.fill.place(&UnitMesh::disc(64), &Instance::at((*p + Vec3::Z * GROUND_LIFT).into(), [1.0; 3]), Vec3::new(2.0 * reach, 2.0 * reach, 1.0), color);
                    }
                    _ => {
                        let shape = if disc { stroke_flags & (FLAG_FLAT | FLAG_ADD) } else { FLAG_SPHERE | stroke_flags & FLAG_ADD };
                        out.strokes.point(*p, color, r, shape);
                    }
                }
                label(&mut out, i, *p, color);
            }
        }
        draw_kind::ARROWS => {
            let mut arrows: Vec<(Vec3, Vec3, usize)> = Vec::new();
            if e.instances.is_empty() {
                arrows.extend((0..points.len() / 2).map(|i| (points[2 * i], points[2 * i + 1], i)));
            } else {
                for (i, inst) in e.instance_list().iter().enumerate() {
                    let (rot, pos) = pose_of(inst);
                    let length = if inst.size[0] != 0.0 { inst.size[0] } else { e.sx };
                    arrows.push((pos, pos + rot * Vec3::X * length, i));
                }
            }
            if style == draw_style::BEACON {
                out.strokes.pen.style = draw_style::ARROW_GLOW;
                out.strokes.pen.glow = 1.0;
            }
            let chevron = e.sub == ARROW_CHEVRON;
            let insts = e.instance_list();
            for (tail, tip, i) in arrows {
                let color = look.color(i);
                let phase = phase0 + item_phase(i);
                out.strokes.phase = phase;
                if chevron {
                    let span = tip - tail;
                    let len = span.length().max(1e-4);
                    let dir = span / len;
                    let up = if let Some(inst) = insts.get(i) { pose_of(inst).0 * Vec3::Z } else { Vec3::Z };
                    let side = up.cross(dir).normalize_or(Vec3::Y);
                    let r = if e.width != 0.0 || !e.radii.is_empty() { look.radius(i) } else { 0.06 * len };
                    out.strokes.chevron(tail, dir, side, len, color, r, stroke_flags);
                } else {
                    out.strokes.arrow(tail, tip, color, look.radius(i), stroke_flags);
                }
                if style == draw_style::BEACON {
                    beacon(&mut out, &fx, tail, (0.6 * tail.distance(tip)).clamp(0.6, 3.0), color, phase);
                }
                label(&mut out, i, tip, color);
            }
        }
        draw_kind::AXES => {
            if e.props().get("tf").is_some() {
                return out;
            }
            let instances = if e.instances.is_empty() { vec![Instance::default()] } else { e.instance_list() };
            let glowing = style == draw_style::ARROW_GLOW;
            out.strokes.pen.style = if glowing { draw_style::ARROW_GLOW } else { draw_style::PLAIN };
            for (i, inst) in instances.iter().enumerate() {
                let (rot, pos) = pose_of(inst);
                let length = if inst.size[0] != 0.0 { inst.size[0] } else if e.sx != 0.0 { e.sx } else { 1.0 };
                let r = match e.radii.get(i) {
                    Some(r) if *r != 0.0 => *r,
                    _ if e.width != 0.0 => 0.5 * e.width,
                    _ => 0.025 * length,
                };
                out.strokes.phase = phase0 + item_phase(i);
                let alpha = match e.colors.get(i) {
                    Some(c) if *c != 0 => unpack_rgba(*c)[3],
                    _ => 255,
                };
                for (axis, color) in [Vec3::X, Vec3::Y, Vec3::Z].into_iter().zip(AXIS_COLORS) {
                    let color = [color[0], color[1], color[2], alpha];
                    out.strokes.arrow(pos, pos + rot * axis * length, color, r, FLAG_TUBE);
                }
                if glowing {
                    out.strokes.point(pos, [255, 255, 255, alpha], r * 1.6, FLAG_SPHERE);
                }
                label(&mut out, i, pos, [255, 255, 255, 255]);
            }
        }
        draw_kind::SHAPES => {
            let instances = if e.instances.is_empty() { vec![Instance::default()] } else { e.instance_list() };
            let fill = flags & draw_flag::FILL_MASK;
            let unit = UnitMesh::of(e.sub);
            let flat_shape = matches!(e.sub, shape_kind::ELLIPSE | shape_kind::RECT | shape_kind::SECTOR | shape_kind::FRUSTUM);
            out.shaded = shaded || (surface_look == LOOK_PLAIN && !flat_shape);
            out.double_sided = flat_shape || surface_look != LOOK_PLAIN;
            for (i, inst) in instances.iter().enumerate() {
                let size = Vec3::new(
                    if inst.size[0] != 0.0 { inst.size[0] } else { e.sx },
                    if inst.size[1] != 0.0 { inst.size[1] } else { e.sy },
                    if inst.size[2] != 0.0 { inst.size[2] } else { e.sz },
                );
                let color = look.color(i);
                let phase = phase0 + item_phase(i);
                let (rot, pos) = pose_of(inst);
                if e.sub == shape_kind::SECTOR {
                    sector(&mut out, &fx, style, inst, size, color, phase, ground, look.radius(i), fill);
                    label(&mut out, i, pos, color);
                    continue;
                }
                if e.sub == shape_kind::FRUSTUM {
                    let near = e.prop_f32("near").unwrap_or(0.0);
                    frustum(&mut out, &fx, inst, size, near, color, phase, look.radius(i));
                    label(&mut out, i, pos, color);
                    continue;
                }
                let reach = 0.5 * size.x.abs().max(size.y.abs());
                let flat = matches!(e.sub, shape_kind::ELLIPSE | shape_kind::RECT);
                let floor = if flat { pos } else { pos - rot * Vec3::Z * (0.5 * size.z) };
                out.fill.look = [surface_look, fx.glow, fx.pulse_hz, phase];
                out.fill.look2 = [fx.urgency, (size.x.abs() + size.y.abs() + size.z.abs()) / 3.0, fx.spacing, fx.fade];
                let solid = fill != draw_flag::FILL_WIRE || surface_look != LOOK_PLAIN;
                match style {
                    draw_style::SHIELD if e.sub == shape_kind::ELLIPSE => {
                        let height = if inst.size[2] > 0.0 { inst.size[2] } else { 0.6 * reach };
                        out.fill.place(&UnitMesh::dome(), inst, Vec3::new(size.x, size.y, 2.0 * height), color);
                        footprint(&mut out, &fx, pos, reach, color, phase);
                    }
                    draw_style::DECAL => {
                        let lifted = floor + Vec3::Z * GROUND_LIFT;
                        let ring: Vec<Vec3> = match e.sub {
                            shape_kind::BOX | shape_kind::RECT => [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                                .iter()
                                .map(|(x, y)| lifted + rot * Vec3::new(0.5 * x * size.x, 0.5 * y * size.y, 0.0))
                                .collect(),
                            _ => circle(Vec3::ZERO, 0.5 * size.x, 0.5 * size.y, 64).into_iter().map(|p| lifted + rot * p).collect(),
                        };
                        let ring = drape_ring(&ring, ground, pos.truncate());
                        let r = look.radius(i);
                        out.strokes.phase = phase;
                        out.strokes.polyline(&ring, &vec![color; ring.len()], &vec![r; ring.len()], true, FLAG_FLAT | FLAG_ADD, false);
                        out.fill.look = [LOOK_DECAL, fx.glow, 0.0, phase];
                        out.fill.decal(&ring, color, ground);
                    }
                    _ if solid => match (&unit, e.sub) {
                        (_, shape_kind::CAPSULE) => out.fill.place(&UnitMesh::capsule(size), inst, Vec3::ONE, color),
                        (Some(unit), _) => out.fill.place(unit, inst, size, color),
                        _ => {}
                    },
                    _ => {}
                }
                match style {
                    draw_style::SHIELD if e.sub != shape_kind::ELLIPSE => footprint(&mut out, &fx, floor, reach, color, phase),
                    draw_style::HOLO => short_beam(&mut out, &fx, pos + rot * Vec3::Z * (0.5 * size.z), (0.15 * reach).clamp(0.05, 0.3), color, phase),
                    draw_style::BEACON => beacon(&mut out, &fx, floor, (1.2 * reach).max(1.0), color, phase),
                    _ => {}
                }
                if fill != draw_flag::FILL_SOLID && surface_look == LOOK_PLAIN && style != draw_style::DECAL {
                    let outline = if fill == draw_flag::FILL_BOTH {
                        if e.color2 != 0 { unpack_rgba(e.color2) } else { [color[0], color[1], color[2], 255] }
                    } else {
                        color
                    };
                    let r = look.radius(i);
                    out.strokes.phase = phase;
                    for (wire, closed) in outline_of(e.sub, size) {
                        let placed: Vec<Vec3> = wire.iter().map(|p| pos + rot * *p).collect();
                        let colors = vec![outline; placed.len()];
                        let radii = vec![r; placed.len()];
                        out.strokes.polyline(&placed, &colors, &radii, closed, stroke_flags & !FLAG_FLAT, false);
                    }
                }
                label(&mut out, i, pos, color);
            }
        }
        draw_kind::POLYGON => {
            let fill = flags & draw_flag::FILL_MASK;
            let lift = Vec3::Z * (GROUND_LIFT + e.prop_f32("z_offset").unwrap_or(0.0));
            out.double_sided = true;
            let (fill_look, line_flags) = match style {
                draw_style::ALERT => (LOOK_ALERT, FLAG_FLAT),
                draw_style::PLAIN => (LOOK_PLAIN, FLAG_FLAT),
                _ => (LOOK_ZONE, FLAG_FLAT | FLAG_ADD),
            };
            for (k, rings) in e.polygon_rings().into_iter().enumerate() {
                let color = look.color(k);
                let rings: Vec<Vec<Vec3>> = rings.iter().map(|r| points[r.clone()].iter().map(|p| *p + lift).collect()).collect();
                if rings.first().is_none_or(|r| r.len() < 3) {
                    continue;
                }
                let phase = phase0 + item_phase(k);
                if fill != draw_flag::FILL_WIRE {
                    out.fill.look = [fill_look, fx.glow, fx.pulse_hz, phase];
                    out.fill.look2 = [fx.urgency, 1.0, fx.spacing, fx.fade];
                    out.fill.polygon(&rings, color, ground, fill_look == LOOK_ZONE);
                }
                if fill != draw_flag::FILL_SOLID {
                    let outline = if fill == draw_flag::FILL_BOTH && e.color2 != 0 { unpack_rgba(e.color2) } else { color };
                    let r = look.radius(k);
                    out.strokes.phase = phase;
                    for ring in &rings {
                        out.strokes.polyline(ring, &vec![outline; ring.len()], &vec![r; ring.len()], true, line_flags, false);
                    }
                }
                let centre = rings[0].iter().copied().sum::<Vec3>() / rings[0].len() as f32;
                label(&mut out, k, centre, color);
            }
        }
        draw_kind::MESH => {
            let asset = e.props().get("asset").unwrap_or_default();
            if let Some(raw) = super::assets::load(&asset) {
                let unit = UnitMesh::from_raw(&raw);
                let instances = if e.instances.is_empty() { vec![Instance::default()] } else { e.instance_list() };
                out.shaded = surface_look == LOOK_PLAIN;
                out.double_sided = surface_look != LOOK_PLAIN;
                for (i, inst) in instances.iter().enumerate() {
                    let pick = |k: usize, d: f32| if inst.size[k] != 0.0 { inst.size[k] } else if d != 0.0 { d } else { 1.0 };
                    let size = Vec3::new(pick(0, e.sx), pick(1, e.sy), pick(2, e.sz));
                    out.fill.look = [surface_look, fx.glow, fx.pulse_hz, phase0 + item_phase(i)];
                    out.fill.look2 = [fx.urgency, 1.0, fx.spacing, fx.fade];
                    let color = if e.color == 0 && e.colors.is_empty() && style == draw_style::PLAIN { faded([190, 196, 204, 255]) } else { look.color(i) };
                    out.fill.place(&unit, inst, size, color);
                    label(&mut out, i, Vec3::from_array(inst.position), color);
                }
            }
        }
        draw_kind::TRIANGLES => {
            out.shaded = shaded;
            out.double_sided = true;
            for t in 0..points.len() / 3 {
                let i = 3 * t;
                out.fill.look[3] = phase0 + item_phase(t);
                out.fill.triangle(points[i], points[i + 1], points[i + 2], [look.color(i), look.color(i + 1), look.color(i + 2)]);
                label(&mut out, t, (points[i] + points[i + 1] + points[i + 2]) / 3.0, look.color(i));
            }
        }
        draw_kind::TEXT => {
            let all = e.props().get("text").unwrap_or_default();
            let color = if e.color != 0 { unpack_rgba(e.color) } else { [240, 242, 245, 255] };
            let background = if e.color2 != 0 { unpack_rgba(e.color2) } else { LABEL_BACKGROUND };
            let size = if e.width != 0.0 { e.width } else { -14.0 };
            for (i, p) in points.iter().enumerate() {
                let text = labels.get(i).filter(|t| !t.is_empty()).cloned().unwrap_or_else(|| all.clone());
                if text.is_empty() {
                    continue;
                }
                let color = match e.colors.get(i) {
                    Some(c) if *c != 0 => unpack_rgba(*c),
                    _ => color,
                };
                let accent = match e.props().get("accent").and_then(|a| a.parse::<u32>().ok()) {
                    Some(a) => unpack_rgba(a),
                    None if e.color != 0 => color,
                    None => PLAN,
                };
                out.labels.push(LabelSpec { at: *p, text, color, background, accent, size, centred: true });
            }
        }
        _ => {}
    }
    out.fill.lay(ground);
    out
}

/// A unit shape centred on the origin, one unit across.
pub struct UnitMesh {
    positions: Vec<Vec3>,
    normals: Vec<Vec3>,
    indices: Vec<u32>,
    /// 1 on a flat disc, 2 on box faces, 0 otherwise.
    kind: f32,
    /// Turns positions into unit-shape coordinates.
    to_unit: Vec3,
}

const ROUND_SEGMENTS: usize = 48;

impl UnitMesh {
    fn empty() -> Self {
        Self { positions: Vec::new(), normals: Vec::new(), indices: Vec::new(), kind: 0.0, to_unit: Vec3::ONE }
    }

    fn of(shape: u32) -> Option<Self> {
        Some(match shape {
            shape_kind::BOX => Self::cuboid(),
            shape_kind::ELLIPSOID => Self::sphere(32, 18),
            shape_kind::CYLINDER => Self::cylinder(ROUND_SEGMENTS, 0.5),
            shape_kind::CONE => Self::cylinder(ROUND_SEGMENTS, 0.0),
            shape_kind::ELLIPSE => Self::disc(64),
            shape_kind::RECT => Self::quad(),
            _ => return None,
        })
    }

    /// A loaded mesh file as it is, its local coordinates scaled to a unit.
    fn from_raw(raw: &super::assets::RawMesh) -> Self {
        let (lo, hi) = raw.positions.iter().fold((Vec3::MAX, Vec3::MIN), |(a, b), p| (a.min(*p), b.max(*p)));
        let extent = (hi - lo).max_element().max(1e-6);
        Self { positions: raw.positions.clone(), normals: raw.normals.clone(), indices: raw.indices.clone(), kind: 0.0, to_unit: Vec3::splat(1.0 / extent) }
    }

    /// A unit square in the XY plane, its edges known to the holo look.
    fn quad() -> Self {
        let mut m = Self { kind: 2.0, ..Self::empty() };
        for (x, y) in [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)] {
            m.positions.push(Vec3::new(x, y, 0.0));
            m.normals.push(Vec3::Z);
        }
        m.indices.extend([0, 1, 2, 0, 2, 3]);
        m
    }

    /// The top half of a unit sphere, open at its base.
    fn dome() -> Self {
        let (slices, stacks) = (40usize, 12usize);
        let mut m = Self::empty();
        for s in 0..=stacks {
            let phi = s as f32 / stacks as f32 * FRAC_PI_2;
            for k in 0..=slices {
                let theta = k as f32 / slices as f32 * TAU;
                let n = Vec3::new(phi.sin() * theta.cos(), phi.sin() * theta.sin(), phi.cos());
                m.positions.push(0.5 * n);
                m.normals.push(n);
            }
        }
        let row = slices as u32 + 1;
        for s in 0..stacks as u32 {
            for k in 0..slices as u32 {
                let (a, b) = (s * row + k, (s + 1) * row + k);
                m.indices.extend([a, b, a + 1, a + 1, b, b + 1]);
            }
        }
        m
    }

    fn cuboid() -> Self {
        let mut m = Self { kind: 2.0, ..Self::empty() };
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            for sign in [1.0f32, -1.0] {
                let n = axis * sign;
                let u = n.any_orthonormal_vector();
                let v = n.cross(u);
                let base = m.positions.len() as u32;
                for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                    m.positions.push(0.5 * (n + u * a + v * b));
                    m.normals.push(n);
                }
                m.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            }
        }
        m
    }

    fn sphere(slices: usize, stacks: usize) -> Self {
        let mut m = Self::empty();
        for s in 0..=stacks {
            let phi = s as f32 / stacks as f32 * std::f32::consts::PI;
            for k in 0..=slices {
                let theta = k as f32 / slices as f32 * TAU;
                let n = Vec3::new(phi.sin() * theta.cos(), phi.sin() * theta.sin(), phi.cos());
                m.positions.push(0.5 * n);
                m.normals.push(n);
            }
        }
        let row = slices as u32 + 1;
        for s in 0..stacks as u32 {
            for k in 0..slices as u32 {
                let (a, b) = (s * row + k, (s + 1) * row + k);
                m.indices.extend([a, b, a + 1, a + 1, b, b + 1]);
            }
        }
        m
    }

    /// A cylinder of top radius `top` (0 for a cone) over a base of 0.5.
    fn cylinder(segments: usize, top: f32) -> Self {
        let mut m = Self::empty();
        let slope = 0.5 - top;
        for k in 0..=segments {
            let theta = k as f32 / segments as f32 * TAU;
            let (s, c) = theta.sin_cos();
            let n = Vec3::new(c, s, slope).normalize();
            m.positions.push(Vec3::new(0.5 * c, 0.5 * s, -0.5));
            m.normals.push(n);
            m.positions.push(Vec3::new(top * c, top * s, 0.5));
            m.normals.push(n);
        }
        for k in 0..segments as u32 {
            let i = 2 * k;
            m.indices.extend([i, i + 2, i + 1, i + 1, i + 2, i + 3]);
        }
        let caps: &[(f32, f32)] = if top > 0.0 { &[(-0.5, 0.5), (0.5, top)] } else { &[(-0.5, 0.5)] };
        for (z, r) in caps.iter().copied() {
            let n = Vec3::Z * z.signum();
            let centre = m.positions.len() as u32;
            m.positions.push(Vec3::new(0.0, 0.0, z));
            m.normals.push(n);
            for k in 0..=segments {
                let theta = k as f32 / segments as f32 * TAU;
                m.positions.push(Vec3::new(r * theta.cos(), r * theta.sin(), z));
                m.normals.push(n);
            }
            for k in 0..segments as u32 {
                let (a, b) = (centre + 1 + k, centre + 2 + k);
                if z > 0.0 { m.indices.extend([centre, a, b]) } else { m.indices.extend([centre, b, a]) }
            }
        }
        m
    }

    fn disc(segments: usize) -> Self {
        let mut m = Self { positions: vec![Vec3::ZERO], normals: vec![Vec3::Z], kind: 1.0, ..Self::empty() };
        for k in 0..=segments {
            let theta = k as f32 / segments as f32 * TAU;
            m.positions.push(Vec3::new(0.5 * theta.cos(), 0.5 * theta.sin(), 0.0));
            m.normals.push(Vec3::Z);
        }
        for k in 0..segments as u32 {
            m.indices.extend([0, k + 1, k + 2]);
        }
        m
    }

    /// A capsule `size.z` long overall with radius from `size.x`, in metres.
    fn capsule(size: Vec3) -> Self {
        let r = 0.5 * size.x.abs().max(size.y.abs());
        let half = (0.5 * size.z.abs() - r).max(0.0);
        let (slices, stacks) = (32usize, 16usize);
        let mut m = Self::empty();
        m.to_unit = Vec3::new(0.5 / r.max(1e-3), 0.5 / r.max(1e-3), 1.0 / size.z.abs().max(1e-3));
        for s in 0..=stacks + 1 {
            let (phi, z) = if s <= stacks / 2 {
                let phi = s as f32 / stacks as f32 * std::f32::consts::PI;
                (phi, half)
            } else {
                let phi = (s - 1) as f32 / stacks as f32 * std::f32::consts::PI;
                (phi, -half)
            };
            for k in 0..=slices {
                let theta = k as f32 / slices as f32 * TAU;
                let n = Vec3::new(phi.sin() * theta.cos(), phi.sin() * theta.sin(), phi.cos());
                m.positions.push(n * r + Vec3::Z * z);
                m.normals.push(n);
            }
        }
        let row = slices as u32 + 1;
        for s in 0..(stacks + 1) as u32 {
            for k in 0..slices as u32 {
                let (a, b) = (s * row + k, (s + 1) * row + k);
                m.indices.extend([a, b, a + 1, a + 1, b, b + 1]);
            }
        }
        m
    }
}

fn ring(rx: f32, ry: f32, z: f32, plane: usize) -> Vec<Vec3> {
    (0..ROUND_SEGMENTS)
        .map(|k| {
            let theta = k as f32 / ROUND_SEGMENTS as f32 * TAU;
            let (a, b) = (rx * theta.cos(), ry * theta.sin());
            match plane {
                0 => Vec3::new(a, b, z),
                1 => Vec3::new(a, z, b),
                _ => Vec3::new(z, a, b),
            }
        })
        .collect()
}

fn arc(r: f32, from: f32, to: f32, centre: Vec3, plane: usize) -> Vec<Vec3> {
    let steps = ROUND_SEGMENTS / 2;
    (0..=steps)
        .map(|k| {
            let t = from + (to - from) * k as f32 / steps as f32;
            let (a, b) = (r * t.cos(), r * t.sin());
            centre + if plane == 0 { Vec3::new(a, 0.0, b) } else { Vec3::new(0.0, a, b) }
        })
        .collect()
}

/// Outline polylines of a shape of `size` in its own frame, with whether
/// each one closes.
fn outline_of(shape: u32, size: Vec3) -> Vec<(Vec<Vec3>, bool)> {
    let h = 0.5 * size;
    match shape {
        shape_kind::BOX => {
            let c = |x: f32, y: f32, z: f32| Vec3::new(x * h.x, y * h.y, z * h.z);
            let mut out = vec![
                (vec![c(-1., -1., -1.), c(1., -1., -1.), c(1., 1., -1.), c(-1., 1., -1.)], true),
                (vec![c(-1., -1., 1.), c(1., -1., 1.), c(1., 1., 1.), c(-1., 1., 1.)], true),
            ];
            for (x, y) in [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)] {
                out.push((vec![c(x, y, -1.), c(x, y, 1.)], false));
            }
            out
        }
        shape_kind::ELLIPSOID => vec![(ring(h.x, h.y, 0.0, 0), true), (ring(h.x, h.z, 0.0, 1), true), (ring(h.y, h.z, 0.0, 2), true)],
        shape_kind::CYLINDER | shape_kind::CONE => {
            let top = if shape == shape_kind::CONE { 0.0 } else { 1.0 };
            let mut out = vec![(ring(h.x, h.y, -h.z, 0), true)];
            if top > 0.0 {
                out.push((ring(h.x, h.y, h.z, 0), true));
            }
            for k in 0..4 {
                let theta = k as f32 * FRAC_PI_2;
                let (c, s) = (theta.cos(), theta.sin());
                out.push((vec![Vec3::new(h.x * c, h.y * s, -h.z), Vec3::new(top * h.x * c, top * h.y * s, h.z)], false));
            }
            out
        }
        shape_kind::CAPSULE => {
            let r = h.x.abs().max(h.y.abs());
            let half = (h.z.abs() - r).max(0.0);
            let mut out = vec![(ring(r, r, half, 0), true), (ring(r, r, -half, 0), true)];
            for k in 0..4 {
                let theta = k as f32 * FRAC_PI_2;
                let side = Vec3::new(r * theta.cos(), r * theta.sin(), 0.0);
                out.push((vec![side + Vec3::Z * half, side - Vec3::Z * half], false));
            }
            for plane in 0..2 {
                out.push((arc(r, 0.0, std::f32::consts::PI, Vec3::Z * half, plane), false));
                out.push((arc(r, std::f32::consts::PI, TAU, -Vec3::Z * half, plane), false));
            }
            out
        }
        shape_kind::ELLIPSE => vec![(ring(h.x, h.y, 0.0, 0), true)],
        shape_kind::RECT => vec![(vec![Vec3::new(-h.x, -h.y, 0.0), Vec3::new(h.x, -h.y, 0.0), Vec3::new(h.x, h.y, 0.0), Vec3::new(-h.x, h.y, 0.0)], true)],
        _ => Vec::new(),
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use gearbox_api::wire::draw::rgba;

    fn prims(s: &Strokes) -> usize {
        s.indices.len() / 6
    }

    fn flags_at(s: &Strokes, corner: usize) -> u32 {
        (s.code[corner] >> 2) & 0xfff
    }

    fn style_at(s: &Strokes, corner: usize) -> u32 {
        s.code[corner] >> 14
    }

    #[test]
    fn a_strip_is_one_capsule_a_segment_joined_to_its_neighbours() {
        let pts = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]];
        let strip = build(&DrawEntity::lines("t", "a", &pts), "t", "a");
        assert_eq!(prims(&strip.strokes), 2);
        assert_eq!(strip.strokes.fx2[0][1], 2.0, "each segment knows its strip's length");
        assert_eq!(strip.strokes.prev[0], pts[0], "the first segment joins nothing before it");
        assert_eq!(strip.strokes.next[0], pts[2], "and the next segment after it");
        let closed = build(&DrawEntity::lines("t", "a", &pts).with_sub(line_mode::LOOP), "t", "a");
        assert_eq!(prims(&closed.strokes), 3);
        assert_eq!(closed.strokes.prev[0], pts[2], "a loop joins round");
        let caps = build(&DrawEntity::lines("t", "a", &pts).with_flags(draw_flag::FLAT_CAPS), "t", "a");
        assert_eq!(flags_at(&caps.strokes, 0), FLAG_FLAT_START);
        assert_eq!(flags_at(&caps.strokes, 4), FLAG_FLAT_END);
    }

    #[test]
    fn colours_and_widths_fall_back_to_the_entity_then_the_path() {
        let pts = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]];
        let plain = build(&DrawEntity::lines("lab", "p", &pts), "lab", "p");
        let auto = unpack_rgba(gearbox_api::draw::default_color("lab", "p"));
        assert_eq!(plain.strokes.color_a[0], auto);
        assert_eq!(plain.strokes.radii[0], [-1.25, -1.25]);
        let red = rgba(255, 0, 0, 128);
        let given = DrawEntity::lines("lab", "p", &pts).with_color(red).with_colors(&[0, rgba(0, 0, 255, 255)]).with_width(0.2);
        let built = build(&given, "lab", "p");
        assert_eq!(built.strokes.color_a[0], [255, 0, 0, 128]);
        assert_eq!(built.strokes.color_b[0], [0, 0, 255, 255]);
        assert_eq!(built.strokes.radii[0], [0.1, 0.1]);
    }

    #[test]
    fn auto_picks_the_style_by_kind_and_role_and_colours_by_role() {
        let line = |ns: &str, id: &str| resolve_style(&DrawEntity::lines(ns, id, &[]), ns, id);
        assert_eq!(line("ondrive/robot", "path"), draw_style::RIBBON_FLOW);
        assert_eq!(line("ondrive/robot/rollouts", "dwa"), draw_style::HAIRLINE);
        assert_eq!(line("ondrive/robot", "horizon"), draw_style::TUBE_PULSE);
        assert_eq!(line("ondrive/robot", "trace"), draw_style::LIGHT_WALL);
        assert_eq!(line("ondrive/robot", "footprint"), draw_style::PLAIN);
        assert_eq!(line("lab", "misc"), draw_style::PLAIN);
        let shapes = DrawEntity::shapes("lab/obstacles", "pred", shape_kind::ELLIPSE, &[]);
        assert_eq!(resolve_style(&shapes, "lab/obstacles", "pred"), draw_style::SHIELD);
        assert_eq!(resolve_style(&DrawEntity::points("lab", "lookahead", &[]), "lab", "lookahead"), draw_style::ORB);
        assert_eq!(resolve_style(&DrawEntity::arrows("lab", "goal", &[]), "lab", "goal"), draw_style::BEACON);
        assert_eq!(resolve_style(&DrawEntity::points("lab", "p", &[]), "lab", "p"), draw_style::POINT_GLOW);
        assert_eq!(resolve_style(&DrawEntity::text("lab", "t", &[], "x"), "lab", "t"), draw_style::LABEL);
        let explicit = DrawEntity::lines("lab", "path", &[]).with_style(draw_style::PLAIN);
        assert_eq!(resolve_style(&explicit, "lab", "path"), draw_style::PLAIN);
        let path = build(&DrawEntity::lines("lab", "path", &[[0.0; 3], [1.0, 0.0, 0.0]]), "lab", "path");
        assert_eq!(path.strokes.color_a[0], PLAN);
        assert_eq!(style_at(&path.strokes, 0), draw_style::RIBBON_FLOW);
        assert_ne!(flags_at(&path.strokes, 0) & FLAG_FLAT, 0, "a path lies flat on the ground");
        assert_eq!(path.strokes.radii[0], [0.175, 0.175]);
    }

    #[test]
    fn effects_take_the_style_default_at_zero_and_switch_off_below_it() {
        let e = DrawEntity::lines("t", "f", &[]);
        let flow = Fx::of(&e, draw_style::RIBBON_FLOW);
        assert_eq!((flow.flow, flow.spacing, flow.glow), (1.5, 1.2, 1.0));
        let given = Fx::of(&e.clone().with_flow(3.0, 0.5).with_glow(-1.0), draw_style::RIBBON_FLOW);
        assert_eq!((given.flow, given.spacing, given.glow), (3.0, 0.5, 0.0));
        let horizon = Fx::of(&e, draw_style::TUBE_PULSE);
        assert_eq!((horizon.pulse_hz, horizon.fade), (1.0, 0.15));
        assert_eq!(Fx::of(&e.clone().with_fade(-1.0), draw_style::TUBE_PULSE).fade, 1.0, "fade off");
        let calm = Fx::of(&e, draw_style::SHIELD);
        let pressing = Fx::of(&e.clone().with_urgency(1.0), draw_style::SHIELD);
        assert!(pressing.pulse_hz > calm.pulse_hz, "urgent shields ripple faster");
        assert_eq!(Fx::of(&e, draw_style::BEACON).glow, 4.0, "the goal is the hero glow");
        assert_eq!(Fx::of(&e.with_pulse(9.0), draw_style::ALERT).pulse_hz, 3.0, "flashes stay under 3 Hz");
    }

    #[test]
    fn arrows_get_a_flat_based_head_and_axes_three_arrows() {
        let arrow = build(&DrawEntity::arrows("t", "a", &[([0.0; 3], [2.0, 0.0, 0.0])]).with_width(0.1), "t", "a");
        assert_eq!(prims(&arrow.strokes), 2);
        assert_eq!(flags_at(&arrow.strokes, 4), FLAG_FLAT_START | FLAG_HEAD);
        assert_eq!(style_at(&arrow.strokes, 0), draw_style::ARROW_GLOW);
        assert!(arrow.strokes.radii[4][0] > arrow.strokes.radii[0][0]);
        let axes = build(&DrawEntity::axes("t", "x", 1.0), "t", "x");
        assert_eq!(prims(&axes.strokes), 6);
    }

    #[test]
    fn shapes_fill_outline_or_both() {
        let one = [Instance::at([0.0; 3], [1.0, 2.0, 0.5])];
        let shape = |fill| build(&DrawEntity::shapes("t", "s", shape_kind::BOX, &one).with_fill(fill), "t", "s");
        let solid = shape(draw_flag::FILL_SOLID);
        assert!(!solid.fill.is_empty() && solid.strokes.is_empty());
        let wire = shape(draw_flag::FILL_WIRE);
        assert!(wire.fill.is_empty() && prims(&wire.strokes) == 12);
        let both = shape(draw_flag::FILL_BOTH);
        assert!(!both.fill.is_empty() && !both.strokes.is_empty());
        let extent = solid.fill.positions.iter().fold(Vec3::ZERO, |m, p| m.max(Vec3::from_array(*p)));
        assert_eq!(extent, Vec3::new(0.5, 1.0, 0.25));
        for kind in [shape_kind::ELLIPSOID, shape_kind::CYLINDER, shape_kind::CONE, shape_kind::CAPSULE, shape_kind::ELLIPSE] {
            let b = build(&DrawEntity::shapes("t", "s", kind, &one).with_fill(draw_flag::FILL_BOTH), "t", "s");
            assert!(!b.fill.is_empty() && !b.strokes.is_empty(), "{}", shape_kind::name(kind));
        }
    }

    #[test]
    fn goals_get_a_ring_and_a_beam_shields_a_dome_and_a_footprint() {
        let goal = [Instance::at([0.0, 0.0, 0.5], [1.0, 1.0, 1.0])];
        let beacon = build(&DrawEntity::shapes("t", "g", shape_kind::BOX, &goal).with_style(draw_style::BEACON).with_beam(6.0), "t", "g");
        let looks: Vec<f32> = beacon.fill.fx.iter().map(|f| f[0]).collect();
        assert!(looks.contains(&LOOK_RING) && looks.contains(&LOOK_BEAM));
        let top = beacon.fill.positions.iter().map(|p| p[2]).fold(f32::MIN, f32::max);
        assert_eq!(top, 6.0, "the beam rises from the floor of the box");
        assert!(beacon.fill.translucent());
        let pred = [Instance::at([2.0, 0.0, 0.0], [2.0, 1.0, 0.0])];
        let shield = build(&DrawEntity::shapes("lab/obstacles", "pred", shape_kind::ELLIPSE, &pred), "lab/obstacles", "pred");
        let looks: Vec<f32> = shield.fill.fx.iter().map(|f| f[0]).collect();
        assert!(looks.contains(&LOOK_SHIELD) && looks.contains(&LOOK_FOOTPRINT));
        let dome = shield.fill.positions.iter().map(|p| p[2]).fold(f32::MIN, f32::max);
        assert!((dome - 0.6).abs() < 1e-4, "a dome 0.6 of its reach tall, got {dome}");
    }

    #[test]
    fn traces_stand_as_walls_footprints_are_thin_outlines() {
        let pts = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]];
        let trace = build(&DrawEntity::lines("lab", "trace", &pts), "lab", "trace");
        assert!(trace.fill.fx.iter().all(|f| f[0] == LOOK_WALL));
        assert_eq!(trace.fill.positions.iter().map(|p| p[2]).fold(f32::MIN, f32::max), 0.25);
        let corners = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        let footprint = build(&DrawEntity::lines("lab", "footprint", &corners).with_sub(line_mode::LOOP), "lab", "footprint");
        assert!(footprint.fill.is_empty());
        assert_eq!(prims(&footprint.strokes), 4);
    }

    fn square(at: f32, side: f32) -> Vec<[f32; 3]> {
        vec![[at, at, 0.0], [at + side, at, 0.0], [at + side, at + side, 0.0], [at, at + side, 0.0]]
    }

    #[test]
    fn polygons_fill_around_their_holes_and_fade_in_from_every_edge() {
        let e = DrawEntity::polygon("t", "p", &[square(0.0, 6.0), square(2.0, 2.0)]);
        let built = build(&e, "t", "p");
        assert_eq!(prims(&built.strokes), 8, "an outline per ring");
        let fill = &built.fill;
        assert!(fill.fx.iter().all(|f| f[0] == LOOK_ZONE));
        let mut area = 0.0;
        for t in fill.indices.chunks_exact(3) {
            let p = |i: u32| Vec3::from_array(fill.positions[i as usize]);
            let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
            area += 0.5 * (b - a).cross(c - a).length();
            let mid = (a + b + c) / 3.0;
            assert!(!(mid.x > 2.0 && mid.x < 4.0 && mid.y > 2.0 && mid.y < 4.0), "no fill inside the hole at {mid}");
        }
        assert!((area - 32.0).abs() < 1e-3, "covers 36 - 4 m², got {area}");
        let deepest = fill.local.iter().map(|l| l[0]).fold(0.0, f32::max);
        assert!((deepest - DECAL_FADE).abs() < 1e-6, "inner vertices sit past the fade");
        assert!(fill.local.iter().any(|l| l[0] == 0.0), "edge vertices at the edge");
        let outline = build(&e.clone().with_fill(draw_flag::FILL_WIRE), "t", "p");
        assert!(outline.fill.is_empty());
        let stop = build(&e.with_style(draw_style::ALERT), "t", "p");
        assert!(stop.on_top && stop.fill.fx.iter().all(|f| f[0] == LOOK_ALERT));
    }

    #[test]
    fn ground_looks_split_and_follow_the_ground() {
        let hill = |x: f32, _y: f32| 0.5 * x;
        let raised = |e: DrawEntity| {
            let mut e = e;
            for p in e.points.chunks_exact_mut(3) {
                p[2] += hill(p[0], p[1]);
            }
            e
        };
        let e = raised(DrawEntity::polygon("t", "p", &[square(0.0, 4.0)]));
        let built = build_on(&e, "t", "p", Some(&hill));
        for t in built.fill.indices.chunks_exact(3) {
            for k in 0..3 {
                let (a, b) = (built.fill.positions[t[k] as usize], built.fill.positions[t[(k + 1) % 3] as usize]);
                let d = Vec2::new(a[0] - b[0], a[1] - b[1]).length();
                assert!(d <= HUG_STEP + 1e-4, "an edge {d} m long");
            }
        }
        for p in &built.fill.positions {
            assert!((p[2] - (0.5 * p[0] + GROUND_LIFT)).abs() < 1e-3, "{p:?} is not on the slope");
        }
        let disc = raised(DrawEntity::points("t", "goal", &[[2.0, 0.0, 0.0]]).with_style(draw_style::BEACON));
        let built = build_on(&disc, "t", "goal", Some(&hill));
        let ring: Vec<&[f32; 3]> = built.fill.positions.iter().zip(&built.fill.fx).filter(|(_, f)| f[0] == LOOK_RING).map(|(p, _)| p).collect();
        assert!(ring.iter().all(|p| (p[2] - (0.5 * p[0] + GROUND_LIFT)).abs() < 1e-3), "the beacon's ring lies on the slope");
    }

    #[test]
    fn rulers_tick_every_round_step_and_say_their_length() {
        let e = DrawEntity::lines("t", "ruler", &[[0.0; 3], [10.0, 0.0, 0.0]]);
        let built = build(&e, "t", "ruler");
        assert_eq!(resolve_style(&e, "t", "ruler"), draw_style::RULER);
        assert_eq!(built.labels.len(), 1);
        assert_eq!(built.labels[0].text, "10.00 m");
        assert_eq!(prims(&built.strokes), 1 + 9 + 2 + 2, "the tube, nine ticks, two caps and two end balls");
        assert!((nice_step(1.7) - 2.0).abs() < 1e-6);
        assert!((nice_step(0.04) - 0.05).abs() < 1e-6);
        assert_eq!(length_text(0.25), "25.0 cm");
    }

    #[test]
    fn arrow_tips_chevrons_and_splats() {
        let tip = build(&DrawEntity::lines("t", "l", &[[0.0; 3], [2.0, 0.0, 0.0]]).with_flags(draw_flag::ARROW_TIP), "t", "l");
        assert_eq!(prims(&tip.strokes), 2);
        assert_ne!(flags_at(&tip.strokes, 4) & FLAG_HEAD, 0);
        let chevrons = DrawEntity::new(draw_kind::ARROWS, "t", "c").with_sub(ARROW_CHEVRON).with_instances(&[Instance::at([0.0; 3], [1.0; 3])]);
        assert_eq!(prims(&build(&chevrons, "t", "c").strokes), 2, "two wings into the tip");
        let splats = DrawEntity::points("t", "cloud", &[[0.0; 3], [1.0, 0.0, 0.0]]).with_sub(POINT_SQUARE).with_values(&[0.0, 1.0]).with_colormap(gearbox_api::colormap::TURBO, 0.0, 1.0);
        let built = build(&splats, "t", "cloud");
        assert!(built.splat);
        assert_eq!(style_at(&built.strokes, 0), draw_style::SPLAT);
        assert_ne!(flags_at(&built.strokes, 0) & FLAG_SQUARE, 0);
        assert_ne!(built.strokes.color_a[0], built.strokes.color_a[4], "coloured by value");
    }

    #[test]
    fn sectors_and_frustums_sweep() {
        let sector = DrawEntity::shapes("t", "fov", shape_kind::SECTOR, &[Instance::at([0.0; 3], [4.0, 1.0, 0.5])]);
        let built = build(&sector, "t", "fov");
        assert!(built.fill.fx.iter().all(|f| f[0] == LOOK_SWEEP));
        let reach = built.fill.positions.iter().map(|p| Vec2::new(p[0], p[1]).length()).fold(0.0, f32::max);
        assert!((reach - 4.0).abs() < 1e-3 && built.fill.positions.iter().all(|p| Vec2::new(p[0], p[1]).length() >= 0.5 - 1e-3));
        let frustum = DrawEntity::shapes("t", "cam", shape_kind::FRUSTUM, &[Instance::at([0.0; 3], [1.2, 0.8, 5.0])]).with_prop("near", "0.5");
        let built = build(&frustum, "t", "cam");
        let looks: Vec<f32> = built.fill.fx.iter().map(|f| f[0]).collect();
        assert!(looks.contains(&LOOK_SWEEP) && looks.contains(&LOOK_PLANE));
        assert_eq!(prims(&built.strokes), 4 + 4 + 4, "edges, far rim and image plane rim");
        let far = built.fill.positions.iter().map(|p| p[0]).fold(0.0, f32::max);
        assert!((far - 5.0).abs() < 1e-4);
    }

    #[test]
    fn text_becomes_centred_labels_and_item_labels_sit_beside() {
        let text = build(&DrawEntity::text("t", "x", &[[0.0; 3], [1.0, 0.0, 0.0]], "hi").with_labels(&["", "two"]), "t", "x");
        assert!(text.strokes.is_empty());
        assert_eq!(text.labels.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["hi", "two"]);
        assert!(text.labels.iter().all(|l| l.centred));
        let pts = DrawEntity::points("t", "p", &[[0.0; 3]]).with_flags(draw_flag::LABELS).with_labels(&["goal"]);
        let built = build(&pts, "t", "p");
        assert_eq!(built.labels.len(), 1);
        assert!(!built.labels[0].centred);
    }
}
