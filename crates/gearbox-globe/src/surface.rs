use bevy::prelude::*;

pub struct PlanetSurfaceShaderPlugin;

impl Plugin for PlanetSurfaceShaderPlugin {
    fn build(&self, app: &mut App) {
        bevy::shader::load_shader_library!(app, "surface.wgsl");
    }
}
