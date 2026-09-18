//! Volumetric clouds, scene sunlight, ambient fill, and camera rendering settings.

use super::daylight::Daylight;
use super::{DaylightUpdate, WeatherSettings};
use crate::OriginalIlluminance;
use bevy::asset::RenderAssetUsages;
use bevy::camera::{Exposure, Hdr};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::light::{CascadeShadowConfigBuilder, DirectionalLightShadowMap, EnvironmentMapLight};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};
use crate::clouds::config::CloudsConfig;
use crate::clouds::{CloudsCamera, CloudsPlugin};

#[derive(Component)]
struct Sun;

pub(super) struct SkyPlugin;

impl Plugin for SkyPlugin {
    fn build(&self, app: &mut App) {
        let settings = app.world().resource::<WeatherSettings>();
        let clouds = CloudsConfig {
            sun_dir: Daylight::from_settings(settings).direction.extend(0.0),
            wind_velocity: Vec3::new(settings.cloud_velocity.x, 0.0, settings.cloud_velocity.y),
            ..settings.clouds
        };
        app.insert_resource(clouds)
            .insert_resource(DirectionalLightShadowMap { size: 4096 })
            .add_plugins(CloudsPlugin)
            .add_systems(Startup, spawn_daylight)
            .add_systems(
                Update,
                (configure_cameras, synchronize_daylight)
                    .chain()
                    .in_set(DaylightUpdate),
            );
    }
}

fn spawn_daylight(mut commands: Commands, settings: Res<WeatherSettings>) {
    commands.insert_resource(ClearColor(settings.fog_color));
    commands.spawn((
        Sun,
        Name::new("Sun"),
        OriginalIlluminance(settings.illuminance),
        Transform::default(),
        DirectionalLight {
            illuminance: settings.illuminance,
            color: settings.sun_color,
            shadow_maps_enabled: true,
            ..default()
        },
        CascadeShadowConfigBuilder {
            num_cascades: 4,
            minimum_distance: 0.1,
            maximum_distance: 800.0,
            first_cascade_far_bound: 40.0,
            overlap_proportion: 0.2,
        }
        .build(),
    ));
}

fn synchronize_daylight(
    settings: Res<WeatherSettings>,
    mut clouds: ResMut<CloudsConfig>,
    mut sun: Query<
        (
            &mut Transform,
            &mut DirectionalLight,
            &mut OriginalIlluminance,
        ),
        With<Sun>,
    >,
    mut cameras: Query<
        (
            &mut DistanceFog,
            &mut AmbientLight,
            &mut EnvironmentMapLight,
        ),
        With<CloudsCamera>,
    >,
    added: Query<Entity, Added<CloudsCamera>>,
    mut images: ResMut<Assets<Image>>,
    mut clear: ResMut<ClearColor>,
    mut sky_map: Local<Option<Handle<Image>>>,
) {
    if !settings.is_changed() && added.is_empty() {
        return;
    }
    let daylight = Daylight::from_settings(&settings);
    let overcast = ((settings.clouds.clouds_coverage - 0.55) / 0.45).clamp(0.0, 1.0);
    for (mut transform, mut light, mut original) in &mut sun {
        let up = if daylight.direction.y.abs() > 0.999 {
            Vec3::Z
        } else {
            Vec3::Y
        };
        *transform =
            Transform::from_translation(daylight.direction).looking_to(-daylight.direction, up);
        light.color = daylight.direct_color;
        light.illuminance =
            settings.illuminance * daylight.direct_strength * (1.0 - 0.85 * overcast * overcast);
        original.0 = light.illuminance;
        light.shadow_maps_enabled = daylight.direct_strength > 0.0;
    }
    let resolution = clouds.render_resolution;
    *clouds = settings.clouds;
    clouds.render_resolution = resolution;
    clouds.wind_velocity = Vec3::new(settings.cloud_velocity.x, 0.0, settings.cloud_velocity.y);
    clouds.sun_dir = daylight.direction.extend(0.0);
    clouds.sun_color = daylight.sun_radiance;
    clouds.sky_zenith_color = daylight.zenith;
    clouds.sky_horizon_color = daylight.horizon;
    clouds.clouds_ambient_color_top = daylight.cloud_top;
    clouds.clouds_ambient_color_bottom = daylight.cloud_bottom;
    clear.0 = daylight.fog;
    if cameras.is_empty() {
        return;
    }
    // One cubemap for the session, rewritten in place: a fresh image per
    // change left views a frame without their environment map while the
    // pipelines still expected one, which wgpu rejects.
    let texels = gradient_texels(
        daylight.indirect_top.into(),
        daylight.indirect_mid.into(),
        daylight.indirect_bottom.into(),
    );
    let handle = match sky_map.clone().filter(|handle| images.contains(handle)) {
        Some(handle) => {
            if let Some(mut image) = images.get_mut(&handle) {
                image.data = Some(texels);
            }
            handle
        }
        None => sky_map.insert(images.add(gradient_cubemap(texels))).clone(),
    };
    let map = EnvironmentMapLight {
        diffuse_map: handle.clone(),
        specular_map: handle,
        intensity: settings.indirect_intensity * daylight.fill_strength,
        ..default()
    };
    for (mut fog, mut ambient, mut environment) in &mut cameras {
        fog.color = daylight.fog;
        fog.falloff = haze_falloff(settings.haze_visibility_km);
        // Haze toward the sun takes its colour: the glow that gives a low sun depth.
        fog.directional_light_color = daylight.direct_color.with_alpha(0.45);
        ambient.color = daylight.ambient;
        ambient.brightness = 50.0 * daylight.fill_strength;
        *environment = map.clone();
    }
}

/// Sky, horizon and ground fill as the six faces of a 1x1 cubemap.
fn gradient_texels(top: LinearRgba, mid: LinearRgba, bottom: LinearRgba) -> Vec<u8> {
    [mid, mid, top, bottom, mid, mid]
        .into_iter()
        .flat_map(|colour| colour.to_f32_array())
        .flat_map(|value| half::f16::from_f32(value).to_le_bytes())
        .collect()
}

fn gradient_cubemap(texels: Vec<u8>) -> Image {
    Image {
        texture_view_descriptor: Some(TextureViewDescriptor {
            dimension: Some(TextureViewDimension::Cube),
            ..default()
        }),
        ..Image::new(
            Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 6,
            },
            TextureDimension::D2,
            texels,
            TextureFormat::Rgba16Float,
            RenderAssetUsages::default(),
        )
    }
}

// Air scatters blue out of what lies behind it and scatters sky blue in, so
// distant land cools and pales toward the sky instead of greying.
fn haze_falloff(visibility_km: f32) -> FogFalloff {
    FogFalloff::from_visibility_colors(
        visibility_km * 1000.0,
        Color::srgb(0.35, 0.5, 0.66),
        Color::srgb(0.8, 0.844, 1.0),
    )
}

fn configure_cameras(
    mut commands: Commands,
    cameras: Query<Entity, Added<Camera3d>>,
    settings: Res<WeatherSettings>,
) {
    for entity in &cameras {
        commands.entity(entity).insert((
            Hdr,
            CloudsCamera,
            Exposure {
                ev100: settings.exposure_ev100,
            },
            Tonemapping::AgX,
            Bloom {
                intensity: settings.bloom_intensity,
                ..Bloom::NATURAL
            },
            Msaa::Sample4,
            DistanceFog {
                color: settings.fog_color,
                falloff: haze_falloff(settings.haze_visibility_km),
                directional_light_exponent: 24.0,
                ..default()
            },
            AmbientLight {
                color: Color::srgb(0.80, 0.88, 1.0),
                brightness: 50.0,
                ..default()
            },
            EnvironmentMapLight::default(),
        ));
    }
}
