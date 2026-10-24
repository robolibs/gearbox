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

/// The leaf atlas of a crop, as PNG bytes.
pub fn atlas(species: &Species) -> &'static [u8] {
    match species.form {
        Form::Rosette => include_bytes!("../../assets/crops/lettuce_leaves.png"),
        Form::Vine => include_bytes!("../../assets/crops/tomato_leaves.png"),
        Form::Bush => include_bytes!("../../assets/crops/potato_leaves.png"),
        Form::Grass => include_bytes!("../../assets/crops/wheat_leaves.png"),
    }
}

/// Width over height of one atlas cell: a leaf card keeps its photo's proportions.
fn cell_aspect(form: Form) -> f32 {
    match form {
        Form::Rosette => 256.0 / 288.0,
        Form::Vine => 160.0 / 288.0,
        Form::Bush => 192.0 / 288.0,
        Form::Grass => 48.0 / 512.0,
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

/// Geometry being built.
#[derive(Default)]
struct Shape {
    positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    colours: Vec<[f32; 4]>,
    indices: Vec<u32>,
}

impl Shape {
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
        let (across, along) = if length < 0.09 { (3, 5) } else { (5, 9) };
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
        let sides = 6;
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
        self.grid(9, 7, |u, v| {
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
pub fn prefab(species: &Species, stage: usize) -> Prefab {
    let s = stage.min(STAGES - 1) as f32 / (STAGES - 1) as f32;
    let aspect = cell_aspect(species.form);
    match species.form {
        Form::Rosette => lettuce(s, aspect),
        Form::Vine => tomato(species, s, aspect),
        Form::Bush => potato(species, s, aspect),
        Form::Grass => wheat(species, s, aspect),
    }
}

/// A head of lettuce: one leaf a day from three to thirty-four, each set a
/// golden angle round from the last, the young ones small and upright in the
/// heart, the old ones broad and spread.
fn lettuce(s: f32, aspect: f32) -> Prefab {
    let count = (3.0 + 31.0 * s).round() as u32;
    let mut leaves = Shape::default();
    let span = 0.035 + 0.10 * s;
    for i in 0..count {
        let age = (count - i) as f32;
        // 0 for the oldest, outermost leaf, 1 for the youngest in the heart.
        let youth = i as f32 / count.max(1) as f32;
        let grown = (1.0 - (-age / 6.0).exp()).max(0.3);
        let length = span * grown * (1.0 - 0.35 * youth) * (1.0 + 0.08 * jitter(7, i));
        let azimuth = i as f32 * GOLDEN_ANGLE + 0.15 * jitter(3, i);
        // Outer leaves lie out over the tray; the heart stands and closes.
        let rise = 0.18 + 1.15 * youth.powf(1.4);
        let forward = Vec3::new(azimuth.cos() * rise.cos(), rise.sin(), azimuth.sin() * rise.cos());
        let out = Vec3::new(azimuth.cos(), 0.0, azimuth.sin()) * 0.006 * (1.0 - youth);
        let width = length * aspect * (1.25 - 0.3 * youth);
        let cup = 1.1 + 1.4 * youth;
        let droop = 0.35 * (1.0 - youth) * grown;
        leaves.leaf(out + Vec3::Y * 0.012 * youth, forward, Vec3::Y, length, width, cup, droop, 0.06, i % CELLS, true);
    }
    Prefab { leaves: leaves.into_mesh().expect("a lettuce has leaves"), parts: None, fruit: None, height: 0.05 + 0.13 * s, radius: span }
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
fn tomato(species: &Species, s: f32, aspect: f32) -> Prefab {
    let height = 0.06 + (species.height_m as f32 - 0.06) * s;
    let (mut leaves, mut parts, mut fruit) = (Shape::default(), Shape::default(), Shape::default());
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
fn potato(species: &Species, s: f32, aspect: f32) -> Prefab {
    let height = 0.05 + (species.height_m as f32 - 0.05) * s;
    let (mut leaves, mut parts) = (Shape::default(), Shape::default());
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

/// Wheat: tillers of arching blades, each stem ending in an ear once it heads,
/// green going gold as the grain ripens.
fn wheat(species: &Species, s: f32, aspect: f32) -> Prefab {
    let height = 0.06 + (species.height_m as f32 - 0.06) * s;
    let (mut leaves, mut parts) = (Shape::default(), Shape::default());
    let tillers = 2 + (s * 4.0) as u32;
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
        if s >= heading {
            for k in 0..9 {
                let at = top + Vec3::Y * (0.008 * k as f32);
                for flank in [-1.0f32, 1.0] {
                    let side = Vec3::new(lean.sin(), 0.0, -lean.cos()) * flank * 0.0035;
                    parts.blob(at + side, Vec3::Y, Vec3::new(0.0028, 0.0055, 0.0022), straw);
                }
            }
        }
    }
    Prefab { leaves: leaves.into_mesh().expect("wheat has leaves"), parts: parts.into_mesh(), fruit: None, height, radius: 0.04 + 0.08 * s }
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
            let young = prefab(species, 0);
            let ripe = prefab(species, STAGES - 1);
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
    fn a_lettuce_adds_a_leaf_a_day_and_tomatoes_set_fruit_late() {
        let leaves = |stage| prefab(&gearbox_greenhouse::species::LETTUCE, stage).leaves.indices().unwrap().len();
        assert!(leaves(STAGES - 1) > 8 * leaves(0));
        let tomato = &gearbox_greenhouse::species::TOMATO;
        assert!(prefab(tomato, 1).fruit.is_none());
        assert!(prefab(tomato, STAGES - 1).fruit.is_some());
    }

    #[test]
    fn every_atlas_decodes_with_its_cells() {
        use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
        for species in ALL {
            let image = Image::from_buffer(
                atlas(species),
                ImageType::Extension("png"),
                CompressedImageFormats::NONE,
                true,
                ImageSampler::Default,
                RenderAssetUsages::MAIN_WORLD,
            )
            .expect("png");
            let cell = image.width() / CELLS;
            assert!((cell as f32 / image.height() as f32 - cell_aspect(species.form)).abs() < 0.01);
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
            std::fs::write(dir.join(format!("{}_leaves.png", species.name)), atlas(species)).unwrap();
            let mtl = format!(
                "newmtl leaves\nmap_Kd {0}_leaves.png\nmap_d {0}_leaves.png\nnewmtl parts\nKd 1 1 1\nnewmtl fruit\nKd 1 1 1\n",
                species.name
            );
            std::fs::write(dir.join(format!("{}.mtl", species.name)), mtl).unwrap();
            for stage in [1, STAGES / 2, STAGES - 1] {
                let prefab = prefab(species, stage);
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
