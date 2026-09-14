//! Volumetric clouds, scene sunlight, ambient fill, and camera rendering settings.

use super::daylight::Daylight;
use super::{DaylightUpdate, EnvironmentSettings};
use crate::viewer::overlays::OriginalIlluminance;
use bevy::camera::{Exposure, Hdr};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::light::{CascadeShadowConfigBuilder, DirectionalLightShadowMap, EnvironmentMapLight};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
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
    let map = EnvironmentMapLight {
        intensity: settings.indirect_intensity * daylight.fill_strength,
        ..EnvironmentMapLight::hemispherical_gradient(
            &mut images,
            daylight.indirect_top,
            daylight.indirect_mid,
            daylight.indirect_bottom,
        )
    };
    for (mut fog, mut ambient, mut environment) in &mut cameras {
        fog.color = daylight.fog;
        ambient.color = daylight.ambient;
        ambient.brightness = 50.0 * daylight.fill_strength;
        *environment = map.clone();
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
