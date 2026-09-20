//! Concrete yard profile: 3 m cast slabs of scanned concrete (Poly Haven
//! `concrete_floor_worn_001` and the mossy `concrete_floor_02`, both CC0)
//! with open joints, and the weeds that root in those joints.

use super::profile::{
    FieldProfile, FieldProfiles, GroundSurface, MaterialSurface, VegetationLayer, WheelMapParams,
    WheelResponse,
};
use super::profile::{SurfaceGeometry, SurfaceGeometryParams};
use bevy::asset::RenderAssetUsages;
use bevy::image::{
    ImageAddressMode, ImageFilterMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor,
};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use std::sync::Arc;

type ConcreteMaterial = ExtendedMaterial<StandardMaterial, ConcreteExtension>;

// One repeating sampler, bound with the first scan, reads all six.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
struct ConcreteExtension {
    #[texture(100)]
    #[sampler(101)]
    clean_albedo: Handle<Image>,
    #[texture(102)]
    clean_normal: Handle<Image>,
    #[texture(103)]
    clean_arm: Handle<Image>,
    #[texture(104, sample_type = "u_int")]
    trample: Handle<Image>,
    #[uniform(105)]
    trample_params: WheelMapParams,
    #[texture(106, sample_type = "float", filterable = false)]
    heightmap: Handle<Image>,
    #[uniform(107)]
    geometry: SurfaceGeometryParams,
    #[texture(108)]
    moss_albedo: Handle<Image>,
    #[texture(109)]
    moss_normal: Handle<Image>,
    #[texture(110)]
    moss_arm: Handle<Image>,
}

impl MaterialExtension for ConcreteExtension {
    fn fragment_shader() -> ShaderRef {
        "embedded://gearbox_fields/concrete/shaders/material.wgsl".into()
    }
}

pub(super) struct ConcretePlugin {
    pub weed_density: f32,
}

impl Plugin for ConcretePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "shaders/yard.wgsl");
        bevy::asset::embedded_asset!(app, "shaders/material.wgsl");
        bevy::asset::embedded_asset!(app, "shaders/vegetation.wgsl");
        bevy::asset::embedded_asset!(app, "textures/concrete_floor_worn_001_diff.jpg");
        bevy::asset::embedded_asset!(app, "textures/concrete_floor_worn_001_nor_gl.jpg");
        bevy::asset::embedded_asset!(app, "textures/concrete_floor_worn_001_arm.jpg");
        bevy::asset::embedded_asset!(app, "textures/concrete_floor_02_diff.jpg");
        bevy::asset::embedded_asset!(app, "textures/concrete_floor_02_nor_gl.jpg");
        bevy::asset::embedded_asset!(app, "textures/concrete_floor_02_arm.jpg");
        app.add_plugins(MaterialPlugin::<ConcreteMaterial>::default());
        let shader = "embedded://gearbox_fields/concrete/shaders/vegetation.wgsl";
        let density = self.weed_density;
        app.world_mut()
            .resource_mut::<FieldProfiles>()
            .register(FieldProfile {
                name: "concrete",
                // Tyres print their tread where they scrub and crush the joint
                // weeds; marks wear off in a few minutes.
                wheel_response: WheelResponse {
                    recovery_seconds: 240.0,
                    bend: 0.85,
                    darkening: 0.2,
                    footprint_length: 0.25,
                    tread: true,
                },
                layers: vec![
                    VegetationLayer {
                        shader,
                        template: joint_tuft_near,
                        density,
                        fade_start: 8.0,
                        fade_end: 60.0,
                        inverse_square_thinning: false,
                        follow_grass: 0.0,
                        albedo: None,
                        lod_band: [0.0, 16.0],
                    },
                    VegetationLayer {
                        shader,
                        template: joint_tuft_far,
                        density,
                        fade_start: 8.0,
                        fade_end: 60.0,
                        inverse_square_thinning: false,
                        follow_grass: 0.0,
                        albedo: None,
                        lod_band: [16.0, 60.0],
                    },
                ],
                ground: create_ground,
                tread: Vec4::ZERO,
                soft_border: 0.0,
                surface_tint: Vec4::new(0.118, 0.118, 0.112, 1.0),
            });
    }
}

fn joint_tuft_near() -> Mesh {
    joint_tuft(5, 4)
}

fn joint_tuft_far() -> Mesh {
    joint_tuft(3, 2)
}

/// A tuft of `blades` leaf strips, `segments` quads tall. `position.x` is
/// the side (-1, 1), `y` the height fraction, `z` the leaf's index.
fn joint_tuft(blades: u32, segments: u32) -> Mesh {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for blade in 0..blades {
        let start = positions.len() as u32;
        for step in 0..=segments {
            let t = step as f32 / segments as f32;
            positions.push([-1.0, t, blade as f32]);
            positions.push([1.0, t, blade as f32]);
        }
        for step in 0..segments {
            let row = start + step * 2;
            indices.extend_from_slice(&[row, row + 2, row + 1, row + 1, row + 2, row + 3]);
        }
    }
    let count = positions.len();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; count])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; count])
        .with_inserted_indices(Indices::U32(indices))
}

// A scan that tiles: repeating, trilinear, anisotropic for grazing views.
fn load_scan(assets: &AssetServer, name: &str, srgb: bool) -> Handle<Image> {
    assets
        .load_builder()
        .with_settings(move |settings: &mut ImageLoaderSettings| {
            settings.is_srgb = srgb;
            settings.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: ImageAddressMode::Repeat,
                address_mode_v: ImageAddressMode::Repeat,
                mag_filter: ImageFilterMode::Linear,
                min_filter: ImageFilterMode::Linear,
                mipmap_filter: ImageFilterMode::Linear,
                anisotropy_clamp: 8,
                ..default()
            });
        })
        .load(format!("embedded://gearbox_fields/concrete/textures/{name}.jpg"))
}

fn create_ground(
    world: &mut World,
    trample: Handle<Image>,
    trample_params: WheelMapParams,
    geometry: SurfaceGeometry,
    _placed: crate::profile::Placed,
) -> Arc<dyn GroundSurface> {
    let assets = world.resource::<AssetServer>();
    let extension = ConcreteExtension {
        clean_albedo: load_scan(assets, "concrete_floor_worn_001_diff", true),
        clean_normal: load_scan(assets, "concrete_floor_worn_001_nor_gl", false),
        clean_arm: load_scan(assets, "concrete_floor_worn_001_arm", false),
        trample,
        trample_params,
        heightmap: geometry.heightmap,
        geometry: geometry.params,
        moss_albedo: load_scan(assets, "concrete_floor_02_diff", true),
        moss_normal: load_scan(assets, "concrete_floor_02_nor_gl", false),
        moss_arm: load_scan(assets, "concrete_floor_02_arm", false),
    };
    let material = world
        .resource_mut::<Assets<ConcreteMaterial>>()
        .add(ExtendedMaterial {
            base: StandardMaterial {
                perceptual_roughness: 0.8,
                metallic: 0.0,
                ..default()
            },
            extension,
        });
    Arc::new(MaterialSurface(material))
}
