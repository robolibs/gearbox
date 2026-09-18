//! Volumetric clouds, scene sunlight, ambient fill, and camera rendering settings.

use super::daylight::Daylight;
use super::{DaylightUpdate, SkyLight, WeatherSettings};
use crate::OriginalIlluminance;
use bevy::asset::RenderAssetUsages;
use bevy::camera::{Exposure, Hdr};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::light::{
    CascadeShadowConfigBuilder, DirectionalLightShadowMap, DirectionalLightTexture,
    EnvironmentMapLight,
};
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
            wind_velocity: cloud_drift(settings),
            ..settings.clouds
        };
        app.insert_resource(clouds)
            .insert_resource(DirectionalLightShadowMap { size: 4096 })
            .add_plugins(CloudsPlugin)
            .add_systems(Startup, spawn_daylight)
            .add_systems(
                Update,
                (
                    configure_cameras,
                    carry_cloud_shadows,
                    synchronize_daylight,
                    follow_with_cloud_shadows,
                )
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
    mut sky_light: ResMut<SkyLight>,
    shadows: Option<Res<crate::clouds::CloudShadowMap>>,
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
    let sky = SkyLight::from_cover(settings.clouds.clouds_coverage);
    if *sky_light != sky {
        *sky_light = sky;
    }
    for (mut transform, mut light, mut original) in &mut sun {
        // Its scale is the reach of the cloud shadow map it carries as a light
        // texture; only its rotation lights and shadows anything.
        *transform = Transform {
            translation: transform.translation,
            rotation: crate::clouds::sun_rotation(daylight.direction),
            scale: Vec3::splat(shadows.as_ref().map_or(1.0, |map| map.half_extent)),
            ..default()
        };
        light.color = daylight.direct_color;
        light.illuminance = settings.illuminance * daylight.direct_strength;
        original.0 = light.illuminance;
        light.shadow_maps_enabled = daylight.direct_strength > 0.0;
    }
    let (resolution, shadow_center) = (clouds.render_resolution, clouds.cloud_shadow_center);
    *clouds = settings.clouds;
    clouds.render_resolution = resolution;
    clouds.cloud_shadow_center = shadow_center;
    clouds.wind_velocity = cloud_drift(&settings);
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
        intensity: settings.indirect_intensity * daylight.fill_strength * sky.diffuse_gain,
        ..default()
    };
    for (mut fog, mut ambient, mut environment) in &mut cameras {
        fog.color = daylight.fog;
        fog.falloff = haze_falloff(settings.haze_visibility_km);
        // Haze toward the sun takes its colour: the glow that gives a low sun depth.
        fog.directional_light_color = daylight.direct_color.with_alpha(0.45);
        ambient.color = daylight.ambient;
        ambient.brightness = 50.0 * daylight.fill_strength * sky.diffuse_gain;
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

/// The sun carries the clouds' shadow as its light texture, so a cloud dims
/// the direct light of everything under it alike — land, plants and
/// machines — and leaves the sky's light alone.
fn carry_cloud_shadows(
    mut commands: Commands,
    shadows: Option<Res<crate::clouds::CloudShadowMap>>,
    suns: Query<Entity, (With<Sun>, Without<DirectionalLightTexture>)>,
) {
    let Some(shadows) = shadows else {
        return;
    };
    for sun in &suns {
        commands.entity(sun).insert(DirectionalLightTexture {
            image: shadows.image.clone(),
            // Tiled: outside an untiled texture the light is simply off, and the
            // land past the map should repeat distant shadows, not lose its sun.
            tiled: true,
        });
    }
}

// Clouds ride the wind: its heading, at their own speed aloft.
fn cloud_drift(settings: &WeatherSettings) -> Vec3 {
    let (sin, cos) = settings.wind_heading_deg.to_radians().sin_cos();
    Vec3::new(cos, 0.0, sin) * settings.cloud_drift_mps.max(0.0)
}

/// The cloud shadow map follows the camera, as engines' cloud shadows do, so
/// its texels are spent where they are seen. The centre moves a whole texel at
/// a time: a map that slid smoothly would make every shadow edge crawl.
fn follow_with_cloud_shadows(
    shadows: Option<Res<crate::clouds::CloudShadowMap>>,
    cameras: Query<&GlobalTransform, With<CloudsCamera>>,
    mut clouds: ResMut<CloudsConfig>,
    mut suns: Query<&mut Transform, With<Sun>>,
) {
    let (Some(shadows), Some(camera)) = (shadows, cameras.iter().next()) else {
        return;
    };
    for mut sun in &mut suns {
        let (right, up) = (sun.rotation * Vec3::X, sun.rotation * Vec3::Y);
        let eye = camera.translation();
        let snap = |along: Vec3| (eye.dot(along) / shadows.texel).round() * shadows.texel;
        let center = right * snap(right) + up * snap(up);
        if sun.translation != center {
            sun.translation = center;
        }
        if clouds.cloud_shadow_center != center {
            clouds.cloud_shadow_center = center;
        }
    }
}
