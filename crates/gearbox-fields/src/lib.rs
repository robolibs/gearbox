//! Field surfaces layered independently over a host's terrain geometry.

#[cfg(test)]
mod shader_contract;

mod canopy;
mod clumps;
mod bare;
mod concrete;
pub mod contacts;
mod geometry;
mod grassland;
mod harvested_wheat;
mod heights;
mod host;
mod layout;
mod profile;
mod render;
mod runtime;
mod textures;
pub mod wind_map;

pub use contacts::{WheelContact, WheelContacts};
pub use heights::HeightGrid;
pub use host::{
    CoverBackdrop, CoverHeights, CoverSurfaceMesh, CoverTerrain, CoverTerrainRoots, CoverWind,
    HeightSource,
};
pub use layout::{FieldBounds, FieldLayout, Hollows};
pub use runtime::{CoverPending, heightmap_image};
pub use profile::{
    FieldProfile, FieldProfiles, GroundSurface, MaterialSurface, SurfaceGeometry,
    SurfaceGeometryParams, VegetationLayer, WheelMapParams, WheelResponse,
};

use bevy::prelude::*;
use bevy::render::extract_resource::ExtractResourcePlugin;

/// The field systems, for hosts that must order them after their own
/// terrain updates.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CoverUpdates;

/// The layout of the covers and how thickly the two bundled ones grow.
pub struct FieldsPlugin {
    pub layout: FieldLayout,
    pub grass_density: f32,
    pub stubble_density: f32,
    /// Weeds per m² of concrete yard; all of them root in the slab joints.
    pub concrete_weed_density: f32,
}

impl Default for FieldsPlugin {
    fn default() -> Self {
        Self {
            layout: FieldLayout::from_env().expect("read field layout"),
            grass_density: density_from_env("GEARBOX_GRASS_DENSITY"),
            stubble_density: density_from_env("GEARBOX_STUBBLE_DENSITY"),
            concrete_weed_density: std::env::var("GEARBOX_CONCRETE_WEEDS")
                .ok()
                .and_then(|value| value.parse::<f32>().ok())
                .filter(|value| value.is_finite() && *value >= 0.0)
                .unwrap_or(36.0),
        }
    }
}

fn density_from_env(key: &str) -> f32 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(6000.0)
}

impl Plugin for FieldsPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "shaders/interaction.wgsl");
        bevy::asset::embedded_asset!(app, "shaders/surface_detail.wgsl");
        bevy::asset::embedded_asset!(app, "shaders/wind.wgsl");
        bevy::asset::embedded_asset!(app, "shaders/canopy.wgsl");
        app.insert_resource(self.layout.clone())
            .init_resource::<profile::FieldProfiles>()
            .init_resource::<render::RenderFields>()
            .init_resource::<runtime::VegetationChunks>()
            .init_resource::<runtime::CoverPending>()
            .init_resource::<contacts::WheelContacts>()
            .init_resource::<CoverWind>()
            .init_resource::<CoverTerrainRoots>()
            .add_plugins(ExtractResourcePlugin::<contacts::WheelContacts>::default())
            .add_systems(First, contacts::begin_wheel_contacts)
            .add_systems(Update, (textures::prepare_textures, render::sync_wind))
            .add_plugins((
                clumps::ClumpsPlugin,
                grassland::GrasslandPlugin { density: self.grass_density },
                bare::BarePlugin,
                harvested_wheat::HarvestedWheatPlugin { density: self.stubble_density },
                concrete::ConcretePlugin { weed_density: self.concrete_weed_density },
                render::VegetationPlugin,
            ))
            .add_systems(
                Update,
                (runtime::ensure_fields, runtime::assign_surfaces)
                    .chain()
                    .in_set(CoverUpdates),
            )
            .add_systems(
                PostUpdate,
                runtime::stream_vegetation
                    .after(bevy::transform::TransformSystems::Propagate)
                    .after(bevy::camera::visibility::VisibilitySystems::UpdateFrusta)
                    .before(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate),
            );
    }
}
