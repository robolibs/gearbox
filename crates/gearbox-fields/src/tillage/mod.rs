//! Ground worked by tools: soil an implement has cut, loosened and ridged.
//!
//! A tool working a field stamps a work map, one per field that can be
//! worked, in the same texels as the field's wheel map. A worked texel keeps
//! which tool cut it, the way it was going and where across the tool's width
//! it lay, and keeps it for good: unlike a wheel mark, worked soil does not
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

/// Encodes a worked texel: the tool (4 bits), the way it went (8 bits) and
/// its working depth (4 bits), then where across the tool the texel lay.
pub fn work_texel(kind: ToolKind, direction: Vec2, across_m: f32, depth_m: f32) -> [u16; 2] {
    let angle = (direction.y.atan2(direction.x) + std::f32::consts::PI) / std::f32::consts::TAU;
    let angle = (angle * 255.0).round() as u16 & 255;
    let depth = ((depth_m / WORK_DEPTH_MAX_M).clamp(0.0, 1.0) * 15.0).round() as u16;
    let across = (across_m / WORK_ACROSS_STEP_M + WORK_ACROSS_BIAS).round().clamp(0.0, 65535.0) as u16;
    [kind.code() << 12 | angle << 4 | depth, across]
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
