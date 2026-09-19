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

use crate::profile::{
    FieldProfile, FieldProfiles, GroundSurface, MaterialSurface, SurfaceGeometry,
    SurfaceGeometryParams, WheelMapParams, WheelResponse,
};

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
            layers: Vec::new(),
            ground: ploughed_ground,
        });
        profiles.register(FieldProfile {
            name: "dirt",
            wheel_response: response,
            layers: Vec::new(),
            ground: dirt_ground,
        });
        profiles.register(FieldProfile {
            name: "sand",
            wheel_response: WheelResponse { darkening: 0.16, ..response },
            layers: Vec::new(),
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
            tint: Vec4::new(0.33, 0.24, 0.16, 1.0),
            grain: Vec4::new(0.42, 1.0, 0.85, 0.55),
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
            tint: Vec4::new(0.42, 0.35, 0.26, 1.0),
            grain: Vec4::new(0.3, 0.45, 0.12, 0.6),
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
            // The last of the tint says it is made rather than read.
            tint: Vec4::new(0.40, 0.31, 0.19, 1.0),
            grain: Vec4::new(1.6, 0.3, 1.0, 0.85),
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
