//! Geometry of one draw entity in its own Z-up frame: strokes (segments and
//! points the stroke shader turns into capsules), surfaces (filled shapes,
//! triangles, ground rings, beams, walls of light and decals), and the
//! labels shown at its items, all in the look of the entity's style.

use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, TAU};

#[path = "stroke_terrain.rs"]
mod terrain;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use gearbox_api::draw::default_color;
use gearbox_api::wire::draw::unpack_rgba;
use gearbox_api::wire::draw_geom::{colormap_rgba, triangulate, value_range};
use gearbox_api::wire::draw_widgets::{ARROW_CHEVRON, BRACKET_FRACTION, POINT_SQUARE};
use gearbox_api::{
    DrawEntity, Instance, colormap, draw_flag, draw_kind, draw_style, line_mode, point_style,
    shape_kind,
};

/// Terrain height over the entity's z = 0 at entity x, y.
pub type Ground<'a> = Option<&'a dyn Fn(f32, f32) -> f32>;

use super::material::{
    ATTR_CODE, ATTR_COLOR_A, ATTR_COLOR_B, ATTR_DIST, ATTR_END, ATTR_FILL_COLOR, ATTR_FX, ATTR_FX2,
    ATTR_LOCAL, ATTR_NEXT, ATTR_PREV, ATTR_RADII, ATTR_SURFACE_FX, ATTR_SURFACE_FX2, ATTR_TERRAIN,
};
use super::visual::{self, AXIS_COLORS};

pub const FLAG_TUBE: u32 = 1;
pub const FLAG_SPHERE: u32 = 2;
pub const FLAG_FLAT_START: u32 = 4;
pub const FLAG_FLAT_END: u32 = 8;
pub const FLAG_DASHED: u32 = 16;
pub const FLAG_FLAT: u32 = 32;
pub const FLAG_ADD: u32 = 64;
pub const FLAG_HEAD: u32 = 128;
pub const FLAG_SQUARE: u32 = 256;
pub const FLAG_TERRAIN: u32 = 512;

/// Looks of the surface shader.
const LOOK_PLAIN: f32 = 0.0;
const LOOK_WALL: f32 = 1.0;
const LOOK_RING: f32 = 2.0;
const LOOK_HOLO: f32 = 3.0;
const LOOK_SHIELD: f32 = 4.0;
const LOOK_ALERT: f32 = 5.0;
const LOOK_DECAL: f32 = 6.0;
const LOOK_BEAM: f32 = 7.0;
const LOOK_SWEEP: f32 = 9.0;
const LOOK_GHOST: f32 = 10.0;
const LOOK_PLANE: f32 = 11.0;
const LOOK_ZONE: f32 = 12.0;
const LOOK_FIELD: f32 = 13.0;
const LOOK_MACHINE: f32 = 14.0;
/// How far a ring's disc reaches past its rim, for the lock brackets.
const RING_REACH: f32 = 1.15;

/// A ring's state for the shader: 0 hover, 1 selected, 2 following.
fn ring_mark(e: &DrawEntity) -> f32 {
    match e.props().get("mark").as_deref() {
        Some("hover") => 0.0,
        Some("following") => 2.0,
        _ => 1.0,
    }
}
const LOCAL_OUTLINE: f32 = 8.0;

/// Role colours (Okabe-Ito) auto styles draw in when no colour is given.
const PLAN: [u8; 4] = [0x56, 0xB4, 0xE9, 255];
const PREDICTION: [u8; 4] = [0x00, 0x9E, 0x73, 255];
const LOOKAHEAD: [u8; 4] = [0xE6, 0x9F, 0x00, 255];
const GOAL: [u8; 4] = [0xF0, 0xE4, 0x42, 255];
const OBSTACLE: [u8; 4] = [0xCC, 0x79, 0xA7, 255];
const DANGER: [u8; 4] = [0xD5, 0x5E, 0x00, 255];
const HISTORY: [u8; 4] = [255, 255, 255, 140];
const SENSOR: [u8; 4] = [0x00, 0x72, 0xB2, 255];

const LABEL_BACKGROUND: [u8; 4] = [0; 4];
/// Lift of ribbons, rings and decals off the plane they lie in, metres.
const GROUND_LIFT: f32 = 0.03;
/// Longest edge left whole where a surface lies on the ground, metres.
const HUG_STEP: f32 = super::drape::STEP_M;
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
        draw_kind::SHAPES if matches!(e.sub, shape_kind::SECTOR | shape_kind::FRUSTUM) => {
            draw_style::SWEEP
        }
        draw_kind::LINES if alert => draw_style::ALERT,
        draw_kind::LINES if has(&["rollout", "sample"]) => draw_style::HAIRLINE,
        draw_kind::LINES if has(&["horizon", "predict", "mpc"]) => draw_style::TUBE_PULSE,
        draw_kind::LINES if has(&["trace", "history", "driven", "trail"]) => draw_style::LIGHT_WALL,
        draw_kind::LINES if has(&["footprint"]) => draw_style::PLAIN,
        draw_kind::LINES if has(&["path", "plan", "route"]) => draw_style::RIBBON_FLOW,
        draw_kind::POINTS if has(&["lookahead", "carrot"]) => draw_style::ORB,
        draw_kind::POINTS | draw_kind::ARROWS | draw_kind::SHAPES if has(&["goal"]) => {
            draw_style::BEACON
        }
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
        draw_style::RING => PLAN,
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
        let pick = visual::effect;
        let defaults = visual::defaults(style);
        let urgency = e.urgency.clamp(0.0, 1.0);
        Self {
            style,
            glow: pick(e.glow, defaults.glow),
            pulse_hz: pick(e.pulse_hz, defaults.pulse).min(if style == draw_style::SWEEP {
                1.0
            } else {
                3.0
            }),
            flow: pick(e.flow_speed, defaults.flow),
            spacing: pick(e.dash, defaults.spacing),
            beam: pick(e.beam_height, defaults.beam),
            fade: if e.fade < 0.0 {
                1.0
            } else {
                pick(e.fade, defaults.fade).min(1.0)
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
    terrain: Vec<[f32; 4]>,
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
    pub fn segment(
        &mut self,
        a: Vec3,
        b: Vec3,
        ca: [u8; 4],
        cb: [u8; 4],
        ra: f32,
        rb: f32,
        dist: [f32; 2],
        flags: u32,
        prev: Vec3,
        next: Vec3,
    ) {
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
            self.code
                .push(corner | (flags & 0xfff) << 2 | (p.style & 0x3f) << 14);
            self.fx.push(fx);
            self.fx2.push(fx2);
            self.prev.push(prev.into());
            self.next.push(next.into());
        }
        self.indices
            .extend([base, base + 1, base + 3, base, base + 3, base + 2]);
    }

    pub fn point(&mut self, p: Vec3, color: [u8; 4], radius: f32, flags: u32) {
        self.strip_len = 0.0;
        self.segment(p, p, color, color, radius, radius, [0.0; 2], flags, p, p);
    }

    /// A polyline, closed when `closed`; `flat_caps` squares its two ends.
    #[allow(clippy::too_many_arguments)]
    pub fn polyline(
        &mut self,
        points: &[Vec3],
        colors: &[[u8; 4]],
        radii: &[f32],
        closed: bool,
        flags: u32,
        flat_caps: bool,
    ) {
        let n = points.len();
        match n {
            0 => {}
            1 => self.point(
                points[0],
                colors[0],
                radii[0],
                flags & (FLAG_FLAT | FLAG_ADD)
                    | if flags & FLAG_TUBE != 0 {
                        FLAG_SPHERE
                    } else {
                        0
                    },
            ),
            _ => {
                let closed = closed && n > 2;
                let count = if closed { n } else { n - 1 };
                self.strip_len = (0..count)
                    .map(|i| points[i].distance(points[(i + 1) % n]))
                    .sum();
                let mut along = 0.0;
                for i in 0..count {
                    let j = (i + 1) % n;
                    let step = points[i].distance(points[j]);
                    let prev = if i > 0 {
                        points[i - 1]
                    } else if closed {
                        points[n - 1]
                    } else {
                        points[i]
                    };
                    let next = if j + 1 < n {
                        points[j + 1]
                    } else if closed {
                        points[(j + 1) % n]
                    } else {
                        points[j]
                    };
                    let mut f = flags;
                    if flat_caps && !closed && i == 0 {
                        f |= FLAG_FLAT_START;
                    }
                    if flat_caps && !closed && i + 1 == count {
                        f |= FLAG_FLAT_END;
                    }
                    self.segment(
                        points[i],
                        points[j],
                        colors[i],
                        colors[j],
                        radii[i],
                        radii[j],
                        [along, along + step],
                        f,
                        prev,
                        next,
                    );
                    along += step;
                }
            }
        }
    }

    /// A partitioned outline with heavier short arms at its vertices.
    pub fn corner_frame(
        &mut self,
        points: &[Vec3],
        colors: &[[u8; 4]],
        radii: &[f32],
        closed: bool,
        flags: u32,
        sources: Option<&[usize]>,
    ) {
        let n = points.len();
        if n < 2 || radii[..n].iter().any(|r| r.signum() != radii[0].signum()) {
            self.polyline(points, colors, radii, closed, flags, false);
            return;
        }
        let closed = closed && n > 2;
        let count = n - usize::from(!closed);
        let mut distances = vec![0.0];
        for i in 0..count {
            distances.push(distances[i] + points[i].distance(points[(i + 1) % n]));
        }
        let corners = sources.map_or_else(|| (0..n).collect::<Vec<_>>(), |s| s.to_vec());
        if corners.first() != Some(&0)
            || corners.last().is_none_or(|i| *i >= n)
            || corners.windows(2).any(|s| s[0] >= s[1])
            || distances.windows(2).any(|d| d[1] - d[0] < 1e-6)
        {
            self.polyline(points, colors, radii, closed, flags, false);
            return;
        }
        struct Part {
            points: [Vec3; 2],
            colors: [[u8; 4]; 2],
            radii: [f32; 2],
            dist: [f32; 2],
        }
        let mut parts = Vec::new();
        self.strip_len = distances[count];
        for edge in 0..corners.len() - usize::from(!closed) {
            let start = corners[edge];
            let end = corners.get(edge + 1).copied().unwrap_or(count);
            let length = distances[end] - distances[start];
            let shoulders = [
                distances[start] + length * BRACKET_FRACTION,
                distances[end] - length * BRACKET_FRACTION,
            ];
            for i in start..end {
                let j = (i + 1) % n;
                let (from, to) = (distances[i], distances[i + 1]);
                let mut cuts = vec![from];
                cuts.extend(
                    shoulders
                        .into_iter()
                        .filter(|d| *d > from + 1e-6 && *d < to - 1e-6),
                );
                cuts.push(to);
                for span in cuts.windows(2) {
                    let dist = [span[0], span[1]];
                    let ts = dist.map(|d| ((d - from) / (to - from)).clamp(0.0, 1.0));
                    let middle = 0.5 * (span[0] + span[1]);
                    let gain = if middle < shoulders[0] || middle > shoulders[1] {
                        visual::STRUCTURE_CORNER_GAIN
                    } else {
                        1.0
                    };
                    parts.push(Part {
                        points: ts.map(|t| {
                            if t <= 0.0 {
                                points[i]
                            } else if t >= 1.0 {
                                points[j]
                            } else {
                                points[i].lerp(points[j], t)
                            }
                        }),
                        colors: ts.map(|t| {
                            std::array::from_fn(|axis| {
                                (colors[i][axis] as f32
                                    + t * (colors[j][axis] as f32 - colors[i][axis] as f32))
                                    .round() as u8
                            })
                        }),
                        radii: ts.map(|t| {
                            (if t >= 1.0 {
                                radii[j]
                            } else {
                                radii[i] + t * (radii[j] - radii[i])
                            }) * gain
                        }),
                        dist,
                    });
                }
            }
        }
        for (i, part) in parts.iter().enumerate() {
            let prev = if i > 0 {
                parts[i - 1].points[0]
            } else if closed {
                parts[parts.len() - 1].points[0]
            } else {
                part.points[0]
            };
            let next = if i + 1 < parts.len() {
                parts[i + 1].points[1]
            } else if closed {
                parts[0].points[1]
            } else {
                part.points[1]
            };
            self.segment(
                part.points[0],
                part.points[1],
                part.colors[0],
                part.colors[1],
                part.radii[0],
                part.radii[1],
                part.dist,
                flags,
                prev,
                next,
            );
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
        let head_len = if radius > 0.0 {
            (radius * 6.5).min(0.45 * len)
        } else {
            0.24 * len
        };
        let neck = tip - dir * head_len;
        self.strip_len = len;
        self.segment(
            tail,
            neck,
            color,
            color,
            radius,
            radius,
            [0.0, len - head_len],
            flags,
            tail,
            tip,
        );
        let head = flags & (FLAG_FLAT | FLAG_ADD) | FLAG_FLAT_START | FLAG_HEAD;
        self.segment(
            neck,
            tip,
            color,
            color,
            radius * 2.7,
            radius * 0.3,
            [len - head_len, len],
            head,
            tail,
            tip,
        );
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
        let head_len = if radius > 0.0 {
            (radius * 6.5).min(0.45 * total)
        } else {
            (0.08 * total).clamp(0.05, 0.6)
        };
        let saved = self.pen.style;
        if matches!(
            saved,
            draw_style::RIBBON_FLOW
                | draw_style::HAIRLINE
                | draw_style::TUBE_PULSE
                | draw_style::LIGHT_WALL
        ) {
            self.pen.style = draw_style::ARROW_GLOW;
        }
        self.strip_len = total;
        let neck = tip - dir * head_len;
        let head = flags & (FLAG_FLAT | FLAG_ADD) | FLAG_FLAT_START | FLAG_HEAD;
        self.segment(
            neck,
            tip,
            color,
            color,
            radius * 2.7,
            radius * 0.3,
            [total - head_len, total],
            head,
            neck,
            tip,
        );
        self.pen.style = saved;
    }

    /// A chevron `length` long pointing along `dir` from `centre`: two
    /// strokes from its wings brightening into its tip.
    pub fn chevron(
        &mut self,
        centre: Vec3,
        dir: Vec3,
        side: Vec3,
        length: f32,
        color: [u8; 4],
        radius: f32,
        flags: u32,
    ) {
        let tip = centre + dir * (0.5 * length);
        let back = centre - dir * (0.3 * length);
        let left = back + side * (0.42 * length);
        let right = back - side * (0.42 * length);
        self.strip_len = left.distance(tip);
        self.segment(
            left,
            tip,
            color,
            color,
            radius,
            radius,
            [0.0, self.strip_len],
            flags,
            left,
            right,
        );
        self.segment(
            tip,
            right,
            color,
            color,
            radius,
            radius,
            [self.strip_len, 0.0],
            flags,
            left,
            right,
        );
    }

    pub fn into_mesh(self) -> Mesh {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
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
        .with_inserted_indices(Indices::U32(self.indices));
        if !self.terrain.is_empty() {
            mesh.insert_attribute(ATTR_TERRAIN, self.terrain);
        }
        mesh
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
            let local = if self.look[0] == LOOK_FIELD {
                let at = *p * size;
                let axis = if n.x.abs() > 0.5 {
                    0.0
                } else if n.y.abs() > 0.5 {
                    1.0
                } else {
                    2.0
                };
                [at.x, at.y, at.z, axis]
            } else {
                let at = *p * unit.to_unit;
                [at.x, at.y, at.z, unit.kind]
            };
            self.vertex(
                pos + rot * (*p * size),
                (rot * (*n * inv)).normalize_or(Vec3::Z),
                color,
                local,
            );
        }
        self.indices.extend(unit.indices.iter().map(|i| base + i));
    }

    /// A flat disc `radius` across in the XY plane at `centre`.
    fn ground_disc(&mut self, centre: Vec3, radius: f32, color: [u8; 4]) {
        self.look2[1] = radius;
        let at = Instance::at((centre + Vec3::Z * GROUND_LIFT).into(), [1.0; 3]);
        let mark = self.mark();
        self.place(
            &UnitMesh::disc(72),
            &at,
            Vec3::new(2.0 * radius, 2.0 * radius, 1.0),
            color,
        );
        self.on_ground(mark, centre.truncate());
    }

    /// An open cylinder `radius` wide and `height` tall standing on `base`.
    fn column(&mut self, base: Vec3, radius: f32, height: f32, color: [u8; 4]) {
        if !radius.is_finite() || !height.is_finite() || radius <= 0.0 || height <= 0.0 {
            return;
        }
        let segments = 40u32;
        let first = self.positions.len() as u32;
        for k in 0..=segments {
            let theta = k as f32 / segments as f32 * TAU;
            let n = Vec3::new(theta.cos(), theta.sin(), 0.0);
            self.vertex(
                base + n * radius,
                n,
                color,
                [k as f32 / segments as f32, 0.0, 0.0, 0.0],
            );
            self.vertex(
                base + n * radius + Vec3::Z * height,
                n,
                color,
                [k as f32 / segments as f32, 1.0, 0.0, 0.0],
            );
        }
        for k in 0..segments {
            let i = first + 2 * k;
            self.indices.extend([i, i + 2, i + 1, i + 1, i + 2, i + 3]);
        }
    }

    /// A wall `height` tall standing on the polyline.
    fn wall(&mut self, points: &[Vec3], height: f32, colors: &[[u8; 4]]) {
        let closed =
            points.len() > 2 && points[0].distance_squared(points[points.len() - 1]) < 1e-8;
        let normals: Vec<Vec3> = (0..points.len())
            .map(|k| {
                let before = if closed && (k == 0 || k == points.len() - 1) {
                    points.len() - 2
                } else {
                    k.saturating_sub(1)
                };
                let after = if closed && (k == 0 || k == points.len() - 1) {
                    1
                } else {
                    (k + 1).min(points.len() - 1)
                };
                (points[after] - points[before])
                    .cross(Vec3::Z)
                    .normalize_or(Vec3::X)
            })
            .collect();
        let mut along = 0.0;
        for (k, pair) in points.windows(2).enumerate() {
            let step = pair[0].distance(pair[1]);
            let up = Vec3::Z * height;
            let i = self.vertex(pair[0], normals[k], colors[k], [along, 0.0, 0.0, 0.0]);
            self.vertex(
                pair[1],
                normals[k + 1],
                colors[k + 1],
                [along + step, 0.0, 0.0, 0.0],
            );
            self.vertex(
                pair[1] + up,
                normals[k + 1],
                colors[k + 1],
                [along + step, 1.0, 0.0, 0.0],
            );
            self.vertex(pair[0] + up, normals[k], colors[k], [along, 1.0, 0.0, 0.0]);
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
    fn polygon(
        &mut self,
        rings: &[Vec<Vec3>],
        color: [u8; 4],
        ground: Ground,
        decal: bool,
        outline: bool,
    ) {
        let flat: Vec<Vec<[f32; 2]>> = rings
            .iter()
            .map(|r| r.iter().map(|p| [p.x, p.y]).collect())
            .collect();
        let refs: Vec<&[[f32; 2]]> = flat.iter().map(|r| r.as_slice()).collect();
        let tris = triangulate(&refs);
        if tris.is_empty() {
            return;
        }
        let all: Vec<Vec3> = rings.iter().flatten().copied().collect();
        let area = gearbox_api::wire::draw_geom::ring_area(&flat[0]).abs();
        let step = if ground.is_some() {
            HUG_STEP
        } else {
            (area.sqrt() / 60.0).clamp(0.6, 4.0)
        };
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
                self.local[k] = [
                    edges.distance([p[0], p[1]]),
                    p[0],
                    p[1],
                    if outline { 4.0 } else { 3.0 },
                ];
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
        self.grounded.push((
            from.0..self.positions.len(),
            from.1..self.indices.len(),
            anchor,
        ));
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
    fn hug_span(
        &mut self,
        verts: std::ops::Range<usize>,
        tris: std::ops::Range<usize>,
        ground: Ground,
        anchor: Option<Vec2>,
        step: f32,
    ) {
        let rise = |x: f32, y: f32| ground.map_or(0.0, |g| g(x, y));
        let anchor_rise = anchor.map(|a| rise(a.x, a.y));
        let mut heights: HashMap<u32, f32> = verts
            .clone()
            .map(|k| {
                let p = self.positions[k];
                (
                    k as u32,
                    p[2] - anchor_rise.unwrap_or_else(|| rise(p[0], p[1])),
                )
            })
            .collect();
        let mut work: Vec<[u32; 3]> = self.indices[tris.clone()]
            .chunks_exact(3)
            .map(|t| [t[0], t[1], t[2]])
            .collect();
        self.indices.drain(tris);
        let laid_from = self.indices.len();
        let mut mids: HashMap<(u32, u32), u32> = HashMap::new();
        let step2 = step * step;
        let budget = 6_000_000usize;
        while let Some(t) = work.pop() {
            let xy =
                |i: u32| Vec2::new(self.positions[i as usize][0], self.positions[i as usize][1]);
            let lens = [
                xy(t[0]).distance_squared(xy(t[1])),
                xy(t[1]).distance_squared(xy(t[2])),
                xy(t[2]).distance_squared(xy(t[0])),
            ];
            let (k, longest) =
                lens.iter()
                    .copied()
                    .enumerate()
                    .fold(
                        (0, 0.0f32),
                        |best, (k, l)| if l > best.1 { (k, l) } else { best },
                    );
            if !(longest > step2) || self.indices.len() + 3 * work.len() > budget {
                self.indices.extend(t);
                continue;
            }
            let (a, b, c) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
            let key = (a.min(b), a.max(b));
            let m = match mids.get(&key) {
                Some(m) => *m,
                None => {
                    let h = 0.5
                        * (heights.get(&a).copied().unwrap_or(0.0)
                            + heights.get(&b).copied().unwrap_or(0.0));
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
            for (k, h) in &heights {
                let p = &mut self.positions[*k as usize];
                p[2] = rise(p[0], p[1]) + *h;
            }
            let mut normals: HashMap<u32, Vec3> =
                heights.keys().map(|k| (*k, Vec3::ZERO)).collect();
            for tri in self.indices[laid_from..].chunks_exact(3) {
                let p = |k: usize| Vec3::from_array(self.positions[tri[k] as usize]);
                let normal = (p(1) - p(0)).cross(p(2) - p(0));
                for k in tri {
                    *normals.entry(*k).or_default() += normal;
                }
            }
            for (k, normal) in normals {
                self.normals[k as usize] = normal
                    .normalize_or(Vec3::from_array(self.normals[k as usize]))
                    .into();
            }
        }
    }

    /// A vertex halfway between `a` and `b`, its attributes averaged.
    fn midpoint(&mut self, a: u32, b: u32) -> u32 {
        let (a, b) = (a as usize, b as usize);
        let mix3 = |u: [f32; 3], v: [f32; 3]| {
            [
                0.5 * (u[0] + v[0]),
                0.5 * (u[1] + v[1]),
                0.5 * (u[2] + v[2]),
            ]
        };
        let mix4 = |u: [f32; 4], v: [f32; 4]| {
            [
                0.5 * (u[0] + v[0]),
                0.5 * (u[1] + v[1]),
                0.5 * (u[2] + v[2]),
                0.5 * (u[3] + v[3]),
            ]
        };
        let (ca, cb) = (self.colors[a], self.colors[b]);
        let color = [0, 1, 2, 3].map(|k| ((ca[k] as u16 + cb[k] as u16) / 2) as u8);
        self.positions
            .push(mix3(self.positions[a], self.positions[b]));
        self.normals.push(
            Vec3::from_array(mix3(self.normals[a], self.normals[b]))
                .normalize_or(Vec3::Z)
                .into(),
        );
        self.colors.push(color);
        self.local.push(mix4(self.local[a], self.local[b]));
        self.fx.push(self.fx[a]);
        self.fx2.push(self.fx2[a]);
        self.positions.len() as u32 - 1
    }

    pub fn into_mesh(self) -> Mesh {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
        .with_inserted_attribute(
            ATTR_FILL_COLOR,
            VertexAttributeValues::Unorm8x4(self.colors),
        )
        .with_inserted_attribute(ATTR_LOCAL, self.local)
        .with_inserted_attribute(ATTR_SURFACE_FX, self.fx)
        .with_inserted_attribute(ATTR_SURFACE_FX2, self.fx2)
        .with_inserted_indices(Indices::U32(self.indices))
    }
}

/// Text shown at a point of the entity.
#[derive(Debug, Clone)]
pub struct LabelSpec {
    pub annotation: super::AnnotationMeta,
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

fn alpha_scaled(mut color: [u8; 4], alpha: u8) -> [u8; 4] {
    color[3] = ((color[3] as u16 * alpha as u16) / 255) as u8;
    color
}

fn adapt_labels(e: &DrawEntity, labels: &mut [LabelSpec]) {
    if labels.is_empty() {
        return;
    }
    let props = e.props();
    let parts = props.get("label_parts").as_deref() == Some("identity-detail-json");
    let neutral = props.get("label_neutral_identity").as_deref() == Some("true");
    let accent = props
        .get("label_accent")
        .and_then(|a| a.parse::<u32>().ok())
        .map(unpack_rgba);
    for label in labels {
        if parts && let Ok([identity, detail]) = serde_json::from_str::<[String; 2]>(&label.text) {
            label.text = identity;
            label.annotation.detail = detail;
        }
        if neutral {
            if e.kind == draw_kind::TEXT {
                label.accent = label.color;
            }
            label.color = alpha_scaled(visual::INK, label.color[3]);
        }
        if let Some(accent) = accent {
            label.accent = alpha_scaled(accent, label.color[3]);
        }
    }
}

fn structural_outline(
    out: &mut Built,
    shape: u32,
    size: Vec3,
    inst: &Instance,
    color: [u8; 4],
    radius: f32,
    flags: u32,
    connected: bool,
) {
    let (rotation, position) = pose_of(inst);
    let saved = out.strokes.pen;
    out.strokes.pen.style = draw_style::PLAIN;
    let corner_radius = radius * visual::STRUCTURE_CORNER_GAIN;
    for (line, closed) in outline_of(shape, size) {
        let points: Vec<_> = line.iter().map(|p| position + rotation * *p).collect();
        if connected || !matches!(shape, shape_kind::BOX | shape_kind::RECT) {
            let stations = if points.len() > 8 {
                Some(if closed {
                    (0..4).map(|k| k * points.len() / 4).collect::<Vec<_>>()
                } else {
                    vec![0, points.len() / 2, points.len() - 1]
                })
            } else {
                None
            };
            out.strokes.corner_frame(
                &points,
                &vec![color; points.len()],
                &vec![radius; points.len()],
                closed,
                flags,
                stations.as_deref(),
            );
            continue;
        }
        let count = points.len() - usize::from(!closed);
        for i in 0..count {
            let (a, b) = (points[i], points[(i + 1) % points.len()]);
            let (start, end) = (a.lerp(b, BRACKET_FRACTION), b.lerp(a, BRACKET_FRACTION));
            for pair in [[a, start], [end, b]] {
                out.strokes
                    .polyline(&pair, &[color; 2], &[corner_radius; 2], false, flags, true);
            }
        }
    }
    out.strokes.pen = saved;
}

fn feature_edges(positions: &[Vec3], indices: &[u32]) -> Vec<[Vec3; 2]> {
    if indices.len() > 600_000 {
        return Vec::new();
    }
    let key = |p: Vec3| p.to_array().map(|v| if v == 0.0 { 0 } else { v.to_bits() });
    let mut edges =
        std::collections::BTreeMap::<[[u32; 3]; 2], ([Vec3; 2], Vec3, u32, bool)>::new();
    for triangle in indices.chunks_exact(3) {
        let (Some(&a), Some(&b), Some(&c)) = (
            positions.get(triangle[0] as usize),
            positions.get(triangle[1] as usize),
            positions.get(triangle[2] as usize),
        ) else {
            continue;
        };
        if !a.is_finite() || !b.is_finite() || !c.is_finite() {
            continue;
        }
        let Some(normal) = (b - a).cross(c - a).try_normalize() else {
            continue;
        };
        for [a, b] in [[a, b], [b, c], [c, a]] {
            let (ka, kb) = (key(a), key(b));
            let id = if ka <= kb { [ka, kb] } else { [kb, ka] };
            edges
                .entry(id)
                .and_modify(|edge| {
                    edge.2 += 1;
                    edge.3 |= edge.1.dot(normal).abs() < 0.819_152;
                })
                .or_insert(([a, b], normal, 1, false));
        }
    }
    let mut selected: Vec<_> = edges
        .into_values()
        .filter_map(|(points, _, count, crease)| (count != 2 || crease).then_some(points))
        .collect();
    selected.sort_by(|a, b| {
        b[0].distance_squared(b[1])
            .total_cmp(&a[0].distance_squared(a[1]))
    });
    selected.truncate(visual::FEATURE_EDGE_LIMIT);
    selected
}

type FeatureEdges = Vec<([Vec3; 2], super::structure::Accent)>;

fn cached_feature_edges(
    raw: &std::sync::Arc<super::assets::RawMesh>,
) -> std::sync::Arc<FeatureEdges> {
    static CACHE: std::sync::Mutex<Option<HashMap<usize, std::sync::Arc<FeatureEdges>>>> =
        std::sync::Mutex::new(None);
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    cache
        .get_or_insert_with(HashMap::new)
        .entry(std::sync::Arc::as_ptr(raw) as usize)
        .or_insert_with(|| {
            let edges = feature_edges(&raw.positions, &raw.indices);
            let accents = super::structure::accents(&edges);
            std::sync::Arc::new(edges.into_iter().zip(accents).collect())
        })
        .clone()
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

pub(super) struct PointLook<'a> {
    look: Look<'a>,
    pub fx: Fx,
    pub flags: u32,
    phase: f32,
}

impl PointLook<'_> {
    pub fn sample(&self, index: usize) -> ([u8; 4], f32, f32) {
        (
            self.look.color(index),
            self.look.radius(index),
            self.phase + item_phase(index),
        )
    }
}

pub(super) fn compact_point_look<'a>(
    e: &'a DrawEntity,
    ns: &str,
    id: &str,
) -> Result<PointLook<'a>, &'static str> {
    if e.kind != draw_kind::POINTS {
        return Err("kind");
    }
    if e.flags & draw_flag::DRAPE != 0 {
        return Err("terrain drape");
    }
    if e.flags & draw_flag::LABELS != 0 && !e.labels.is_empty() {
        return Err("point labels");
    }
    let style = resolve_style(e, ns, id);
    if !matches!(
        style,
        draw_style::PLAIN
            | draw_style::HAIRLINE
            | draw_style::ORB
            | draw_style::POINT_GLOW
            | draw_style::SPLAT
    ) {
        return Err("decorated point style");
    }
    if e.flags & !0xfff != 0 {
        return Err("unknown flags");
    }
    let base = faded(if e.color != 0 {
        unpack_rgba(e.color)
    } else {
        role_color(style).unwrap_or_else(|| unpack_rgba(default_color(ns, id)))
    });
    let radius = if e.width != 0.0 && e.width.is_finite() {
        0.5 * e.width
    } else {
        default_radius(e.kind, style)
    };
    let stroke_flags = if e.flags & draw_flag::FLAT != 0 {
        FLAG_FLAT
    } else {
        0
    } | if e.flags & draw_flag::ADDITIVE != 0 {
        FLAG_ADD
    } else {
        0
    };
    let flags = match style {
        draw_style::SPLAT => {
            (if e.sub == POINT_SQUARE {
                FLAG_SQUARE
            } else {
                0
            }) | stroke_flags & FLAG_FLAT
        }
        draw_style::ORB => stroke_flags & FLAG_ADD,
        _ if e.sub == POINT_SQUARE => FLAG_SQUARE | stroke_flags,
        _ if e.sub == point_style::DISC || e.flags & draw_flag::FLAT != 0 => stroke_flags,
        _ => FLAG_SPHERE | stroke_flags & FLAG_ADD,
    };
    Ok(PointLook {
        look: Look::new(e, base, radius),
        fx: Fx::of(e, style),
        flags,
        phase: path_phase(ns, id),
    })
}

impl<'a> Look<'a> {
    fn new(e: &'a DrawEntity, base: [u8; 4], radius: f32) -> Self {
        let mapped = e.colormap != colormap::NONE && !e.values.is_empty();
        let values = if !mapped {
            Vec::new()
        } else if e.kind == draw_kind::LINES
            && e.values.len() != e.point_count()
            && e.values.len() > 1
        {
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
        Self {
            e,
            base,
            radius,
            values,
            range,
        }
    }

    fn color(&self, i: usize) -> [u8; 4] {
        match self.e.colors.get(i) {
            Some(c) if *c != 0 => faded(unpack_rgba(*c)),
            _ if !self.values.is_empty() => {
                let v = if self.values.len() == 1 {
                    self.values[0]
                } else {
                    self.values.get(i).copied().unwrap_or(0.0)
                };
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
        (draw_style::RULER, _) => -0.75 * visual::RULE_PX,
        (draw_style::SPLAT, _) => -2.0,
        (draw_style::SWEEP, _) => -0.5 * visual::RULE_PX,
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
    let q = if q.length_squared() > 1e-8 {
        q.normalize()
    } else {
        Quat::IDENTITY
    };
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
                        buckets
                            .entry((x, y))
                            .or_default()
                            .push([a[0], a[1], b[0], b[1]]);
                    }
                }
            }
        }
        Self {
            cell,
            reach,
            buckets,
        }
    }

    /// Distance from `p` to the nearest edge, at most `reach`.
    fn distance(&self, p: [f32; 2]) -> f32 {
        let key = (
            (p[0] / self.cell).floor() as i32,
            (p[1] / self.cell).floor() as i32,
        );
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
    drape_ring_indexed(ring, ground, anchor).0
}

fn drape_ring_indexed(ring: &[Vec3], ground: Ground, anchor: Vec2) -> (Vec<Vec3>, Vec<usize>) {
    let Some(g) = ground else {
        return (ring.to_vec(), (0..ring.len()).collect());
    };
    let base = g(anchor.x, anchor.y);
    let mut out = Vec::with_capacity(ring.len() * 2);
    let mut sources = Vec::with_capacity(ring.len());
    for i in 0..ring.len() {
        sources.push(out.len());
        let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
        let n = ((a.truncate().distance(b.truncate()) / HUG_STEP).ceil() as usize).max(1);
        for k in 0..n {
            let p = a.lerp(b, k as f32 / n as f32);
            out.push(Vec3::new(p.x, p.y, g(p.x, p.y) + p.z - base));
        }
    }
    (out, sources)
}

/// A round number of metres near `rough`: 1, 2 or 5 times a power of ten.
fn nice_step(rough: f32) -> f32 {
    if !(rough > 0.0) {
        return 1.0;
    }
    let p = 10f32.powf(rough.log10().floor());
    let f = rough / p;
    p * if f < 1.5 {
        1.0
    } else if f < 3.5 {
        2.0
    } else if f < 7.5 {
        5.0
    } else {
        10.0
    }
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
fn ruler_marks(
    out: &mut Built,
    fx: &Fx,
    strip: &[Vec3],
    colors: &[[u8; 4]],
    radii: &[f32],
    entity: &DrawEntity,
    flags: u32,
) {
    let lengths: Vec<f32> = strip.windows(2).map(|w| w[0].distance(w[1])).collect();
    let total: f32 = lengths.iter().sum();
    if total < 1e-6 {
        return;
    }
    let spacing = if fx.spacing > 0.0 {
        fx.spacing
    } else {
        nice_step(total / 10.0)
    };
    let count = (total / spacing).floor() as usize;
    let at = |d: f32| -> (Vec3, Vec3, [u8; 4], f32) {
        let mut left = d;
        for (k, l) in lengths.iter().enumerate() {
            if left <= *l || k + 1 == lengths.len() {
                let dir = (strip[k + 1] - strip[k]).normalize_or(Vec3::X);
                let t = (left / l.max(1e-6)).clamp(0.0, 1.0);
                let color = [0, 1, 2, 3].map(|i| {
                    (colors[k][i] as f32 * (1.0 - t) + colors[k + 1][i] as f32 * t).round() as u8
                });
                let radius = if radii[k].signum() != radii[k + 1].signum() {
                    if t < 0.5 { radii[k] } else { radii[k + 1] }
                } else {
                    radii[k] * (1.0 - t) + radii[k + 1] * t
                };
                return (strip[k] + dir * left.min(*l), dir, color, radius);
            }
            left -= l;
        }
        (strip[0], Vec3::X, colors[0], radii[0])
    };
    let across = |dir: Vec3| -> Vec3 {
        let side = dir.cross(Vec3::Z);
        if side.length_squared() > 1e-6 {
            side.normalize()
        } else {
            dir.cross(Vec3::X).normalize_or(Vec3::Y)
        }
    };
    let half = (0.12 * spacing).clamp(0.01, 0.4);
    out.strokes.pen.style = draw_style::RULER;
    for k in 1..=count {
        let d = k as f32 * spacing;
        if d >= total - 1e-4 {
            break;
        }
        let (p, dir, color, radius) = at(d);
        let tick_r = if radius < 0.0 {
            radius * (2.0 / 3.0)
        } else {
            radius * 0.7
        };
        let side = across(dir);
        let len = if k % 5 == 0 { 2.0 * half } else { half };
        out.strokes.polyline(
            &[p - side * len, p + side * len],
            &[color; 2],
            &[tick_r; 2],
            false,
            flags,
            true,
        );
    }
    for (p, dir, color, radius) in [at(0.0), at(total)] {
        let side = across(dir);
        let cap = 3.0 * half;
        out.strokes.polyline(
            &[p - side * cap, p + side * cap],
            &[color; 2],
            &[radius; 2],
            false,
            flags,
            true,
        );
    }
    let (mid, _, color, _) = at(0.5 * total);
    out.labels.push(LabelSpec {
        annotation: super::AnnotationMeta::of(entity),
        at: mid,
        text: length_text(total),
        color: alpha_scaled(visual::INK, color[3]),
        background: alpha_scaled(
            super::AnnotationMeta::background(entity).unwrap_or(LABEL_BACKGROUND),
            color[3],
        ),
        accent: color,
        size: -16.0,
        centred: false,
    });
}

/// A flat wedge about +x of `inst`: size is its radius, opening angle and
/// inner radius. Sweep style fills it with the sensor look; others draw its
/// outline (and a decal or plain fill).
#[allow(clippy::too_many_arguments)]
fn sector(
    out: &mut Built,
    fx: &Fx,
    style: u32,
    inst: &Instance,
    size: Vec3,
    color: [u8; 4],
    outline: [u8; 4],
    phase: f32,
    ground: Ground,
    radius: f32,
    fill: u32,
    flags: u32,
    structured: bool,
) {
    let (rot, pos) = pose_of(inst);
    let outer = size.x.abs().max(1e-3);
    let angle = size.y.abs().clamp(1e-3, TAU);
    let inner = size.z.abs().min(0.999 * outer);
    let half = 0.5 * angle;
    let segments = ((angle / TAU) * 96.0).ceil().max(6.0) as usize;
    let lift = Vec3::Z * GROUND_LIFT;
    let at =
        |r: f32, theta: f32| pos + rot * (Vec3::new(r * theta.cos(), r * theta.sin(), 0.0) + lift);
    if style == draw_style::SWEEP && fill != draw_flag::FILL_WIRE {
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
                let bounds = 1.0
                    + if fill == draw_flag::FILL_SOLID {
                        0.0
                    } else {
                        LOCAL_OUTLINE
                    };
                out.fill.vertex(
                    at(r, theta),
                    normal,
                    color,
                    [r / outer, theta / half, half, bounds],
                );
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
        if fill == draw_flag::FILL_SOLID {
            return;
        }
    }
    let mut ring: Vec<Vec3> = (0..=segments)
        .map(|k| at(outer, -half + angle * k as f32 / segments as f32))
        .collect();
    if inner > 1e-4 {
        ring.extend(
            (0..=segments)
                .rev()
                .map(|k| at(inner, -half + angle * k as f32 / segments as f32)),
        );
    } else if angle < TAU - 1e-3 {
        ring.push(pos + rot * lift);
    }
    let mut stations = vec![0, segments];
    if inner > 1e-4 {
        stations.extend([segments + 1, ring.len() - 1]);
    } else if angle < TAU - 1e-3 {
        stations.push(ring.len() - 1);
    }
    let (ring, sources) = drape_ring_indexed(&ring, ground, pos.truncate());
    out.strokes.phase = phase;
    let flags = flags
        | FLAG_FLAT
        | if style == draw_style::DECAL {
            FLAG_ADD
        } else {
            0
        };
    if fill != draw_flag::FILL_SOLID || style == draw_style::DECAL {
        let saved = out.strokes.pen;
        out.strokes.pen.style = draw_style::PLAIN;
        if angle >= TAU - 1e-3 {
            for reach in [outer, inner].into_iter().filter(|r| *r > 1e-4) {
                let circle: Vec<_> = (0..segments)
                    .map(|k| at(reach, -half + angle * k as f32 / segments as f32))
                    .collect();
                let (circle, sources) = drape_ring_indexed(&circle, ground, pos.truncate());
                let stations: Vec<_> = (0..4).map(|k| sources[k * segments / 4]).collect();
                if structured {
                    out.strokes.corner_frame(
                        &circle,
                        &vec![outline; circle.len()],
                        &vec![radius; circle.len()],
                        true,
                        flags,
                        Some(&stations),
                    );
                } else {
                    out.strokes.polyline(
                        &circle,
                        &vec![outline; circle.len()],
                        &vec![radius; circle.len()],
                        true,
                        flags,
                        false,
                    );
                }
            }
        } else if structured {
            let stations: Vec<_> = stations.into_iter().map(|i| sources[i]).collect();
            out.strokes.corner_frame(
                &ring,
                &vec![outline; ring.len()],
                &vec![radius; ring.len()],
                true,
                flags,
                Some(&stations),
            );
        } else {
            out.strokes.polyline(
                &ring,
                &vec![outline; ring.len()],
                &vec![radius; ring.len()],
                true,
                flags,
                false,
            );
        }
        out.strokes.pen = saved;
    }
    if style != draw_style::SWEEP && fill != draw_flag::FILL_WIRE {
        let local = |r: f32, k: usize| {
            let theta = -half + angle * k as f32 / segments as f32;
            Vec3::new(r * theta.cos(), r * theta.sin(), GROUND_LIFT)
        };
        let rings = if angle >= TAU - 1e-3 {
            [outer, inner]
                .into_iter()
                .filter(|r| *r > 1e-4)
                .map(|r| (0..segments).map(|k| local(r, k)).collect::<Vec<_>>())
                .collect::<Vec<_>>()
        } else {
            let mut boundary: Vec<_> = (0..=segments).map(|k| local(outer, k)).collect();
            if inner > 1e-4 {
                boundary.extend((0..=segments).rev().map(|k| local(inner, k)));
            } else {
                boundary.push(lift);
            }
            vec![boundary]
        };
        let decal = style == draw_style::DECAL;
        out.fill.look = [
            if decal { LOOK_DECAL } else { LOOK_PLAIN },
            fx.glow,
            fx.pulse_hz,
            phase,
        ];
        let mark = out.fill.mark();
        out.fill.polygon(&rings, color, None, decal, false);
        for k in mark.0..out.fill.positions.len() {
            out.fill.positions[k] = (pos + rot * Vec3::from_array(out.fill.positions[k])).into();
            out.fill.normals[k] = (rot * Vec3::from_array(out.fill.normals[k])).into();
            if decal {
                out.fill.local[k][3] += LOCAL_OUTLINE;
            }
        }
        out.fill.on_ground(mark, pos.truncate());
    }
}

/// A view frustum from `inst` along +x: size is the horizontal and vertical
/// field of view and the range. Faint faces with a scan running out, glowing
/// edges, and a faint image plane `near` metres out.
#[allow(clippy::too_many_arguments)]
fn frustum(
    out: &mut Built,
    fx: &Fx,
    inst: &Instance,
    size: Vec3,
    near: f32,
    color: [u8; 4],
    outline: [u8; 4],
    phase: f32,
    radius: f32,
    plane_pulse: f32,
    flags: u32,
    fill: u32,
    structured: bool,
) {
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
    for k in 0..if fill == draw_flag::FILL_WIRE { 0 } else { 4 } {
        let (a, b) = (far[k], far[(k + 1) % 4]);
        let n = (a - pos).cross(b - pos).normalize_or(Vec3::Z);
        let i = out.fill.vertex(pos, n, color, [0.0, 0.0, 10.0, 0.0]);
        out.fill.vertex(a, n, color, [1.0, 0.0, 10.0, 0.0]);
        out.fill.vertex(b, n, color, [1.0, 0.0, 10.0, 0.0]);
        out.fill.indices.extend([i, i + 1, i + 2]);
    }
    let near = if near > 0.0 {
        near.min(range)
    } else {
        (0.15 * range).min(1.0)
    };
    let plane: Vec<Vec3> = signs.iter().map(|(y, z)| corner(near, *y, *z)).collect();
    out.fill.look = [LOOK_PLANE, fx.glow, plane_pulse, phase];
    let normal = rot * Vec3::X;
    let uv = [[1.0, 0.0], [0.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
    let i = out.fill.positions.len() as u32;
    if fill != draw_flag::FILL_WIRE {
        for (p, t) in plane.iter().zip(uv) {
            out.fill.vertex(
                *p,
                normal,
                color,
                [
                    t[0],
                    t[1],
                    0.0,
                    if fill == draw_flag::FILL_SOLID {
                        0.0
                    } else {
                        LOCAL_OUTLINE
                    },
                ],
            );
        }
        out.fill.indices.extend([i, i + 1, i + 2, i, i + 2, i + 3]);
    }
    let saved = out.strokes.pen;
    out.strokes.pen.style = draw_style::PLAIN;
    out.strokes.phase = phase;
    let mut wire = |points: &[Vec3], closed| {
        if structured {
            out.strokes.corner_frame(
                points,
                &vec![outline; points.len()],
                &vec![radius; points.len()],
                closed,
                flags,
                None,
            );
        } else {
            out.strokes.polyline(
                points,
                &vec![outline; points.len()],
                &vec![radius; points.len()],
                closed,
                flags,
                false,
            );
        }
    };
    for c in &far {
        wire(&[pos, *c], false);
    }
    if range - near > range * 1e-5 {
        wire(&far, true);
    }
    if fill != draw_flag::FILL_SOLID {
        wire(&plane, true);
    }
    out.strokes.pen = saved;
}

/// A goal's ground ring of `radius` at `base` and its beam of light.
fn beacon(out: &mut Built, fx: &Fx, base: Vec3, radius: f32, color: [u8; 4], phase: f32) {
    let fill = &mut out.fill;
    fill.look = [LOOK_RING, fx.glow * 0.8, fx.pulse_hz, phase];
    fill.ground_disc(base, radius, color);
    fill.look = [LOOK_BEAM, fx.glow, fx.pulse_hz, phase];
    fill.look2 = [fx.urgency, 0.25, fx.beam, 1.0];
    fill.column(base, 0.25, fx.beam, color);
    fill.look = [LOOK_BEAM, fx.glow * 0.3, fx.pulse_hz, phase + 0.5];
    fill.column(base, 0.6, fx.beam * 0.8, color);
}

/// A short beam of light rising from `base`.
fn short_beam(out: &mut Built, fx: &Fx, base: Vec3, radius: f32, color: [u8; 4], phase: f32) {
    out.fill.look = [LOOK_BEAM, fx.glow * 0.8, fx.pulse_hz, phase];
    out.fill.look2 = [fx.urgency, radius, fx.beam, 1.0];
    out.fill.column(base, radius, fx.beam, color);
}

/// [`build_on`] off the ground.
#[cfg(test)]
pub fn build(e: &DrawEntity, ns: &str, id: &str) -> Built {
    build_on(e, ns, id, None)
}

/// Everything entity `e` draws; ground looks lie on `ground` when given.
pub fn build_on(e: &DrawEntity, ns: &str, id: &str, ground: Ground) -> Built {
    let style = resolve_style(e, ns, id);
    let style = if style == draw_style::FIELD
        && !(e.kind == draw_kind::SHAPES && matches!(e.sub, shape_kind::BOX | shape_kind::RECT))
    {
        draw_style::PLAIN
    } else {
        style
    };
    let fx = Fx::of(e, style);
    let base = faded(if e.color != 0 {
        unpack_rgba(e.color)
    } else if e.kind == draw_kind::MESH && style == draw_style::PLAIN {
        [190, 196, 204, 255]
    } else {
        role_color(style).unwrap_or_else(|| unpack_rgba(default_color(ns, id)))
    });
    let radius = if e.width != 0.0 && e.width.is_finite() {
        0.5 * e.width
    } else {
        default_radius(e.kind, style)
    };
    let look = Look::new(e, base, radius);
    let phase0 = path_phase(ns, id);
    let mut out = Built {
        on_top: e.flags & draw_flag::ON_TOP != 0 || style == draw_style::ALERT,
        ..default()
    };
    out.strokes.pen = fx;
    let flags = e.flags;
    let shaded = flags & draw_flag::SHADED != 0;
    let additive = flags & draw_flag::ADDITIVE != 0;
    let mut stroke_flags = if shaded { FLAG_TUBE } else { 0 }
        | if flags & draw_flag::DASHED != 0 {
            FLAG_DASHED
        } else {
            0
        }
        | if flags & draw_flag::FLAT != 0 {
            FLAG_FLAT
        } else {
            0
        }
        | if additive { FLAG_ADD } else { 0 };
    let points: Vec<Vec3> = e.point_list().into_iter().map(Vec3::from_array).collect();
    out.strokes.reserve(match e.kind {
        draw_kind::LINES | draw_kind::POINTS | draw_kind::ARROWS => {
            points.len() + e.strip_ranges().len()
        }
        draw_kind::AXES => 6 * e.instance_count().max(1),
        _ => 0,
    });
    let surface_look = match style {
        draw_style::HOLO => LOOK_HOLO,
        draw_style::SHIELD => LOOK_SHIELD,
        draw_style::ALERT => LOOK_ALERT,
        draw_style::GHOST => LOOK_GHOST,
        draw_style::FIELD => LOOK_FIELD,
        _ => LOOK_PLAIN,
    };
    out.fill.look = [surface_look, fx.glow, fx.pulse_hz, phase0];
    out.fill.look2 = [fx.urgency, 1.0, fx.spacing, fx.fade];
    let labels = e.label_list();
    let show_labels = flags & draw_flag::LABELS != 0 && !labels.is_empty();
    let label = |out: &mut Built, i: usize, at: Vec3, color: [u8; 4]| {
        if show_labels && let Some(text) = labels.get(i).filter(|t| !t.is_empty()) {
            out.labels.push(LabelSpec {
                annotation: super::AnnotationMeta::of(e),
                at,
                text: text.clone(),
                color: alpha_scaled(visual::INK, color[3]),
                background: alpha_scaled(
                    super::AnnotationMeta::background(e).unwrap_or(LABEL_BACKGROUND),
                    color[3],
                ),
                accent: color,
                size: -16.0,
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
                    Vec3::Z
                        * if e.flags & draw_flag::DRAPE != 0 {
                            GROUND_LIFT
                        } else {
                            0.0
                        }
                }
                _ => Vec3::ZERO,
            };
            let points: Vec<Vec3> = points.iter().map(|p| *p + lift).collect();
            let arrow_tip = flags & draw_flag::ARROW_TIP != 0 && e.sub != line_mode::LOOP;
            if e.sub == line_mode::LIST {
                for i in (0..points.len() / 2 * 2).step_by(2) {
                    let pair = [points[i], points[i + 1]];
                    out.strokes.phase = phase0 + item_phase(i);
                    out.strokes.polyline(
                        &pair,
                        &[look.color(i), look.color(i + 1)],
                        &[look.radius(i), look.radius(i + 1)],
                        false,
                        stroke_flags,
                        flat_caps,
                    );
                    if arrow_tip {
                        out.strokes
                            .tip(&pair, look.color(i + 1), look.radius(i + 1), stroke_flags);
                    }
                }
            } else {
                let ranges = e.strip_ranges();
                let corner_frames = style == draw_style::PLAIN
                    && ranges.len() <= visual::STRUCTURE_LIMIT
                    && e.props().get("corner_frame").as_deref() == Some("true");
                let corner_sources = e
                    .props()
                    .get(super::drape::CORNER_SOURCES)
                    .and_then(|s| serde_json::from_str::<Vec<usize>>(&s).ok());
                for (s, r) in ranges.into_iter().enumerate() {
                    let colors: Vec<[u8; 4]> = r.clone().map(|i| look.color(i)).collect();
                    let radii: Vec<f32> = r.clone().map(|i| look.radius(i)).collect();
                    let strip = &points[r.clone()];
                    let closed = e.sub == line_mode::LOOP
                        || (style == draw_style::DECAL
                            && strip.len() > 3
                            && strip[0].distance(strip[strip.len() - 1]) < 1e-4);
                    let ring: Vec<Vec3> = if closed
                        && strip.len() > 3
                        && strip[0].distance(strip[strip.len() - 1]) < 1e-4
                    {
                        strip[..strip.len() - 1].to_vec()
                    } else {
                        strip.to_vec()
                    };
                    out.strokes.phase = phase0 + item_phase(s);
                    let sources = corner_sources.as_ref().map(|indices| {
                        indices
                            .iter()
                            .filter_map(|i| {
                                (*i >= r.start && *i < r.start + ring.len()).then(|| i - r.start)
                            })
                            .collect::<Vec<_>>()
                    });
                    let corners = sources.as_ref().map_or(ring.len(), Vec::len);
                    if corner_frames && closed && (3..=visual::STRUCTURE_LIMIT).contains(&corners) {
                        out.strokes.corner_frame(
                            &ring,
                            &colors,
                            &radii,
                            closed,
                            stroke_flags,
                            sources.as_deref(),
                        );
                    } else {
                        out.strokes.polyline(
                            &ring,
                            &colors,
                            &radii,
                            closed,
                            stroke_flags,
                            flat_caps,
                        );
                    }
                    if arrow_tip && !closed {
                        out.strokes.tip(
                            strip,
                            *colors.last().unwrap(),
                            *radii.last().unwrap(),
                            stroke_flags,
                        );
                    }
                    if style == draw_style::RULER {
                        ruler_marks(
                            &mut out,
                            &fx,
                            strip,
                            &colors,
                            &radii,
                            e,
                            stroke_flags & (FLAG_FLAT | FLAG_ADD | FLAG_TUBE),
                        );
                        continue;
                    }
                    if style == draw_style::LIGHT_WALL && fx.beam > 0.0 {
                        let length: f32 = strip.windows(2).map(|w| w[0].distance(w[1])).sum();
                        out.fill.look = [LOOK_WALL, fx.glow, fx.pulse_hz, phase0 + item_phase(s)];
                        out.fill.look2 = [fx.urgency, fx.beam, length, fx.fade];
                        out.fill.wall(strip, fx.beam, &colors);
                        let top: Vec<Vec3> = ring.iter().map(|p| *p + Vec3::Z * fx.beam).collect();
                        let radii: Vec<f32> = radii.iter().map(|r| r * 0.8).collect();
                        out.strokes.polyline(
                            &top,
                            &colors,
                            &radii,
                            closed,
                            stroke_flags,
                            flat_caps,
                        );
                    }
                    if style == draw_style::DECAL && closed {
                        out.fill.look = [LOOK_DECAL, fx.glow, fx.pulse_hz, phase0];
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
                        // Splats retain alpha-to-coverage and depth writes, without additive blending.
                        out.splat = true;
                        let shape = if e.sub == POINT_SQUARE {
                            FLAG_SQUARE
                        } else {
                            0
                        };
                        out.strokes
                            .point(*p, color, r, shape | stroke_flags & FLAG_FLAT);
                    }
                    draw_style::ORB => out.strokes.point(*p, color, r, stroke_flags & FLAG_ADD),
                    draw_style::RING => {
                        out.fill.look = [LOOK_MACHINE, fx.glow, fx.pulse_hz, phase];
                        out.fill.look2 = [fx.urgency, 0.0, fx.spacing, ring_mark(e)];
                        out.fill.ground_disc(*p, metres(r, 1.0) * RING_REACH, color);
                    }
                    draw_style::BEACON => {
                        out.strokes.pen.style = draw_style::ORB;
                        out.strokes.point(
                            *p,
                            color,
                            metres(r, 0.25) * 0.6,
                            stroke_flags & FLAG_ADD,
                        );
                        beacon(&mut out, &fx, *p, metres(r, 0.25) * 4.0, color, phase);
                    }
                    draw_style::HOLO => {
                        out.strokes
                            .point(*p, color, r, FLAG_SPHERE | stroke_flags & FLAG_ADD);
                        short_beam(&mut out, &fx, *p, metres(r, 0.1).max(0.08), color, phase);
                    }
                    draw_style::SHIELD => {
                        let reach = metres(r, 0.5);
                        out.fill.look[3] = phase;
                        out.fill.place(
                            &UnitMesh::dome(),
                            &Instance::at((*p).into(), [1.0; 3]),
                            Vec3::new(2.0 * reach, 2.0 * reach, 2.0 * 0.6 * reach),
                            color,
                        );
                        out.fill.look = [LOOK_SHIELD, fx.glow, fx.pulse_hz, phase0];
                        out.fill.look2 = [fx.urgency, 1.0, fx.spacing, fx.fade];
                    }
                    draw_style::DECAL => {
                        let reach = metres(r, 0.5);
                        let ring = drape_ring(
                            &circle(*p + Vec3::Z * GROUND_LIFT, reach, reach, 64),
                            ground,
                            p.truncate(),
                        );
                        out.strokes.polyline(
                            &ring,
                            &vec![color; ring.len()],
                            &vec![0.025; ring.len()],
                            true,
                            FLAG_FLAT | FLAG_ADD,
                            false,
                        );
                        out.fill.look = [LOOK_DECAL, fx.glow, fx.pulse_hz, phase];
                        out.fill.decal(&ring, color, ground);
                    }
                    draw_style::ALERT => {
                        let reach = metres(r, 0.5);
                        out.fill.look[3] = phase;
                        out.fill.look2 = [fx.urgency, 2.0 * reach, fx.spacing, 1.0];
                        out.fill.place(
                            &UnitMesh::disc(64),
                            &Instance::at((*p + Vec3::Z * GROUND_LIFT).into(), [1.0; 3]),
                            Vec3::new(2.0 * reach, 2.0 * reach, 1.0),
                            color,
                        );
                    }
                    _ => {
                        let shape = if e.sub == POINT_SQUARE {
                            FLAG_SQUARE | stroke_flags & (FLAG_FLAT | FLAG_ADD)
                        } else if disc {
                            stroke_flags & (FLAG_FLAT | FLAG_ADD)
                        } else {
                            FLAG_SPHERE | stroke_flags & FLAG_ADD
                        };
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
                    let length = if inst.size[0] != 0.0 {
                        inst.size[0]
                    } else {
                        e.sx
                    };
                    arrows.push((pos, pos + rot * Vec3::X * length, i));
                }
            }
            if style == draw_style::BEACON {
                out.strokes.pen.style = draw_style::ARROW_GLOW;
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
                    let up = if let Some(inst) = insts.get(i) {
                        pose_of(inst).0 * Vec3::Z
                    } else {
                        Vec3::Z
                    };
                    let side = up.cross(dir).normalize_or(Vec3::Y);
                    let r = if e.width != 0.0 || !e.radii.is_empty() {
                        look.radius(i)
                    } else {
                        0.06 * len
                    };
                    out.strokes
                        .chevron(tail, dir, side, len, color, r, stroke_flags);
                } else {
                    out.strokes
                        .arrow(tail, tip, color, look.radius(i), stroke_flags);
                }
                if style == draw_style::BEACON {
                    beacon(
                        &mut out,
                        &fx,
                        tail,
                        (0.6 * tail.distance(tip)).clamp(0.6, 3.0),
                        color,
                        phase,
                    );
                }
                label(&mut out, i, tip, color);
            }
        }
        draw_kind::AXES => {
            if e.props().get("tf").is_some() {
                return out;
            }
            let instances = if e.instances.is_empty() {
                vec![Instance::default()]
            } else {
                e.instance_list()
            };
            let glowing = style == draw_style::ARROW_GLOW;
            out.strokes.pen.style = if glowing {
                draw_style::ARROW_GLOW
            } else {
                draw_style::PLAIN
            };
            for (i, inst) in instances.iter().enumerate() {
                let (rot, pos) = pose_of(inst);
                let length = if inst.size[0] != 0.0 {
                    inst.size[0]
                } else if e.sx != 0.0 {
                    e.sx
                } else {
                    1.0
                };
                let r = match e.radii.get(i) {
                    Some(r) if *r != 0.0 => *r,
                    _ if e.width != 0.0 => 0.5 * e.width,
                    _ => -0.75 * visual::RULE_PX,
                };
                out.strokes.phase = phase0 + item_phase(i);
                let alpha = look.color(i)[3];
                for (axis, color) in [Vec3::X, Vec3::Y, Vec3::Z].into_iter().zip(AXIS_COLORS) {
                    let color = [color[0], color[1], color[2], alpha];
                    out.strokes.arrow(
                        pos,
                        pos + rot * axis * length,
                        color,
                        r,
                        stroke_flags & (FLAG_TUBE | FLAG_ADD),
                    );
                }
                label(&mut out, i, pos, [255, 255, 255, alpha]);
            }
        }
        draw_kind::SHAPES => {
            let instances = if e.instances.is_empty() {
                vec![Instance::default()]
            } else {
                e.instance_list()
            };
            let fill = flags & draw_flag::FILL_MASK;
            let unit = UnitMesh::of(e.sub);
            let flat_shape = matches!(
                e.sub,
                shape_kind::ELLIPSE | shape_kind::RECT | shape_kind::SECTOR | shape_kind::FRUSTUM
            );
            out.shaded = shaded || (surface_look == LOOK_PLAIN && !flat_shape);
            out.double_sided = flat_shape || surface_look != LOOK_PLAIN;
            for (i, inst) in instances.iter().enumerate() {
                let size = Vec3::new(
                    if inst.size[0] != 0.0 {
                        inst.size[0]
                    } else {
                        e.sx
                    },
                    if inst.size[1] != 0.0 {
                        inst.size[1]
                    } else {
                        e.sy
                    },
                    if inst.size[2] != 0.0 {
                        inst.size[2]
                    } else {
                        e.sz
                    },
                );
                let color = look.color(i);
                let phase = phase0 + item_phase(i);
                let (rot, pos) = pose_of(inst);
                let outline = if fill == draw_flag::FILL_BOTH && e.color2 != 0 {
                    alpha_scaled(unpack_rgba(e.color2), color[3])
                } else {
                    color
                };
                let structured = instances.len() <= visual::STRUCTURE_LIMIT
                    && (matches!(
                        style,
                        draw_style::HOLO
                            | draw_style::SHIELD
                            | draw_style::GHOST
                            | draw_style::FIELD
                            | draw_style::SWEEP
                            | draw_style::DECAL
                    ) || (style == draw_style::PLAIN
                        && e.props().get("corner_frame").as_deref() == Some("true")));
                let structure_radius = if e.width == 0.0 && e.radii.get(i).is_none_or(|r| *r == 0.0)
                {
                    -0.5 * visual::RULE_PX
                } else {
                    look.radius(i)
                };
                if e.sub == shape_kind::SECTOR {
                    sector(
                        &mut out,
                        &fx,
                        style,
                        inst,
                        size,
                        color,
                        outline,
                        phase,
                        ground,
                        if structured {
                            structure_radius
                        } else {
                            look.radius(i)
                        },
                        fill,
                        stroke_flags,
                        structured,
                    );
                    label(&mut out, i, pos, color);
                    continue;
                }
                if e.sub == shape_kind::FRUSTUM {
                    let near = e.prop_f32("near").unwrap_or(0.0);
                    let plane_pulse = if e.pulse_hz > 0.0 { fx.pulse_hz } else { 0.0 };
                    frustum(
                        &mut out,
                        &fx,
                        inst,
                        size,
                        near,
                        color,
                        outline,
                        phase,
                        if structured {
                            structure_radius
                        } else {
                            look.radius(i)
                        },
                        plane_pulse,
                        stroke_flags,
                        fill,
                        structured,
                    );
                    label(&mut out, i, pos, color);
                    continue;
                }
                let reach = 0.5 * size.x.abs().max(size.y.abs());
                let flat = matches!(e.sub, shape_kind::ELLIPSE | shape_kind::RECT);
                let floor = if flat {
                    pos
                } else {
                    pos - rot * Vec3::Z * (0.5 * size.z)
                };
                out.fill.look = [surface_look, fx.glow, fx.pulse_hz, phase];
                out.fill.look2 = [
                    fx.urgency,
                    (size.x.abs() + size.y.abs() + size.z.abs()) / 3.0,
                    fx.spacing,
                    fx.fade,
                ];
                if surface_look == LOOK_FIELD {
                    out.fill.look2 = [size.x.abs(), size.y.abs(), size.z.abs(), fx.spacing];
                }
                let solid = fill != draw_flag::FILL_WIRE;
                let surface_start = out.fill.local.len();
                match style {
                    draw_style::SHIELD if e.sub == shape_kind::ELLIPSE && solid => {
                        let height = if inst.size[2] > 0.0 {
                            inst.size[2]
                        } else {
                            0.6 * reach
                        };
                        out.fill.place(
                            &UnitMesh::dome(),
                            inst,
                            Vec3::new(size.x, size.y, 2.0 * height),
                            color,
                        );
                    }
                    draw_style::DECAL => {
                        let lifted = floor + Vec3::Z * GROUND_LIFT;
                        let ring: Vec<Vec3> = match e.sub {
                            shape_kind::BOX | shape_kind::RECT => {
                                [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                                    .iter()
                                    .map(|(x, y)| {
                                        lifted
                                            + rot
                                                * Vec3::new(0.5 * x * size.x, 0.5 * y * size.y, 0.0)
                                    })
                                    .collect()
                            }
                            _ => circle(Vec3::ZERO, 0.5 * size.x, 0.5 * size.y, 64)
                                .into_iter()
                                .map(|p| lifted + rot * p)
                                .collect(),
                        };
                        let stations: Vec<_> = (0..4).map(|k| k * ring.len() / 4).collect();
                        let (ring, sources) = drape_ring_indexed(&ring, ground, pos.truncate());
                        let r = if structured {
                            structure_radius
                        } else {
                            look.radius(i)
                        };
                        out.strokes.phase = phase;
                        if structured {
                            let stations: Vec<_> =
                                stations.into_iter().map(|i| sources[i]).collect();
                            out.strokes.corner_frame(
                                &ring,
                                &vec![outline; ring.len()],
                                &vec![r; ring.len()],
                                true,
                                stroke_flags | FLAG_FLAT | FLAG_ADD,
                                Some(&stations),
                            );
                        } else {
                            out.strokes.polyline(
                                &ring,
                                &vec![outline; ring.len()],
                                &vec![r; ring.len()],
                                true,
                                stroke_flags | FLAG_FLAT | FLAG_ADD,
                                false,
                            );
                        }
                        if solid {
                            out.fill.look = [LOOK_DECAL, fx.glow, fx.pulse_hz, phase];
                            out.fill.decal(&ring, color, ground);
                        }
                    }
                    _ if solid => match (&unit, e.sub) {
                        (_, shape_kind::CAPSULE) => {
                            out.fill
                                .place(&UnitMesh::capsule(size), inst, Vec3::ONE, color)
                        }
                        (Some(unit), _) => out.fill.place(unit, inst, size, color),
                        _ => {}
                    },
                    _ => {}
                }
                if fill == draw_flag::FILL_BOTH
                    || style == draw_style::DECAL
                    || (structured && !matches!(e.sub, shape_kind::BOX | shape_kind::RECT))
                {
                    for local in &mut out.fill.local[surface_start..] {
                        local[3] += LOCAL_OUTLINE;
                    }
                }
                match style {
                    draw_style::HOLO if solid => short_beam(
                        &mut out,
                        &fx,
                        pos + rot * Vec3::Z * (0.5 * size.z),
                        (0.15 * reach).clamp(0.05, 0.3),
                        color,
                        phase,
                    ),
                    draw_style::BEACON if solid => {
                        beacon(&mut out, &fx, floor, (1.2 * reach).max(1.0), color, phase)
                    }
                    _ => {}
                }
                if fill != draw_flag::FILL_SOLID && style != draw_style::DECAL && !structured {
                    let r = look.radius(i);
                    out.strokes.phase = phase;
                    let saved = out.strokes.pen;
                    out.strokes.pen.style = draw_style::PLAIN;
                    for (wire, closed) in outline_of(e.sub, size) {
                        let placed: Vec<Vec3> = wire.iter().map(|p| pos + rot * *p).collect();
                        let colors = vec![outline; placed.len()];
                        let radii = vec![r; placed.len()];
                        out.strokes.polyline(
                            &placed,
                            &colors,
                            &radii,
                            closed,
                            stroke_flags & !FLAG_FLAT,
                            false,
                        );
                    }
                    out.strokes.pen = saved;
                }
                if structured && style != draw_style::DECAL {
                    out.strokes.phase = phase;
                    structural_outline(
                        &mut out,
                        e.sub,
                        size,
                        inst,
                        outline,
                        structure_radius,
                        stroke_flags & !FLAG_FLAT,
                        fill != draw_flag::FILL_SOLID,
                    );
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
            let polygons = e.polygon_rings();
            let structured = polygons.len() <= visual::STRUCTURE_LIMIT
                && (style != draw_style::PLAIN
                    || e.props().get("corner_frame").as_deref() == Some("true"));
            let corner_sources = e
                .props()
                .get(super::drape::CORNER_SOURCES)
                .and_then(|s| serde_json::from_str::<Vec<usize>>(&s).ok());
            for (k, ranges) in polygons.into_iter().enumerate() {
                let color = look.color(k);
                let rings: Vec<Vec<Vec3>> = ranges
                    .iter()
                    .map(|r| points[r.clone()].iter().map(|p| *p + lift).collect())
                    .collect();
                if rings.first().is_none_or(|r| r.len() < 3) {
                    continue;
                }
                let phase = phase0 + item_phase(k);
                if fill != draw_flag::FILL_WIRE {
                    out.fill.look = [fill_look, fx.glow, fx.pulse_hz, phase];
                    out.fill.look2 = [fx.urgency, 1.0, fx.spacing, fx.fade];
                    out.fill.polygon(
                        &rings,
                        color,
                        ground,
                        fill_look == LOOK_ZONE || fill_look == LOOK_ALERT,
                        fill != draw_flag::FILL_SOLID,
                    );
                }
                if fill != draw_flag::FILL_SOLID {
                    let outline = if fill == draw_flag::FILL_BOTH && e.color2 != 0 {
                        alpha_scaled(unpack_rgba(e.color2), color[3])
                    } else {
                        color
                    };
                    let r = look.radius(k);
                    out.strokes.phase = phase;
                    for (ring, range) in rings.iter().zip(&ranges) {
                        let ring =
                            if ring.len() > 3 && ring[0].distance(*ring.last().unwrap()) < 1e-4 {
                                &ring[..ring.len() - 1]
                            } else {
                                ring
                            };
                        let sources = corner_sources.as_ref().map(|indices| {
                            indices
                                .iter()
                                .filter_map(|i| {
                                    (*i >= range.start && *i < range.start + ring.len())
                                        .then(|| i - range.start)
                                })
                                .collect::<Vec<_>>()
                        });
                        let corners = sources.as_ref().map_or(ring.len(), Vec::len);
                        if structured && (3..=visual::STRUCTURE_LIMIT).contains(&corners) {
                            out.strokes.corner_frame(
                                ring,
                                &vec![outline; ring.len()],
                                &vec![r; ring.len()],
                                true,
                                line_flags,
                                sources.as_deref(),
                            );
                        } else {
                            out.strokes.polyline(
                                ring,
                                &vec![outline; ring.len()],
                                &vec![r; ring.len()],
                                true,
                                line_flags,
                                false,
                            );
                        }
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
                let instances = if e.instances.is_empty() {
                    vec![Instance::default()]
                } else {
                    e.instance_list()
                };
                let structured = instances.len() <= visual::STRUCTURE_LIMIT
                    && (matches!(
                        style,
                        draw_style::HOLO | draw_style::SHIELD | draw_style::GHOST
                    ) || super::AnnotationMeta::of(e).selected);
                let edges = structured.then(|| cached_feature_edges(&raw));
                out.shaded = surface_look == LOOK_PLAIN;
                out.double_sided = surface_look != LOOK_PLAIN;
                for (i, inst) in instances.iter().enumerate() {
                    let pick = |k: usize, d: f32| {
                        if inst.size[k] != 0.0 {
                            inst.size[k]
                        } else if d != 0.0 {
                            d
                        } else {
                            1.0
                        }
                    };
                    let size = Vec3::new(pick(0, e.sx), pick(1, e.sy), pick(2, e.sz));
                    out.fill.look = [surface_look, fx.glow, fx.pulse_hz, phase0 + item_phase(i)];
                    out.fill.look2 = [fx.urgency, 1.0, fx.spacing, fx.fade];
                    let color = look.color(i);
                    out.fill.place(&unit, inst, size, color);
                    if let Some(edges) = &edges {
                        let saved = out.strokes.pen;
                        out.strokes.pen.style = draw_style::PLAIN;
                        let (rotation, position) = pose_of(inst);
                        let r = if e.width == 0.0 && e.radii.get(i).is_none_or(|r| *r == 0.0) {
                            -0.5 * visual::RULE_PX
                        } else {
                            look.radius(i)
                        };
                        let tint = alpha_scaled(color, 160);
                        for (edge, accent) in edges
                            .iter()
                            .take(visual::FEATURE_EDGE_LIMIT / instances.len().max(1))
                        {
                            let points = edge.map(|p| position + rotation * (p * size));
                            let length = points[0].distance(points[1]);
                            for [start, end, gain] in accent.spans() {
                                let a = points[0].lerp(points[1], start);
                                let b = points[0].lerp(points[1], end);
                                out.strokes.segment(
                                    a,
                                    b,
                                    tint,
                                    tint,
                                    r * gain,
                                    r * gain,
                                    [length * start, length * end],
                                    stroke_flags & FLAG_ADD,
                                    points[0],
                                    points[1],
                                );
                            }
                        }
                        out.strokes.pen = saved;
                    }
                    label(&mut out, i, Vec3::from_array(inst.position), color);
                }
            }
        }
        draw_kind::TRIANGLES => {
            out.shaded = shaded;
            out.double_sided = true;
            let mark = out.fill.mark();
            for t in 0..points.len() / 3 {
                let i = 3 * t;
                out.fill.look[3] = phase0 + item_phase(t);
                out.fill.triangle(
                    points[i],
                    points[i + 1],
                    points[i + 2],
                    [look.color(i), look.color(i + 1), look.color(i + 2)],
                );
                label(
                    &mut out,
                    t,
                    (points[i] + points[i + 1] + points[i + 2]) / 3.0,
                    look.color(i),
                );
            }
            if ground.is_some() {
                out.fill.hug(mark, ground, None, HUG_STEP);
            }
        }
        draw_kind::TEXT => {
            let all = e.props().get("text").unwrap_or_default();
            let color = faded(if e.color != 0 {
                unpack_rgba(e.color)
            } else {
                visual::INK
            });
            let background = super::AnnotationMeta::background(e).unwrap_or(LABEL_BACKGROUND);
            let size = if e.width != 0.0 { e.width } else { -16.0 };
            for (i, p) in points.iter().enumerate() {
                let text = labels
                    .get(i)
                    .filter(|t| !t.is_empty())
                    .cloned()
                    .unwrap_or_else(|| all.clone());
                if text.is_empty() {
                    continue;
                }
                let color = match e.colors.get(i) {
                    Some(c) if *c != 0 => faded(unpack_rgba(*c)),
                    _ => color,
                };
                let accent = match e.props().get("accent").and_then(|a| a.parse::<u32>().ok()) {
                    Some(a) => alpha_scaled(unpack_rgba(a), color[3]),
                    None if e.color != 0 || e.colors.get(i).is_some_and(|c| *c != 0) => color,
                    None => alpha_scaled(visual::ACCENT, color[3]),
                };
                out.labels.push(LabelSpec {
                    annotation: super::AnnotationMeta::of(e),
                    at: *p,
                    text,
                    color,
                    background: alpha_scaled(background, color[3]),
                    accent,
                    size,
                    centred: true,
                });
            }
        }
        _ => {}
    }
    adapt_labels(e, &mut out.labels);
    out.fill.lay(ground);
    out.strokes.lay(ground);
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
        Self {
            positions: Vec::new(),
            normals: Vec::new(),
            indices: Vec::new(),
            kind: 0.0,
            to_unit: Vec3::ONE,
        }
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
        let (lo, hi) = raw
            .positions
            .iter()
            .fold((Vec3::MAX, Vec3::MIN), |(a, b), p| (a.min(*p), b.max(*p)));
        let extent = (hi - lo).max_element().max(1e-6);
        Self {
            positions: raw.positions.clone(),
            normals: raw.normals.clone(),
            indices: raw.indices.clone(),
            kind: 0.0,
            to_unit: Vec3::splat(1.0 / extent),
        }
    }

    /// A unit square in the XY plane, its edges known to the holo look.
    fn quad() -> Self {
        let mut m = Self {
            kind: 2.0,
            ..Self::empty()
        };
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
        let mut m = Self {
            kind: 2.0,
            ..Self::empty()
        };
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
                m.indices
                    .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
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
        let caps: &[(f32, f32)] = if top > 0.0 {
            &[(-0.5, 0.5), (0.5, top)]
        } else {
            &[(-0.5, 0.5)]
        };
        for (z, r) in caps.iter().copied() {
            let n = Vec3::Z * z.signum();
            let centre = m.positions.len() as u32;
            m.positions.push(Vec3::new(0.0, 0.0, z));
            m.normals.push(n);
            for k in 0..=segments {
                let theta = k as f32 / segments as f32 * TAU;
                m.positions
                    .push(Vec3::new(r * theta.cos(), r * theta.sin(), z));
                m.normals.push(n);
            }
            for k in 0..segments as u32 {
                let (a, b) = (centre + 1 + k, centre + 2 + k);
                if z > 0.0 {
                    m.indices.extend([centre, a, b])
                } else {
                    m.indices.extend([centre, b, a])
                }
            }
        }
        m
    }

    fn disc(segments: usize) -> Self {
        let mut m = Self {
            positions: vec![Vec3::ZERO],
            normals: vec![Vec3::Z],
            kind: 1.0,
            ..Self::empty()
        };
        for k in 0..=segments {
            let theta = k as f32 / segments as f32 * TAU;
            m.positions
                .push(Vec3::new(0.5 * theta.cos(), 0.5 * theta.sin(), 0.0));
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
        m.to_unit = Vec3::new(
            0.5 / r.max(1e-3),
            0.5 / r.max(1e-3),
            1.0 / size.z.abs().max(1e-3),
        );
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
            centre
                + if plane == 0 {
                    Vec3::new(a, 0.0, b)
                } else {
                    Vec3::new(0.0, a, b)
                }
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
                (
                    vec![
                        c(-1., -1., -1.),
                        c(1., -1., -1.),
                        c(1., 1., -1.),
                        c(-1., 1., -1.),
                    ],
                    true,
                ),
                (
                    vec![
                        c(-1., -1., 1.),
                        c(1., -1., 1.),
                        c(1., 1., 1.),
                        c(-1., 1., 1.),
                    ],
                    true,
                ),
            ];
            for (x, y) in [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)] {
                out.push((vec![c(x, y, -1.), c(x, y, 1.)], false));
            }
            out
        }
        shape_kind::ELLIPSOID => vec![
            (ring(h.x, h.y, 0.0, 0), true),
            (ring(h.x, h.z, 0.0, 1), true),
            (ring(h.y, h.z, 0.0, 2), true),
        ],
        shape_kind::CYLINDER | shape_kind::CONE => {
            let top = if shape == shape_kind::CONE { 0.0 } else { 1.0 };
            let mut out = vec![(ring(h.x, h.y, -h.z, 0), true)];
            if top > 0.0 {
                out.push((ring(h.x, h.y, h.z, 0), true));
            }
            for k in 0..4 {
                let theta = k as f32 * FRAC_PI_2;
                let (c, s) = (theta.cos(), theta.sin());
                out.push((
                    vec![
                        Vec3::new(h.x * c, h.y * s, -h.z),
                        Vec3::new(top * h.x * c, top * h.y * s, h.z),
                    ],
                    false,
                ));
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
                out.push((
                    arc(r, 0.0, std::f32::consts::PI, Vec3::Z * half, plane),
                    false,
                ));
                out.push((
                    arc(r, std::f32::consts::PI, TAU, -Vec3::Z * half, plane),
                    false,
                ));
            }
            out
        }
        shape_kind::ELLIPSE => vec![(ring(h.x, h.y, 0.0, 0), true)],
        shape_kind::RECT => vec![(
            vec![
                Vec3::new(-h.x, -h.y, 0.0),
                Vec3::new(h.x, -h.y, 0.0),
                Vec3::new(h.x, h.y, 0.0),
                Vec3::new(-h.x, h.y, 0.0),
            ],
            true,
        )],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_point_inputs_match_expanded_stroke_records() {
        for style in [
            draw_style::PLAIN,
            draw_style::HAIRLINE,
            draw_style::ORB,
            draw_style::POINT_GLOW,
            draw_style::SPLAT,
        ] {
            for sub in [point_style::SPHERE, point_style::DISC, POINT_SQUARE] {
                for flags in [
                    0,
                    draw_flag::FLAT,
                    draw_flag::ADDITIVE,
                    draw_flag::SHADED,
                    draw_flag::ON_TOP,
                    draw_flag::DASHED,
                ] {
                    let mut e = DrawEntity::points(
                        "compact",
                        "samples",
                        &[[1.0, 2.0, 3.0], [-1.0, 0.0, 4.0]],
                    )
                    .with_style(style);
                    e.sub = sub;
                    e.flags = flags;
                    e.colors = vec![gearbox_api::wire::draw::rgba(220, 40, 100, 128), 0];
                    e.radii = vec![-0.2, 0.4];
                    e.values = vec![0.1, 0.9];
                    e.colormap = colormap::VIRIDIS;
                    let look = compact_point_look(&e, "compact", "samples").unwrap();
                    let built = build(&e, "compact", "samples");
                    for index in 0..2 {
                        let (color, radius, phase) = look.sample(index);
                        let vertex = index * 4;
                        assert_eq!(built.strokes.a[vertex], e.point_at(index));
                        assert_eq!(built.strokes.color_a[vertex], color);
                        assert_eq!(built.strokes.radii[vertex], [radius; 2]);
                        assert_eq!(built.strokes.code[vertex], look.flags << 2 | style << 14);
                        assert_eq!(
                            built.strokes.fx[vertex],
                            [
                                look.fx.glow,
                                look.fx.pulse_hz,
                                look.fx.flow,
                                look.fx.spacing
                            ]
                        );
                        assert_eq!(
                            built.strokes.fx2[vertex],
                            [look.fx.fade, 0.0, look.fx.urgency, phase]
                        );
                    }
                }
            }
        }
    }
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

    fn tint_cases() -> Vec<DrawEntity> {
        let points = [[0.0; 3], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]];
        let instance = [Instance::at([0.0; 3], [1.2, 0.8, 2.0])];
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/draw_vfx/faceted.obj"
        );
        let line = DrawEntity::lines("color", "line", &points);
        let point = DrawEntity::points("color", "point", &points[..1]);
        let arrow = DrawEntity::arrows("color", "arrow", &[([0.0; 3], [1.0, 0.0, 0.0])]);
        let shape = DrawEntity::shapes("color", "shape", shape_kind::BOX, &instance);
        let polygon = DrawEntity::polygon("color", "polygon", &[square(0.0, 1.0)]);
        let text = DrawEntity::text("color", "text", &points[..1], "Caller α / Mixed Case");
        let mesh = DrawEntity::mesh("color", "mesh", path);
        let mut cases = Vec::new();
        for style in 0..=draw_style::FIELD {
            let e = match style {
                draw_style::HEAT => continue,
                draw_style::BEACON
                | draw_style::ORB
                | draw_style::POINT_GLOW
                | draw_style::SPLAT => point.clone(),
                draw_style::HOLO | draw_style::SHIELD | draw_style::FIELD => shape.clone(),
                draw_style::DECAL => polygon.clone(),
                draw_style::ARROW_GLOW => arrow.clone(),
                draw_style::LABEL => text.clone(),
                draw_style::GHOST => mesh.clone(),
                draw_style::SWEEP => {
                    DrawEntity::shapes("color", "sweep", shape_kind::SECTOR, &instance)
                }
                _ => line.clone(),
            };
            cases.push(e.with_style(style));
        }
        for e in [
            line,
            point.clone(),
            arrow.clone(),
            shape,
            polygon.clone(),
            text,
            mesh.clone(),
            DrawEntity::triangles("color", "triangles", &points),
        ] {
            cases.push(e.with_style(draw_style::PLAIN));
        }
        for style in [
            draw_style::HOLO,
            draw_style::SHIELD,
            draw_style::DECAL,
            draw_style::ALERT,
        ] {
            cases.push(point.clone().with_style(style));
            cases.push(polygon.clone().with_style(style));
        }
        cases.push(arrow.with_style(draw_style::BEACON));
        cases.push(
            point
                .with_sub(point_style::DISC)
                .with_style(draw_style::POINT_GLOW),
        );
        for style in [draw_style::HOLO, draw_style::SHIELD, draw_style::GHOST] {
            cases.push(mesh.clone().with_style(style));
        }
        for kind in [
            shape_kind::BOX,
            shape_kind::RECT,
            shape_kind::ELLIPSOID,
            shape_kind::ELLIPSE,
            shape_kind::CYLINDER,
            shape_kind::CONE,
            shape_kind::CAPSULE,
            shape_kind::SECTOR,
            shape_kind::FRUSTUM,
        ] {
            cases.push(
                DrawEntity::shapes("color", "shape", kind, &instance)
                    .with_style(draw_style::PLAIN)
                    .with_fill(draw_flag::FILL_BOTH),
            );
        }
        cases
    }

    #[test]
    fn every_geometry_style_preserves_arbitrary_header_rgba_in_its_color_attributes() {
        let colors = [
            rgba(235, 71, 95, 255),
            rgba(43, 185, 111, 137),
            rgba(236, 180, 22, 97),
            rgba(160, 67, 218, 0),
            rgba(0, 0, 0, 255),
        ];
        for e in tint_cases() {
            for packed in colors {
                let tint = faded(unpack_rgba(packed));
                let built = build(&e.clone().with_color(packed), "color", "given");
                assert!(
                    !built.strokes.is_empty() || !built.fill.is_empty() || !built.labels.is_empty(),
                    "kind={} style={}",
                    e.kind,
                    e.style
                );
                for c in built
                    .strokes
                    .color_a
                    .iter()
                    .chain(&built.strokes.color_b)
                    .chain(&built.fill.colors)
                {
                    assert_eq!(
                        &c[..3],
                        &tint[..3],
                        "kind={} style={} color={packed}",
                        e.kind,
                        e.style
                    );
                    assert!(
                        c[3] <= tint[3],
                        "kind={} style={} alpha={}",
                        e.kind,
                        e.style,
                        c[3]
                    );
                }
                for label in &built.labels {
                    assert_eq!(label.accent, tint, "kind={} style={}", e.kind, e.style);
                    if e.kind == draw_kind::TEXT {
                        assert_eq!(label.color, tint);
                    }
                }
            }
        }
    }

    #[test]
    fn per_item_colors_override_header_and_zero_entries_fall_back_across_kinds() {
        let points = [[0.0; 3], [2.0, 0.0, 0.0], [4.0, 0.0, 0.0]];
        let instances: Vec<_> = points.iter().map(|p| Instance::at(*p, [1.0; 3])).collect();
        let polygons: Vec<_> = (0..3).map(|i| vec![square(i as f32 * 2.0, 1.0)]).collect();
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/draw_vfx/faceted.obj"
        );
        let entities = [
            DrawEntity::points("color", "points", &points).with_style(draw_style::POINT_GLOW),
            DrawEntity::points("color", "cloud", &points).with_style(draw_style::SPLAT),
            DrawEntity::lines("color", "line", &points).with_style(draw_style::LIGHT_WALL),
            DrawEntity::arrows(
                "color",
                "arrow",
                &points.map(|p| (p, [p[0] + 1.0, p[1], p[2]])),
            )
            .with_style(draw_style::BEACON),
            DrawEntity::shapes("color", "shape", shape_kind::BOX, &instances)
                .with_style(draw_style::HOLO),
            DrawEntity::polygons("color", "polygon", &polygons).with_style(draw_style::DECAL),
            DrawEntity::triangles(
                "color",
                "triangles",
                &[[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            ),
            DrawEntity::mesh("color", "mesh", path)
                .with_instances(&instances)
                .with_style(draw_style::GHOST),
            DrawEntity::text("color", "text", &points, "Explicit color"),
        ];
        let header = rgba(231, 170, 31, 185);
        let colors = [rgba(80, 205, 120, 210), 0, rgba(190, 70, 205, 0)];
        let expected = [
            faded(unpack_rgba(colors[0])),
            faded(unpack_rgba(header)),
            faded(unpack_rgba(colors[2])),
        ];
        for e in entities {
            let built = build(
                &e.clone().with_color(header).with_colors(&colors),
                "color",
                "items",
            );
            let attributes: Vec<_> = built
                .strokes
                .color_a
                .iter()
                .chain(&built.strokes.color_b)
                .chain(&built.fill.colors)
                .collect();
            for c in &attributes {
                let source = expected
                    .iter()
                    .find(|tint| tint[..3] == c[..3])
                    .unwrap_or_else(|| panic!("unexpected tint kind={} {c:?}", e.kind));
                assert!(c[3] <= source[3]);
            }
            for tint in expected {
                if e.kind == draw_kind::TEXT {
                    assert!(built.labels.iter().any(|label| label.color == tint));
                } else {
                    assert!(
                        attributes.iter().any(|c| c[..3] == tint[..3]),
                        "kind={} missing {tint:?}",
                        e.kind
                    );
                }
            }
        }
    }

    #[test]
    fn secondary_outlines_and_ruler_structure_use_caller_tints() {
        let primary = rgba(210, 80, 170, 192);
        let secondary = rgba(40, 190, 130, 160);
        let entities = [
            DrawEntity::shapes(
                "color",
                "shape",
                shape_kind::BOX,
                &[Instance::at([0.0; 3], [1.0; 3])],
            )
            .with_fill(draw_flag::FILL_BOTH),
            DrawEntity::polygon("color", "polygon", &[square(0.0, 1.0)]),
        ];
        for e in entities {
            let built = build(
                &e.with_style(draw_style::PLAIN)
                    .with_color(primary)
                    .with_color2(secondary),
                "color",
                "secondary",
            );
            assert!(!built.strokes.is_empty() && !built.fill.is_empty());
            assert!(
                built
                    .fill
                    .colors
                    .iter()
                    .all(|c| *c == faded(unpack_rgba(primary)))
            );
            let outline = alpha_scaled(unpack_rgba(secondary), faded(unpack_rgba(primary))[3]);
            assert!(
                built
                    .strokes
                    .color_a
                    .iter()
                    .chain(&built.strokes.color_b)
                    .all(|c| *c == outline)
            );
        }
        let e = DrawEntity::lines("color", "ruler", &[[0.0; 3], [3.0, 0.0, 0.0]])
            .with_style(draw_style::RULER)
            .with_color(primary);
        let ruler = build(&e, "color", "ruler");
        assert!(
            ruler
                .strokes
                .color_a
                .iter()
                .chain(&ruler.strokes.color_b)
                .all(|c| *c == faded(unpack_rgba(primary)))
        );
        assert_eq!(ruler.labels[0].accent, faded(unpack_rgba(primary)));
        assert_eq!(
            ruler.labels[0].color,
            alpha_scaled(visual::INK, faded(unpack_rgba(primary))[3])
        );
    }

    #[test]
    fn plain_mesh_colormap_precedes_neutral_fallback_and_item_rgba_precedes_map() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/draw_vfx/faceted.obj"
        );
        let instances: Vec<_> = (0..3)
            .map(|i| Instance::at([i as f32 * 2.0, 0.0, 0.0], [1.0; 3]))
            .collect();
        let values = [0.0, 0.5, 1.0];
        let e = DrawEntity::mesh("color", "mesh", path)
            .with_instances(&instances)
            .with_style(draw_style::PLAIN)
            .with_values(&values)
            .with_colormap(colormap::VIRIDIS, 0.0, 1.0);
        let vertices = super::super::assets::load(path).unwrap().positions.len();
        for header in [0, rgba(230, 70, 120, 128)] {
            let base = e.clone().with_color(header);
            let built = build(&base, "color", "mesh");
            let alpha = faded(if header == 0 {
                [255; 4]
            } else {
                unpack_rgba(header)
            })[3];
            for (i, colors) in built.fill.colors.chunks_exact(vertices).enumerate() {
                let expected =
                    alpha_scaled(colormap_rgba(colormap::VIRIDIS, values[i], 0.0, 1.0), alpha);
                assert!(colors.iter().all(|c| *c == expected));
            }
            let override_ = rgba(170, 60, 210, 0);
            let overridden = build(&base.with_colors(&[0, override_, 0]), "color", "mesh");
            assert_eq!(
                &overridden.fill.colors[..vertices],
                &built.fill.colors[..vertices]
            );
            assert!(
                overridden.fill.colors[vertices..2 * vertices]
                    .iter()
                    .all(|c| *c == faded(unpack_rgba(override_)))
            );
            assert_eq!(
                &overridden.fill.colors[2 * vertices..],
                &built.fill.colors[2 * vertices..]
            );
        }
        let neutral = DrawEntity::mesh("color", "mesh", path)
            .with_instances(&instances)
            .with_style(draw_style::PLAIN);
        for e in [neutral.clone(), neutral.with_colors(&[0, 0, 0])] {
            let built = build(&e, "color", "mesh");
            assert!(
                built
                    .fill
                    .colors
                    .iter()
                    .all(|c| *c == faded([190, 196, 204, 255]))
            );
        }
    }

    #[test]
    fn raw_text_item_tint_drives_default_accent_without_overriding_explicit_accent() {
        let colors = [rgba(230, 80, 150, 128), rgba(30, 200, 120, 0)];
        let e = DrawEntity::text(
            "color",
            "text",
            &[[0.0; 3], [1.0, 0.0, 0.0]],
            "Raw α / Mixed Case",
        )
        .with_colors(&colors);
        let built = build(&e, "color", "text");
        for (i, label) in built.labels.iter().enumerate() {
            assert_eq!(label.color, faded(unpack_rgba(colors[i])));
            assert_eq!(label.accent, label.color);
            assert_eq!(label.text, "Raw α / Mixed Case");
        }
        let explicit = rgba(230, 180, 30, 170);
        for property in ["accent", "label_accent"] {
            let changed = build(
                &e.clone().with_prop(property, &explicit.to_string()),
                "color",
                "text",
            );
            for (i, label) in changed.labels.iter().enumerate() {
                assert_eq!(label.color, built.labels[i].color);
                assert_eq!(
                    label.accent,
                    alpha_scaled(unpack_rgba(explicit), label.color[3])
                );
            }
        }
    }

    #[test]
    fn axes_preserve_semantic_rgb_and_use_caller_alpha() {
        let e = DrawEntity::axes("color", "axes", 1.0)
            .with_style(draw_style::ARROW_GLOW)
            .with_color(rgba(210, 60, 160, 170))
            .with_colors(&[rgba(80, 190, 30, 110)]);
        let built = build(&e, "color", "axes");
        let alpha = faded([0, 0, 0, 110])[3];
        for (axis, rgb) in AXIS_COLORS.iter().enumerate() {
            let expected = [rgb[0], rgb[1], rgb[2], alpha];
            assert!(
                built.strokes.color_a[axis * 8..(axis + 1) * 8]
                    .iter()
                    .all(|c| *c == expected)
            );
        }
        assert_eq!(
            built.strokes.color_a.len(),
            24,
            "three axes without an extra origin marker"
        );
        assert_eq!(built.strokes.radii[0], [-0.75; 2]);
        let explicit = build(
            &e.with_width(0.12).with_flags(draw_flag::SHADED),
            "color",
            "axes",
        );
        assert_eq!(explicit.strokes.radii[0], [0.06; 2]);
        assert_ne!(flags_at(&explicit.strokes, 0) & FLAG_TUBE, 0);
    }

    #[test]
    fn a_strip_is_one_capsule_a_segment_joined_to_its_neighbours() {
        let pts = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]];
        let strip = build(&DrawEntity::lines("t", "a", &pts), "t", "a");
        assert_eq!(prims(&strip.strokes), 2);
        assert_eq!(
            strip.strokes.fx2[0][1], 2.0,
            "each segment knows its strip's length"
        );
        assert_eq!(
            strip.strokes.prev[0], pts[0],
            "the first segment joins nothing before it"
        );
        assert_eq!(
            strip.strokes.next[0], pts[2],
            "and the next segment after it"
        );
        let closed = build(
            &DrawEntity::lines("t", "a", &pts).with_sub(line_mode::LOOP),
            "t",
            "a",
        );
        assert_eq!(prims(&closed.strokes), 3);
        assert_eq!(closed.strokes.prev[0], pts[2], "a loop joins round");
        let caps = build(
            &DrawEntity::lines("t", "a", &pts).with_flags(draw_flag::FLAT_CAPS),
            "t",
            "a",
        );
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
        let given = DrawEntity::lines("lab", "p", &pts)
            .with_color(red)
            .with_colors(&[0, rgba(0, 0, 255, 255)])
            .with_width(0.2);
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
        assert_eq!(
            resolve_style(&shapes, "lab/obstacles", "pred"),
            draw_style::SHIELD
        );
        assert_eq!(
            resolve_style(
                &DrawEntity::points("lab", "lookahead", &[]),
                "lab",
                "lookahead"
            ),
            draw_style::ORB
        );
        assert_eq!(
            resolve_style(&DrawEntity::arrows("lab", "goal", &[]), "lab", "goal"),
            draw_style::BEACON
        );
        assert_eq!(
            resolve_style(&DrawEntity::points("lab", "p", &[]), "lab", "p"),
            draw_style::POINT_GLOW
        );
        assert_eq!(
            resolve_style(&DrawEntity::text("lab", "t", &[], "x"), "lab", "t"),
            draw_style::LABEL
        );
        let explicit = DrawEntity::lines("lab", "path", &[]).with_style(draw_style::PLAIN);
        assert_eq!(resolve_style(&explicit, "lab", "path"), draw_style::PLAIN);
        let path = build(
            &DrawEntity::lines("lab", "path", &[[0.0; 3], [1.0, 0.0, 0.0]]),
            "lab",
            "path",
        );
        assert_eq!(path.strokes.color_a[0], PLAN);
        assert_eq!(style_at(&path.strokes, 0), draw_style::RIBBON_FLOW);
        assert_ne!(
            flags_at(&path.strokes, 0) & FLAG_FLAT,
            0,
            "a path lies flat on the ground"
        );
        assert_eq!(path.strokes.radii[0], [0.175, 0.175]);
    }

    #[test]
    fn effects_take_the_style_default_at_zero_and_switch_off_below_it() {
        let e = DrawEntity::lines("t", "f", &[]);
        let flow = Fx::of(&e, draw_style::RIBBON_FLOW);
        assert_eq!((flow.flow, flow.spacing, flow.glow), (0.6, 1.2, 0.70));
        let given = Fx::of(
            &e.clone().with_flow(3.0, 0.5).with_glow(-1.0),
            draw_style::RIBBON_FLOW,
        );
        assert_eq!((given.flow, given.spacing, given.glow), (3.0, 0.5, 0.0));
        let horizon = Fx::of(&e, draw_style::TUBE_PULSE);
        assert_eq!((horizon.pulse_hz, horizon.fade), (visual::ENERGY_HZ, 0.15));
        assert_eq!(
            Fx::of(&e.clone().with_fade(-1.0), draw_style::TUBE_PULSE).fade,
            1.0,
            "fade off"
        );
        let calm = Fx::of(&e, draw_style::SHIELD);
        let pressing = Fx::of(&e.clone().with_urgency(1.0), draw_style::SHIELD);
        assert_eq!(
            (pressing.pulse_hz, calm.pulse_hz, pressing.urgency),
            (visual::ENERGY_HZ, visual::ENERGY_HZ, 1.0)
        );
        assert_eq!(Fx::of(&e, draw_style::BEACON).glow, 0.95);
        assert_eq!(
            Fx::of(&e.clone().with_glow(2.6), draw_style::BEACON).glow,
            2.6
        );
        assert_eq!(
            Fx::of(&e.with_pulse(9.0), draw_style::ALERT).pulse_hz,
            3.0,
            "flashes stay under 3 Hz"
        );
    }

    #[test]
    fn energy_motion_and_aura_controls_remain_independent() {
        let e = DrawEntity::points("t", "energy", &[[0.0; 3]]);
        for style in [
            draw_style::TUBE_PULSE,
            draw_style::LIGHT_WALL,
            draw_style::BEACON,
            draw_style::HOLO,
            draw_style::SHIELD,
            draw_style::ORB,
            draw_style::ARROW_GLOW,
            draw_style::POINT_GLOW,
            draw_style::SWEEP,
            draw_style::GHOST,
            draw_style::FIELD,
        ] {
            let no_aura = Fx::of(&e.clone().with_glow(-1.0), style);
            assert_eq!((no_aura.glow, no_aura.pulse_hz), (0.0, visual::ENERGY_HZ));
            let still = Fx::of(&e.clone().with_pulse(-1.0).with_flow(-1.0, 0.0), style);
            assert_eq!((still.pulse_hz, still.flow), (0.0, 0.0));
            let explicit = Fx::of(&e.clone().with_glow(-1.0).with_pulse(0.75), style);
            assert_eq!((explicit.glow, explicit.pulse_hz), (0.0, 0.75));
        }
        let no_aura = Fx::of(&e.clone().with_glow(-1.0), draw_style::RIBBON_FLOW);
        assert_eq!((no_aura.glow, no_aura.flow), (0.0, 0.6));
        assert_eq!(
            Fx::of(&e.with_flow(-1.0, 0.0), draw_style::RIBBON_FLOW).flow,
            0.0
        );
    }

    #[test]
    fn energized_wall_and_tube_keep_data_geometry_and_colors_when_motion_changes() {
        let points = [[0.0, 0.0, 0.2], [2.0, 0.0, 0.3], [3.0, 1.0, 0.5]];
        let colors = [
            rgba(220, 90, 130, 170),
            rgba(60, 170, 220, 210),
            rgba(130, 210, 80, 0),
        ];
        for style in [
            draw_style::LIGHT_WALL,
            draw_style::TUBE_PULSE,
            draw_style::RIBBON_FLOW,
        ] {
            let e = DrawEntity::lines("t", "energy", &points)
                .with_style(style)
                .with_colors(&colors)
                .with_width(0.12);
            let active = build(&e.clone().with_glow(-1.0), "t", "energy");
            let static_ = build(
                &e.with_glow(-1.0).with_pulse(-1.0).with_flow(-1.0, 0.0),
                "t",
                "energy",
            );
            assert_eq!(active.strokes.a, static_.strokes.a);
            assert_eq!(active.strokes.b, static_.strokes.b);
            assert_eq!(active.strokes.radii, static_.strokes.radii);
            assert_eq!(active.strokes.color_a, static_.strokes.color_a);
            assert_eq!(active.strokes.color_b, static_.strokes.color_b);
            assert_eq!(active.fill.positions, static_.fill.positions);
            assert_eq!(active.fill.colors, static_.fill.colors);
            assert!(
                active
                    .strokes
                    .fx
                    .iter()
                    .all(|fx| fx[0] == 0.0 && (fx[1] > 0.0 || fx[2] > 0.0))
            );
            assert!(
                static_
                    .strokes
                    .fx
                    .iter()
                    .all(|fx| fx[0] == 0.0 && fx[1] == 0.0 && fx[2] == 0.0)
            );
            assert!(
                active
                    .fill
                    .fx
                    .iter()
                    .all(|fx| fx[1] == 0.0 && fx[2] == visual::ENERGY_HZ)
            );
            assert!(
                static_
                    .fill
                    .fx
                    .iter()
                    .all(|fx| fx[1] == 0.0 && fx[2] == 0.0)
            );
        }
    }

    #[test]
    fn arrows_get_a_flat_based_head_and_axes_three_arrows() {
        let arrow = build(
            &DrawEntity::arrows("t", "a", &[([0.0; 3], [2.0, 0.0, 0.0])]).with_width(0.1),
            "t",
            "a",
        );
        assert_eq!(prims(&arrow.strokes), 2);
        assert_eq!(flags_at(&arrow.strokes, 4), FLAG_FLAT_START | FLAG_HEAD);
        assert_eq!(style_at(&arrow.strokes, 0), draw_style::ARROW_GLOW);
        assert!(arrow.strokes.radii[4][0] > arrow.strokes.radii[0][0]);
        let axes = build(&DrawEntity::axes("t", "x", 1.0), "t", "x");
        assert_eq!(prims(&axes.strokes), 6);
    }

    #[test]
    fn beacon_arrow_respects_disabled_emission_without_removing_its_core() {
        let e = DrawEntity::arrows("t", "goal", &[([0.0; 3], [2.0, 0.0, 0.0])])
            .with_style(draw_style::BEACON)
            .with_glow(-1.0)
            .with_pulse(-1.0)
            .with_beam(-1.0);
        let built = build(&e, "t", "goal");
        assert_eq!(prims(&built.strokes), 2);
        assert!(
            built
                .strokes
                .fx
                .iter()
                .all(|fx| fx[0] == 0.0 && fx[1] == 0.0)
        );
        assert!(built.strokes.color_a.iter().all(|color| color[3] > 0));
        assert!(built.fill.positions.iter().all(|p| p[2] <= GROUND_LIFT));
        assert!(!built.on_top);
    }

    #[test]
    fn feature_edges_exclude_coplanar_diagonals_and_weld_duplicate_vertices() {
        let quad = UnitMesh::quad();
        let edge = feature_edges(&quad.positions, &quad.indices);
        assert_eq!(edge.len(), 4);
        assert!(
            edge.iter()
                .all(|[a, b]| (a.distance(*b) - 1.0).abs() < 1e-6)
        );
        let duplicated: Vec<_> = quad
            .indices
            .iter()
            .map(|i| quad.positions[*i as usize])
            .collect();
        assert_eq!(
            feature_edges(&duplicated, &(0..6).collect::<Vec<_>>()).len(),
            4
        );
        let cube = UnitMesh::cuboid();
        assert_eq!(feature_edges(&cube.positions, &cube.indices).len(), 12);
        assert!(
            feature_edges(
                &[Vec3::ZERO, Vec3::NAN, Vec3::ONE],
                &[0, 1, 2, 0, 1, 99, 0, 0, 0]
            )
            .is_empty()
        );
        assert!(feature_edges(&quad.positions, &vec![0; 600_003]).is_empty());
    }

    #[test]
    fn feature_edge_extraction_has_a_fixed_output_budget() {
        let positions: Vec<_> = (0..6_000)
            .flat_map(|i| {
                let x = 2.0 * i as f32;
                [
                    Vec3::new(x, 0.0, 0.0),
                    Vec3::new(x + 1.0, 0.0, 0.0),
                    Vec3::new(x, 1.0, 0.0),
                ]
            })
            .collect();
        let indices: Vec<_> = (0..positions.len() as u32).collect();
        let edges = feature_edges(&positions, &indices);
        assert_eq!(edges.len(), visual::FEATURE_EDGE_LIMIT);
        assert!(edges.iter().flatten().all(|p| p.is_finite()));
    }

    #[test]
    fn explicit_corner_frames_work_on_plain_detection_boxes_and_image_loops() {
        let rect = [
            [-1.0, -1.0, 2.0],
            [1.0, -1.0, 2.0],
            [1.0, 1.0, 2.0],
            [-1.0, 1.0, 2.0],
        ];
        for (entity, edges) in [
            (
                DrawEntity::lines("t", "image", &rect).with_sub(line_mode::LOOP),
                4,
            ),
            (
                DrawEntity::shapes(
                    "t",
                    "box",
                    shape_kind::BOX,
                    &[Instance::at([0.0; 3], [2.0; 3])],
                )
                .with_fill(draw_flag::FILL_WIRE),
                12,
            ),
        ] {
            let entity = entity
                .with_style(draw_style::PLAIN)
                .with_width(-1.0)
                .with_color(0x8050c080);
            let ordinary = build(&entity, "t", "frame");
            let framed = build(
                &entity.clone().with_prop("corner_frame", "true"),
                "t",
                "frame",
            );
            assert_eq!(prims(&ordinary.strokes), edges);
            assert_eq!(prims(&framed.strokes), 3 * edges);
            assert!(framed.fill.is_empty());
            assert!(framed.strokes.fx.iter().all(|fx| fx[0] == 0.0));
            assert!(
                framed
                    .strokes
                    .color_a
                    .iter()
                    .all(|color| *color == unpack_rgba(entity.color))
            );
            let off = build(&entity.with_prop("corner_frame", "false"), "t", "frame");
            assert_eq!(off.strokes.a, ordinary.strokes.a);
            assert_eq!(off.strokes.radii, ordinary.strokes.radii);
        }
    }

    #[test]
    fn corner_frames_preserve_vertex_data_and_leave_mixed_units_unmodified() {
        let points = [
            [0.0, 0.0, 1.0],
            [2.0, 0.0, 1.0],
            [2.0, 2.0, 1.0],
            [0.0, 2.0, 1.0],
        ];
        let colors = [0x8050c080, 0xc0d02080, 0xff30c0f0, 0x40908070];
        for radii in [[0.02, 0.04, 0.06, 0.08], [-0.5, -1.0, -1.5, -2.0]] {
            let e = DrawEntity::lines("t", "frame", &points)
                .with_sub(line_mode::LOOP)
                .with_style(draw_style::PLAIN)
                .with_colors(&colors)
                .with_radii(&radii)
                .with_prop("corner_frame", "true")
                .with_flags(draw_flag::DASHED);
            let built = build(&e, "t", "frame");
            assert_eq!(prims(&built.strokes), 12);
            for i in 0..4 {
                let vertex = 12 * i;
                assert_eq!(built.strokes.a[vertex], points[i]);
                assert_eq!(built.strokes.color_a[vertex], unpack_rgba(colors[i]));
                assert_eq!(
                    built.strokes.radii[vertex][0],
                    radii[i] * visual::STRUCTURE_CORNER_GAIN
                );
                assert_eq!(built.strokes.fx2[vertex][1], 8.0);
                assert_ne!(flags_at(&built.strokes, vertex) & FLAG_DASHED, 0);
                assert_eq!(built.strokes.dist[vertex][0], 2.0 * i as f32);
            }
        }
        for entity in [
            DrawEntity::lines("t", "mixed", &points)
                .with_sub(line_mode::LOOP)
                .with_radii(&[0.02, -1.0, 0.03, -2.0]),
            DrawEntity::lines("t", "open", &points),
            DrawEntity::lines("t", "degenerate", &points[..2]).with_sub(line_mode::LOOP),
            DrawEntity::strips(
                "t",
                "dense",
                &vec![points.to_vec(); visual::STRUCTURE_LIMIT + 1],
            )
            .with_sub(line_mode::LOOP),
        ] {
            let entity = entity.with_style(draw_style::PLAIN);
            let ordinary = build(&entity, "t", "frame");
            let framed = build(&entity.with_prop("corner_frame", "true"), "t", "frame");
            assert_eq!(framed.strokes.a, ordinary.strokes.a);
            assert_eq!(framed.strokes.b, ordinary.strokes.b);
            assert_eq!(framed.strokes.radii, ordinary.strokes.radii);
            assert_eq!(framed.strokes.dist, ordinary.strokes.dist);
        }
    }

    #[test]
    fn curved_shapes_share_subtle_arc_accents_without_moving_the_outline() {
        assert!(visual::STRUCTURE_CORNER_GAIN > 1.0 && visual::STRUCTURE_CORNER_GAIN < 2.0);
        for shape in [
            shape_kind::ELLIPSOID,
            shape_kind::ELLIPSE,
            shape_kind::CYLINDER,
            shape_kind::CONE,
            shape_kind::CAPSULE,
        ] {
            for style in [draw_style::HOLO, draw_style::GHOST, draw_style::SHIELD] {
                for radius in [-1.0, 0.02] {
                    let e = DrawEntity::shapes(
                        "t",
                        "curves",
                        shape,
                        &[Instance::at([0.0; 3], [2.0, 2.0, 3.0])],
                    )
                    .with_style(style)
                    .with_fill(draw_flag::FILL_BOTH)
                    .with_radii(&[radius])
                    .with_beam(-1.0)
                    .with_color(0x8040c0ff)
                    .with_color2(0xc0ff8030);
                    let built = build(&e, "t", "curves");
                    let mut total = 0.0;
                    let mut bold = 0.0;
                    for i in (0..built.strokes.a.len()).step_by(4) {
                        let a = Vec3::from_array(built.strokes.a[i]);
                        let b = Vec3::from_array(built.strokes.b[i]);
                        let length = a.distance(b);
                        total += length;
                        if built.strokes.radii[i] == [radius * visual::STRUCTURE_CORNER_GAIN; 2] {
                            bold += length;
                        } else {
                            assert_eq!(built.strokes.radii[i], [radius; 2]);
                        }
                        assert_eq!(
                            built.strokes.color_a[i],
                            alpha_scaled(unpack_rgba(e.color2), 128)
                        );
                    }
                    let expected: f32 = outline_of(shape, Vec3::new(2.0, 2.0, 3.0))
                        .into_iter()
                        .map(|(p, closed)| {
                            p.windows(2).map(|w| w[0].distance(w[1])).sum::<f32>()
                                + if closed {
                                    p[0].distance(*p.last().unwrap())
                                } else {
                                    0.0
                                }
                        })
                        .sum();
                    assert!(
                        (total - expected).abs() < 0.001,
                        "exact outline length for {shape}"
                    );
                    assert!(
                        (bold / total - 2.0 * BRACKET_FRACTION).abs() < 0.0001,
                        "short arcs for {shape}"
                    );
                    assert!(!built.fill.is_empty());
                    assert!(built.fill.local.iter().all(|p| p[3] >= LOCAL_OUTLINE));
                    let solid = build(&e.with_fill(draw_flag::FILL_SOLID), "t", "curves");
                    assert!(solid.fill.local.iter().all(|p| p[3] >= LOCAL_OUTLINE));
                    assert_eq!(solid.strokes.radii, built.strokes.radii);
                }
            }
        }
    }

    #[test]
    fn sectors_and_frustums_share_two_line_weights_without_duplicate_borders() {
        for (shape, size) in [
            (shape_kind::FRUSTUM, [1.0, 0.8, 3.0]),
            (shape_kind::SECTOR, [3.0, 2.0, 0.0]),
            (shape_kind::SECTOR, [3.0, 2.0, 1.0]),
            (shape_kind::SECTOR, [3.0, TAU, 0.0]),
            (shape_kind::SECTOR, [3.0, TAU, 1.0]),
        ] {
            let e = DrawEntity::shapes("t", "extent", shape, &[Instance::at([0.0; 3], size)])
                .with_style(draw_style::SWEEP)
                .with_fill(draw_flag::FILL_WIRE)
                .with_width(-2.0);
            let built = build(&e, "t", "extent");
            assert!(built.fill.is_empty());
            assert!(built.strokes.radii.contains(&[-1.0; 2]));
            assert!(
                built
                    .strokes
                    .radii
                    .contains(&[-visual::STRUCTURE_CORNER_GAIN; 2])
            );
            assert!(
                built
                    .strokes
                    .a
                    .iter()
                    .zip(&built.strokes.b)
                    .all(|(a, b)| Vec3::from_array(*a).distance(Vec3::from_array(*b)) > 1e-6)
            );
        }
    }

    #[test]
    fn polygon_outer_and_hole_boundaries_share_accents_and_keep_plain_opt_out() {
        let e = DrawEntity::polygon("t", "region", &[square(0.0, 4.0), square(1.0, 2.0)])
            .with_style(draw_style::DECAL)
            .with_fill(draw_flag::FILL_BOTH)
            .with_width(-2.0)
            .with_color(0x8040c0ff)
            .with_color2(0xc0ff8030);
        let built = build(&e, "t", "region");
        assert_eq!(prims(&built.strokes), 24);
        assert!(built.strokes.radii.contains(&[-1.0; 2]));
        assert!(
            built
                .strokes
                .radii
                .contains(&[-visual::STRUCTURE_CORNER_GAIN; 2])
        );
        assert!(
            built
                .strokes
                .color_a
                .iter()
                .all(|c| *c == alpha_scaled(unpack_rgba(e.color2), 128))
        );
        let plain = build(&e.clone().with_style(draw_style::PLAIN), "t", "region");
        assert_eq!(prims(&plain.strokes), 8);
        let explicit = build(
            &e.with_style(draw_style::PLAIN)
                .with_prop("corner_frame", "true"),
            "t",
            "region",
        );
        assert_eq!(explicit.strokes.a, built.strokes.a);
        assert_eq!(explicit.strokes.b, built.strokes.b);
    }

    #[test]
    fn decal_shape_boundaries_share_weights_without_extra_volume_outlines() {
        for (shape, edges) in [
            (shape_kind::RECT, 4),
            (shape_kind::BOX, 4),
            (shape_kind::ELLIPSE, 64),
            (shape_kind::ELLIPSOID, 64),
        ] {
            for radius in [-1.0, 0.02] {
                let entity = DrawEntity::shapes(
                    "t",
                    "decal",
                    shape,
                    &[Instance::at([1.0, 2.0, 3.0], [4.0, 2.0, 2.0])],
                )
                .with_style(draw_style::DECAL)
                .with_fill(draw_flag::FILL_BOTH)
                .with_radii(&[radius])
                .with_color(0x8040c0ff)
                .with_color2(0xc0ff8030);
                let built = build(&entity, "t", "decal");
                assert_eq!(prims(&built.strokes), edges + 8);
                assert!(built.strokes.radii.contains(&[radius; 2]));
                assert!(
                    built
                        .strokes
                        .radii
                        .contains(&[radius * visual::STRUCTURE_CORNER_GAIN; 2])
                );
                assert!(
                    built
                        .strokes
                        .color_a
                        .iter()
                        .all(|c| *c == alpha_scaled(unpack_rgba(entity.color2), 128))
                );
                let z = if matches!(shape, shape_kind::RECT | shape_kind::ELLIPSE) {
                    3.0
                } else {
                    2.0
                };
                assert!(
                    built.strokes.a.iter().chain(&built.strokes.b).all(|p| (p[2]
                        - z
                        - GROUND_LIFT)
                        .abs()
                        < 1e-5)
                );
                let wire = build(&entity.with_fill(draw_flag::FILL_WIRE), "t", "decal");
                assert!(wire.fill.is_empty());
                assert_eq!(wire.strokes.a, built.strokes.a);
            }
        }
    }

    #[test]
    fn sector_defaults_include_the_center_and_explicit_inner_extents_survive() {
        for style in [draw_style::PLAIN, draw_style::DECAL, draw_style::SWEEP] {
            for (instance_inner, entity_inner, expected) in
                [(0.0, 0.0, 0.0), (0.0, 0.5, 0.5), (0.8, 0.5, 0.8)]
            {
                let e = DrawEntity::shapes(
                    "t",
                    "sector",
                    shape_kind::SECTOR,
                    &[Instance::at([0.0; 3], [2.0, 1.5, instance_inner])],
                )
                .with_style(style)
                .with_fill(draw_flag::FILL_BOTH);
                assert_eq!(e.sz, 0.0);
                let e = if entity_inner > 0.0 {
                    e.with_scale([1.0, 1.0, entity_inner])
                } else {
                    e
                };
                let built = build(&e, "t", "sector");
                let inner = built
                    .fill
                    .positions
                    .iter()
                    .map(|p| Vec2::new(p[0], p[1]).length())
                    .fold(f32::INFINITY, f32::min);
                assert!(
                    (inner - expected).abs() < 1e-5,
                    "style {style}: {inner} != {expected}"
                );
            }
        }
    }

    #[test]
    fn sector_fills_preserve_inner_holes_and_exact_area_when_rotated() {
        for style in [draw_style::PLAIN, draw_style::DECAL, draw_style::SWEEP] {
            for angle in [1.5, 4.5, TAU] {
                let rotation = Quat::from_rotation_y(0.8);
                let mut inst = Instance::at([0.0; 3], [3.0, angle, 1.2]);
                inst.rotation = [rotation.w, rotation.x, rotation.y, rotation.z];
                let entity = DrawEntity::shapes("t", "annular-sector", shape_kind::SECTOR, &[inst])
                    .with_style(style)
                    .with_fill(draw_flag::FILL_BOTH);
                let built = build(&entity, "t", "annular-sector");
                let segments = ((angle / TAU) * 96.0).ceil().max(6.0);
                let expected = 0.5 * segments * (angle / segments).sin() * (9.0 - 1.44);
                let inner_chord = 1.2 * (0.5 * angle / segments).cos();
                let mut area = 0.0;
                for triangle in built.fill.indices.chunks_exact(3) {
                    let p = |k: usize| {
                        rotation.inverse()
                            * Vec3::from_array(built.fill.positions[triangle[k] as usize])
                    };
                    let (a, b, c) = (p(0), p(1), p(2));
                    area += 0.5 * (b - a).cross(c - a).length();
                    assert!(
                        ((a + b + c) / 3.0).truncate().length() >= inner_chord - 1e-4,
                        "filled hole: {style} {angle}"
                    );
                }
                assert!(
                    (area - expected).abs() < 0.002,
                    "sector area {area} != {expected}: {style} {angle}"
                );
                if angle == TAU {
                    let length: f32 = built
                        .strokes
                        .a
                        .iter()
                        .zip(&built.strokes.b)
                        .step_by(4)
                        .map(|(a, b)| Vec3::from_array(*a).distance(Vec3::from_array(*b)))
                        .sum();
                    let perimeter = segments * 2.0 * (std::f32::consts::PI / segments).sin() * 4.2;
                    assert!(
                        (length - perimeter).abs() < 0.001,
                        "no radial bridges across a full annulus: {style}"
                    );
                }
            }
        }
    }

    #[test]
    fn energy_box_outline_has_three_bolder_arms_at_every_vertex() {
        for style in [
            draw_style::HOLO,
            draw_style::GHOST,
            draw_style::SHIELD,
            draw_style::FIELD,
        ] {
            for fill in [draw_flag::FILL_WIRE, draw_flag::FILL_BOTH] {
                let e = DrawEntity::shapes(
                    "t",
                    "corners",
                    shape_kind::BOX,
                    &[Instance::at([0.0; 3], [2.0; 3])],
                )
                .with_style(style)
                .with_fill(fill)
                .with_width(-2.0)
                .with_beam(-1.0)
                .with_glow(-1.0);
                let built = build(&e, "t", "corners");
                assert_eq!(
                    prims(&built.strokes),
                    36,
                    "one partitioned outline: {style} {fill}"
                );
                let mut corners = std::collections::BTreeMap::new();
                let mut middle = 0;
                let mut total = 0.0;
                for i in (0..built.strokes.a.len()).step_by(4) {
                    let a = Vec3::from_array(built.strokes.a[i]);
                    let b = Vec3::from_array(built.strokes.b[i]);
                    total += a.distance(b);
                    let radius = built.strokes.radii[i];
                    if radius == [-visual::STRUCTURE_CORNER_GAIN; 2] {
                        let vertex = [a, b].into_iter().find(|p| p.abs() == Vec3::ONE).unwrap();
                        let key = vertex.to_array().map(|v| v as i32);
                        let axes = corners.entry(key).or_insert([0; 3]);
                        for axis in 0..3 {
                            axes[axis] += usize::from((a[axis] - b[axis]).abs() > 1e-5);
                        }
                        assert!((a.distance(b) - 2.0 * BRACKET_FRACTION).abs() < 1e-5);
                    } else {
                        assert_eq!(radius, [-1.0; 2]);
                        assert!(a.abs() != Vec3::ONE && b.abs() != Vec3::ONE);
                        middle += 1;
                    }
                    assert_eq!(built.strokes.fx[i][0], 0.0);
                }
                assert_eq!(corners.len(), 8);
                assert!(corners.values().all(|axes| *axes == [1; 3]));
                assert_eq!(middle, 12);
                assert!((total - 24.0).abs() < 1e-4, "no stacked edge coverage");
                assert_eq!(built.fill.is_empty(), fill == draw_flag::FILL_WIRE);
                for i in 0..12 {
                    assert_eq!(built.strokes.fx2[4 * i][1], 8.0, "continuous ring length");
                    if i > 0 {
                        assert_eq!(built.strokes.a[4 * i], built.strokes.b[4 * (i - 1)]);
                        assert_eq!(
                            built.strokes.dist[4 * i][0],
                            built.strokes.dist[4 * (i - 1)][1]
                        );
                        assert_eq!(built.strokes.prev[4 * i], built.strokes.a[4 * (i - 1)]);
                    }
                }
            }
        }
    }

    #[test]
    fn shape_structure_keeps_true_bounds_widths_and_batched_density_budget() {
        let q = Quat::from_euler(EulerRot::ZYX, 0.4, -0.2, 0.3);
        let instance = Instance::at([4.0, -2.0, 1.5], [2.0, 4.0, 3.0]).turned([q.w, q.x, q.y, q.z]);
        let e = DrawEntity::shapes("t", "shape", shape_kind::BOX, &[instance])
            .with_style(draw_style::HOLO)
            .with_width(-2.0)
            .with_glow(-1.0)
            .with_beam(-1.0)
            .with_color(rgba(180, 70, 210, 180));
        let built = build(&e, "t", "shape");
        assert_eq!(prims(&built.strokes), 24);
        assert!(!built.on_top);
        assert!(
            built
                .strokes
                .radii
                .iter()
                .all(|r| *r == [-visual::STRUCTURE_CORNER_GAIN; 2])
        );
        assert!(built.strokes.fx.iter().all(|fx| fx[0] == 0.0));
        let (rotation, position) = pose_of(&instance);
        let half = Vec3::from_array(instance.size) * 0.5;
        for point in built.strokes.a.iter().chain(&built.strokes.b) {
            let local = rotation.inverse() * (Vec3::from_array(*point) - position);
            assert!(local.abs().cmple(half + Vec3::splat(1e-5)).all());
            assert!(
                local.abs().abs_diff_eq(half, 1e-5)
                    || (local.abs() - half).abs().min_element() < 1e-5
            );
        }
        let dense = build(
            &e.with_instances(&vec![instance; visual::STRUCTURE_LIMIT + 1]),
            "t",
            "shape",
        );
        assert!(dense.strokes.is_empty());
        assert_eq!(
            dense.fill.positions.len(),
            24 * (visual::STRUCTURE_LIMIT + 1)
        );
    }

    #[test]
    fn disabled_and_degenerate_beams_do_not_allocate_surface_geometry() {
        let mut surface = Surfaces::default();
        for (radius, height) in [
            (0.25, 0.0),
            (0.25, -1.0),
            (0.0, 1.0),
            (-0.25, 1.0),
            (0.25, f32::NAN),
            (f32::INFINITY, 1.0),
        ] {
            surface.column(Vec3::ZERO, radius, height, [255; 4]);
            assert!(surface.is_empty());
            assert!(surface.positions.is_empty());
        }
        surface.column(Vec3::ZERO, 0.25, 1.0, [255; 4]);
        assert_eq!(surface.positions.len(), 82);
        assert_eq!(surface.indices.len(), 240);
    }

    #[test]
    fn hologram_projector_is_present_by_default_and_explicitly_removable() {
        let e = DrawEntity::shapes(
            "t",
            "holo",
            shape_kind::BOX,
            &[Instance::at([0.0; 3], [1.0; 3])],
        )
        .with_style(draw_style::HOLO);
        let built = build(&e, "t", "holo");
        assert!(built.fill.fx.iter().any(|fx| fx[0] == LOOK_BEAM));
        assert_eq!(built.fill.positions.len(), 24 + 82);
        let top = built
            .fill
            .positions
            .iter()
            .map(|p| p[2])
            .fold(f32::MIN, f32::max);
        assert!((top - 1.3).abs() < 1e-6);
        let disabled = build(&e.with_beam(-1.0), "t", "holo");
        assert_eq!(disabled.fill.positions.len(), 24);
        assert!(disabled.fill.fx.iter().all(|fx| fx[0] == LOOK_HOLO));
    }

    #[test]
    fn shape_structure_preserves_per_item_metre_and_pixel_radii() {
        let instances: Vec<_> = (0..4)
            .map(|i| Instance::at([2.0 * i as f32, 0.0, 0.0], [1.0; 3]))
            .collect();
        let e = DrawEntity::shapes("t", "shape", shape_kind::BOX, &instances)
            .with_style(draw_style::HOLO)
            .with_radii(&[0.025, -1.5, 0.0]);
        let built = build(&e, "t", "shape");
        let vertices = 24 * 4;
        assert!(
            built.strokes.radii[..vertices]
                .iter()
                .all(|r| *r == [0.025 * visual::STRUCTURE_CORNER_GAIN; 2])
        );
        assert!(
            built.strokes.radii[vertices..2 * vertices]
                .iter()
                .all(|r| *r == [-1.5 * visual::STRUCTURE_CORNER_GAIN; 2])
        );
        assert!(
            built.strokes.radii[2 * vertices..]
                .iter()
                .all(|r| *r == [-0.5 * visual::RULE_PX * visual::STRUCTURE_CORNER_GAIN; 2])
        );
    }

    #[test]
    fn rectangular_corner_frames_share_width_units_and_keep_dense_outlines() {
        let instances: Vec<_> = (0..3)
            .map(|i| Instance::at([3.0 * i as f32, 0.0, 0.0], [2.0; 3]))
            .collect();
        for shape in [shape_kind::BOX, shape_kind::RECT] {
            for style in [
                draw_style::HOLO,
                draw_style::GHOST,
                draw_style::SHIELD,
                draw_style::FIELD,
            ] {
                let e = DrawEntity::shapes("t", "frame", shape, &instances)
                    .with_style(style)
                    .with_fill(draw_flag::FILL_BOTH)
                    .with_radii(&[0.025, -1.5, 0.0])
                    .with_beam(-1.0)
                    .with_color(0x8040c0ff)
                    .with_color2(0xc0ff8030);
                let built = build(&e, "t", "frame");
                let edges = if shape == shape_kind::BOX { 12 } else { 4 };
                assert_eq!(prims(&built.strokes), 3 * 3 * edges);
                for (item, radius) in [0.025, -1.5, -0.5 * visual::RULE_PX]
                    .into_iter()
                    .enumerate()
                {
                    for edge in 0..edges {
                        for (part, gain) in [
                            visual::STRUCTURE_CORNER_GAIN,
                            1.0,
                            visual::STRUCTURE_CORNER_GAIN,
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            let vertex = 4 * (item * edges * 3 + edge * 3 + part);
                            assert_eq!(built.strokes.radii[vertex], [radius * gain; 2]);
                            assert_eq!(
                                built.strokes.color_a[vertex],
                                alpha_scaled(unpack_rgba(e.color2), 128)
                            );
                        }
                    }
                }
                let dense = build(
                    &e.with_instances(&vec![instances[0]; visual::STRUCTURE_LIMIT + 1]),
                    "t",
                    "frame",
                );
                assert_eq!(prims(&dense.strokes), edges * (visual::STRUCTURE_LIMIT + 1));
                assert!(!dense.fill.is_empty());
            }
        }
    }

    #[test]
    fn loaded_mesh_structure_uses_real_edges_and_reuses_extraction() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/draw_vfx/faceted.obj"
        );
        let raw = super::super::assets::load(path).unwrap();
        let edges = cached_feature_edges(&raw);
        assert!(std::sync::Arc::ptr_eq(&edges, &cached_feature_edges(&raw)));
        assert_eq!(edges.len(), 8);
        let q = Quat::from_rotation_z(0.4);
        let instances = [
            Instance::at([3.0, -1.0, 2.0], [2.0, 0.5, 1.5]).turned([q.w, q.x, q.y, q.z]),
            Instance::at([-2.0, 3.0, 1.0], [1.0; 3]),
        ];
        let e = DrawEntity::mesh("t", "mesh", path)
            .with_instances(&instances)
            .with_style(draw_style::GHOST)
            .with_radii(&[0.03, -1.0])
            .with_glow(-1.0)
            .with_flags(draw_flag::ADDITIVE);
        let built = build(&e, "t", "mesh");
        assert_eq!(prims(&built.strokes), 48);
        for (i, instance) in instances.iter().enumerate() {
            let (rotation, position) = pose_of(instance);
            let size = Vec3::from_array(instance.size);
            for (k, ([a, b], _)) in edges.iter().enumerate() {
                let vertex = 12 * (i * edges.len() + k);
                assert!(
                    Vec3::from_array(built.strokes.a[vertex])
                        .abs_diff_eq(position + rotation * (*a * size), 1e-5)
                );
                assert!(
                    Vec3::from_array(built.strokes.b[vertex + 8])
                        .abs_diff_eq(position + rotation * (*b * size), 1e-5)
                );
                for (part, gain) in [
                    visual::STRUCTURE_CORNER_GAIN,
                    1.0,
                    visual::STRUCTURE_CORNER_GAIN,
                ]
                .into_iter()
                .enumerate()
                {
                    assert_eq!(
                        built.strokes.radii[vertex + 4 * part],
                        [[0.03 * gain; 2], [-gain; 2]][i]
                    );
                }
                assert_ne!(flags_at(&built.strokes, vertex) & FLAG_ADD, 0);
                assert_eq!(built.strokes.fx[vertex][0], 0.0);
            }
        }
        let plain = build(&e.with_style(draw_style::PLAIN), "t", "mesh");
        assert!(plain.strokes.is_empty());
        assert_eq!(plain.fill.positions, built.fill.positions);
    }

    #[test]
    fn shapes_fill_outline_or_both() {
        let one = [Instance::at([0.0; 3], [1.0, 2.0, 0.5])];
        let shape = |fill| {
            build(
                &DrawEntity::shapes("t", "s", shape_kind::BOX, &one).with_fill(fill),
                "t",
                "s",
            )
        };
        let solid = shape(draw_flag::FILL_SOLID);
        assert!(!solid.fill.is_empty() && solid.strokes.is_empty());
        let wire = shape(draw_flag::FILL_WIRE);
        assert!(wire.fill.is_empty() && prims(&wire.strokes) == 12);
        let both = shape(draw_flag::FILL_BOTH);
        assert!(!both.fill.is_empty() && !both.strokes.is_empty());
        let extent = solid
            .fill
            .positions
            .iter()
            .fold(Vec3::ZERO, |m, p| m.max(Vec3::from_array(*p)));
        assert_eq!(extent, Vec3::new(0.5, 1.0, 0.25));
        for kind in [
            shape_kind::ELLIPSOID,
            shape_kind::CYLINDER,
            shape_kind::CONE,
            shape_kind::CAPSULE,
            shape_kind::ELLIPSE,
        ] {
            let b = build(
                &DrawEntity::shapes("t", "s", kind, &one).with_fill(draw_flag::FILL_BOTH),
                "t",
                "s",
            );
            assert!(
                !b.fill.is_empty() && !b.strokes.is_empty(),
                "{}",
                shape_kind::name(kind)
            );
        }
    }

    #[test]
    fn wire_shapes_never_emit_surface_or_beam_geometry_for_any_energy_style() {
        for style in [
            draw_style::PLAIN,
            draw_style::HOLO,
            draw_style::GHOST,
            draw_style::SHIELD,
            draw_style::ALERT,
            draw_style::FIELD,
            draw_style::BEACON,
            draw_style::DECAL,
            draw_style::SWEEP,
        ] {
            for shape in [
                shape_kind::BOX,
                shape_kind::ELLIPSOID,
                shape_kind::CYLINDER,
                shape_kind::CONE,
                shape_kind::CAPSULE,
                shape_kind::ELLIPSE,
                shape_kind::RECT,
                shape_kind::FRUSTUM,
                shape_kind::SECTOR,
            ] {
                let e = DrawEntity::shapes(
                    "t",
                    "wire",
                    shape,
                    &[Instance::at([0.0, 0.0, 1.0], [1.2, 1.0, 2.0])],
                )
                .with_style(style)
                .with_fill(draw_flag::FILL_WIRE)
                .with_beam(2.0)
                .with_color(gearbox_api::draw_color::MAGENTA)
                .with_color2(0xffa0d040);
                let built = build(&e, "t", "wire");
                assert!(
                    built.fill.is_empty(),
                    "wire surface: style {style}, shape {shape}"
                );
                assert!(
                    !built.strokes.is_empty(),
                    "missing outline: style {style}, shape {shape}"
                );
                assert!(
                    built
                        .strokes
                        .color_a
                        .iter()
                        .all(|c| *c == unpack_rgba(e.color)),
                    "wire primary tint: {style}, {shape}"
                );
            }
        }
    }

    #[test]
    fn both_uses_secondary_outline_without_duplicate_structure_and_keeps_surface_energy() {
        for (shape, style) in [
            (shape_kind::BOX, draw_style::HOLO),
            (shape_kind::BOX, draw_style::GHOST),
            (shape_kind::BOX, draw_style::SHIELD),
            (shape_kind::RECT, draw_style::FIELD),
            (shape_kind::SECTOR, draw_style::SWEEP),
            (shape_kind::FRUSTUM, draw_style::SWEEP),
            (shape_kind::RECT, draw_style::DECAL),
            (shape_kind::SECTOR, draw_style::DECAL),
        ] {
            let e = DrawEntity::shapes(
                "t",
                "both",
                shape,
                &[Instance::at([0.0, 0.0, 1.0], [1.2, 1.0, 2.0])],
            )
            .with_style(style)
            .with_fill(draw_flag::FILL_BOTH)
            .with_beam(-1.0)
            .with_color(0x8040c0ff)
            .with_color2(0xc0ff8030)
            .with_pulse(0.7)
            .with_glow(0.3);
            let built = build(&e, "t", "both");
            assert!(
                !built.fill.is_empty() && !built.strokes.is_empty(),
                "{style} {shape}"
            );
            assert!(
                built
                    .strokes
                    .color_a
                    .iter()
                    .all(|c| *c == alpha_scaled(unpack_rgba(e.color2), 128)),
                "outline RGBA: {style} {shape}"
            );
            assert!(built.fill.colors.iter().all(|c| *c == unpack_rgba(e.color)));
            assert!(built.fill.fx.iter().all(|fx| fx[2] == 0.7));
            if shape == shape_kind::BOX {
                assert_eq!(
                    prims(&built.strokes),
                    36,
                    "three non-overlapping sections per edge"
                );
                assert!(built.fill.local.iter().all(|p| p[3] >= LOCAL_OUTLINE));
            }
            if shape == shape_kind::FRUSTUM {
                assert_eq!(
                    prims(&built.strokes),
                    36,
                    "partitioned near and far borders"
                );
                assert!(
                    built
                        .fill
                        .local
                        .iter()
                        .zip(&built.fill.fx)
                        .filter(|(_, fx)| fx[0] == LOOK_PLANE)
                        .all(|(p, _)| p[3] >= LOCAL_OUTLINE)
                );
            }
        }
    }

    #[test]
    fn energy_shapes_do_not_stack_full_outlines_or_ground_rings() {
        for style in [draw_style::HOLO, draw_style::GHOST, draw_style::SHIELD] {
            for (shape, corners) in [
                (shape_kind::BOX, 24),
                (shape_kind::RECT, 8),
                (shape_kind::ELLIPSOID, 3 * (ROUND_SEGMENTS + 8)),
            ] {
                let entity = DrawEntity::shapes(
                    "t",
                    "surface",
                    shape,
                    &[Instance::at([0.0, 0.0, 1.0], [2.0; 3])],
                )
                .with_style(style)
                .with_beam(-1.0)
                .with_color(gearbox_api::draw_color::MAGENTA);
                let built = build(&entity, "t", "surface");
                assert_eq!(prims(&built.strokes), corners, "{style} {shape}");
                assert!(!built.fill.is_empty());
                assert!(built.fill.fx.iter().all(|fx| fx[0] == built.fill.fx[0][0]));
            }
        }
    }

    #[test]
    fn a_ring_point_is_one_machine_disc_carrying_its_mark() {
        for (mark, value) in [("hover", 0.0), ("selected", 1.0), ("following", 2.0)] {
            let e = DrawEntity::points("t", "r", &[[0.0; 3]])
                .with_style(draw_style::RING)
                .with_width(6.0)
                .with_prop("mark", mark);
            let built = build(&e, "t", "r");
            assert!(built.strokes.is_empty(), "a ring draws no point glyph");
            assert!(!built.fill.fx.is_empty());
            assert!(built.fill.fx.iter().all(|fx| fx[0] == LOOK_MACHINE));
            for fx2 in &built.fill.fx2 {
                assert_eq!(fx2[3], value, "{mark}");
                assert!((fx2[1] - 3.0 * RING_REACH).abs() < 1e-5, "the disc reaches past the rim");
            }
        }
    }

    #[test]
    fn goals_keep_their_beam_and_ring_shields_use_only_their_surface() {
        let goal = [Instance::at([0.0, 0.0, 0.5], [1.0, 1.0, 1.0])];
        let beacon = build(
            &DrawEntity::shapes("t", "g", shape_kind::BOX, &goal)
                .with_style(draw_style::BEACON)
                .with_beam(6.0),
            "t",
            "g",
        );
        let looks: Vec<f32> = beacon.fill.fx.iter().map(|f| f[0]).collect();
        assert!(looks.contains(&LOOK_RING) && looks.contains(&LOOK_BEAM));
        let top = beacon
            .fill
            .positions
            .iter()
            .map(|p| p[2])
            .fold(f32::MIN, f32::max);
        assert_eq!(top, 6.0, "the beam rises from the floor of the box");
        assert!(beacon.fill.translucent());
        let pred = [Instance::at([2.0, 0.0, 0.0], [2.0, 1.0, 0.0])];
        let shield = build(
            &DrawEntity::shapes("lab/obstacles", "pred", shape_kind::ELLIPSE, &pred),
            "lab/obstacles",
            "pred",
        );
        let looks: Vec<f32> = shield.fill.fx.iter().map(|f| f[0]).collect();
        assert!(looks.iter().all(|look| *look == LOOK_SHIELD));
        let dome = shield
            .fill
            .positions
            .iter()
            .map(|p| p[2])
            .fold(f32::MIN, f32::max);
        assert!(
            (dome - 0.6).abs() < 1e-4,
            "a dome 0.6 of its reach tall, got {dome}"
        );
    }

    #[test]
    fn traces_stand_as_walls_footprints_are_thin_outlines() {
        let pts = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]];
        let trace = build(&DrawEntity::lines("lab", "trace", &pts), "lab", "trace");
        assert!(trace.fill.fx.iter().all(|f| f[0] == LOOK_WALL));
        assert_eq!(
            trace
                .fill
                .positions
                .iter()
                .map(|p| p[2])
                .fold(f32::MIN, f32::max),
            0.25
        );
        let corners = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        let footprint = build(
            &DrawEntity::lines("lab", "footprint", &corners).with_sub(line_mode::LOOP),
            "lab",
            "footprint",
        );
        assert!(footprint.fill.is_empty());
        assert_eq!(prims(&footprint.strokes), 4);
    }

    #[test]
    fn light_walls_carry_color_gradients_and_a_continuous_crest() {
        let points = [[0.0, 0.0, 0.1], [2.0, 0.0, 0.4], [2.0, 1.0, 0.2]];
        let colors = [
            rgba(64, 160, 255, 180),
            rgba(32, 220, 160, 220),
            rgba(220, 100, 64, 140),
        ];
        let e = DrawEntity::lines("t", "trace", &points)
            .with_colors(&colors)
            .with_radii(&[-2.0, -3.0, -4.0])
            .with_beam(0.6);
        let built = build(&e, "t", "trace");
        assert_eq!(prims(&built.strokes), 4);
        assert_eq!(built.strokes.radii[0], [-2.0, -3.0]);
        for (got, expected) in built.strokes.radii[8].into_iter().zip([-1.6, -2.4]) {
            assert!((got - expected).abs() < 1e-6);
        }
        for k in 0..points.len() - 1 {
            let at = 4 * k;
            assert_eq!(built.fill.colors[at], built.strokes.color_a[at]);
            assert_eq!(built.fill.colors[at + 1], built.strokes.color_b[at]);
            assert!((built.fill.positions[at + 3][2] - points[k][2] - 0.6).abs() < 1e-6);
            assert_eq!(built.fill.local[at + 3][1], 1.0);
        }
        assert_eq!(built.strokes.color_a[8], built.strokes.color_a[0]);
        assert_eq!(built.fill.normals[1], built.fill.normals[4]);
        assert_eq!(built.fill.normals[2], built.fill.normals[7]);
        assert!(built.fill.fx.iter().all(|fx| fx[2] == visual::ENERGY_HZ));
        assert_eq!(built.fill.fx[0][3], built.strokes.fx2[0][3]);
    }

    #[test]
    fn disabled_pulse_freezes_walls_and_hologram_beams() {
        let wall = DrawEntity::lines("t", "trace", &[[0.0; 3], [2.0, 0.0, 0.0]]).with_pulse(-1.0);
        let built = build(&wall, "t", "trace");
        assert!(built.fill.fx.iter().all(|fx| fx[2] == 0.0));
        assert!(built.strokes.fx.iter().all(|fx| fx[1] == 0.0));
        let hidden = build(&wall.with_beam(-1.0), "t", "trace");
        assert!(hidden.fill.is_empty());
        assert_eq!(prims(&hidden.strokes), 1);
        let box_ = DrawEntity::shapes(
            "t",
            "holo",
            shape_kind::BOX,
            &[Instance::at([0.0; 3], [1.0; 3])],
        )
        .with_pulse(-1.0)
        .with_beam(2.0);
        let holo = build(&box_, "t", "holo");
        assert!(holo.fill.fx.iter().any(|fx| fx[0] == LOOK_BEAM));
        assert!(holo.fill.fx.iter().all(|fx| fx[2] == 0.0));
    }

    #[test]
    fn decal_surfaces_are_static_by_default_and_honor_explicit_pulse_without_aura() {
        let entities = [
            DrawEntity::lines(
                "t",
                "decal",
                &[[0.0; 3], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
            )
            .with_sub(line_mode::LOOP),
            DrawEntity::points("t", "decal", &[[0.0; 3]]),
            DrawEntity::shapes(
                "t",
                "decal",
                shape_kind::RECT,
                &[Instance::at([0.0; 3], [2.0, 1.0, 0.0])],
            ),
            DrawEntity::shapes(
                "t",
                "decal",
                shape_kind::SECTOR,
                &[Instance::at([0.0; 3], [2.0, 1.0, 0.2])],
            ),
        ];
        for e in entities {
            let e = e.with_style(draw_style::DECAL).with_glow(-1.0);
            let base = build(&e, "t", "decal");
            assert!(!base.fill.is_empty());
            assert!(
                base.fill
                    .fx
                    .iter()
                    .all(|fx| fx[0] == LOOK_DECAL && fx[1] == 0.0 && fx[2] == 0.0)
            );
            for pulse in [-1.0, 0.75] {
                let changed = build(&e.clone().with_pulse(pulse), "t", "decal");
                assert_eq!(changed.fill.positions, base.fill.positions);
                assert_eq!(changed.fill.colors, base.fill.colors);
                assert!(
                    changed
                        .fill
                        .fx
                        .iter()
                        .all(|fx| fx[1] == 0.0 && fx[2] == pulse.max(0.0))
                );
            }
        }
    }

    #[test]
    fn image_plane_activity_is_explicit_while_frustum_sweep_keeps_energy_default() {
        let e = DrawEntity::shapes(
            "t",
            "camera",
            shape_kind::FRUSTUM,
            &[Instance::at([0.0; 3], [1.2, 0.8, 4.0])],
        )
        .with_style(draw_style::SWEEP)
        .with_glow(-1.0);
        let base = build(&e, "t", "camera");
        assert!(
            base.fill
                .fx
                .iter()
                .any(|fx| fx[0] == LOOK_SWEEP && fx[2] == visual::ENERGY_HZ)
        );
        assert!(
            base.fill
                .fx
                .iter()
                .filter(|fx| fx[0] == LOOK_PLANE)
                .all(|fx| fx[2] == 0.0)
        );
        for pulse in [-1.0, 0.75] {
            let changed = build(&e.clone().with_pulse(pulse), "t", "camera");
            assert_eq!(changed.fill.positions, base.fill.positions);
            assert_eq!(changed.fill.colors, base.fill.colors);
            assert!(
                changed
                    .fill
                    .fx
                    .iter()
                    .all(|fx| fx[1] == 0.0 && fx[2] == pulse.max(0.0))
            );
        }
    }

    #[test]
    fn field_defaults_and_disabled_effects_keep_the_same_transparent_geometry() {
        let e = DrawEntity::shapes(
            "t",
            "field",
            shape_kind::BOX,
            &[Instance::at([0.0; 3], [3.0, 1.5, 2.0])],
        )
        .with_style(draw_style::FIELD)
        .with_color(rgba(64, 160, 220, 255));
        let fx = Fx::of(&e, draw_style::FIELD);
        assert_eq!(
            (fx.glow, fx.pulse_hz, fx.spacing),
            (0.60, visual::ENERGY_HZ, 1.0)
        );
        let built = build(&e, "t", "field");
        let quiet = build(&e.clone().with_glow(-1.0).with_pulse(-1.0), "t", "field");
        assert!(built.fill.translucent() && quiet.fill.translucent());
        assert!(built.double_sided && !built.on_top && !built.shaded);
        assert_eq!(prims(&built.strokes), 24);
        assert_eq!(built.strokes.a, quiet.strokes.a);
        assert_eq!(built.strokes.b, quiet.strokes.b);
        assert_eq!(built.strokes.radii, quiet.strokes.radii);
        assert!(
            quiet
                .strokes
                .fx
                .iter()
                .all(|fx| fx[0] == 0.0 && fx[1] == 0.0)
        );
        assert_eq!(built.fill.positions.len(), 24);
        assert_eq!(built.fill.indices.len(), 36);
        assert_eq!(built.fill.positions, quiet.fill.positions);
        assert_eq!(built.fill.colors, quiet.fill.colors);
        assert!(
            quiet
                .fill
                .fx
                .iter()
                .all(|fx| fx[0] == LOOK_FIELD && fx[1] == 0.0 && fx[2] == 0.0)
        );
        let invisible = build(&e.with_color(rgba(64, 160, 220, 0)), "t", "field");
        assert!(invisible.fill.colors.iter().all(|color| color[3] == 0));
    }

    #[test]
    fn field_boxes_keep_full_local_extents_and_face_axes_across_rotated_instances() {
        let q = Quat::from_euler(EulerRot::ZYX, 0.6, -0.4, 0.3);
        let instances = [
            Instance::at([5.0, -2.0, 3.0], [4.0, 1.0, 2.0]).turned([q.w, q.x, q.y, q.z]),
            Instance::at([-7.0, 4.0, 1.0], [0.8, 5.0, 1.5]).yawed(-0.9),
        ];
        let e = DrawEntity::shapes("t", "field", shape_kind::BOX, &instances)
            .with_style(draw_style::FIELD)
            .with_dash(0.75);
        let built = build(&e, "t", "field");
        let plain = build(&e.with_style(draw_style::PLAIN), "t", "field");
        assert_eq!(built.fill.positions, plain.fill.positions);
        assert_eq!(built.fill.normals, plain.fill.normals);
        for (k, instance) in instances.iter().enumerate() {
            let (rotation, position) = pose_of(instance);
            let size = Vec3::from_array(instance.size);
            let mut lo = Vec3::MAX;
            let mut hi = Vec3::MIN;
            for i in k * 24..(k + 1) * 24 {
                let [x, y, z, axis] = built.fill.local[i];
                let local = Vec3::new(x, y, z);
                lo = lo.min(local);
                hi = hi.max(local);
                assert!(
                    Vec3::from_array(built.fill.positions[i])
                        .abs_diff_eq(position + rotation * local, 1e-5)
                );
                let normal = rotation.inverse() * Vec3::from_array(built.fill.normals[i]);
                assert!(normal[axis as usize].abs() > 0.999);
                assert_eq!(built.fill.fx2[i], [size.x, size.y, size.z, 0.75]);
                assert_eq!(built.fill.fx[i][0], LOOK_FIELD);
            }
            assert!(lo.abs_diff_eq(-size * 0.5, 1e-5));
            assert!(hi.abs_diff_eq(size * 0.5, 1e-5));
        }
        for face in built.fill.local.chunks_exact(4) {
            assert!(face.iter().all(|vertex| vertex[3] == face[0][3]));
        }
    }

    #[test]
    fn field_rectangles_use_local_xy_and_other_geometry_matches_plain_style() {
        let rect = DrawEntity::shapes(
            "t",
            "field",
            shape_kind::RECT,
            &[Instance::at([1.0, 2.0, 3.0], [6.0, 2.0, 0.5]).yawed(0.8)],
        )
        .with_style(draw_style::FIELD);
        let built = build(&rect, "t", "field");
        assert_eq!(built.fill.positions.len(), 4);
        assert!(built.fill.translucent());
        assert!(
            built
                .fill
                .local
                .iter()
                .all(|v| v[0].abs() == 3.0 && v[1].abs() == 1.0 && v[2] == 0.0 && v[3] == 2.0)
        );
        for shape in [
            shape_kind::ELLIPSOID,
            shape_kind::CYLINDER,
            shape_kind::CONE,
            shape_kind::CAPSULE,
            shape_kind::ELLIPSE,
            shape_kind::FRUSTUM,
            shape_kind::SECTOR,
        ] {
            let e = DrawEntity::shapes(
                "t",
                "field",
                shape,
                &[Instance::at([0.0; 3], [1.0, 1.0, 2.0])],
            )
            .with_style(draw_style::FIELD);
            let field = build(&e, "t", "field");
            let plain = build(&e.with_style(draw_style::PLAIN), "t", "field");
            assert_eq!(field.fill.positions, plain.fill.positions);
            assert_eq!(field.fill.fx, plain.fill.fx);
            assert_eq!(field.fill.fx2, plain.fill.fx2);
            assert_eq!(field.strokes.fx, plain.strokes.fx);
        }
        let triangles =
            DrawEntity::triangles("t", "field", &[[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]])
                .with_style(draw_style::FIELD);
        assert!(
            build(&triangles, "t", "field")
                .fill
                .fx
                .iter()
                .all(|fx| fx[0] == LOOK_PLAIN)
        );
    }

    fn square(at: f32, side: f32) -> Vec<[f32; 3]> {
        vec![
            [at, at, 0.0],
            [at + side, at, 0.0],
            [at + side, at + side, 0.0],
            [at, at + side, 0.0],
        ]
    }

    #[test]
    fn polygons_fill_around_their_holes_and_fade_in_from_every_edge() {
        let e = DrawEntity::polygon("t", "p", &[square(0.0, 6.0), square(2.0, 2.0)]);
        let built = build(&e, "t", "p");
        assert_eq!(
            prims(&built.strokes),
            24,
            "one partitioned outline per ring"
        );
        let fill = &built.fill;
        assert!(fill.fx.iter().all(|f| f[0] == LOOK_ZONE));
        let mut area = 0.0;
        for t in fill.indices.chunks_exact(3) {
            let p = |i: u32| Vec3::from_array(fill.positions[i as usize]);
            let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
            area += 0.5 * (b - a).cross(c - a).length();
            let mid = (a + b + c) / 3.0;
            assert!(
                !(mid.x > 2.0 && mid.x < 4.0 && mid.y > 2.0 && mid.y < 4.0),
                "no fill inside the hole at {mid}"
            );
        }
        assert!((area - 32.0).abs() < 1e-3, "covers 36 - 4 m², got {area}");
        let deepest = fill.local.iter().map(|l| l[0]).fold(0.0, f32::max);
        assert!(
            (deepest - DECAL_FADE).abs() < 1e-6,
            "inner vertices sit past the fade"
        );
        assert!(
            fill.local.iter().any(|l| l[0] == 0.0),
            "edge vertices at the edge"
        );
        let outline = build(&e.clone().with_fill(draw_flag::FILL_WIRE), "t", "p");
        assert!(outline.fill.is_empty());
        let stop = build(&e.with_style(draw_style::ALERT), "t", "p");
        assert!(stop.on_top && stop.fill.fx.iter().all(|f| f[0] == LOOK_ALERT));
        assert!(stop.fill.local.iter().all(|v| v[3] == 4.0));
        assert!(stop.fill.local.iter().any(|v| v[0] == 0.0));
        assert!(stop.fill.local.iter().any(|v| v[0] > 0.0));
    }

    #[test]
    fn filled_polygons_have_one_boundary_owner_and_keep_outline_tint() {
        let e = DrawEntity::polygon("t", "zone", &[square(0.0, 2.0)])
            .with_color(rgba(20, 140, 80, 128))
            .with_color2(rgba(240, 80, 20, 255));
        for style in [draw_style::DECAL, draw_style::ALERT] {
            let both = build(&e.clone().with_style(style), "t", "zone");
            assert!(both.fill.local.iter().all(|v| v[3] == 4.0));
            assert!(
                both.strokes
                    .color_a
                    .iter()
                    .all(|c| *c == [240, 80, 20, 128])
            );
            let solid = build(
                &e.clone().with_style(style).with_fill(draw_flag::FILL_SOLID),
                "t",
                "zone",
            );
            assert!(solid.fill.local.iter().all(|v| v[3] == 3.0));
            assert!(solid.strokes.is_empty());
            assert_eq!(both.fill.positions, solid.fill.positions);
        }
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
                let (a, b) = (
                    built.fill.positions[t[k] as usize],
                    built.fill.positions[t[(k + 1) % 3] as usize],
                );
                let d = Vec2::new(a[0] - b[0], a[1] - b[1]).length();
                assert!(d <= HUG_STEP + 1e-4, "an edge {d} m long");
            }
        }
        for p in &built.fill.positions {
            assert!(
                (p[2] - (0.5 * p[0] + GROUND_LIFT)).abs() < 1e-3,
                "{p:?} is not on the slope"
            );
        }
        let disc = raised(
            DrawEntity::points("t", "goal", &[[2.0, 0.0, 0.0]]).with_style(draw_style::BEACON),
        );
        let built = build_on(&disc, "t", "goal", Some(&hill));
        let ring: Vec<&[f32; 3]> = built
            .fill
            .positions
            .iter()
            .zip(&built.fill.fx)
            .filter(|(_, f)| f[0] == LOOK_RING)
            .map(|(p, _)| p)
            .collect();
        assert!(
            ring.iter()
                .all(|p| (p[2] - (0.5 * p[0] + GROUND_LIFT)).abs() < 1e-3),
            "the beacon's ring lies on the slope"
        );
    }

    #[test]
    fn draped_triangles_sample_their_interiors_and_update_normals() {
        let hill = |x: f32, y: f32| 0.1 * x * x + 0.2 * y;
        let points = [
            [0.0, 0.0, 0.1],
            [4.0, 0.0, hill(4.0, 0.0) + 0.1],
            [0.0, 4.0, hill(0.0, 4.0) + 0.1],
        ];
        let e = DrawEntity::triangles("t", "hill", &points).with_style(draw_style::PLAIN);
        let built = build_on(&e, "t", "hill", Some(&hill));
        assert!(built.fill.positions.len() > 3);
        for (p, n) in built.fill.positions.iter().zip(&built.fill.normals) {
            assert!((p[2] - hill(p[0], p[1]) - 0.1).abs() < 1e-5);
            let actual = Vec3::from_array(*n);
            let expected = Vec3::new(-0.2 * p[0], -0.2, 1.0).normalize();
            assert!(actual.dot(expected) > 0.99, "normal {actual} at {p:?}");
        }
        assert_eq!(build(&e, "t", "hill").fill.positions.len(), 3);
    }

    #[test]
    fn rulers_tick_every_round_step_and_say_their_length() {
        let e = DrawEntity::lines("t", "ruler", &[[0.0; 3], [10.0, 0.0, 0.0]]);
        let built = build(&e, "t", "ruler");
        assert_eq!(resolve_style(&e, "t", "ruler"), draw_style::RULER);
        assert_eq!(built.labels.len(), 1);
        assert_eq!(built.labels[0].text, "10.00 m");
        assert_eq!(
            prims(&built.strokes),
            1 + 9 + 2,
            "one rule, nine ticks and two end caps"
        );
        assert_eq!(built.strokes.radii[0], [-0.75; 2]);
        assert!((nice_step(1.7) - 2.0).abs() < 1e-6);
        assert!((nice_step(0.04) - 0.05).abs() < 1e-6);
        assert_eq!(length_text(0.25), "25.0 cm");
    }

    #[test]
    fn ruler_details_keep_endpoint_colors_and_label_controls() {
        let e = DrawEntity::lines("t", "ruler", &[[0.0; 3], [2.0, 0.0, 0.0]])
            .with_colors(&[rgba(200, 40, 80, 0), rgba(20, 180, 90, 200)])
            .with_width(-4.0)
            .with_spacing(0.5)
            .with_color2(rgba(40, 30, 20, 80))
            .with_glow(-1.0)
            .with_prop("label_icon", "measurement")
            .with_prop("label_detail", "Supplied tolerance");
        let b = build(&e, "t", "ruler");
        let end = b.strokes.a.len() - 4;
        assert_eq!(b.strokes.color_a[end - 4], [200, 40, 80, 0]);
        assert_eq!(b.strokes.color_a[end], [20, 180, 90, 200]);
        assert_eq!(b.strokes.radii[end], [-2.0; 2]);
        assert_eq!(b.labels[0].color[3], 100);
        assert_eq!(b.labels[0].background, [40, 30, 20, 31]);
        assert_eq!(b.labels[0].annotation.icon, "measurement");
        assert_eq!(b.labels[0].annotation.detail, "Supplied tolerance");
        assert_eq!(b.labels[0].annotation.glow, Some(0.0));
    }

    #[test]
    fn arrow_tips_chevrons_and_splats() {
        let tip = build(
            &DrawEntity::lines("t", "l", &[[0.0; 3], [2.0, 0.0, 0.0]])
                .with_flags(draw_flag::ARROW_TIP),
            "t",
            "l",
        );
        assert_eq!(prims(&tip.strokes), 2);
        assert_ne!(flags_at(&tip.strokes, 4) & FLAG_HEAD, 0);
        let chevrons = DrawEntity::new(draw_kind::ARROWS, "t", "c")
            .with_sub(ARROW_CHEVRON)
            .with_instances(&[Instance::at([0.0; 3], [1.0; 3])]);
        assert_eq!(
            prims(&build(&chevrons, "t", "c").strokes),
            2,
            "two wings into the tip"
        );
        let splats = DrawEntity::points("t", "cloud", &[[0.0; 3], [1.0, 0.0, 0.0]])
            .with_sub(POINT_SQUARE)
            .with_values(&[0.0, 1.0])
            .with_colormap(gearbox_api::colormap::TURBO, 0.0, 1.0);
        let built = build(&splats, "t", "cloud");
        assert!(built.splat);
        assert_eq!(style_at(&built.strokes, 0), draw_style::SPLAT);
        assert_ne!(flags_at(&built.strokes, 0) & FLAG_SQUARE, 0);
        assert_ne!(
            built.strokes.color_a[0], built.strokes.color_a[4],
            "coloured by value"
        );
    }

    #[test]
    fn chevron_wings_share_one_join_and_brighten_into_the_same_tip() {
        let color = [214, 78, 142, 170];
        let mut s = Strokes::default();
        s.chevron(
            Vec3::ZERO,
            Vec3::X,
            Vec3::Y,
            2.0,
            color,
            0.025,
            FLAG_FLAT | FLAG_ADD,
        );
        assert_eq!(s.a.len(), 8);
        assert_eq!(s.b[0], s.a[4]);
        assert_eq!(s.next[0], s.b[4]);
        assert_eq!(s.prev[4], s.a[0]);
        assert_eq!(s.dist[0][1], s.dist[4][0]);
        assert_eq!(s.dist[0][0], 0.0);
        assert_eq!(s.dist[4][1], 0.0);
        assert_eq!(s.fx2[0][1], s.fx2[4][1]);
        assert!(s.color_a.iter().chain(&s.color_b).all(|c| *c == color));
        assert!(s.radii.iter().all(|r| *r == [0.025; 2]));
    }

    #[test]
    fn square_points_keep_their_shape_outside_the_splat_style() {
        for style in [draw_style::PLAIN, draw_style::POINT_GLOW, draw_style::SPLAT] {
            let e = DrawEntity::points("t", "square", &[[0.0; 3]])
                .with_sub(POINT_SQUARE)
                .with_style(style)
                .with_color(0x99aa33ff)
                .with_width(-8.0);
            let b = build(&e, "t", "square");
            assert_ne!(flags_at(&b.strokes, 0) & FLAG_SQUARE, 0);
            assert_eq!(flags_at(&b.strokes, 0) & FLAG_SPHERE, 0);
            assert_eq!(b.strokes.color_a[0], [255, 51, 170, 153]);
            assert_eq!(b.strokes.radii[0], [-4.0; 2]);
        }
    }

    #[test]
    fn non_splat_point_styles_forward_additive_without_changing_geometry_or_source() {
        for style in [draw_style::ORB, draw_style::BEACON, draw_style::HOLO] {
            let e = DrawEntity::points("t", "point", &[[1.0, 2.0, 0.5]])
                .with_style(style)
                .with_color(rgba(180, 70, 210, 160))
                .with_glow(-1.0)
                .with_pulse(-1.0)
                .with_beam(-1.0);
            let plain = build(&e, "t", "point");
            let additive = build(&e.with_flags(draw_flag::ADDITIVE), "t", "point");
            assert!(!plain.splat && !additive.splat);
            assert_eq!(plain.strokes.a, additive.strokes.a);
            assert_eq!(plain.strokes.b, additive.strokes.b);
            assert_eq!(plain.strokes.radii, additive.strokes.radii);
            assert_eq!(plain.strokes.color_a, additive.strokes.color_a);
            assert_eq!(plain.fill.positions, additive.fill.positions);
            assert!(additive.strokes.fx.iter().all(|fx| fx[0] == 0.0));
            assert!(additive.strokes.color_a.iter().all(|color| color[3] == 160));
            for vertex in 0..plain.strokes.code.len() {
                assert_eq!(
                    flags_at(&additive.strokes, vertex),
                    flags_at(&plain.strokes, vertex) | FLAG_ADD
                );
            }
        }
    }

    #[test]
    fn splat_additive_keeps_opaque_data_pipeline_and_per_point_colors() {
        let colors = [rgba(60, 180, 90, 255), rgba(190, 70, 220, 0)];
        let e = DrawEntity::points("t", "cloud", &[[0.0; 3], [1.0, 0.0, 0.0]])
            .with_style(draw_style::SPLAT)
            .with_colors(&colors)
            .with_width(-3.0);
        let plain = build(&e, "t", "cloud");
        let additive = build(&e.with_flags(draw_flag::ADDITIVE), "t", "cloud");
        assert!(plain.splat && additive.splat);
        assert_eq!(plain.strokes.code, additive.strokes.code);
        assert_eq!(plain.strokes.radii, additive.strokes.radii);
        assert_eq!(additive.strokes.color_a[0], unpack_rgba(colors[0]));
        assert_eq!(additive.strokes.color_a[4], unpack_rgba(colors[1]));
        assert_eq!(flags_at(&additive.strokes, 0) & FLAG_ADD, 0);
    }

    #[test]
    fn sectors_and_frustums_sweep() {
        let sector = DrawEntity::shapes(
            "t",
            "fov",
            shape_kind::SECTOR,
            &[Instance::at([0.0; 3], [4.0, 1.0, 0.5])],
        );
        let built = build(&sector, "t", "fov");
        assert!(built.fill.fx.iter().all(|f| f[0] == LOOK_SWEEP));
        let reach = built
            .fill
            .positions
            .iter()
            .map(|p| Vec2::new(p[0], p[1]).length())
            .fold(0.0, f32::max);
        assert!(
            (reach - 4.0).abs() < 1e-3
                && built
                    .fill
                    .positions
                    .iter()
                    .all(|p| Vec2::new(p[0], p[1]).length() >= 0.5 - 1e-3)
        );
        let frustum = DrawEntity::shapes(
            "t",
            "cam",
            shape_kind::FRUSTUM,
            &[Instance::at([0.0; 3], [1.2, 0.8, 5.0])],
        )
        .with_prop("near", "0.5");
        let built = build(&frustum, "t", "cam");
        let looks: Vec<f32> = built.fill.fx.iter().map(|f| f[0]).collect();
        assert!(looks.contains(&LOOK_SWEEP) && looks.contains(&LOOK_PLANE));
        assert_eq!(
            prims(&built.strokes),
            24,
            "partitioned edges and far rim; the image-plane shader owns its border"
        );
        assert!(
            built
                .strokes
                .radii
                .iter()
                .all(|r| *r == [-0.5 * visual::RULE_PX; 2]
                    || *r == [-0.5 * visual::RULE_PX * visual::STRUCTURE_CORNER_GAIN; 2])
        );
        assert!(
            built
                .strokes
                .code
                .iter()
                .enumerate()
                .all(|(i, _)| flags_at(&built.strokes, i) & FLAG_ADD == 0)
        );
        let coincident = build(&frustum.clone().with_prop("near", "5.0"), "t", "cam");
        assert_eq!(
            prims(&coincident.strokes),
            12,
            "coincident image plane owns the far border"
        );
        let explicit = build(
            &frustum.with_width(0.12).with_flags(draw_flag::ADDITIVE),
            "t",
            "cam",
        );
        assert!(
            explicit
                .strokes
                .radii
                .iter()
                .all(|r| *r == [0.06; 2] || *r == [0.06 * visual::STRUCTURE_CORNER_GAIN; 2])
        );
        assert!(
            explicit
                .strokes
                .code
                .iter()
                .enumerate()
                .all(|(i, _)| flags_at(&explicit.strokes, i) & FLAG_ADD != 0)
        );
        let far = built
            .fill
            .positions
            .iter()
            .map(|p| p[0])
            .fold(0.0, f32::max);
        assert!((far - 5.0).abs() < 1e-4);
    }

    #[test]
    fn a_closed_wall_crest_does_not_repeat_its_closing_point() {
        let e = DrawEntity::lines(
            "t",
            "trace",
            &[
                [0.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [2.0, 2.0, 0.0],
                [0.0, 2.0, 0.0],
                [0.0, 0.0, 0.0],
            ],
        )
        .with_sub(line_mode::LOOP)
        .with_style(draw_style::LIGHT_WALL);
        let built = build(&e, "t", "trace");
        assert_eq!(prims(&built.strokes), 8);
        for (a, b) in built.strokes.a.iter().zip(&built.strokes.b) {
            assert_ne!(a, b);
        }
    }

    #[test]
    fn text_becomes_centred_labels_and_item_labels_sit_beside() {
        let text = build(
            &DrawEntity::text("t", "x", &[[0.0; 3], [1.0, 0.0, 0.0]], "hi")
                .with_labels(&["", "two"]),
            "t",
            "x",
        );
        assert!(text.strokes.is_empty());
        assert_eq!(
            text.labels
                .iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>(),
            ["hi", "two"]
        );
        assert!(text.labels.iter().all(|l| l.centred));
        let pts = DrawEntity::points("t", "p", &[[0.0; 3]])
            .with_flags(draw_flag::LABELS)
            .with_labels(&["goal"]);
        let built = build(&pts, "t", "p");
        assert_eq!(built.labels.len(), 1);
        assert!(!built.labels[0].centred);
    }

    #[test]
    fn geometry_outline_does_not_become_an_unreadable_label_backing() {
        let entity = DrawEntity::shapes("t", "label", shape_kind::BOX, &[Instance::default()])
            .with_color(0xff40c0ff)
            .with_color2(0xffeeeeee)
            .with_labels(&["Outline"])
            .with_flags(draw_flag::LABELS)
            .with_fill(draw_flag::FILL_BOTH);
        let built = build(&entity, "t", "label");
        assert_eq!(built.labels[0].background, [0; 4]);
        assert!(!built.labels[0].annotation.background_explicit);
        assert!(
            built
                .strokes
                .color_a
                .iter()
                .all(|color| *color == unpack_rgba(entity.color2))
        );
        let backed = build(
            &entity.clone().with_prop("label_background", "#102030c0"),
            "t",
            "label",
        );
        assert_eq!(backed.labels[0].background, [16, 32, 48, 192]);
        assert!(backed.labels[0].annotation.background_explicit);
        let clear = build(&entity.with_prop("label_background", "0"), "t", "label");
        assert_eq!(clear.labels[0].background, [0; 4]);
        assert!(clear.labels[0].annotation.background_explicit);
        let text =
            DrawEntity::text("t", "text", &[[0.0; 3]], "Caller text").with_color2(0xff302010);
        assert_eq!(
            build(&text, "t", "text").labels[0].background,
            [16, 32, 48, 255]
        );
    }

    #[test]
    fn labels_preserve_text_metadata_explicit_size_and_transparent_default() {
        let e = DrawEntity::text("t", "text", &[[0.0; 3]], "robot α / Mixed Case")
            .with_prop("label_category", "SENSOR")
            .with_prop("label_detail", "2.4 m")
            .with_prop("label_icon", "sensor")
            .with_prop("label_selected", "true");
        let built = build(&e, "t", "text");
        let label = &built.labels[0];
        assert_eq!(label.text, "robot α / Mixed Case");
        assert_eq!((label.size, label.background), (-16.0, [0; 4]));
        assert_eq!(label.annotation.category, "SENSOR");
        assert_eq!(label.annotation.detail, "2.4 m");
        assert_eq!(label.annotation.icon, "sensor");
        assert!(label.annotation.selected);
        for size in [-23.0, 0.28] {
            let built = build(
                &e.clone()
                    .with_width(size)
                    .with_color2(rgba(20, 40, 60, 128)),
                "t",
                "text",
            );
            assert_eq!(built.labels[0].size, size);
            assert_eq!(built.labels[0].background, [20, 40, 60, 128]);
        }
    }

    #[test]
    fn label_accent_overrides_raw_item_and_ruler_accents_without_recoloring_data() {
        let entities = [
            DrawEntity::text("t", "text", &[[0.0; 3]], "Robot α / Mixed Case"),
            DrawEntity::points("t", "point", &[[0.0; 3]]).labelled(&["Robot α / Mixed Case"]),
            DrawEntity::lines("t", "ruler", &[[0.0; 3], [1.0, 0.0, 0.0]])
                .with_style(draw_style::RULER),
        ];
        let accent = rgba(80, 170, 220, 192);
        for e in entities {
            let e = e
                .with_color(rgba(210, 80, 170, 128))
                .with_prop("accent", &rgba(255, 210, 40, 255).to_string())
                .with_prop("label_category", "SENSOR")
                .with_prop("label_selected", "true");
            let plain = build(&e, "t", "label");
            let changed = build(
                &e.clone().with_prop("label_accent", &accent.to_string()),
                "t",
                "label",
            );
            assert_eq!(plain.strokes.color_a, changed.strokes.color_a);
            assert_eq!(plain.fill.colors, changed.fill.colors);
            assert_eq!(plain.strokes.a, changed.strokes.a);
            for (a, b) in plain.labels.iter().zip(&changed.labels) {
                assert_eq!(a.text, b.text);
                assert_eq!(a.color, b.color);
                assert_eq!(a.background, b.background);
                assert_eq!(a.annotation, b.annotation);
                assert_eq!(b.accent, [80, 170, 220, 96]);
            }
            let hidden = build(
                &e.clone()
                    .with_color(rgba(210, 80, 170, 0))
                    .with_prop("label_accent", &accent.to_string()),
                "t",
                "label",
            );
            assert!(
                hidden
                    .labels
                    .iter()
                    .all(|l| l.color[3] == 0 && l.accent[3] == 0)
            );
            let clear = build(&e.clone().with_prop("label_accent", "0"), "t", "label");
            assert!(clear.labels.iter().all(|l| l.accent == [0; 4]));
            let invalid = build(&e.with_prop("label_accent", "not a color"), "t", "label");
            assert!(
                plain
                    .labels
                    .iter()
                    .zip(&invalid.labels)
                    .all(|(a, b)| a.accent == b.accent)
            );
        }
    }

    #[test]
    fn structured_labels_require_exact_opt_in_and_keep_malformed_text_literal() {
        let encoded =
            serde_json::to_string(&["Robot α / Mixed Case", "confidence 0.82\nframe: camera"])
                .unwrap();
        let e = DrawEntity::text("t", "label", &[[0.0; 3]], &encoded)
            .with_prop("label_detail", "caller detail");
        for mode in ["", "json", "Identity-detail-json"] {
            let built = build(&e.clone().with_prop("label_parts", mode), "t", "label");
            assert_eq!(built.labels[0].text, encoded);
            assert_eq!(built.labels[0].annotation.detail, "caller detail");
        }
        let parsed = build(
            &e.with_prop("label_parts", "identity-detail-json"),
            "t",
            "label",
        );
        assert_eq!(parsed.labels[0].text, "Robot α / Mixed Case");
        assert_eq!(
            parsed.labels[0].annotation.detail,
            "confidence 0.82\nframe: camera"
        );
        for text in [
            "plain / arbitrary text",
            "[\"one\"]",
            "[\"one\", 2]",
            "[\"one\",\"two\",\"three\"]",
            "{\"identity\":\"robot\"}",
        ] {
            let e = DrawEntity::text("t", "label", &[[0.0; 3]], text)
                .with_prop("label_parts", "identity-detail-json")
                .with_prop("label_detail", "unchanged");
            let built = build(&e, "t", "label");
            assert_eq!(built.labels[0].text, text);
            assert_eq!(built.labels[0].annotation.detail, "unchanged");
        }
    }

    #[test]
    fn neutral_label_identity_preserves_per_item_tint_alpha_and_geometry() {
        let colors = [rgba(30, 100, 220, 128), rgba(210, 70, 160, 0)];
        let labels = [
            serde_json::to_string(&["First α", "0.8 / frame A"]).unwrap(),
            serde_json::to_string(&["Second", "0.6 / frame B"]).unwrap(),
        ];
        for kind in [draw_kind::TEXT, draw_kind::POINTS] {
            let e = DrawEntity::new(kind, "t", "labels")
                .with_points(&[[0.0; 3], [1.0, 0.0, 0.0]])
                .with_colors(&colors)
                .labelled(&labels)
                .with_prop("label_parts", "identity-detail-json")
                .with_prop("label_category", "DETECTION")
                .with_prop("label_selected", "true");
            let original = build(&e, "t", "labels");
            let neutral = build(
                &e.clone().with_prop("label_neutral_identity", "true"),
                "t",
                "labels",
            );
            assert_eq!(original.strokes.a, neutral.strokes.a);
            assert_eq!(original.strokes.color_a, neutral.strokes.color_a);
            for (i, label) in neutral.labels.iter().enumerate() {
                let tint = unpack_rgba(colors[i]);
                assert_eq!(label.color, alpha_scaled(visual::INK, tint[3]));
                assert_eq!(label.accent, tint);
                assert_eq!(label.annotation.category, "DETECTION");
                assert!(label.annotation.selected);
                assert_eq!(label.text, ["First α", "Second"][i]);
                assert_eq!(
                    label.annotation.detail,
                    ["0.8 / frame A", "0.6 / frame B"][i]
                );
            }
            let override_ = build(
                &e.with_prop("label_neutral_identity", "true")
                    .with_prop("label_accent", &rgba(90, 180, 140, 192).to_string()),
                "t",
                "labels",
            );
            assert_eq!(override_.labels[0].accent, [90, 180, 140, 96]);
            assert_eq!(override_.labels[1].accent[3], 0);
        }
    }

    #[test]
    fn structured_batched_labels_survive_native_chunks_with_escaped_content() {
        let points: Vec<_> = (0..400).map(|i| [i as f32 * 0.1, 0.0, 1.0]).collect();
        let fields: Vec<_> = (0..points.len())
            .map(|i| {
                [
                    format!("Object {i}: \"α\"\nsecond line"),
                    format!("frame: camera/{i} / confidence supplied"),
                ]
            })
            .collect();
        let labels: Vec<_> = fields
            .iter()
            .map(|parts| serde_json::to_string(parts).unwrap())
            .collect();
        let e = DrawEntity::text("t", "detections", &points, "")
            .with_labels(&labels)
            .with_color(rgba(30, 100, 220, 128))
            .with_prop("label_parts", "identity-detail-json")
            .with_prop("label_neutral_identity", "true");
        let chunks = e.chunks(gearbox_api::wire::DRAW_CHUNK_BYTES);
        assert!(chunks.len() > 1);
        let mut store = gearbox_api::draw::DrawStore::default();
        for chunk in chunks {
            chunk.validate().unwrap();
            store.apply(chunk, 0.0).unwrap();
        }
        let restored = &store.get("t", "detections").unwrap().entity;
        assert_eq!(restored.point_count(), points.len());
        assert_eq!(restored.label_list(), labels);
        let built = build(restored, "t", "detections");
        assert_eq!(built.labels.len(), points.len());
        for (label, parts) in built.labels.iter().zip(fields) {
            assert_eq!(label.text, parts[0]);
            assert_eq!(label.annotation.detail, parts[1]);
            assert_eq!(label.color, alpha_scaled(visual::INK, 128));
            assert_eq!(label.accent, [30, 100, 220, 128]);
        }
    }

    #[test]
    fn zero_alpha_hides_generated_labels_axes_and_secondary_outlines() {
        let invisible = rgba(170, 80, 210, 0);
        let entities = [
            DrawEntity::points("t", "p", &[[0.0; 3]]).with_style(draw_style::POINT_GLOW),
            DrawEntity::axes("t", "a", 1.0),
            DrawEntity::shapes(
                "t",
                "s",
                shape_kind::BOX,
                &[Instance::at([0.0; 3], [1.0; 3])],
            )
            .with_fill(draw_flag::FILL_BOTH),
            DrawEntity::lines("t", "ruler", &[[0.0; 3], [1.0, 0.0, 0.0]])
                .with_style(draw_style::RULER),
            DrawEntity::text("t", "text", &[[0.0; 3]], "raw text"),
        ];
        for e in entities {
            let e = e
                .with_color(invisible)
                .with_color2(rgba(255, 255, 255, 255))
                .with_prop("accent", &rgba(255, 200, 40, 255).to_string())
                .with_flags(draw_flag::LABELS)
                .with_labels(&["item"]);
            let built = build(&e, "t", "item");
            assert!(!built.labels.is_empty());
            assert!(
                built
                    .labels
                    .iter()
                    .all(|l| l.color[3] == 0 && l.background[3] == 0 && l.accent[3] == 0)
            );
            assert!(
                built
                    .strokes
                    .color_a
                    .iter()
                    .chain(&built.strokes.color_b)
                    .all(|c| c[3] == 0)
            );
            assert!(built.fill.colors.iter().all(|c| c[3] == 0));
        }
    }
}
