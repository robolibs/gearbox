//! Harvested-wheat soil and cut-hay rows for the multi-bale USD field.

use super::profile::{
    FieldProfile, FieldProfiles, GroundSurface, MaterialSurface, VegetationLayer, WheelMapParams,
    WheelResponse,
};
use super::profile::{SurfaceGeometry, SurfaceGeometryParams};
use crate::world::{
    collect_descendants, is_usd_terrain_root_name, is_usd_terrain_scene_instantiated,
};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Extent3d, TextureDimension, TextureFormat};
use bevy::shader::ShaderRef;
use std::collections::HashMap;
use std::sync::Arc;

type AntiRepeatTerrainMaterial = ExtendedMaterial<StandardMaterial, AntiRepeatTerrainExtension>;

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
struct AntiRepeatTerrainExtension {
    #[texture(100)]
    #[sampler(101)]
    terrain_albedo: Handle<Image>,
    #[texture(102)]
    terrain_height: Handle<Image>,
    #[texture(104)]
    terrain_detail_albedo: Handle<Image>,
    #[texture(106)]
    terrain_detail_height: Handle<Image>,
    /// RGB multiplies the field colour, A scales the cut-hay rows. Comes
    /// from the USD material's constant `diffuseColor`; white and full hay
    /// when the terrain material is textured or unset.
    #[uniform(108)]
    tint: Vec4,
    #[texture(109, sample_type = "u_int")]
    tracks: Handle<Image>,
    #[uniform(110)]
    wheels: WheelMapParams,
    #[texture(111, sample_type = "float", filterable = false)]
    heightmap: Option<Handle<Image>>,
    #[uniform(112)]
    geometry: SurfaceGeometryParams,
}

impl MaterialExtension for AntiRepeatTerrainExtension {
    fn fragment_shader() -> ShaderRef {
        "embedded://gearbox_sim/fields/harvested_wheat/shaders/material.wgsl".into()
    }
}

#[derive(Component, Debug, Clone, Copy)]
struct AntiRepeatTerrainMaterialApplied;

/// Hay-row strength for a tinted (crop) terrain; untinted soil keeps 1.0.
const TINTED_TERRAIN_HAY_STRENGTH: f32 = 0.15;

/// Tint for a terrain mesh's authored material: its constant
/// `diffuseColor`, or white when it is textured or left at the default.
fn terrain_tint_from_material(material: Option<&StandardMaterial>) -> Vec4 {
    let Some(material) = material else {
        return Vec4::ONE;
    };
    let base = LinearRgba::from(material.base_color);
    let is_default = (base.red - 0.8).abs() < 1e-3
        && (base.green - 0.8).abs() < 1e-3
        && (base.blue - 0.8).abs() < 1e-3;
    if material.base_color_texture.is_some() || is_default {
        return Vec4::ONE;
    }
    Vec4::new(base.red, base.green, base.blue, TINTED_TERRAIN_HAY_STRENGTH)
}

fn apply_anti_repeat_material_to_usd_terrain(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut materials: ResMut<Assets<AntiRepeatTerrainMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut empty_tracks: Local<Option<Handle<Image>>>,
    standard_materials: Res<Assets<StandardMaterial>>,
    terrain_roots: Query<(Entity, &Name), With<usd_bevy::UsdSceneRoot>>,
    children: Query<&Children>,
    terrain_meshes: Query<
        Option<&MeshMaterial3d<StandardMaterial>>,
        (With<Mesh3d>, Without<AntiRepeatTerrainMaterialApplied>),
    >,
    mut material_handles: Local<HashMap<[u32; 4], Handle<AntiRepeatTerrainMaterial>>>,
) {
    let inactive = empty_tracks
        .get_or_insert_with(|| {
            images.add(Image::new_fill(
                Extent3d {
                    width: 2,
                    height: 2,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                &[0u8; 4],
                TextureFormat::Rg16Uint,
                RenderAssetUsages::RENDER_WORLD,
            ))
        })
        .clone();
    let mut applied = 0usize;
    for (root, name) in terrain_roots.iter() {
        if !is_usd_terrain_root_name(name.as_str())
            || !is_usd_terrain_scene_instantiated(root, &children)
        {
            continue;
        }
        for entity in collect_descendants(root, &children) {
            let Ok(authored) = terrain_meshes.get(entity) else {
                continue;
            };
            let tint = terrain_tint_from_material(
                authored.and_then(|handle| standard_materials.get(&handle.0)),
            );
            let handle = material_handles
                .entry(tint.to_array().map(f32::to_bits))
                .or_insert_with(|| {
                    materials.add(ExtendedMaterial {
                        base: StandardMaterial {
                            double_sided: true,
                            cull_mode: None,
                            perceptual_roughness: 0.98,
                            metallic: 0.0,
                            ..default()
                        },
                        extension: AntiRepeatTerrainExtension {
                            terrain_albedo: asset_server.load("embedded://gearbox_sim/fields/harvested_wheat/textures/soil_albedo.jpg"),
                            terrain_height: asset_server.load("embedded://gearbox_sim/fields/harvested_wheat/textures/soil_height.jpg"),
                            terrain_detail_albedo: asset_server.load("embedded://gearbox_sim/fields/harvested_wheat/textures/detail_albedo.jpg"),
                            terrain_detail_height: asset_server.load("embedded://gearbox_sim/fields/harvested_wheat/textures/detail_height.jpg"),
                            tint,
                            tracks: inactive.clone(),
                            wheels: WheelMapParams { origin: Vec2::splat(1_000_000.0),
                                texels_per_metre: 1.0, width: 2.0, height: 2.0, recovery_seconds: 1800.0, ..default() },
                            heightmap: None,
                            geometry: SurfaceGeometryParams::default(),
                        },
                    })
                })
                .clone();
            commands
                .entity(entity)
                .remove::<MeshMaterial3d<StandardMaterial>>()
                .insert((MeshMaterial3d(handle), AntiRepeatTerrainMaterialApplied));
            applied += 1;
        }
    }
    if applied > 0 {
        info!("world: applied anti-repeating terrain material to {applied} USD terrain mesh(es)");
    }
}

pub(super) struct HarvestedWheatPlugin;

impl Plugin for HarvestedWheatPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "shaders/patches.wgsl");
        bevy::asset::embedded_asset!(app, "shaders/material.wgsl");
        bevy::asset::embedded_asset!(app, "shaders/vegetation.wgsl");
        bevy::asset::embedded_asset!(app, "textures/soil_albedo.jpg");
        bevy::asset::embedded_asset!(app, "textures/soil_height.jpg");
        bevy::asset::embedded_asset!(app, "textures/detail_albedo.jpg");
        bevy::asset::embedded_asset!(app, "textures/detail_height.jpg");
        app.add_plugins(MaterialPlugin::<AntiRepeatTerrainMaterial>::default())
            .add_systems(Update, apply_anti_repeat_material_to_usd_terrain);
        let density = std::env::var("GEARBOX_STUBBLE_DENSITY")
            .ok()
            .and_then(|value| value.parse::<f32>().ok())
            .filter(|value| value.is_finite() && *value >= 0.0)
            .unwrap_or(6000.0);
        app.world_mut()
            .resource_mut::<FieldProfiles>()
            .register(FieldProfile {
                name: "harvested_wheat",
                wheel_response: WheelResponse {
                    recovery_seconds: 1800.0,
                    bend: 0.94,
                    darkening: 0.44,
                    footprint_length: 0.30,
                },
                layers: vec![VegetationLayer {
                    shader: "embedded://gearbox_sim/fields/harvested_wheat/shaders/vegetation.wgsl",
                    template: stalk_template,
                    density,
                    fade_start: 4.0,
                    fade_end: 32.0,
                    inverse_square_thinning: true,
                }, VegetationLayer {
                    shader: "embedded://gearbox_sim/fields/harvested_wheat/shaders/vegetation.wgsl",
                    template: leaf_template,
                    density: if density > 0.0 { 60.0 } else { 0.0 },
                    fade_start: 24.0,
                    fade_end: 96.0,
                    inverse_square_thinning: true,
                }, VegetationLayer {
                    shader: "embedded://gearbox_sim/fields/harvested_wheat/shaders/vegetation.wgsl",
                    template: super::canopy::template,
                    density: (density / 6000.0) * 80.0,
                    fade_start: 12.0,
                    fade_end: super::canopy::FADE_END_M,
                    inverse_square_thinning: true,
                }],
                ground: create_ground,
            });
    }
}

fn create_ground(
    world: &mut World,
    tracks: Handle<Image>,
    wheels: WheelMapParams,
    geometry: SurfaceGeometry,
) -> Arc<dyn GroundSurface> {
    let assets = world.resource::<AssetServer>();
    let extension = AntiRepeatTerrainExtension {
        terrain_albedo: assets
            .load_builder()
            .with_settings(|settings: &mut bevy::image::ImageLoaderSettings| {
                settings.sampler = bevy::image::ImageSampler::linear();
            })
            .load("embedded://gearbox_sim/fields/harvested_wheat/textures/soil_albedo.jpg"),
        terrain_height: assets
            .load("embedded://gearbox_sim/fields/harvested_wheat/textures/soil_height.jpg"),
        terrain_detail_albedo: assets
            .load("embedded://gearbox_sim/fields/harvested_wheat/textures/detail_albedo.jpg"),
        terrain_detail_height: assets
            .load("embedded://gearbox_sim/fields/harvested_wheat/textures/detail_height.jpg"),
        tint: Vec4::ONE,
        tracks,
        wheels,
        heightmap: Some(geometry.heightmap),
        geometry: geometry.params,
    };
    let material = world
        .resource_mut::<Assets<AntiRepeatTerrainMaterial>>()
        .add(ExtendedMaterial {
            base: StandardMaterial {
                perceptual_roughness: 0.98,
                metallic: 0.0,
                diffuse_transmission: 0.10,
                thickness: 0.0006,
                ..default()
            },
            extension,
        });
    Arc::new(MaterialSurface(material))
}

/// Six segmented leaves for clover and rosettes; position.z selects the leaf.
fn leaf_template() -> Mesh {
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

fn stalk_template() -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [-1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [-1.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
        ],
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; 4])
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; 4])
    .with_inserted_indices(Indices::U32(vec![0, 2, 1, 1, 2, 3]))
}
