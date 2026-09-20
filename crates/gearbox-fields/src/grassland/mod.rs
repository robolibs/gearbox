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
    #[uniform(108)]
    edges: MeadowEdges,
}

/// Where this meadow sits and what lies across each of its sides, so it meets
/// its neighbours in a wash rather than on a line — the same blend the bare
/// grounds make, from the other side of it. A way is a track worn across the
/// meadow itself: wear is not a thing bare ground alone can have, or a road
/// could never cross a field without the field being cut in three.
#[derive(bevy::render::render_resource::ShaderType, Reflect, Debug, Clone, Copy, Default)]
struct MeadowEdges {
    extent: Vec4,
    west: Vec4,
    east: Vec4,
    south: Vec4,
    north: Vec4,
    reach: Vec4,
    tread: Vec4,
    way: Mat4,
    way_more: Mat4,
    way_shape: Vec4,
    /// What a way wears the meadow down to, shared with the bare grounds so a
    /// track does not change colour where it leaves one field for the next.
    soil: Vec4,
    stony: Vec4,
}

impl MaterialExtension for MeadowExtension {
    fn fragment_shader() -> ShaderRef {
        "embedded://gearbox_fields/grassland/shaders/material.wgsl".into()
    }
}

pub(super) struct GrasslandPlugin {
    pub density: f32,
}

impl Plugin for GrasslandPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "shaders/palette.wgsl");
        bevy::asset::embedded_asset!(app, "shaders/material.wgsl");
        bevy::asset::embedded_asset!(app, "shaders/vegetation.wgsl");
        bevy::asset::embedded_asset!(app, "textures/grass_albedo.jpg");
        bevy::asset::embedded_asset!(app, "textures/soil_albedo.jpg");
        app.add_plugins(MaterialPlugin::<MeadowMaterial>::default());
        let shader = "embedded://gearbox_fields/grassland/shaders/vegetation.wgsl";
        let density = self.density;
        let share = density / 6000.0;
        // Near and mid meshes fold two blades from each root.
        // Half the width apiece, so half again as many of them: a thinner
        // blade shows more ground through the sward than a thick one does.
        let blades = density / 2.0 * 1.5;
        app.world_mut()
            .resource_mut::<FieldProfiles>()
            .register(FieldProfile {
                name: "grassland",
                wheel_response: WheelResponse {
                    recovery_seconds: 300.0,
                    bend: 0.9,
                    darkening: 0.43,
                    footprint_length: 0.25,
                    tread: false,
                },
                layers: vec![
                    // One blade population at three levels of detail.
                    VegetationLayer {
                        shader,
                        template: got_near,
                        density: blades,
                        fade_start: 8.0,
                        fade_end: 128.0,
                        inverse_square_thinning: true,
                        follow_grass: 0.0,
                        albedo: None,
                        lod_band: GOT_NEAR_BAND,
                    },
                    VegetationLayer {
                        shader,
                        template: got_mid,
                        density: blades,
                        fade_start: 8.0,
                        fade_end: 128.0,
                        inverse_square_thinning: true,
                        follow_grass: 0.0,
                        albedo: None,
                        lod_band: GOT_MID_BAND,
                    },
                    VegetationLayer {
                        shader,
                        template: got_far,
                        density: blades,
                        fade_start: 8.0,
                        fade_end: 128.0,
                        inverse_square_thinning: true,
                        follow_grass: 0.0,
                        albedo: None,
                        lod_band: GOT_FAR_BAND,
                    },
                    VegetationLayer {
                        shader,
                        template: meadow_detail_template,
                        density: if density > 0.0 { 80.0 } else { 0.0 },
                        fade_start: 32.0,
                        fade_end: 144.0,
                        inverse_square_thinning: true,
                        follow_grass: 0.0,
                        albedo: None,
                        lod_band: [0.0, f32::MAX],
                    },
                    VegetationLayer {
                        shader,
                        template: flower_template,
                        density: if density > 0.0 { 36.0 } else { 0.0 },
                        fade_start: 6.0,
                        fade_end: 45.0,
                        inverse_square_thinning: true,
                        follow_grass: 0.0,
                        albedo: None,
                        lod_band: [0.0, f32::MAX],
                    },
                    VegetationLayer {
                        shader,
                        template: super::canopy::template,
                        density: (density / 6000.0) * 80.0,
                        fade_start: 12.0,
                        fade_end: super::canopy::FADE_END_M,
                        inverse_square_thinning: true,
                        follow_grass: 0.0,
                        albedo: None,
                        lod_band: [0.0, f32::MAX],
                    },
                    super::clumps::bermuda(share * 120.0, 40.0),
                    super::clumps::meadow_tufts(share * 50.0, 36.0),
                    super::clumps::sorrel(share * 30.0, 32.0),
                    // Scanned plants, a few dozen in view against the sward's
                    // millions of blades: they carry the detail the blades
                    // cannot, and are rare enough that the eye never finds the
                    // same one twice.
                    super::clumps::dandelion(share * 2.5, 38.0),
                    super::clumps::nettle(share * 0.8, 34.0),
                    super::clumps::celandine(share * 1.0, 40.0),
                    // Chippings where a track crosses the meadow. A recoloured
                    // ground is not a road; something has to lie loose on it.
                    super::bare::way_grit(2800.0),
                ],
                ground: create_ground,
                tread: Vec4::ZERO,
                soft_border: 1.3,
                surface_tint: Vec4::new(0.055, 0.082, 0.030, 1.0),
            });
    }
}

fn create_ground(
    world: &mut World,
    trample: Handle<Image>,
    trample_params: WheelMapParams,
    geometry: SurfaceGeometry,
    placed: crate::profile::Placed,
) -> Arc<dyn GroundSurface> {
    let assets = world.resource::<AssetServer>();
    let grass_albedo = assets
        .load_builder()
        .with_settings(|settings: &mut bevy::image::ImageLoaderSettings| {
            settings.sampler = bevy::image::ImageSampler::linear();
        })
        .load("embedded://gearbox_fields/grassland/textures/grass_albedo.jpg");
    let dirt_albedo = assets
        .load_builder()
        .with_settings(|settings: &mut bevy::image::ImageLoaderSettings| {
            settings.sampler = bevy::image::ImageSampler::linear();
        })
        .load("embedded://gearbox_fields/grassland/textures/soil_albedo.jpg");
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
                edges: MeadowEdges {
                    extent: Vec4::new(
                        placed.bounds.min.x, placed.bounds.min.y,
                        placed.bounds.max.x, placed.bounds.max.y),
                    west: placed.tint[0],
                    east: placed.tint[1],
                    south: placed.tint[2],
                    north: placed.tint[3],
                    reach: Vec4::from_array(placed.reach),
                    tread: placed.tread(),
                    way: placed.way.packed().0,
                    way_more: placed.way.packed().1,
                    way_shape: placed.way.packed().2,
                    // Dark loam: what is under turf that has never been broken.
                    soil: Vec4::new(0.072, 0.050, 0.030, 1.0),
                    stony: crate::bare::WAY_HARDCORE,
                },
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

/// Distance bands of the three blade meshes (metres from the camera).
const GOT_NEAR_BAND: [f32; 2] = [0.0, 8.0];
const GOT_MID_BAND: [f32; 2] = [8.0, 32.0];
const GOT_FAR_BAND: [f32; 2] = [32.0, 1.0e9];

/// A blade mesh of one detail level: `blades` strips of `segments` from one
/// root (normal.x numbers the strip, so a short blade folds into twins);
/// uv carries the distance band it draws in, and the same roots swap meshes
/// across the bands.
fn got_template(segments: u32, blades: u32, band: [f32; 2]) -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    for blade in 0..blades {
        let start = positions.len() as u32;
        for k in 0..=segments {
            let t = k as f32 / segments as f32;
            let sides: &[f32] = if k == segments { &[0.0] } else { &[-1.0, 1.0] };
            for side in sides {
                positions.push([*side, t, 0.0]);
                normals.push([blade as f32, 1.0, 0.0]);
            }
        }
        for k in 0..segments.saturating_sub(1) {
            let l = start + k * 2;
            indices.extend_from_slice(&[l, l + 2, l + 1, l + 1, l + 2, l + 3]);
        }
        let last = start + (segments - 1) * 2;
        indices.extend_from_slice(&[last, last + 2, last + 1]);
    }
    let count = positions.len();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![band; count])
        .with_inserted_indices(Indices::U32(indices))
}

fn got_near() -> Mesh {
    got_template(4, 2, GOT_NEAR_BAND)
}

fn got_mid() -> Mesh {
    got_template(2, 2, GOT_MID_BAND)
}

fn got_far() -> Mesh {
    got_template(1, 1, GOT_FAR_BAND)
}

/// One flower: a stem (z = 10), eight petals (z = 11..18) and two leaves
/// (z = 30, 31); the shader shapes them per kind.
fn flower_template() -> Mesh {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    let mut strip = |part: f32, segments: u32| {
        let start = positions.len() as u32;
        for step in 0..=segments {
            let t = step as f32 / segments as f32;
            positions.push([-1.0, t, part]);
            positions.push([1.0, t, part]);
        }
        for step in 0..segments {
            let i = start + step * 2;
            indices.extend_from_slice(&[i, i + 2, i + 1, i + 1, i + 2, i + 3]);
        }
    };
    strip(10.0, 4);
    for petal in 0..8 {
        strip(11.0 + petal as f32, 2);
    }
    for leaf in 0..2 {
        strip(30.0 + leaf as f32, 3);
    }
    let count = positions.len();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; count])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; count])
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
