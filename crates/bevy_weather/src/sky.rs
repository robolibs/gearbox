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
                    thin_air_with_height,
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
            scale: transform.scale,
        };
        light.color = daylight.direct_color;
        light.illuminance = settings.illuminance * daylight.direct_strength;
        original.0 = light.illuminance;
        light.shadow_maps_enabled = daylight.direct_strength > 0.0;
    }
    let (resolution, center, extent) =
        (clouds.render_resolution, clouds.cloud_shadow_center, clouds.cloud_shadow_extent);
    *clouds = settings.clouds;
    clouds.render_resolution = resolution;
    clouds.cloud_shadow_center = center;
    clouds.cloud_shadow_extent = extent;
    clouds.planet_from_site = Mat3::from_quat(settings.planet_from_site);
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

/// The cloud shadow map follows the view, as engines' cloud shadows do, so its
/// texels are spent where they are seen: it centres on the ground the camera
/// looks at and reaches further as the camera climbs. The centre moves a whole
/// texel at a time and the reach in steps, because a map that slid or
/// stretched smoothly would make every shadow edge crawl.
fn follow_with_cloud_shadows(
    settings: Res<WeatherSettings>,
    shadows: Option<Res<crate::clouds::CloudShadowMap>>,
    cameras: Query<&GlobalTransform, With<CloudsCamera>>,
    mut clouds: ResMut<CloudsConfig>,
    mut suns: Query<&mut Transform, With<Sun>>,
) {
    let (Some(shadows), Some(camera)) = (shadows, cameras.iter().next()) else {
        return;
    };
    let (eye, forward) = (camera.translation(), camera.forward().as_vec3());
    // From height more land is in view; the reach doubles in half-steps.
    let wanted = (shadows.half_extent + 1.6 * eye.y.max(0.0)) / shadows.half_extent;
    // Beyond a few tens of kilometres single cloud shadows are below a pixel.
    let half = (shadows.half_extent * 2f32.powf((wanted.log2() * 2.0).ceil() / 2.0)).min(32_000.0);
    // The ground in the middle of the view, or a way ahead when the view
    // never meets it.
    let ahead = if forward.y < -0.05 { eye.y.max(0.0) / -forward.y } else { f32::MAX };
    let focus = eye + forward * ahead.min(0.6 * half);
    let texel = 2.0 * half / shadows.resolution as f32;
    for mut sun in &mut suns {
        let (right, up) = (sun.rotation * Vec3::X, sun.rotation * Vec3::Y);
        let snap = |along: Vec3| (focus.dot(along) / texel).round() * texel;
        let center = right * snap(right) + up * snap(up);
        if sun.translation != center || sun.scale != Vec3::splat(half) {
            sun.translation = center;
            sun.scale = Vec3::splat(half);
        }
        if clouds.cloud_shadow_center != center || clouds.cloud_shadow_extent != 2.0 * half {
            clouds.cloud_shadow_center = center;
            clouds.cloud_shadow_extent = 2.0 * half;
        }
        // From high enough no single cloud's shadow can be made out, and the
        // map's repeats would show as a grid instead: the shadows fade away.
        let opacity = settings.clouds.cloud_shadow_opacity
            * (1.0 - ((eye.y - 8_000.0) / 22_000.0).clamp(0.0, 1.0));
        if (clouds.cloud_shadow_opacity - opacity).abs() > 0.005 {
            clouds.cloud_shadow_opacity = opacity;
        }
    }
}

/// Haze and sky blue are the air between the eye and the scene, and the air
/// thins with height: its haze layer is some 1.5 km deep, so a camera well
/// above it looks down through a fixed depth of haze however far it climbs,
/// and the blue overhead fades out with the pressure, to black in orbit.
fn thin_air_with_height(
    settings: Res<WeatherSettings>,
    mut cameras: Query<(&GlobalTransform, &mut DistanceFog), With<CloudsCamera>>,
    mut clouds: ResMut<CloudsConfig>,
    mut applied: Local<Option<f32>>,
) {
    const HAZE_DEPTH_M: f32 = 1_500.0;
    const SKY_SCALE_HEIGHT_M: f32 = 8_400.0;
    let Some(height) = cameras.iter().map(|(at, _)| at.translation().y.max(1.0)).reduce(f32::max)
    else {
        return;
    };
    // Mean haze along a sight line to the ground, against haze at the ground.
    let haze = HAZE_DEPTH_M / height * (1.0 - (-height / HAZE_DEPTH_M).exp());
    if !settings.is_changed() && applied.is_some_and(|was| (was - haze).abs() < 0.01 * was) {
        return;
    }
    *applied = Some(haze);
    for (_, mut fog) in &mut cameras {
        fog.falloff = haze_falloff(settings.haze_visibility_km / haze);
    }
    let sky = (-height / SKY_SCALE_HEIGHT_M).exp();
    let daylight = Daylight::from_settings(&settings);
    clouds.sky_zenith_color = daylight.zenith * sky;
    clouds.sky_horizon_color = daylight.horizon * sky;
}
