//! Crop prefabs: each crop's shape at each stage of its life, built once from
//! photographed leaves, the way Wageningen's lettuce simulator builds its
//! plants (leaves sampled from real leaf images, tips set in a spiral around
//! the centre; LettuceVisSim, 2026). Leaves are alpha-cut cards from a CC0
//! leaf atlas (ambientCG, see `assets/crops/README.md`), bent and cupped;
//! stems, fruit and ears are plain coloured geometry.
//!
//! A plant picks the prefab of its stage and is scaled and turned on its own;
//! its health tints its material.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use gearbox_greenhouse::{Form, Species};

/// Prefabs per crop, from just up to ripe.
pub const STAGES: usize = 8;
/// Cells in each leaf atlas.
const CELLS: u32 = 4;
const GOLDEN_ANGLE: f32 = 2.399_963;

/// A leaf atlas: its PNG, and the width over height of one cell, so a leaf
/// card keeps its photo's proportions.
pub struct Atlas {
    pub png: &'static [u8],
    pub aspect: f32,
}

macro_rules! atlas {
    ($file:literal, $w:literal, $h:literal) => {
        Atlas { png: include_bytes!(concat!("../../assets/crops/", $file, "_leaves.png")), aspect: $w as f32 / $h as f32 }
    };
}

/// The leaf atlas a crop's leaves are cut from.
pub fn atlas(species: &Species) -> Atlas {
    match species.name {
        "tomato" => atlas!("tomato", 160, 288),
        "potato" => atlas!("potato", 192, 288),
        "wheat" | "rice" | "green_onion" => atlas!("wheat", 48, 512),
        "spinach" | "chard" | "pak_choi" | "red_beet" => atlas!("spinach", 160, 288),
        "kale" => atlas!("kale", 256, 288),
        "radish" => atlas!("radish", 80, 288),
        "carrot" => atlas!("carrot", 112, 288),
        "pepper" => atlas!("pepper", 128, 288),
        "strawberry" => atlas!("strawberry", 160, 288),
        "soybean" | "dry_bean" | "snap_bean" | "pea" | "peanut" => atlas!("bean", 160, 288),
        "sweet_potato" => atlas!("sweet_potato", 224, 288),
        _ => atlas!("lettuce", 256, 288),
    }
}

/// What a sound plant's leaf photo is multiplied by: the photographed leaves
/// are their own greens, each brought to its crop's.
pub fn sound_tint(species: &Species) -> Vec3 {
    match species.name {
        "lettuce" => Vec3::new(0.78, 1.08, 0.62),
        "tomato" => Vec3::new(0.86, 1.0, 0.80),
        "potato" => Vec3::new(0.84, 0.96, 0.78),
        "wheat" | "rice" => Vec3::new(0.86, 1.02, 0.66),
        "spinach" | "chard" => Vec3::new(0.50, 0.78, 0.40),
        "pak_choi" => Vec3::new(0.74, 0.98, 0.62),
        "kale" => Vec3::new(0.70, 0.86, 0.82),
        "radish" => Vec3::new(0.68, 0.92, 0.52),
        "red_beet" => Vec3::new(0.56, 0.74, 0.44),
        "carrot" => Vec3::new(1.0, 1.08, 0.80),
        "green_onion" => Vec3::new(0.66, 0.96, 0.66),
        "soybean" | "peanut" => Vec3::new(0.52, 0.76, 0.40),
        "dry_bean" | "snap_bean" => Vec3::new(0.60, 0.84, 0.48),
        "pea" => Vec3::new(0.58, 0.84, 0.62),
        _ => Vec3::new(0.88, 1.04, 0.82),
    }
}

/// One crop at one stage: its leaf cards, and the stems, flowers, fruit or
/// ears drawn in vertex colour. Fruit is apart, so picking can take it away.
pub struct Prefab {
    pub leaves: Mesh,
    pub parts: Option<Mesh>,
    pub fruit: Option<Mesh>,
    /// Height of the plant, metres.
    pub height: f32,
    /// Radius of ground it covers, metres.
    pub radius: f32,
}

/// How finely a prefab is built: close up, or for plants seen from afar,
/// their leaves, stems and fruit on coarse grids with the same outline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Detail {
    Near,
    Far,
}

/// Geometry being built.
#[derive(Default)]
struct Shape {
    positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    colours: Vec<[f32; 4]>,
    indices: Vec<u32>,
    coarse: bool,
}

impl Shape {
    fn of(detail: Detail) -> Self {
        Self { coarse: detail == Detail::Far, ..default() }
    }

    fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// A grid of `across` × `along` vertices from `point(u, v)`, u and v in 0..1.
    fn grid(&mut self, across: u32, along: u32, mut point: impl FnMut(f32, f32) -> ([f32; 3], [f32; 2], [f32; 4])) {
        let start = self.positions.len() as u32;
        for j in 0..along {
            for i in 0..across {
                let (p, uv, c) = point(i as f32 / (across - 1) as f32, j as f32 / (along - 1) as f32);
                self.positions.push(p);
                self.uvs.push(uv);
                self.colours.push(c);
            }
        }
        for j in 0..along - 1 {
            for i in 0..across - 1 {
                let a = start + j * across + i;
                let (b, c, d) = (a + 1, a + across, a + across + 1);
                self.indices.extend([a, c, b, b, c, d]);
            }
        }
    }

    /// A leaf card: from `base` out along `forward`, cupped about its midrib,
    /// arching over by `droop` towards its tip, its margin waved by `ruffle`.
    /// `flip` turns the photo round so its broad base lies outward and its
    /// tip, petiole cut off, at the crown: an obovate leaf from an ovate one.
    #[allow(clippy::too_many_arguments)]
    fn leaf(&mut self, base: Vec3, forward: Vec3, up: Vec3, length: f32, width: f32, cup: f32, droop: f32, ruffle: f32, cell: u32, flip: bool) {
        // A leaflet a few centimetres long bends as smoothly on a coarser grid.
        let (across, along) = if self.coarse { (2, 3) } else if length < 0.09 { (3, 5) } else { (5, 9) };
        let forward = forward.normalize();
        let side = forward.cross(up).normalize_or(Vec3::X);
        let normal = side.cross(forward).normalize();
        let (u0, u1) = (cell as f32 / CELLS as f32, (cell + 1) as f32 / CELLS as f32);
        self.grid(across, along, |u, v| {
            let across = u - 0.5;
            // The blade bends down along its length and curls up at its edges.
            let bend = -droop * length * v * v;
            let curl = cup * width * across * across * (0.4 + v);
            let wave = ruffle * width * (v * 19.0).sin() * (2.0 * across).abs().powi(3) * v;
            let reach = length * v * (1.0 - 0.25 * droop * v);
            let p = base + forward * reach + side * (width * across) + normal * (bend + curl + wave);
            let row = if flip { 0.93 * v } else { 1.0 - v };
            ([p.x, p.y, p.z], [u0 + (u1 - u0) * u, row], [1.0; 4])
        });
    }

    /// A tube along `path` of radius `radius`, tapering to `tip` of it, in `colour`.
    fn stem(&mut self, path: &[Vec3], radius: f32, tip: f32, colour: [f32; 4]) {
        // Seen from afar, stalks a few millimetres thick are not seen at all.
        if self.coarse && radius < 0.002 {
            return;
        }
        let sides = if self.coarse { 3 } else { 6 };
        let start = self.positions.len() as u32;
        for (k, &p) in path.iter().enumerate() {
            let along = if k + 1 < path.len() { path[k + 1] - p } else { p - path[k - 1] };
            let along = along.normalize_or(Vec3::Y);
            let side = along.any_orthonormal_vector();
            let other = along.cross(side);
            let r = radius * (1.0 - (1.0 - tip) * k as f32 / (path.len() - 1).max(1) as f32);
            for s in 0..sides {
                let a = s as f32 / sides as f32 * std::f32::consts::TAU;
                let q = p + (side * a.cos() + other * a.sin()) * r;
                self.positions.push([q.x, q.y, q.z]);
                self.uvs.push([0.0, 0.0]);
                self.colours.push(colour);
            }
        }
        for k in 0..path.len() as u32 - 1 {
            for s in 0..sides {
                let a = start + k * sides + s;
                let b = start + k * sides + (s + 1) % sides;
                let (c, d) = (a + sides, b + sides);
                self.indices.extend([a, b, c, b, d, c]);
            }
        }
    }

    /// An ellipsoid of radii `r` at `centre`, its long axis along `axis`.
    fn blob(&mut self, centre: Vec3, axis: Vec3, r: Vec3, colour: [f32; 4]) {
        let axis = axis.normalize_or(Vec3::Y);
        let side = axis.any_orthonormal_vector();
        let other = axis.cross(side);
        let (around, up) = if self.coarse { (4, 3) } else { (9, 7) };
        self.grid(around, up, |u, v| {
            let (lon, lat) = (u * std::f32::consts::TAU, (v - 0.5) * std::f32::consts::PI);
            let p = centre + axis * (r.y * lat.sin()) + (side * lon.cos() * r.x + other * lon.sin() * r.z) * lat.cos();
            ([p.x, p.y, p.z], [0.0, 0.0], colour)
        });
    }

    fn into_mesh(self) -> Option<Mesh> {
        if self.is_empty() {
            return None;
        }
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs)
            .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.colours)
            .with_inserted_indices(Indices::U32(self.indices));
        mesh.compute_smooth_normals();
        Some(mesh)
    }
}

/// A repeatable spread of numbers in -1..1 for a plant's parts.
fn jitter(seed: u32, n: u32) -> f32 {
    let mut h = seed.wrapping_mul(0x9E37_79B9) ^ n.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    (h & 0xFFFF) as f32 / 32_767.5 - 1.0
}

const STEM_GREEN: [f32; 4] = [0.30, 0.48, 0.16, 1.0];

/// The prefab of `species` at `stage` (0 just up, `STAGES - 1` ripe).
pub fn prefab(species: &Species, stage: usize, detail: Detail) -> Prefab {
    let s = stage.min(STAGES - 1) as f32 / (STAGES - 1) as f32;
    let aspect = atlas(species).aspect;
    match species.form {
        Form::Rosette => rosette(species, s, aspect, detail),
        Form::Vine => tomato(species, s, aspect, detail),
        Form::Bush => potato(species, s, aspect, detail),
        Form::Grass => grass(species, s, aspect, detail),
        Form::Legume => legume(species, s, aspect, detail),
        Form::Creeper => creeper(species, s, aspect, detail),
        Form::Root => root(species, s, aspect, detail),
        Form::Allium => allium(species, s, aspect, detail),
        Form::Shrub => shrub(species, s, aspect, detail),
        Form::Crown => crown(species, s, aspect, detail),
    }
}

/// Share of a crop's life, from 0 to 1, by which its edible part has begun to form.
fn setting(species: &Species) -> f32 {
    ((species.edible_days / species.harvest_days) as f32).min(0.95)
}

/// How a head of leaves is shaped.
struct Head {
    /// Leaves it adds over its life, and its reach at the start and added by harvest, m.
    leaves: f32,
    reach: (f32, f32),
    /// How steeply the outer and inner leaves rise, how much they cup, and their ruffle.
    rise: (f32, f32),
    cup: (f32, f32),
    ruffle: f32,
    /// The photo turned round, so its broad end lies outward.
    flip: bool,
    /// Share of a leaf that is bare stalk, and the stalk's colour and radius.
    stalk: f32,
    stalk_colour: [f32; 4],
    stalk_radius: f32,
}

fn head(species: &Species) -> Head {
    let lettuce = Head {
        leaves: 31.0,
        reach: (0.035, 0.10),
        rise: (0.18, 1.15),
        cup: (1.1, 1.4),
        ruffle: 0.06,
        flip: true,
        stalk: 0.0,
        stalk_colour: STEM_GREEN,
        stalk_radius: 0.0,
    };
    match species.name {
        "spinach" => Head { leaves: 14.0, reach: (0.03, 0.09), rise: (0.35, 0.9), cup: (0.5, 0.6), ruffle: 0.02, flip: false, stalk: 0.35, stalk_radius: 0.0018, ..lettuce },
        "chard" => Head {
            leaves: 12.0,
            reach: (0.04, 0.20),
            rise: (0.95, 0.45),
            cup: (0.35, 0.4),
            ruffle: 0.04,
            flip: false,
            stalk: 0.45,
            stalk_colour: [0.72, 0.10, 0.12, 1.0],
            stalk_radius: 0.004,
        },
        "pak_choi" => Head {
            leaves: 12.0,
            reach: (0.03, 0.10),
            rise: (0.9, 0.45),
            cup: (0.9, 0.6),
            ruffle: 0.0,
            flip: false,
            stalk: 0.45,
            stalk_colour: [0.86, 0.92, 0.80, 1.0],
            stalk_radius: 0.0055,
        },
        "kale" => Head {
            leaves: 12.0,
            reach: (0.04, 0.20),
            rise: (0.95, 0.45),
            cup: (0.25, 0.3),
            ruffle: 0.10,
            flip: false,
            stalk: 0.35,
            stalk_colour: [0.45, 0.25, 0.42, 1.0],
            stalk_radius: 0.003,
        },
        _ => lettuce,
    }
}

/// A head of leaves (lettuce, spinach, chard, pak choi, kale): one leaf after
/// another, each set a golden angle round from the last, the young ones small
/// and upright in the heart, the old ones broad and spread; on a stalk where
/// the crop has one.
fn rosette(species: &Species, s: f32, aspect: f32, detail: Detail) -> Prefab {
    let h = head(species);
    let count = (3.0 + h.leaves * s).round() as u32;
    let (mut leaves, mut parts) = (Shape::of(detail), Shape::of(detail));
    let span = h.reach.0 + h.reach.1 * s;
    for i in 0..count {
        let age = (count - i) as f32;
        // 0 for the oldest, outermost leaf, 1 for the youngest in the heart.
        let youth = i as f32 / count.max(1) as f32;
        let grown = (1.0 - (-age / 6.0).exp()).max(0.3);
        let length = span * grown * (1.0 - 0.35 * youth) * (1.0 + 0.08 * jitter(7, i));
        let azimuth = i as f32 * GOLDEN_ANGLE + 0.15 * jitter(3, i);
        // Outer leaves lie out over the tray; the heart stands and closes.
        let rise = h.rise.0 + h.rise.1 * youth.powf(1.4);
        let forward = Vec3::new(azimuth.cos() * rise.cos(), rise.sin(), azimuth.sin() * rise.cos());
        let mut base = Vec3::new(azimuth.cos(), 0.0, azimuth.sin()) * 0.006 * (1.0 - youth) + Vec3::Y * 0.012 * youth;
        if h.stalk > 0.0 {
            let end = base + forward * (length * h.stalk);
            parts.stem(&[base, (base + end) / 2.0 + Vec3::Y * length * 0.03, end], h.stalk_radius * (0.5 + 0.5 * s), 0.7, h.stalk_colour);
            base = end;
        }
        let blade = length * (1.0 - h.stalk);
        let width = blade * aspect * (1.25 - 0.3 * youth);
        let cup = h.cup.0 + h.cup.1 * youth;
        let droop = 0.35 * (1.0 - youth) * grown;
        leaves.leaf(base, forward, Vec3::Y, blade, width, cup, droop, h.ruffle, i % CELLS, h.flip);
    }
    let height = (0.05 + 0.13 * s).max(span * h.rise.0.sin() + 0.02);
    Prefab { leaves: leaves.into_mesh().expect("a head has leaves"), parts: parts.into_mesh(), fruit: None, height, radius: span }
}

/// A leaf on its own stalk from `base`: the stalk along `forward` for
/// `stalk` metres in `colour`, the blade standing on along `blade`.
#[allow(clippy::too_many_arguments)]
fn stalked(leaves: &mut Shape, parts: &mut Shape, base: Vec3, forward: Vec3, blade: Vec3, stalk: f32, length: f32, width: f32, cell: u32, colour: [f32; 4]) {
    let forward = forward.normalize();
    let end = base + forward * stalk;
    parts.stem(&[base, (base + end) / 2.0 + Vec3::Y * stalk * 0.08, end], 0.0014, 0.7, colour);
    leaves.leaf(end, blade, Vec3::Y, length, width, 0.45, 0.3, 0.0, cell, false);
}

/// A trifoliate leaf: a stalk from `base` along `forward`, and three
/// leaflets at its end, the middle one longest.
#[allow(clippy::too_many_arguments)]
fn trifoliate(leaves: &mut Shape, parts: &mut Shape, base: Vec3, forward: Vec3, stalk: f32, leaflet: f32, aspect: f32, seed: u32) {
    let forward = forward.normalize();
    let side = forward.cross(Vec3::Y).normalize_or(Vec3::X);
    let end = base + forward * stalk + Vec3::Y * stalk * 0.05;
    parts.stem(&[base, base + forward * (stalk * 0.55) + Vec3::Y * (stalk * 0.12), end], 0.0014, 0.6, STEM_GREEN);
    let ahead = (forward + Vec3::Y * 0.15).normalize();
    leaves.leaf(end, ahead, Vec3::Y, leaflet * 1.1, leaflet * 1.1 * aspect, 0.4, 0.3, 0.0, seed % CELLS, false);
    for flank in [-1.0f32, 1.0] {
        let out = (side * flank + forward * 0.35).normalize();
        leaves.leaf(end, out, Vec3::Y, leaflet * 0.9, leaflet * 0.9 * aspect, 0.4, 0.3, 0.0, (seed + 1 + (flank > 0.0) as u32) % CELLS, false);
    }
}

/// Radish, carrot and beet: a few plants to a place, each a small rosette of
/// stalked leaves over its swollen root, the root's shoulder showing at the
/// surface once it fills out.
fn root(species: &Species, s: f32, aspect: f32, detail: Detail) -> Prefab {
    let (plants, gap, shoulder, colour, most, flip) = match species.name {
        "carrot" => (6, 0.035, 0.011, [0.93, 0.45, 0.10, 1.0], 9.0, true),
        "red_beet" => (2, 0.07, 0.028, [0.38, 0.04, 0.12, 1.0], 8.0, false),
        _ => (4, 0.045, 0.016, [0.82, 0.10, 0.18, 1.0], 6.0, false),
    };
    let start = setting(species);
    let swell = ((s - start) / (1.0 - start)).clamp(0.0, 1.0);
    let tall = 0.03 + (species.height_m as f32 * 0.95 - 0.03) * s;
    let (mut leaves, mut parts) = (Shape::of(detail), Shape::of(detail));
    let stalk_colour = if species.name == "red_beet" { [0.55, 0.08, 0.16, 1.0] } else { STEM_GREEN };
    for p in 0..plants {
        let (row, col) = ((p / 3) as f32, (p % 3) as f32);
        let at = match plants {
            2 => Vec3::new((p as f32 - 0.5) * gap, 0.0, 0.0),
            4 => Vec3::new(((p % 2) as f32 - 0.5) * gap, 0.0, ((p / 2) as f32 - 0.5) * gap),
            _ => Vec3::new((col - 1.0) * gap, 0.0, (row - 0.5) * gap),
        } + Vec3::new(jitter(p, 1), 0.0, jitter(p, 2)) * gap * 0.15;
        let count = 2 + (most * s) as u32;
        for i in 0..count {
            let azimuth = i as f32 * GOLDEN_ANGLE + p as f32;
            let youth = i as f32 / count as f32;
            let rise = 0.95 + 0.4 * youth;
            let forward = Vec3::new(azimuth.cos() * rise.cos(), rise.sin(), azimuth.sin() * rise.cos());
            let length = tall * (1.0 - 0.3 * youth) * (1.0 + 0.1 * jitter(p * 31, i));
            let stalk = length * 0.35;
            let end = at + forward * stalk;
            parts.stem(&[at, end], 0.0012, 0.7, stalk_colour);
            let blade = length - stalk;
            leaves.leaf(end, forward, Vec3::Y, blade, blade * aspect * 1.1, 0.5, 0.25, 0.02, (p + i) % CELLS, flip);
        }
        if swell > 0.0 {
            let r = shoulder * (0.25 + 0.75 * swell);
            let (up, squash) = match species.name {
                "carrot" => (0.2, 0.7),
                "red_beet" => (0.4, 0.95),
                _ => (0.25, 0.9),
            };
            parts.blob(at + Vec3::Y * (r * up), Vec3::Y, Vec3::new(r, r * squash, r), colour);
        }
    }
    Prefab { leaves: leaves.into_mesh().expect("a root crop has leaves"), parts: parts.into_mesh(), fruit: None, height: tall, radius: gap + 0.5 * tall }
}

/// Green onions: a bunch of slender white shanks, each with a few hollow
/// leaves standing up and leaning out at the tips.
fn allium(species: &Species, s: f32, _aspect: f32, detail: Detail) -> Prefab {
    let tall = 0.04 + (species.height_m as f32 * 1.1 - 0.04) * s;
    let (mut leaves, mut parts) = (Shape::of(detail), Shape::of(detail));
    for b in 0..6 {
        let a = b as f32 * GOLDEN_ANGLE;
        let base = Vec3::new(a.cos(), 0.0, a.sin()) * 0.014 * (b > 0) as u8 as f32;
        let shank = base + Vec3::new(a.cos() * 0.004, tall * 0.22, a.sin() * 0.004);
        parts.stem(&[base, shank], 0.0035 + 0.002 * s, 0.85, [0.92, 0.94, 0.86, 1.0]);
        for l in 0..(2 + (s * 2.5) as u32) {
            let turn = a + l as f32 * 2.1;
            let forward = Vec3::new(turn.cos() * 0.14, 1.0, turn.sin() * 0.14);
            let length = tall * 0.8 * (1.0 - 0.15 * l as f32) * (1.0 + 0.08 * jitter(b, l));
            // A blade curled hard about its midrib reads as a hollow leaf.
            leaves.leaf(shank, forward, Vec3::Y, length, 0.006 + 0.004 * s, 3.5, 0.35, 0.0, (b + l) % CELLS, false);
        }
    }
    Prefab { leaves: leaves.into_mesh().expect("onions have leaves"), parts: parts.into_mesh(), fruit: None, height: tall, radius: 0.03 + 0.08 * s }
}

/// A chile pepper (APH's 'Española Improved'): a stem that forks into a few
/// branches with simple leaves on short stalks, small white flowers, then
/// single pods hanging under the leaves, green and then red.
fn shrub(species: &Species, s: f32, aspect: f32, detail: Detail) -> Prefab {
    let height = 0.06 + (species.height_m as f32 - 0.06) * s;
    let (mut leaves, mut parts, mut fruit) = (Shape::of(detail), Shape::of(detail), Shape::of(detail));
    let fork = Vec3::Y * height * 0.3;
    parts.stem(&[Vec3::ZERO, fork], 0.003 + 0.003 * s, 0.8, STEM_GREEN);
    for k in 0..2 {
        let azimuth = k as f32 * 2.4;
        let out = Vec3::new(azimuth.cos(), 0.3, azimuth.sin());
        let length = 0.035 + 0.05 * s;
        stalked(&mut leaves, &mut parts, fork * (0.5 + 0.4 * k as f32), out, out, 0.012, length, length * aspect, k, STEM_GREEN);
    }
    let start = setting(species);
    let ripe = ((s - start) / (1.0 - start)).clamp(0.0, 1.0);
    let branches = 4 + (s * 4.0) as u32;
    for b in 0..branches {
        let lean = b as f32 * GOLDEN_ANGLE;
        let spread = 0.3 + 0.2 * (b % 3) as f32;
        let top = fork + Vec3::new(lean.cos() * spread * height, height * (0.55 + 0.15 * (b % 2) as f32), lean.sin() * spread * height);
        let path: Vec<Vec3> = (0..7).map(|k| fork.lerp(top, k as f32 / 6.0) + Vec3::Y * 0.01 * (k as f32 * 0.6).sin()).collect();
        parts.stem(&path, 0.0022 + 0.002 * s, 0.6, STEM_GREEN);
        for k in 1..7 {
            // A pair of leaves at every node, the upper ones smaller.
            for pair in 0..2 {
                let azimuth = lean + k as f32 * GOLDEN_ANGLE + pair as f32 * 2.6;
                let out = Vec3::new(azimuth.cos(), 0.3 + 0.15 * pair as f32, azimuth.sin());
                let length = (0.04 + 0.08 * s) * (1.15 - 0.1 * k as f32) * (1.0 - 0.2 * pair as f32);
                stalked(&mut leaves, &mut parts, path[k], out, out, 0.014, length, length * aspect, (b + k as u32 + pair) % CELLS, STEM_GREEN);
            }
            let azimuth = lean + k as f32 * GOLDEN_ANGLE;
            if (start - 0.15..start + 0.2).contains(&s) && k % 2 == 1 {
                parts.blob(path[k] + Vec3::new(-azimuth.cos() * 0.012, -0.004, -azimuth.sin() * 0.012), Vec3::Y, Vec3::new(0.005, 0.003, 0.005), [0.95, 0.95, 0.92, 1.0]);
            }
            if s >= start && k % 2 == 0 && k < 6 {
                let long = 0.02 + 0.055 * ripe;
                let hang = path[k] + Vec3::new(-azimuth.cos() * 0.014, -0.008 - long * 0.5, -azimuth.sin() * 0.014);
                let colour = [0.16 + 0.66 * ripe, 0.36 - 0.31 * ripe, 0.06, 1.0];
                parts.stem(&[path[k], hang + Vec3::Y * long * 0.5], 0.0018, 0.9, STEM_GREEN);
                fruit.blob(hang, Vec3::new(0.15 * azimuth.cos(), 1.0, 0.15 * azimuth.sin()), Vec3::new(0.011, long * 0.5, 0.011), colour);
            }
        }
    }
    Prefab { leaves: leaves.into_mesh().expect("a pepper has leaves"), parts: parts.into_mesh(), fruit: fruit.into_mesh(), height, radius: 0.06 + 0.16 * s }
}

/// A day-neutral strawberry: a crown of trifoliate leaves on long arching
/// stalks, white flowers, then trusses hanging out past the leaves with
/// berries going from green to white to red.
fn crown(species: &Species, s: f32, aspect: f32, detail: Detail) -> Prefab {
    let (mut leaves, mut parts, mut fruit) = (Shape::of(detail), Shape::of(detail), Shape::of(detail));
    parts.blob(Vec3::Y * 0.008, Vec3::Y, Vec3::new(0.012, 0.01, 0.012), [0.36, 0.30, 0.16, 1.0]);
    let count = 5 + (11.0 * s) as u32;
    for i in 0..count {
        let azimuth = i as f32 * GOLDEN_ANGLE;
        let youth = i as f32 / count as f32;
        let rise = 0.75 + 0.5 * youth;
        let forward = Vec3::new(azimuth.cos() * rise.cos(), rise.sin(), azimuth.sin() * rise.cos());
        let stalk = (0.04 + 0.11 * s) * (1.0 - 0.3 * youth);
        trifoliate(&mut leaves, &mut parts, Vec3::Y * 0.01, forward, stalk, 0.025 + 0.03 * s, aspect, i);
    }
    let start = setting(species);
    let ripe = ((s - start) / (1.0 - start)).clamp(0.0, 1.0);
    if s > start - 0.2 {
        for t in 0..3 {
            let azimuth = t as f32 * 2.1 + 0.4;
            let out = Vec3::new(azimuth.cos(), 0.0, azimuth.sin());
            let reach = 0.09 + 0.06 * s;
            let top = Vec3::Y * 0.06 + out * reach * 0.6;
            let tip = out * reach + Vec3::Y * 0.035;
            parts.stem(&[Vec3::Y * 0.01, top, tip], 0.0013, 0.7, STEM_GREEN);
            for b in 0..3 {
                let at = tip + Vec3::new(jitter(t, b), -0.6, jitter(b, t)) * 0.012 * b as f32;
                if s < start {
                    parts.blob(at, Vec3::Y, Vec3::new(0.008, 0.003, 0.008), [0.96, 0.96, 0.92, 1.0]);
                } else {
                    let r = 0.006 + 0.008 * ripe;
                    let colour = if ripe < 0.5 { [0.55 + 0.8 * ripe, 0.70, 0.40 + 0.4 * ripe, 1.0] } else { [0.95, 0.70 - 1.2 * (ripe - 0.5), 0.60 - 1.0 * (ripe - 0.5), 1.0] };
                    fruit.blob(at - Vec3::Y * r, Vec3::Y, Vec3::new(r, r * 1.3, r), colour);
                    parts.blob(at, Vec3::Y, Vec3::new(r * 0.8, 0.002, r * 0.8), STEM_GREEN);
                }
            }
        }
    }
    Prefab { leaves: leaves.into_mesh().expect("a strawberry has leaves"), parts: parts.into_mesh(), fruit: fruit.into_mesh(), height: 0.06 + 0.18 * s, radius: 0.06 + 0.12 * s }
}

/// Soybean, beans, pea and peanut: a stem with a couple of branches and a
/// trifoliate leaf at every node (pea's leaflets in pairs), flowers mid-season,
/// then pods: short and clustered on soybean, long on beans and peas, none to
/// see on peanut, whose pods grow underground.
fn legume(species: &Species, s: f32, aspect: f32, detail: Detail) -> Prefab {
    let height = 0.05 + (species.height_m as f32 - 0.05) * s;
    let (mut leaves, mut parts, mut fruit) = (Shape::of(detail), Shape::of(detail), Shape::of(detail));
    let (pod, pod_r, per_node, flower) = match species.name {
        "soybean" => (0.035, 0.0055, 3, [0.62, 0.42, 0.80, 1.0]),
        "dry_bean" => (0.08, 0.0055, 2, [0.95, 0.94, 0.90, 1.0]),
        "snap_bean" => (0.11, 0.0048, 2, [0.95, 0.94, 0.90, 1.0]),
        "pea" => (0.065, 0.0065, 2, [0.96, 0.96, 0.98, 1.0]),
        _ => (0.0, 0.0, 0, [0.98, 0.82, 0.18, 1.0]),
    };
    let start = setting(species);
    let ripe = ((s - start) / (1.0 - start)).clamp(0.0, 1.0);
    // Dry pods go from green to straw; snap beans are picked green.
    let pod_colour = if species.edible == gearbox_greenhouse::Edible::Fruit {
        [0.32, 0.56, 0.18, 1.0]
    } else {
        [0.34 + 0.42 * ripe, 0.54 + 0.08 * ripe, 0.18 + 0.20 * ripe, 1.0]
    };
    // Peanut spreads low from many branches; the rest stand up from one stem.
    let low = species.name == "peanut";
    let stems = if low { 2 + (s * 4.0) as u32 } else { 1 + (s * 2.0) as u32 };
    for st in 0..stems {
        let lean = st as f32 * GOLDEN_ANGLE + 0.3;
        let splay = if low { 0.6 } else if st == 0 { 0.05 } else { 0.35 };
        let tall = height * if low { 0.6 } else if st == 0 { 1.0 } else { 0.8 };
        let path: Vec<Vec3> = (0..6)
            .map(|k| {
                let t = k as f32 / 5.0;
                Vec3::new(lean.cos() * splay * tall * t, tall * t, lean.sin() * splay * tall * t)
            })
            .collect();
        parts.stem(&path, 0.0022 + 0.0025 * s, 0.6, STEM_GREEN);
        let nodes = 2 + (s * 4.0) as u32;
        for k in 0..nodes {
            let t = 0.25 + 0.75 * k as f32 / nodes as f32;
            let at = path[((t * 5.0).round() as usize).min(5)];
            let azimuth = lean + k as f32 * 2.6;
            let out = Vec3::new(azimuth.cos(), 0.45, azimuth.sin());
            let size = (0.025 + 0.03 * s) * (1.1 - 0.3 * t);
            if species.name == "pea" {
                compound(&mut leaves, &mut parts, at, out, size * 2.2, size * 0.7, aspect, 2, st * 5 + k);
            } else {
                trifoliate(&mut leaves, &mut parts, at, out, 0.03 + 0.04 * s, size, aspect, st * 5 + k);
            }
            let flowering = (start - 0.2..start + 0.1).contains(&s);
            if flowering && k % 2 == 0 {
                parts.blob(at + Vec3::new(-azimuth.cos(), 0.2, -azimuth.sin()) * 0.01, Vec3::Y, Vec3::new(0.004, 0.003, 0.004), flower);
            }
            if s >= start && pod > 0.0 && t < 0.9 {
                for p in 0..per_node {
                    let a = azimuth + PI_HALF + p as f32 * 0.7;
                    let hang = at + Vec3::new(a.cos() * 0.01, -0.004, a.sin() * 0.01);
                    let long = pod * (0.4 + 0.6 * ripe.max(0.3));
                    let axis = Vec3::new(a.cos() * 0.35, -1.0, a.sin() * 0.35);
                    let target = if species.edible == gearbox_greenhouse::Edible::Fruit { &mut fruit } else { &mut parts };
                    target.blob(hang + axis.normalize() * long * 0.5, axis, Vec3::new(pod_r, long * 0.5, pod_r * 0.6), pod_colour);
                }
            }
        }
    }
    Prefab { leaves: leaves.into_mesh().expect("a legume has leaves"), parts: parts.into_mesh(), fruit: fruit.into_mesh(), height, radius: 0.06 + 0.16 * s }
}

const PI_HALF: f32 = std::f32::consts::FRAC_PI_2;

/// Sweet potato: vines trailing out over the bed from the crown, with a
/// heart-shaped leaf standing up on a long stalk at every node.
fn creeper(species: &Species, s: f32, aspect: f32, detail: Detail) -> Prefab {
    let (mut leaves, mut parts) = (Shape::of(detail), Shape::of(detail));
    let vines = 3 + (s * 4.0) as u32;
    let reach = 0.05 + 0.21 * s;
    let tall = species.height_m as f32 * 0.45 * s;
    for v in 0..vines {
        let azimuth = v as f32 * GOLDEN_ANGLE;
        let out = Vec3::new(azimuth.cos(), 0.0, azimuth.sin());
        let long = reach * (0.8 + 0.2 * jitter(v, 3).abs());
        let path: Vec<Vec3> = (0..6).map(|k| {
            let t = k as f32 / 5.0;
            out * long * t + Vec3::Y * (0.02 + 0.03 * (t * std::f32::consts::PI).sin())
        }).collect();
        parts.stem(&path, 0.0025, 0.6, [0.42, 0.40, 0.18, 1.0]);
        let nodes = 2 + (s * 4.0) as u32;
        for k in 0..nodes {
            let t = (k + 1) as f32 / (nodes + 1) as f32;
            let at = path[((t * 5.0).round() as usize).min(5)];
            let side = out.cross(Vec3::Y) * if k % 2 == 0 { 1.0 } else { -1.0 };
            let up = (Vec3::Y + side * 0.25 + out * 0.2).normalize();
            let length = (0.035 + 0.045 * s) * (1.0 + 0.1 * jitter(v, k));
            let stalk = 0.03 + tall * 0.6 * (1.0 - 0.4 * t);
            let blade = (out * 0.6 + side * 0.3 + Vec3::Y * 0.35).normalize();
            stalked(&mut leaves, &mut parts, at, up, blade, stalk, length, length * aspect, (v + k) % CELLS, STEM_GREEN);
        }
    }
    Prefab { leaves: leaves.into_mesh().expect("a sweet potato has leaves"), parts: parts.into_mesh(), fruit: None, height: 0.05 + tall, radius: reach }
}

/// A compound leaf: a rachis from `base` along `forward`, arching over under
/// its own weight, with leaflet pairs along it and one at its tip.
#[allow(clippy::too_many_arguments)]
fn compound(leaves: &mut Shape, parts: &mut Shape, base: Vec3, forward: Vec3, length: f32, leaflet: f32, aspect: f32, pairs: u32, seed: u32) {
    let forward = forward.normalize();
    let side = forward.cross(Vec3::Y).normalize_or(Vec3::X);
    let droop = |t: f32| Vec3::Y * (-0.55 * length * t * t);
    let path: Vec<Vec3> = (0..6).map(|k| base + forward * (length * k as f32 / 5.0) + droop(k as f32 / 5.0)).collect();
    parts.stem(&path, 0.0016, 0.5, STEM_GREEN);
    for p in 0..pairs {
        let t = 0.25 + 0.65 * p as f32 / pairs.max(1) as f32;
        let at = base + forward * (length * t) + droop(t);
        let size = leaflet * (0.7 + 0.35 * t) * (1.0 + 0.12 * jitter(seed, p));
        for flank in [-1.0f32, 1.0] {
            let out = (side * flank + forward * 0.45 - Vec3::Y * 0.15).normalize();
            leaves.leaf(at, out, Vec3::Y, size, size * aspect * 1.1, 0.55, 0.35, 0.0, (seed + p + (flank > 0.0) as u32) % CELLS, false);
        }
    }
    let tip = base + forward * length + droop(1.0);
    let bend = (forward - Vec3::Y * 0.5).normalize();
    leaves.leaf(tip, bend, Vec3::Y, leaflet * 1.15, leaflet * 1.15 * aspect * 1.1, 0.55, 0.4, 0.0, seed % CELLS, false);
}

/// A dwarf tomato (like ISS Veggie's 'Red Robin'): a short stem with a
/// compound leaf every few centimetres, crowded and arching, and once fruit
/// sets, trusses that go from green to red as they ripen.
fn tomato(species: &Species, s: f32, aspect: f32, detail: Detail) -> Prefab {
    let height = 0.06 + (species.height_m as f32 - 0.06) * s;
    let (mut leaves, mut parts, mut fruit) = (Shape::of(detail), Shape::of(detail), Shape::of(detail));
    let nodes = (3.0 + height / 0.03).round() as u32;
    let stem: Vec<Vec3> = (0..=nodes).map(|k| {
        let t = k as f32 / nodes as f32;
        // A stem zigzags a little from node to node.
        let zig = if k % 2 == 0 { 0.006 } else { -0.006 };
        Vec3::new(zig + 0.01 * (t * 4.0).sin(), height * t, 0.008 * (t * 3.0).cos() - 0.008)
    }).collect();
    parts.stem(&stem, 0.003 + 0.0025 * s, 0.45, STEM_GREEN);
    let setting = ((species.edible_days / species.harvest_days) as f32).min(0.95);
    for k in 1..=nodes {
        let t = k as f32 / nodes as f32;
        let azimuth = k as f32 * GOLDEN_ANGLE;
        let out = Vec3::new(azimuth.cos(), 0.75 - 0.35 * t, azimuth.sin());
        // Older leaves low on the stem are longest; the youngest at the top are small.
        let length = (0.07 + 0.15 * s) * (1.15 - 0.55 * t);
        let pairs = 2 + (s * 2.5 * (1.0 - 0.4 * t)) as u32;
        compound(&mut leaves, &mut parts, stem[k.min(nodes) as usize], out, length, 0.028 + 0.024 * s, aspect, pairs, k);
        if s >= setting && k % 3 == 1 && t < 0.85 {
            let ripe = ((s - setting) / (1.0 - setting)).clamp(0.0, 1.0);
            let colour = [0.30 + 0.58 * ripe, 0.50 - 0.42 * ripe, 0.10 - 0.04 * ripe, 1.0];
            let hang = stem[k as usize] + Vec3::new(-azimuth.cos(), -0.8, -azimuth.sin()) * 0.03;
            parts.stem(&[stem[k as usize], hang], 0.0012, 0.8, STEM_GREEN);
            for f in 0..6 {
                let a = f as f32 * 1.25 + azimuth;
                let r = 0.012 + 0.003 * jitter(k, f);
                let at = hang + Vec3::new(a.cos() * 0.02, -0.011 * f as f32, a.sin() * 0.02);
                fruit.blob(at, Vec3::Y, Vec3::splat(r) * Vec3::new(1.0, 0.9, 1.0), colour);
                parts.blob(at + Vec3::Y * r * 0.9, Vec3::Y, Vec3::new(0.005, 0.0012, 0.005), STEM_GREEN);
            }
        }
    }
    Prefab {
        leaves: leaves.into_mesh().expect("a tomato has leaves"),
        parts: parts.into_mesh(),
        fruit: fruit.into_mesh(),
        height,
        radius: 0.06 + 0.12 * s,
    }
}

/// A potato plant: a few leaning stems crowded with compound leaves into a
/// low bush, white flowers mid-season while tubers form below.
fn potato(species: &Species, s: f32, aspect: f32, detail: Detail) -> Prefab {
    let height = 0.05 + (species.height_m as f32 - 0.05) * s;
    let (mut leaves, mut parts) = (Shape::of(detail), Shape::of(detail));
    let stems = 3 + (s * 2.5) as u32;
    let flowering = (0.45..0.8).contains(&s);
    for st in 0..stems {
        let lean = st as f32 * GOLDEN_ANGLE;
        let tilt = 0.25 + 0.15 * (st % 2) as f32;
        let tall = height * (0.8 + 0.2 * jitter(st, 1).abs());
        let path: Vec<Vec3> = (0..7)
            .map(|k| {
                let t = k as f32 / 6.0;
                Vec3::new(lean.cos() * tilt * tall * t, tall * t * (1.0 - 0.1 * t), lean.sin() * tilt * tall * t)
            })
            .collect();
        parts.stem(&path, 0.003 + 0.003 * s, 0.5, STEM_GREEN);
        let nodes = 3 + (s * 5.0) as u32;
        for k in 0..nodes {
            // Leaves crowd up to the tip, so no stem stands bare above the bush.
            let t = 0.2 + 0.8 * k as f32 / (nodes - 1).max(1) as f32;
            let at = path[((t * 6.0).round() as usize).min(6)];
            let azimuth = lean + k as f32 * GOLDEN_ANGLE;
            let out = Vec3::new(azimuth.cos(), 0.55, azimuth.sin());
            let length = (0.07 + 0.13 * s) * (1.1 - 0.4 * t);
            compound(&mut leaves, &mut parts, at, out, length, 0.035 + 0.03 * s, aspect, 2 + (s * 2.0) as u32, st * 7 + k);
        }
        if flowering {
            let top = path[6];
            for f in 0..5 {
                let a = f as f32 * 1.3 + lean;
                let at = top + Vec3::new(a.cos() * 0.022, 0.03 + 0.012 * jitter(st, f), a.sin() * 0.022);
                parts.stem(&[top, at], 0.0012, 0.8, STEM_GREEN);
                parts.blob(at, Vec3::Y, Vec3::new(0.011, 0.003, 0.011), [0.94, 0.93, 0.98, 1.0]);
                parts.blob(at + Vec3::Y * 0.004, Vec3::Y, Vec3::new(0.0025, 0.004, 0.0025), [0.96, 0.82, 0.12, 1.0]);
            }
        }
    }
    Prefab { leaves: leaves.into_mesh().expect("a potato has leaves"), parts: parts.into_mesh(), fruit: None, height, radius: 0.08 + 0.2 * s }
}

/// Wheat and rice: tillers of arching blades, each stem ending in an ear
/// (wheat) or a drooping panicle (rice) once it heads, green going gold as
/// the grain ripens.
fn grass(species: &Species, s: f32, aspect: f32, detail: Detail) -> Prefab {
    let rice = species.name == "rice";
    let height = 0.06 + (species.height_m as f32 - 0.06) * s;
    let (mut leaves, mut parts) = (Shape::of(detail), Shape::of(detail));
    let tillers = if rice { 3 + (s * 6.0) as u32 } else { 2 + (s * 4.0) as u32 };
    let heading = ((species.edible_days / species.harvest_days) as f32 * 0.85).min(0.9);
    let gold = ((s - heading) / (1.0 - heading)).clamp(0.0, 1.0);
    let straw = [0.30 + 0.48 * gold, 0.48 + 0.20 * gold, 0.16 + 0.12 * gold, 1.0];
    for t in 0..tillers {
        let lean = t as f32 * GOLDEN_ANGLE;
        let splay = 0.05 + 0.04 * (t % 3) as f32;
        let top = Vec3::new(lean.cos() * splay * height, height * (0.9 + 0.1 * jitter(t, 2)), lean.sin() * splay * height);
        let path: Vec<Vec3> = (0..6).map(|k| top * (k as f32 / 5.0)).collect();
        parts.stem(&path, 0.0018, 0.7, straw);
        for b in 0..(4 + (s * 2.0) as u32) {
            let a = lean + b as f32 * 2.3 + 0.3 * jitter(t, b);
            let at = top * (0.05 + 0.13 * b as f32).min(0.7);
            let length = (0.10 + 0.15 * s) * (1.0 - 0.08 * b as f32);
            let out = Vec3::new(a.cos() * 0.4, 1.0, a.sin() * 0.4);
            leaves.leaf(at, out, Vec3::Y, length, length * aspect * 0.5, 0.35, 0.95, 0.0, (t + b) % CELLS, false);
        }
        if s >= heading && rice {
            // A panicle arches over from the top, its spikelets on short branches hanging off it.
            let out = Vec3::new(lean.cos(), 0.0, lean.sin());
            let arch: Vec<Vec3> = (0..6).map(|k| {
                let t = k as f32 / 5.0;
                top + Vec3::Y * (0.06 * t * (1.0 - 0.9 * t)) + out * (0.07 * t) - Vec3::Y * (0.05 * t * t * gold.max(0.4))
            }).collect();
            parts.stem(&arch, 0.0012, 0.6, straw);
            for (k, at) in arch.iter().enumerate().skip(1) {
                for b in 0..3 {
                    let a = lean + b as f32 * 2.1 + k as f32;
                    let grain = *at + Vec3::new(a.cos() * 0.006, -0.006 - 0.004 * b as f32, a.sin() * 0.006);
                    parts.blob(grain, Vec3::new(a.cos(), -1.0, a.sin()), Vec3::new(0.0018, 0.0042, 0.0016), straw);
                }
            }
        } else if s >= heading {
            for k in 0..9 {
                let at = top + Vec3::Y * (0.008 * k as f32);
                for flank in [-1.0f32, 1.0] {
                    let side = Vec3::new(lean.sin(), 0.0, -lean.cos()) * flank * 0.0035;
                    parts.blob(at + side, Vec3::Y, Vec3::new(0.0028, 0.0055, 0.0022), straw);
                }
            }
        }
    }
    Prefab { leaves: leaves.into_mesh().expect("a grass has leaves"), parts: parts.into_mesh(), fruit: None, height, radius: 0.04 + 0.08 * s }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::mesh::VertexAttributeValues;
    use gearbox_greenhouse::species::ALL;

    fn bounds(mesh: &Mesh) -> (Vec3, Vec3) {
        let Some(VertexAttributeValues::Float32x3(points)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!() };
        points.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| {
            (lo.min(Vec3::from_array(*p)), hi.max(Vec3::from_array(*p)))
        })
    }

    #[test]
    fn every_crop_grows_from_a_sprout_to_its_mature_size() {
        for species in ALL {
            let young = prefab(species, 0, Detail::Near);
            let ripe = prefab(species, STAGES - 1, Detail::Near);
            let (lo, hi) = bounds(&ripe.leaves);
            let (ylo, yhi) = bounds(&young.leaves);
            assert!((hi - lo).max_element() > 2.0 * (yhi - ylo).max_element(), "{} grows", species.name);
            assert!(hi.y < species.height_m as f32 * 1.4 && lo.y > -0.12, "{} stands {lo} to {hi}", species.name);
            assert!((hi - lo).xz().max_element() < 0.7, "{} spreads {lo} to {hi}", species.name);
            let triangles = ripe.leaves.indices().unwrap().len() / 3;
            assert!(triangles < 12_000, "{}: {triangles} leaf triangles", species.name);
        }
    }

    #[test]
    fn a_far_plant_keeps_its_outline_on_a_fraction_of_the_triangles() {
        let triangles = |p: &Prefab| {
            [Some(&p.leaves), p.parts.as_ref(), p.fruit.as_ref()].into_iter().flatten().map(|m| m.indices().unwrap().len() / 3).sum::<usize>()
        };
        for species in ALL {
            let (near, far) = (prefab(species, STAGES - 1, Detail::Near), prefab(species, STAGES - 1, Detail::Far));
            // A cheap plant need not lose much more.
            let (f, n) = (triangles(&far), triangles(&near));
            assert!(f * 3 <= n || f < 500, "{}: {f} far, {n} near", species.name);
            let (lo, hi) = bounds(&near.leaves);
            let (flo, fhi) = bounds(&far.leaves);
            assert!((fhi - flo - (hi - lo)).abs().max_element() < 0.15 * (hi - lo).max_element(), "{} keeps its size", species.name);
        }
    }

    #[test]
    fn a_lettuce_adds_a_leaf_a_day_and_tomatoes_set_fruit_late() {
        let leaves = |stage| prefab(&gearbox_greenhouse::species::LETTUCE, stage, Detail::Near).leaves.indices().unwrap().len();
        assert!(leaves(STAGES - 1) > 8 * leaves(0));
        let tomato = &gearbox_greenhouse::species::TOMATO;
        assert!(prefab(tomato, 1, Detail::Near).fruit.is_none());
        assert!(prefab(tomato, STAGES - 1, Detail::Near).fruit.is_some());
    }

    #[test]
    fn every_atlas_decodes_with_its_cells() {
        use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
        for species in ALL {
            let atlas = atlas(species);
            let image = Image::from_buffer(
                atlas.png,
                ImageType::Extension("png"),
                CompressedImageFormats::NONE,
                true,
                ImageSampler::Default,
                RenderAssetUsages::MAIN_WORLD,
            )
            .expect("png");
            let cell = image.width() / CELLS;
            assert!((cell as f32 / image.height() as f32 - atlas.aspect).abs() < 0.01);
            // Some texels are cut away and some kept: the leaves' outlines.
            let alphas: Vec<f32> = (0..image.width()).step_by(7).map(|x| image.get_color_at(x, image.height() / 2).unwrap().alpha()).collect();
            assert!(alphas.iter().any(|a| *a < 0.1) && alphas.iter().any(|a| *a > 0.9), "{}", species.name);
        }
    }
}

/// Writes every prefab as OBJ, its leaf atlas beside it, to look at in a
/// modelling tool: `cargo test -p gearbox-sim --bin gearbox export_crop_prefabs -- --ignored`.
#[cfg(test)]
mod export {
    use super::*;
    use bevy::mesh::VertexAttributeValues;
    use gearbox_greenhouse::species::ALL;
    use std::fmt::Write as _;

    fn obj(out: &mut String, mesh: &Mesh, material: &str, base: &mut usize) {
        let Some(VertexAttributeValues::Float32x3(points)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { return };
        let Some(VertexAttributeValues::Float32x2(uvs)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0) else { return };
        let Some(VertexAttributeValues::Float32x4(colours)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else { return };
        let Some(Indices::U32(indices)) = mesh.indices() else { return };
        let _ = writeln!(out, "usemtl {material}");
        for (p, c) in points.iter().zip(colours) {
            let _ = writeln!(out, "v {} {} {} {} {} {}", p[0], p[1], p[2], c[0], c[1], c[2]);
        }
        for uv in uvs {
            let _ = writeln!(out, "vt {} {}", uv[0], 1.0 - uv[1]);
        }
        for t in indices.chunks_exact(3) {
            let [a, b, c] = [t[0] as usize + *base + 1, t[1] as usize + *base + 1, t[2] as usize + *base + 1];
            let _ = writeln!(out, "f {a}/{a} {b}/{b} {c}/{c}");
        }
        *base += points.len();
    }

    #[test]
    #[ignore]
    fn export_crop_prefabs() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/crops/prefabs");
        std::fs::create_dir_all(&dir).unwrap();
        for species in ALL {
            std::fs::write(dir.join(format!("{}_leaves.png", species.name)), atlas(species).png).unwrap();
            let mtl = format!(
                "newmtl leaves\nmap_Kd {0}_leaves.png\nmap_d {0}_leaves.png\nnewmtl parts\nKd 1 1 1\nnewmtl fruit\nKd 1 1 1\n",
                species.name
            );
            std::fs::write(dir.join(format!("{}.mtl", species.name)), mtl).unwrap();
            for stage in [1, STAGES / 2, STAGES - 1] {
                let prefab = prefab(species, stage, Detail::Near);
                let mut out = format!("mtllib {}.mtl\n", species.name);
                let mut base = 0;
                obj(&mut out, &prefab.leaves, "leaves", &mut base);
                for (part, name) in [(&prefab.parts, "parts"), (&prefab.fruit, "fruit")] {
                    if let Some(mesh) = part {
                        obj(&mut out, mesh, name, &mut base);
                    }
                }
                std::fs::write(dir.join(format!("{}_{stage}.obj", species.name)), out).unwrap();
            }
        }
    }
}
