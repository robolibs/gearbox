//! The world's biomes and the air over them.
//!
//! The field layout says what is where; this hands it to `bevy_biomes` as
//! biomes with their borders, and keeps the motes over them in the same wind
//! and over the same ground as the plants.

use bevy::prelude::*;
use bevy_biomes::{AmbientWind, Biome, Biomes, BiomesPlugin, Covers, Field, Land, MoteBudget, Region};

pub struct WorldBiomesPlugin;

impl Plugin for WorldBiomesPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(BiomesPlugin)
            .insert_resource(Covers::built_in())
            .add_systems(Update, (publish_biomes, carry_the_air));
    }
}

/// The cover profiles the fields are written in, as biomes.
fn biome_of(profile: &str) -> Biome {
    match profile {
        "harvested_wheat" => Field::Stubble.into(),
        "concrete" => Field::Yard.into(),
        _ => Land::Grassland.into(),
    }
}

fn publish_biomes(
    mut commands: Commands,
    layout: Res<gearbox_fields::FieldLayout>,
    held: Option<Res<Biomes>>,
) {
    if held.is_some() && !layout.is_changed() {
        return;
    }
    let mut biomes = Biomes::new(biome_of(&layout.default));
    for spec in &layout.fields {
        let bounds = spec.bounds();
        biomes = biomes.with(Region::new(
            biome_of(&spec.profile),
            Rect::from_corners(bounds.min, bounds.max),
        ));
    }
    commands.insert_resource(biomes);
}

fn carry_the_air(
    wind: Res<gearbox_fields::CoverWind>,
    mut ambient: ResMut<AmbientWind>,
    mut budget: ResMut<MoteBudget>,
    mut images: ResMut<Assets<Image>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
) {
    if ambient.gust_map.is_none() {
        ambient.gust_map = Some(images.add(gust_map()));
    }
    ambient.heading_deg = wind.heading_deg;
    ambient.speed_mps = wind.speed_mps;
    ambient.gustiness = wind.gustiness;
    if let Ok(camera) = cameras.single() {
        let eye = camera.translation();
        budget.ground_m = crate::world::terrain_height_m(eye.x, eye.z);
    }
}

/// The plants' own gust map, as an image the air can read: one map, so the
/// motes surge where the blades lean.
fn gust_map() -> Image {
    use bevy::asset::RenderAssetUsages;
    use bevy::image::ImageSampler;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

    let size = gearbox_fields::wind_map::SIZE;
    let mut image = Image::new(
        Extent3d { width: size, height: size, depth_or_array_layers: 1 },
        TextureDimension::D2,
        gearbox_fields::wind_map::bake(),
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::linear();
    image
}
