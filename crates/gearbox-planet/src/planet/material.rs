//! Terrain material: StandardMaterial PBR extended with a CDLOD displacement
//! vertex stage and slope/altitude coloring fragment stage.
//!
//! One material instance is shared by every node entity; per-node data lives in
//! a storage buffer indexed via `MeshTag` (see quadtree.rs).

use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;

pub type TerrainMaterial = ExtendedMaterial<StandardMaterial, TerrainExtension>;

/// The shaders travel inside the binary, as every other gearbox shader does,
/// so a run needs no assets folder beside it.
pub const TERRAIN_SHADER_PATH: &str = "embedded://gearbox_planet/../shaders/terrain.wgsl";

/// Debug flag bits (mirrored in terrain.wgsl).
pub const DEBUG_GRID: u32 = 1;
pub const DEBUG_LOD_TINT: u32 = 2;
pub const DEBUG_MORPH_HEAT: u32 = 4;
/// Freezes shader-side animation (waves, caustics, foam) so screenshots are
/// reproducible frame to frame — used by the verification scripts.
pub const DEBUG_STATIC_TIME: u32 = 8;

/// Rarely-changing terrain uniforms. Camera position and time come from
/// Bevy's built-in `view`/`globals` WGSL bindings instead, so the material
/// assets are only mutated when the debug flags change — per-frame material
/// mutation forced a full bind-group re-prepare every frame, whose transient
/// unavailability windows flickered the transparent water tiles.
#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct TerrainGlobals {
    pub radius: f32,
    pub height_amp: f32,
    pub debug_flags: u32,
    pub _pad: u32,
}

/// 32-byte per-node record, one slot per pool entity. Mirrors `TerrainNode`
/// in terrain.wgsl (std430).
#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct TerrainNodeGpu {
    /// Face-uv of the node's min corner, in [-1,1].
    pub origin: Vec2,
    /// Node extent in face-uv units (full face = 2.0).
    pub scale: f32,
    /// face index in bits 0..2, lod in bits 3..
    pub face_lod: u32,
    /// (morph start distance, 1 / (morph end - morph start))
    pub morph_consts: Vec2,
    pub atlas_layer: u32,
    /// Lower bound on the morph factor, easing 1 -> 0 after the node is first
    /// emitted. A gated (bake-delayed) split otherwise swaps in children at
    /// morph < 1 — instantly finer than the parent they replace — which pops
    /// distant silhouettes ("tiles flickering"). Starting fully morphed makes
    /// the swap bit-exact with the parent, then detail fades in.
    pub morph_floor: f32,
}

#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct TerrainExtension {
    #[uniform(100)]
    pub globals: TerrainGlobals,
    #[texture(101, dimension = "2d_array", sample_type = "float")]
    #[sampler(102)]
    pub heightmaps: Handle<Image>,
    #[storage(103, read_only)]
    pub nodes: Handle<ShaderBuffer>,
}

impl MaterialExtension for TerrainExtension {
    fn vertex_shader() -> ShaderRef {
        TERRAIN_SHADER_PATH.into()
    }
    fn fragment_shader() -> ShaderRef {
        TERRAIN_SHADER_PATH.into()
    }
}

/// Translucent water-surface material: same bind-group layout and per-node
/// records as the terrain (shared `nodes` ShaderBuffer, indexed via MeshTag),
/// different shaders. Transparency/double-sidedness come from the base
/// StandardMaterial (alpha_mode: Blend, cull_mode: None).
pub type WaterMaterial = ExtendedMaterial<StandardMaterial, WaterExtension>;

pub const WATER_SHADER_PATH: &str = "embedded://gearbox_planet/../shaders/water.wgsl";

#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct WaterExtension {
    #[uniform(100)]
    pub globals: TerrainGlobals,
    #[texture(101, dimension = "2d_array", sample_type = "float")]
    #[sampler(102)]
    pub heightmaps: Handle<Image>,
    #[storage(103, read_only)]
    pub nodes: Handle<ShaderBuffer>,
}

impl MaterialExtension for WaterExtension {
    fn vertex_shader() -> ShaderRef {
        WATER_SHADER_PATH.into()
    }
    fn fragment_shader() -> ShaderRef {
        WATER_SHADER_PATH.into()
    }
}
