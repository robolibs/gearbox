//! Field surfaces layered independently over terrain geometry.

mod canopy;
mod clumps;
pub(crate) mod contacts;
mod geometry;
mod grassland;
mod harvested_wheat;
mod layout;
mod profile;
mod render;
mod runtime;
mod textures;

use bevy::prelude::*;
use bevy::render::extract_resource::ExtractResourcePlugin;

pub struct FieldsPlugin;

impl Plugin for FieldsPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "shaders/interaction.wgsl");
        bevy::asset::embedded_asset!(app, "shaders/surface_detail.wgsl");
        bevy::asset::embedded_asset!(app, "shaders/wind.wgsl");
        bevy::asset::embedded_asset!(app, "shaders/canopy.wgsl");
        app.insert_resource(layout::FieldLayout::from_env().expect("read field layout"))
            .init_resource::<profile::FieldProfiles>()
            .init_resource::<render::RenderFields>()
            .init_resource::<runtime::VegetationChunks>()
            .init_resource::<contacts::WheelContacts>()
            .add_plugins(ExtractResourcePlugin::<contacts::WheelContacts>::default())
            .add_systems(First, contacts::begin_wheel_contacts)
            .add_systems(Update, textures::prepare_textures)
            .add_plugins((
                clumps::ClumpsPlugin,
                grassland::GrasslandPlugin,
                harvested_wheat::HarvestedWheatPlugin,
                render::VegetationPlugin,
            ))
            .add_systems(
                Update,
                (runtime::ensure_fields, runtime::assign_surfaces)
                    .chain()
                    .after(crate::terrain::TerrainUpdates),
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
