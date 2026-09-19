//! Bare ground: soil with nothing standing in it.
//!
//! One ground serves sand, dirt and ploughed earth alike; they differ in
//! their colour, how coarse their clods are and whether the wind has combed
//! ripples into them. It carries wheel marks like any field, and it borrows
//! the soil scans the stubble field already ships rather than its own.

use std::sync::Arc;

use bevy::asset::AssetServer;
use bevy::image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor};
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;

use crate::profile::{
    FieldProfile, FieldProfiles, GroundSurface, MaterialSurface, SurfaceGeometry,
    SurfaceGeometryParams, VegetationLayer, WheelMapParams, WheelResponse,
};

/// A stone, as a lump with `rings` bands of `around` faces, pushed in and out
/// so no two of its faces lie flat. `position.z` under a half marks it a
/// stone rather than a tuft.
fn pebble() -> Mesh {
    const AROUND: u32 = 7;
    const RINGS: u32 = 4;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    let dent = |ring: u32, step: u32| {
        let n = (ring * 7 + step * 13) as f32;
        0.72 + 0.28 * ((n * 1.7).sin() * 0.5 + 0.5)
    };
    for ring in 0..=RINGS {
        let v = ring as f32 / RINGS as f32;
        let lift = (v * std::f32::consts::PI).cos();
        let round = (v * std::f32::consts::PI).sin().max(0.22);
        for step in 0..AROUND {
            let u = step as f32 / AROUND as f32 * std::f32::consts::TAU;
            let radius = dent(ring, step);
            let point = Vec3::new(u.cos() * round * radius, lift * radius, u.sin() * round * radius);
            positions.push([point.x, point.y, 0.0_f32.max(0.0)]);
            normals.push(point.normalize().to_array());
        }
    }
    // The z of a position marks the kind, so the shape is carried in x and y
    // and the third axis of each point is put back by the shader's own turn.
    for (index, point) in positions.iter_mut().enumerate() {
        let ring = index as u32 / AROUND;
        let step = index as u32 % AROUND;
        let v = ring as f32 / RINGS as f32;
        let u = step as f32 / AROUND as f32 * std::f32::consts::TAU;
        let radius = dent(ring, step);
        let round = (v * std::f32::consts::PI).sin().max(0.22);
        point[0] = u.cos() * round * radius;
        point[1] = (v * std::f32::consts::PI).cos() * radius;
        point[2] = u.sin() * round * radius;
    }
    for ring in 0..RINGS {
        for step in 0..AROUND {
            let next = (step + 1) % AROUND;
            let a = ring * AROUND + step;
            let b = ring * AROUND + next;
            let c = (ring + 1) * AROUND + step;
            let d = (ring + 1) * AROUND + next;
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals.clone())
        // The first channel marks the kind: nought a stone, one a tuft. A stone's
        // own coordinates run the whole way round it, so they cannot say.
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; normals.len()])
        .with_inserted_indices(bevy::mesh::Indices::U32(indices))
}

/// A tuft of three leaves, each a strip two quads tall. `position.z` is the
/// leaf's number, raised by one so it cannot be taken for a stone.
fn tuft() -> Mesh {
    let (leaves, steps) = (3u32, 2u32);
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for leaf in 0..leaves {
        let start = positions.len() as u32;
        for step in 0..=steps {
            let t = step as f32 / steps as f32;
            positions.push([-1.0, t, leaf as f32 + 1.0]);
            positions.push([1.0, t, leaf as f32 + 1.0]);
        }
        for step in 0..steps {
            let row = start + step * 2;
            indices.extend_from_slice(&[row, row + 2, row + 1, row + 1, row + 2, row + 3]);
        }
    }
    let count = positions.len();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; count])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[1.0, 0.0]; count])
        .with_inserted_indices(bevy::mesh::Indices::U32(indices))
}

/// Stones lying in the soil, and the tufts that take it back.
fn standing() -> Vec<VegetationLayer> {
    let shader = "embedded://gearbox_fields/bare/shaders/vegetation.wgsl";
    vec![
        VegetationLayer {
            shader,
            template: pebble,
            density: 9.0,
            fade_start: 6.0,
            fade_end: 42.0,
            inverse_square_thinning: true,
            albedo: None,
            lod_band: [0.0, f32::MAX],
        },
        VegetationLayer {
            shader,
            template: tuft,
            density: 1100.0,
            fade_start: 6.0,
            fade_end: 34.0,
            inverse_square_thinning: true,
            albedo: None,
            lod_band: [0.0, f32::MAX],
        },
    ]
}

type BareMaterial = ExtendedMaterial<StandardMaterial, BareExtension>;

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
struct BareExtension {
    #[texture(100)]
    #[sampler(101)]
    soil_albedo: Handle<Image>,
    #[texture(102)]
    soil_height: Handle<Image>,
    #[texture(103)]
    grit_albedo: Handle<Image>,
    #[texture(104)]
    grit_height: Handle<Image>,
    #[texture(105, sample_type = "u_int")]
    trample: Handle<Image>,
    #[uniform(106)]
    trample_params: WheelMapParams,
    #[texture(107, sample_type = "float", filterable = false)]
    heightmap: Handle<Image>,
    #[uniform(108)]
    geometry: SurfaceGeometryParams,
    #[uniform(109)]
    ground: BareGround,
}

/// What kind of bare ground this is.
#[derive(ShaderType, Reflect, Debug, Clone, Copy)]
pub struct BareGround {
    /// Multiplies the soil's own colour.
    pub tint: Vec4,
    /// Metres across the clods, how deep they sit, how much the wind has
    /// combed it into ripples, and how far apart those ripples run.
    pub grain: Vec4,
    /// The colour of the grass that grows through it, and how much of the
    /// ground it takes: nought is barren.
    pub grass: Vec4,
}

impl MaterialExtension for BareExtension {
    fn fragment_shader() -> ShaderRef {
        "embedded://gearbox_fields/bare/shaders/material.wgsl".into()
    }
}

pub(super) struct BarePlugin;

impl Plugin for BarePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "shaders/material.wgsl");
        bevy::asset::embedded_asset!(app, "shaders/vegetation.wgsl");
        app.add_plugins(MaterialPlugin::<BareMaterial>::default());
        // Nothing stands in it, so it has no vegetation layers at all: the
        // whole of it is its ground.
        let response = WheelResponse {
            recovery_seconds: 900.0,
            bend: 0.9,
            darkening: 0.3,
            footprint_length: 0.3,
            tread: true,
        };
        let mut profiles = app.world_mut().resource_mut::<FieldProfiles>();
        profiles.register(FieldProfile {
            name: "ploughed",
            wheel_response: response,
            layers: standing(),
            ground: ploughed_ground,
        });
        profiles.register(FieldProfile {
            name: "dirt",
            wheel_response: response,
            layers: standing(),
            ground: dirt_ground,
        });
        profiles.register(FieldProfile {
            name: "sand",
            wheel_response: WheelResponse { darkening: 0.16, ..response },
            layers: standing(),
            ground: sand_ground,
        });
    }
}

/// Turned earth: dark, coarse and clodded, with the plough's own combing.
fn ploughed_ground(
    world: &mut World,
    trample: Handle<Image>,
    trample_params: WheelMapParams,
    geometry: SurfaceGeometry,
) -> Arc<dyn GroundSurface> {
    ground(
        world,
        trample,
        trample_params,
        geometry,
        BareGround {
            tint: Vec4::new(0.060, 0.034, 0.018, 1.0),
            grain: Vec4::new(0.34, 1.0, 1.0, 1.25),
            grass: Vec4::new(0.055, 0.085, 0.022, 0.0),
        },
        0.93,
    )
}

/// A worn track or yard: paler, packed flat, barely combed.
fn dirt_ground(
    world: &mut World,
    trample: Handle<Image>,
    trample_params: WheelMapParams,
    geometry: SurfaceGeometry,
) -> Arc<dyn GroundSurface> {
    ground(
        world,
        trample,
        trample_params,
        geometry,
        BareGround {
            tint: Vec4::new(0.115, 0.070, 0.038, 1.0),
            grain: Vec4::new(0.3, 0.40, 0.1, 1.6),
            grass: Vec4::new(0.05, 0.075, 0.02, 0.0),
        },
        0.88,
    )
}

/// Sand: pale, fine-grained, and combed into ripples by the wind.
fn sand_ground(
    world: &mut World,
    trample: Handle<Image>,
    trample_params: WheelMapParams,
    geometry: SurfaceGeometry,
) -> Arc<dyn GroundSurface> {
    ground(
        world,
        trample,
        trample_params,
        geometry,
        BareGround {
            tint: Vec4::new(0.230, 0.178, 0.108, 1.0),
            grain: Vec4::new(1.6, 0.3, 1.0, 0.9),
            grass: Vec4::new(0.06, 0.08, 0.03, 0.0),
        },
        0.82,
    )
}

fn ground(
    world: &mut World,
    trample: Handle<Image>,
    trample_params: WheelMapParams,
    geometry: SurfaceGeometry,
    bare: BareGround,
    roughness: f32,
) -> Arc<dyn GroundSurface> {
    let assets = world.resource::<AssetServer>();
    let extension = BareExtension {
        soil_albedo: soil(assets, "soil_albedo", true),
        soil_height: soil(assets, "soil_height", false),
        grit_albedo: soil(assets, "detail_albedo", true),
        grit_height: soil(assets, "detail_height", false),
        trample,
        trample_params,
        heightmap: geometry.heightmap,
        geometry: geometry.params,
        ground: bare,
    };
    let material = world
        .resource_mut::<Assets<BareMaterial>>()
        .add(ExtendedMaterial {
            base: StandardMaterial {
                perceptual_roughness: roughness,
                metallic: 0.0,
                ..default()
            },
            extension,
        });
    Arc::new(MaterialSurface(material))
}

fn soil(assets: &AssetServer, name: &str, srgb: bool) -> Handle<Image> {
    assets
        .load_with_settings(
            format!("embedded://gearbox_fields/harvested_wheat/textures/{name}.jpg"),
            move |settings: &mut ImageLoaderSettings| {
                settings.is_srgb = srgb;
                settings.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                    address_mode_u: ImageAddressMode::Repeat,
                    address_mode_v: ImageAddressMode::Repeat,
                    ..ImageSamplerDescriptor::linear()
                });
            },
        )
}
