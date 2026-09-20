//! The globe itself: a cube-sphere quadtree refined towards the camera, its
//! heights baked on the GPU, so the planet carries real ground from orbit down
//! to the field instead of being a painted ball behind the one square of
//! terrain that is simulated.
//!
//! The renderer is [terra](https://github.com/ahmadaliadeel/terra) — a
//! GPU-driven, crack-free CDLOD cube-sphere planet for Bevy 0.19 — carried in
//! here as a library and pointed at one Earth-sized world in the site grid
//! gearbox already keeps. What changed from upstream: it spawns no solar
//! system of its own (gearbox says where its planet goes), its shaders are
//! embedded rather than loaded from an assets folder, and the debug overlay is
//! reduced to the flags the selector reads.

pub mod config;
mod debug;
pub mod planet;

use bevy::asset::embedded_asset;
use bevy::camera::visibility::VisibilitySystems;
use bevy::pbr::MaterialPlugin;
use bevy::prelude::*;

pub use config::PlanetConfig;
pub use debug::DebugSettings;
pub use planet::{PlanetView, nearest_planet, planet_view, spawn_planet};

use planet::bake::{TileBakePlugin, TileBakeQueue};
use planet::material::{TerrainMaterial, WaterMaterial};

/// Everything the globe needs, short of a planet to draw: gearbox spawns that
/// itself with [`spawn_planet`], in the grid its sites already live in.
pub struct PlanetPlugin;

impl Plugin for PlanetPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "../shaders/terrain.wgsl");
        embedded_asset!(app, "../shaders/tile_bake.wgsl");
        embedded_asset!(app, "../shaders/water.wgsl");
        app.add_plugins((
            MaterialPlugin::<TerrainMaterial>::default(),
            MaterialPlugin::<WaterMaterial>::default(),
            TileBakePlugin,
        ))
        .init_resource::<TileBakeQueue>()
        .init_resource::<DebugSettings>()
        .add_systems(
            PostUpdate,
            // After propagation: big_space recentres camera and planet inside
            // `TransformSystems::Propagate`, so selecting before it would read
            // a pose the renderer does not use this frame. Still before
            // visibility, because selection writes tile visibility.
            planet::quadtree::select_and_sync
                .after(TransformSystems::Propagate)
                .before(VisibilitySystems::VisibilityPropagate),
        );
    }
}
