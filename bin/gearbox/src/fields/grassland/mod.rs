//! Grassland profile: material, vegetation templates, palette, and wheel response.

use super::profile::{
    FieldProfile, FieldProfiles, GroundSurface, MaterialSurface, VegetationLayer, WheelMapParams,
    WheelResponse,
};
use super::profile::{SurfaceGeometry, SurfaceGeometryParams};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use std::sync::Arc;

type MeadowMaterial = ExtendedMaterial<StandardMaterial, MeadowExtension>;

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
struct MeadowExtension {
    #[texture(100)]
    #[sampler(101)]
    grass_albedo: Handle<Image>,
    #[texture(102)]
    #[sampler(103)]
    dirt_albedo: Handle<Image>,
    #[texture(104, sample_type = "u_int")]
    trample: Handle<Image>,
    #[uniform(105)]
    trample_params: WheelMapParams,
    #[texture(106, sample_type = "float", filterable = false)]
    heightmap: Handle<Image>,
    #[uniform(107)]
    geometry: SurfaceGeometryParams,
}

impl MaterialExtension for MeadowExtension {
    fn fragment_shader() -> ShaderRef {
        "embedded://gearbox_sim/fields/grassland/shaders/material.wgsl".into()
    }
}

pub struct GrasslandPlugin;

impl Plugin for GrasslandPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "shaders/palette.wgsl");
        bevy::asset::embedded_asset!(app, "shaders/material.wgsl");
        bevy::asset::embedded_asset!(app, "shaders/vegetation.wgsl");
        bevy::asset::embedded_asset!(app, "textures/grass_albedo.jpg");
        bevy::asset::embedded_asset!(app, "textures/soil_albedo.jpg");
        app.add_plugins(MaterialPlugin::<MeadowMaterial>::default());
        let shader = "embedded://gearbox_sim/fields/grassland/shaders/vegetation.wgsl";
        let density = std::env::var("GEARBOX_GRASS_DENSITY")
            .ok()
            .and_then(|value| value.parse::<f32>().ok())
            .filter(|value| value.is_finite() && *value >= 0.0)
            .unwrap_or(6000.0);
        app.world_mut()
            .resource_mut::<FieldProfiles>()
            .register(FieldProfile {
                name: "grassland",
                wheel_response: WheelResponse {
                    recovery_seconds: 300.0,
                    bend: 0.9,
                    darkening: 0.25,
                    footprint_length: 0.25,
                },
                layers: vec![
                    VegetationLayer {
                        shader,
                        template: base_template,
                        density,
                        fade_start: 4.0,
                        fade_end: 32.0,
                        inverse_square_thinning: true,
                    },
                    VegetationLayer {
                        shader,
                        template: meadow_detail_template,
                        density: if density > 0.0 { 80.0 } else { 0.0 },
                        fade_start: 8.0,
                        fade_end: 24.0,
                        inverse_square_thinning: false,
                    },
                    VegetationLayer {
                        shader,
                        template: super::canopy::template,
                        density: (density / 6000.0) * 80.0,
                        fade_start: 12.0,
                        fade_end: super::canopy::FADE_END_M,
                        inverse_square_thinning: true,
                    },
                ],
                ground: create_ground,
            });
    }
}

fn create_ground(
    world: &mut World,
    trample: Handle<Image>,
    trample_params: WheelMapParams,
    geometry: SurfaceGeometry,
) -> Arc<dyn GroundSurface> {
    let assets = world.resource::<AssetServer>();
    let grass_albedo = assets
        .load_builder()
        .with_settings(|settings: &mut bevy::image::ImageLoaderSettings| {
            settings.sampler = bevy::image::ImageSampler::linear();
        })
        .load("embedded://gearbox_sim/fields/grassland/textures/grass_albedo.jpg");
    let dirt_albedo = assets
        .load_builder()
        .with_settings(|settings: &mut bevy::image::ImageLoaderSettings| {
            settings.sampler = bevy::image::ImageSampler::linear();
        })
        .load("embedded://gearbox_sim/fields/grassland/textures/soil_albedo.jpg");
    let material = world
        .resource_mut::<Assets<MeadowMaterial>>()
        .add(ExtendedMaterial {
            base: StandardMaterial {
                perceptual_roughness: 0.95,
                metallic: 0.0,
                diffuse_transmission: 0.25,
                thickness: 0.0002,
                ..default()
            },
            extension: MeadowExtension {
                grass_albedo,
                dirt_albedo,
                trample,
                trample_params,
                heightmap: geometry.heightmap,
                geometry: geometry.params,
            },
        });
    Arc::new(MaterialSurface(material))
}

fn base_template() -> Mesh {
    blade_template(1)
}

fn blade_template(segments: u32) -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    for k in 0..=segments {
        let t = k as f32 / segments as f32;
        let sides: &[f32] = if k == segments { &[0.0] } else { &[-1.0, 1.0] };
        for side in sides {
            positions.push([*side, t, 0.0]);
            normals.push([0.0, 1.0, 0.0]);
            uvs.push([0.5 + 0.5 * side, t]);
        }
    }
    let mut indices = Vec::new();
    for k in 0..segments.saturating_sub(1) {
        let l = k * 2;
        indices.extend_from_slice(&[l, l + 2, l + 1, l + 1, l + 2, l + 3]);
    }
    let last = (segments - 1) * 2;
    indices.extend_from_slice(&[last, last + 2, last + 1]);
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

/// Six segmented leaves; position.z selects the leaf within a plant.
fn meadow_detail_template() -> Mesh {
    let segments = 5u32;
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for leaf in 0..6 {
        let start = positions.len() as u32;
        for step in 0..=segments {
            let t = step as f32 / segments as f32;
            if step == segments {
                positions.push([0.0, t, (leaf + 1) as f32]);
            } else {
                positions.push([-1.0, t, (leaf + 1) as f32]);
                positions.push([1.0, t, (leaf + 1) as f32]);
            }
        }
        for step in 0..segments - 1 {
            let i = start + step * 2;
            indices.extend_from_slice(&[i, i + 2, i + 1, i + 1, i + 2, i + 3]);
        }
        let i = start + (segments - 1) * 2;
        indices.extend_from_slice(&[i, i + 2, i + 1]);
    }
    let count = positions.len();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; count])
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; count])
    .with_inserted_indices(Indices::U32(indices))
}
