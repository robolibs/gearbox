//! Ground worked by tools: soil an implement has cut, loosened and ridged.
//!
//! A tool working a field stamps a work map, one per field that can be
//! worked, in the same texels as the field's wheel map. A worked texel keeps
//! which tool cut it, the way it was going, where across the tool's width
//! it lay and how far inside the strip's edge, and keeps it for good: unlike
//! a wheel mark, worked soil does not
//! spring back. The cover reads the map to bury what stood on the ground, and
//! a relief layer streamed round the camera raises the worked soil itself —
//! loosened, ridged by the discs and broken into clods — as real geometry,
//! not as a texture laid over the stubble.

use super::layout::FieldBounds;
use super::profile::VegetationLayer;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::platform::collections::HashSet;
use bevy::prelude::*;
use bevy::render::extract_resource::ExtractResource;

/// What cut the ground. Kept in four bits of the work map, nought for ground
/// no tool has touched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    /// Gangs of concave discs: shallow, leaves the soil in low ridges a disc
    /// spacing apart and broken into clods, with some straw left on top.
    DiscHarrow,
}

impl ToolKind {
    /// The tool a machine of this `gearbox:machine:kind` works the ground as.
    pub fn of_machine(kind: &str) -> Option<Self> {
        match kind {
            "disc_harrow" | "disc" | "harrow" | "short_disc" => Some(Self::DiscHarrow),
            _ => None,
        }
    }

    fn code(self) -> u16 {
        match self {
            Self::DiscHarrow => 1,
        }
    }
}

/// One part of a tool on the ground this frame: the patch it is working.
#[derive(Clone, Copy, Debug)]
pub struct ToolContact {
    pub kind: ToolKind,
    /// The middle of the worked patch, on the ground, in world metres.
    pub position: Vec3,
    /// The way the tool is going, in world XZ.
    pub direction: Vec2,
    /// Where across the whole tool the patch's middle lies, in metres from
    /// the tool's middle: the parts of one tool share one set of ridges.
    pub across: f32,
    /// Working width across the way it goes.
    pub width: f32,
    /// The whole tool's outer edges, measured as `across` is, along the
    /// direction turned a quarter clockwise.
    pub edges: Vec2,
    /// How much of the way it goes the strip covers this frame.
    pub length: f32,
    /// How deep the tool works, in metres.
    pub depth: f32,
}

/// Tool contacts collected during one frame, stamped into the work maps by
/// the render world.
#[derive(Resource, ExtractResource, Clone, Default)]
pub struct ToolContacts {
    pub contacts: Vec<ToolContact>,
}

pub(crate) fn begin_tool_contacts(mut contacts: ResMut<ToolContacts>) {
    contacts.contacts.clear();
}

/// Side of the cells `WorkedCells` keeps, in metres.
pub const WORKED_CELL_M: f32 = 4.0;

/// The ground any tool has worked, kept coarse on this side so the relief
/// layer streams only where there is worked soil to raise. The texels
/// themselves live on the GPU alone.
#[derive(Resource, Default)]
pub struct WorkedCells(pub HashSet<(i32, i32)>);

impl WorkedCells {
    /// Whether any worked cell touches `bounds`.
    pub fn touches(&self, bounds: FieldBounds) -> bool {
        let low = (bounds.min / WORKED_CELL_M).floor();
        let high = (bounds.max / WORKED_CELL_M).floor();
        (low.y as i32..=high.y as i32)
            .any(|z| (low.x as i32..=high.x as i32).any(|x| self.0.contains(&(x, z))))
    }
}

/// Marks the cells under this frame's tool strips.
pub(crate) fn keep_worked_cells(contacts: Res<ToolContacts>, mut cells: ResMut<WorkedCells>) {
    for contact in &contacts.contacts {
        let reach = contact.width.hypot(contact.length) * 0.5;
        let middle = Vec2::new(contact.position.x, contact.position.z);
        let low = ((middle - Vec2::splat(reach)) / WORKED_CELL_M).floor();
        let high = ((middle + Vec2::splat(reach)) / WORKED_CELL_M).floor();
        for z in low.y as i32..=high.y as i32 {
            for x in low.x as i32..=high.x as i32 {
                cells.0.insert((x, z));
            }
        }
    }
}

/// Across offsets are kept in these steps, biased to sit in sixteen bits.
pub const WORK_ACROSS_STEP_M: f32 = 0.005;
const WORK_ACROSS_BIAS: f32 = 32768.0;
/// Working depth is kept in sixteen steps up to this.
pub const WORK_DEPTH_MAX_M: f32 = 0.30;
/// How far inside the strip's edge a texel lies, in these steps and biased
/// like the across offset; nought is a texel nothing stamped.
pub const WORK_EDGE_STEP_M: f32 = 0.001;
/// Texels past a strip's sides that still take its edge distance.
pub const WORK_FRINGE_TEXELS: f32 = 2.0;

/// Encodes how far inside the strip's edge a texel lies, negative outside.
pub fn work_edge(inside_m: f32) -> u16 {
    (inside_m / WORK_EDGE_STEP_M + WORK_ACROSS_BIAS).round().clamp(1.0, 65535.0) as u16
}

/// Encodes a worked texel: the tool (4 bits), the way it went (8 bits) and
/// its working depth (4 bits), then where across the tool the texel lay.
pub fn work_texel(kind: ToolKind, direction: Vec2, across_m: f32, depth_m: f32) -> [u16; 2] {
    let angle = (direction.y.atan2(direction.x) + std::f32::consts::PI) / std::f32::consts::TAU;
    let angle = (angle * 255.0).round() as u16 & 255;
    let depth = ((depth_m / WORK_DEPTH_MAX_M).clamp(0.0, 1.0) * 15.0).round() as u16;
    let across = (across_m / WORK_ACROSS_STEP_M + WORK_ACROSS_BIAS).round().clamp(0.0, 65535.0) as u16;
    [kind.code() << 12 | angle << 4 | depth, across]
}

/// Where a work map lies: its first texel's world XZ, its resolution and size.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WorkMap {
    pub origin: Vec2,
    pub texels_per_metre: f32,
    pub size: IVec2,
}

impl WorkMap {
    /// The texels a patch and its fringe can reach, if any fall on the map.
    pub fn reach(&self, contact: &ToolContact) -> Option<(IVec2, IVec2)> {
        let tpm = self.texels_per_metre;
        let centre = (Vec2::new(contact.position.x, contact.position.z) - self.origin) * tpm;
        let half = Vec2::new(contact.width * 0.5 * tpm, (contact.length * 0.5 * tpm).max(0.5));
        let reach = (half + Vec2::X * WORK_FRINGE_TEXELS).length();
        let low = (centre - Vec2::splat(reach)).ceil().as_ivec2().max(IVec2::ZERO);
        let high = (centre + Vec2::splat(reach)).floor().as_ivec2().min(self.size - IVec2::ONE);
        low.cmple(high).all().then_some((low, high))
    }

    /// Stamps a patch into the map's texels; the texels it wrote, low and high.
    /// Inside takes the newest pass. The edge distance keeps the furthest in,
    /// so passes union and a fringe never cuts into worked ground.
    pub fn stamp(&self, texels: &mut [[u16; 4]], contact: &ToolContact) -> Option<(IVec2, IVec2)> {
        let (low, high) = self.reach(contact)?;
        let tpm = self.texels_per_metre;
        let centre = (Vec2::new(contact.position.x, contact.position.z) - self.origin) * tpm;
        let along = contact.direction.normalize_or(Vec2::X);
        let axle = along.perp();
        let half = Vec2::new(contact.width * 0.5 * tpm, (contact.length * 0.5 * tpm).max(0.5));
        let mut dirty: Option<(IVec2, IVec2)> = None;
        for z in low.y..=high.y {
            for x in low.x..=high.x {
                let d = Vec2::new(x as f32, z as f32) - centre;
                let past = d.dot(axle).abs() - half.x;
                if past > WORK_FRINGE_TEXELS || d.dot(along).abs() > half.y {
                    continue;
                }
                let across = contact.across + d.dot(axle) / tpm;
                // The edges are measured the way the contact measures its across.
                let offset = contact.across - d.dot(axle) / tpm;
                let inside = (offset - contact.edges.x).min(contact.edges.y - offset);
                let texel = &mut texels[(z * self.size.x + x) as usize];
                if past <= 0.0 {
                    let [state, across] = work_texel(contact.kind, along, across, contact.depth);
                    *texel = [state, across, texel[2].max(work_edge(inside)), 0];
                } else {
                    texel[2] = texel[2].max(work_edge(inside.min(-past / tpm)));
                }
                let at = IVec2::new(x, z);
                dirty = Some(dirty.map_or((at, at), |(l, h)| (l.min(at), h.max(at))));
            }
        }
        dirty
    }
}

/// Draw bands of the two relief meshes, metres from the camera.
const RELIEF_NEAR_BAND: [f32; 2] = [0.0, 18.0];
const RELIEF_FAR_BAND: [f32; 2] = [18.0, 64.0];
/// Quads along each side of the one-metre patch at either detail.
const RELIEF_NEAR_QUADS: u32 = 40;
const RELIEF_FAR_QUADS: u32 = 10;

/// The worked-soil relief, as two vegetation layers: one patch a square metre
/// at each level of detail, laid edge to edge over the chunk and lifted into
/// shape by the shader where the work map says the ground was worked.
pub(crate) fn relief_layers() -> Vec<VegetationLayer> {
    let layer = |template: fn() -> Mesh, band: [f32; 2]| VegetationLayer {
        shader: "embedded://gearbox_fields/tillage/shaders/worked_soil.wgsl",
        template,
        // One patch a square metre, every one of them inside the band: the
        // soil is a surface, and a thinned surface has holes in it.
        density: 1.0,
        fade_start: band[1] + 3.9,
        fade_end: band[1] + 4.0,
        inverse_square_thinning: false,
        follow_grass: 0.0,
        cutout: false,
        way_only: false,
        worked_only: true,
        blade: None,
        sieve: None,
        albedo: Some("embedded://gearbox_fields/harvested_wheat/textures/soil_albedo.jpg"),
        lod_band: band,
    };
    vec![layer(relief_near, RELIEF_NEAR_BAND), layer(relief_far, RELIEF_FAR_BAND)]
}

/// A one-metre square of `quads` by `quads`, its far edge at one; uv carries
/// the draw band so the shader fades the relief out where the next takes over.
fn relief_patch(quads: u32, band: [f32; 2]) -> Mesh {
    let side = quads + 1;
    let mut positions = Vec::with_capacity((side * side) as usize);
    for z in 0..side {
        for x in 0..side {
            positions.push([x as f32 / quads as f32, 0.0, z as f32 / quads as f32]);
        }
    }
    let mut indices = Vec::with_capacity((quads * quads * 6) as usize);
    for z in 0..quads {
        for x in 0..quads {
            let a = z * side + x;
            let b = a + 1;
            let c = a + side;
            let d = c + 1;
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    let count = positions.len();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; count])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![band; count])
        .with_inserted_indices(Indices::U32(indices))
}

fn relief_near() -> Mesh {
    relief_patch(RELIEF_NEAR_QUADS, RELIEF_NEAR_BAND)
}

fn relief_far() -> Mesh {
    relief_patch(RELIEF_FAR_QUADS, RELIEF_FAR_BAND)
}

pub(crate) struct TillagePlugin;

impl Plugin for TillagePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "shaders/work.wgsl");
        bevy::asset::embedded_asset!(app, "shaders/worked_soil.wgsl");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_worked_texel_keeps_the_tool_the_way_and_the_place_across() {
        let [state, across] = work_texel(ToolKind::DiscHarrow, Vec2::new(0.0, 1.0), -1.25, 0.10);
        assert_eq!(state >> 12, 1, "the tool");
        let angle = ((state >> 4) & 255) as f32 / 255.0 * std::f32::consts::TAU - std::f32::consts::PI;
        assert!((angle - std::f32::consts::FRAC_PI_2).abs() < 0.03, "the way: {angle}");
        assert_eq!(state & 15, 5, "the depth");
        let back = (across as f32 - WORK_ACROSS_BIAS) * WORK_ACROSS_STEP_M;
        assert!((back + 1.25).abs() < 1.0e-6, "across: {back}");
    }

    #[test]
    fn untouched_ground_is_nought_and_worked_ground_never_is() {
        let [state, _] = work_texel(ToolKind::DiscHarrow, Vec2::X, 0.0, 0.0);
        assert_ne!(state, 0);
    }

    // The worked amount at a point, read as `work_at` reads it.
    fn amount_at(map: &WorkMap, texels: &[[u16; 4]], place: Vec2) -> f32 {
        let t = (place - map.origin) * map.texels_per_metre;
        let (i, f) = (t.floor().as_ivec2(), t - t.floor());
        let at = |dx: i32, dy: i32| texels[((i.y + dy) * map.size.x + i.x + dx) as usize];
        let [a, b, c, d] = [at(0, 0), at(1, 0), at(0, 1), at(1, 1)];
        let mix = |x: f32, y: f32, s: f32| x + (y - x) * s;
        if [a, b, c, d].iter().all(|texel| texel[2] != 0) {
            let edge = |texel: [u16; 4]| (texel[2] as f32 - WORK_ACROSS_BIAS) * WORK_EDGE_STEP_M;
            let inside = mix(mix(edge(a), edge(b), f.x), mix(edge(c), edge(d), f.x), f.y);
            return (inside * map.texels_per_metre + 0.5).clamp(0.0, 1.0);
        }
        let worked = |texel: [u16; 4]| if texel[0] >> 12 != 0 { 1.0 } else { 0.0 };
        mix(mix(worked(a), worked(b), f.x), mix(worked(c), worked(d), f.x), f.y)
    }

    // A strip 3 m wide driven 4° off the grid in 0.15 m strides, by `parts`
    // side by side; how far its cut edges stray from the true ones, and the
    // least worked amount down its middle.
    fn driven_strip(parts: usize) -> (f32, f32) {
        let map = WorkMap { origin: Vec2::ZERO, texels_per_metre: 8.0, size: IVec2::new(240, 240) };
        let mut texels = vec![[0u16; 4]; (map.size.x * map.size.y) as usize];
        let heading = Vec2::from_angle(4f32.to_radians()).rotate(Vec2::Y);
        // The producer measures across along the heading turned clockwise.
        let across_dir = -heading.perp();
        let (start, width) = (Vec2::new(15.0, 2.0), 3.0);
        let part = width / parts as f32;
        for step in 0..160 {
            let middle = start + heading * (step as f32 * 0.15);
            for index in 0..parts {
                let across = -width * 0.5 + part * (index as f32 + 0.5);
                let at = middle + across_dir * across;
                map.stamp(&mut texels, &ToolContact {
                    kind: ToolKind::DiscHarrow,
                    position: Vec3::new(at.x, 0.0, at.y),
                    direction: heading,
                    across,
                    width: part,
                    edges: Vec2::new(-width * 0.5, width * 0.5),
                    length: 0.25,
                    depth: 0.1,
                });
            }
        }
        let (mut stray, mut middle_least) = (0.0f32, 1.0f32);
        for k in 0..200 {
            let on = start + heading * (3.0 + k as f32 * 0.08);
            middle_least = middle_least.min(amount_at(&map, &texels, on));
            for side in [-1.0f32, 1.0] {
                let (mut inner, mut outer) = (0.0f32, 0.5f32);
                for _ in 0..30 {
                    let mid = (inner + outer) * 0.5;
                    let place = on + across_dir * side * (width * 0.5 - 0.25 + mid);
                    if amount_at(&map, &texels, place) >= 0.5 { inner = mid } else { outer = mid }
                }
                stray = stray.max((inner - 0.25).abs());
            }
        }
        (stray, middle_least)
    }

    #[test]
    fn a_strip_off_the_grid_keeps_straight_edges_and_no_seam_between_its_parts() {
        for parts in [1, 2, 12] {
            let (stray, middle) = driven_strip(parts);
            assert!(stray < 0.01, "{parts} parts: the edge strays {stray} m");
            assert!(middle > 0.99, "{parts} parts: the middle is only {middle} worked");
        }
    }

    #[test]
    fn an_edge_distance_keeps_its_order_and_is_never_nought() {
        assert!(work_edge(-0.2) < work_edge(0.0) && work_edge(0.0) < work_edge(0.05));
        assert_ne!(work_edge(-1000.0), 0);
        let back = (work_edge(-0.123) as f32 - WORK_ACROSS_BIAS) * WORK_EDGE_STEP_M;
        assert!((back + 0.123).abs() < 1.0e-6, "{back}");
    }

    #[test]
    fn worked_cells_cover_the_whole_strip() {
        let mut cells = WorkedCells::default();
        let mut app = App::new();
        app.insert_resource(ToolContacts {
            contacts: vec![ToolContact {
                kind: ToolKind::DiscHarrow,
                position: Vec3::new(10.0, 0.0, 10.0),
                direction: Vec2::X,
                across: 0.0,
                width: 6.0,
                edges: Vec2::new(-3.0, 3.0),
                length: 1.0,
                depth: 0.1,
            }],
        })
        .init_resource::<WorkedCells>()
        .add_systems(Update, keep_worked_cells);
        app.update();
        std::mem::swap(&mut cells, &mut app.world_mut().resource_mut::<WorkedCells>());
        for corner in [Vec2::new(7.0, 7.0), Vec2::new(13.0, 13.0)] {
            let bounds = FieldBounds { min: corner, max: corner + Vec2::splat(0.5) };
            assert!(cells.touches(bounds), "{corner} not marked");
        }
        let far = FieldBounds { min: Vec2::splat(40.0), max: Vec2::splat(41.0) };
        assert!(!cells.touches(far));
    }

    #[test]
    fn a_relief_patch_tiles_its_square_metre() {
        let mesh = relief_near();
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("no positions");
        };
        let side = (RELIEF_NEAR_QUADS + 1) as usize;
        assert_eq!(positions.len(), side * side);
        assert_eq!(positions[0], [0.0, 0.0, 0.0]);
        assert_eq!(positions[side * side - 1], [1.0, 0.0, 1.0]);
    }
}
