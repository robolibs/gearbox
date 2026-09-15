//! Volumetric clouds, scene sunlight, ambient fill, and camera rendering settings.

use super::daylight::Daylight;
use super::{DaylightUpdate, EnvironmentSettings};
use crate::viewer::overlays::OriginalIlluminance;
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
use bevy_volumetric_clouds::config::CloudsConfig;
use bevy_volumetric_clouds::{CloudsCamera, CloudsPlugin};

#[derive(Component)]
struct Sun;

pub(super) struct SkyboxPlugin;

impl Plugin for SkyboxPlugin {
    fn build(&self, app: &mut App) {
        let settings = app.world().resource::<EnvironmentSettings>();
        let clouds = bevy_volumetric_clouds::config::CloudsConfig {
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

fn spawn_daylight(mut commands: Commands, settings: Res<EnvironmentSettings>) {
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
    settings: Res<EnvironmentSettings>,
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

fn configure_cameras(
    mut commands: Commands,
    cameras: Query<Entity, Added<Camera3d>>,
    settings: Res<EnvironmentSettings>,
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
                falloff: FogFalloff::Atmospheric {
                    extinction: Vec3::splat(settings.fog_extinction),
                    inscattering: Vec3::splat(settings.fog_extinction),
                },
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
