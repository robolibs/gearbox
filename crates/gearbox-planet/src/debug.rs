//! The flags the tile selector reads. Upstream drives these from an on-screen
//! overlay and function keys; here they are a plain resource anything may set.

use bevy::prelude::*;

use crate::planet::material::{DEBUG_GRID, DEBUG_LOD_TINT, DEBUG_MORPH_HEAT, DEBUG_STATIC_TIME};

#[derive(Resource, Default)]
pub struct DebugSettings {
    /// Draw the lattice lines in the terrain shader.
    pub grid: bool,
    /// Tint each tile by its level of detail.
    pub lod_tint: bool,
    /// Tint by how far a tile has morphed towards its parent.
    pub morph_heat: bool,
    /// Stop choosing tiles, so what is on screen can be looked at.
    pub freeze: bool,
    /// Hold the shader's clock still, so two captures can be compared.
    pub static_time: bool,
}

impl DebugSettings {
    pub fn flag_bits(&self) -> u32 {
        (self.grid as u32) * DEBUG_GRID
            | (self.lod_tint as u32) * DEBUG_LOD_TINT
            | (self.morph_heat as u32) * DEBUG_MORPH_HEAT
            | (self.static_time as u32) * DEBUG_STATIC_TIME
    }
}
