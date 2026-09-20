//! Volumetric clouds and procedural sky, from evroon's `bevy-volumetric-clouds`
//! (MIT, see `LICENSE` and `UPSTREAM.md` beside this file).


mod compute;
pub mod config;
mod images;
mod render;
mod skybox;
mod uniforms;

use self::{
    compute::{CameraMatrices, CloudsComputePlugin},
    config::CloudsConfig,
    images::build_images,
    render::{CloudsMaterial, CloudsShaderPlugin},
    skybox::{SkyboxMaterials, init_skybox_mesh, update_skybox_transform},
    uniforms::CloudsImage,
};
use bevy::prelude::*;

/// Selects the scene camera used to render the cloud layer.
#[derive(Component)]
pub struct CloudsCamera;

/// The clouds' shadow as the sun sees it: each texel is one sun ray, its red
/// channel 1 where the ray reaches the land and 0 where solid cloud stops
/// it. It covers rays within `half_extent` metres of the world origin, in the
/// frame of [`sun_rotation`], which is how a directional light reads a
/// light texture.
#[derive(Resource, Clone)]
pub struct CloudShadowMap {
    pub image: Handle<Image>,
    /// Half the side the map covers when the camera is near the ground; it
    /// reaches further as the camera climbs.
    pub half_extent: f32,
    /// Texels along a side.
    pub resolution: u32,
}

/// The sun's orientation: it shines along its local -Z, and its local X and Y
/// span the cloud shadow map.
pub fn sun_rotation(towards_sun: Vec3) -> Quat {
    let up = if towards_sun.y.abs() > 0.999 { Vec3::Z } else { Vec3::Y };
    Transform::IDENTITY.looking_to(-towards_sun, up).rotation
}

/// Renders upstream volumetric clouds for the selected scene camera.
pub struct CloudsPlugin;

impl Plugin for CloudsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CloudsConfig>()
            .add_plugins((CloudsComputePlugin, CloudsShaderPlugin))
            .add_systems(Startup, clouds_setup)
            .add_systems(Update, sync_sun_disc)
            .add_systems(
                PostUpdate,
                (update_skybox_transform, update_camera_matrices)
                    .after(TransformSystems::Propagate),
            );
    }
}

fn clouds_setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<CloudsMaterial>>,
    config: Res<CloudsConfig>,
) {
    let fields = build_images(&mut images, config.render_resolution.as_uvec2());
    commands.insert_resource(CloudShadowMap {
        image: fields.ground_shadow_image.clone(),
        half_extent: 0.5 * config.cloud_shadow_extent,
        resolution: images::GROUND_SHADOW_SIZE,
    });
    // The sky behind the scene, and the clouds over it for a camera above them.
    for overlay in [0.0, 1.0] {
        let material = materials.add(CloudsMaterial {
            cloud_render_image: fields.cloud_render_image.clone(),
            sky_image: fields.sky_image.clone(),
            sun: sun_disc(&config),
            sun_radiance: sun_disc_radiance(&config),
            shell: cloud_shell(&config, overlay),
        });
        init_skybox_mesh(
            &mut commands,
            &mut meshes,
            SkyboxMaterials::from_one_material(MeshMaterial3d(material)),
        );
    }
    commands.insert_resource(fields);
    commands.insert_resource(CameraMatrices {
        translation: Vec3::ZERO,
        inverse_camera_projection: Mat4::IDENTITY,
        inverse_camera_view: Mat4::IDENTITY,
    });
}

fn update_camera_matrices(
    camera: Single<(&GlobalTransform, &Camera), With<CloudsCamera>>,
    mut matrices: ResMut<CameraMatrices>,
    mut config: ResMut<CloudsConfig>,
    mut fields: ResMut<CloudsImage>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<CloudsMaterial>>,
) {
    let (transform, camera) = *camera;
    matrices.translation = transform.translation();
    matrices.inverse_camera_view = transform.to_matrix();
    matrices.inverse_camera_projection = camera.computed.clip_from_view.inverse();
    let Some(viewport) = camera.physical_viewport_size() else {
        return;
    };
    let scale = config
        .render_scale
        .clamp(0.1, 1.0)
        .min(1280.0 / viewport.x.max(1) as f32);
    let size = (viewport.as_vec2() * scale)
        .ceil()
        .as_uvec2()
        .max(UVec2::splat(8));
    if config.render_resolution.as_uvec2() == size {
        return;
    }
    config.render_resolution = size.as_vec2();
    let old_output = fields.cloud_render_image.id();
    let extent = bevy::render::render_resource::Extent3d {
        width: size.x,
        height: size.y,
        depth_or_array_layers: 1,
    };
    let mut target = || {
        images.add(images::cloud_image(
            extent,
            bevy::render::render_resource::TextureDimension::D2,
        ))
    };
    fields.cloud_render_image = target();
    fields.history_image = target();
    fields.sky_image = target();
    for (_, material) in materials.iter_mut() {
        if material.cloud_render_image.id() == old_output {
            material.cloud_render_image = fields.cloud_render_image.clone();
            material.sky_image = fields.sky_image.clone();
        }
    }
}

/// The real sun's angular radius, in radians.
const SUN_ANGULAR_RADIUS: f32 = 0.004_65;

fn cloud_shell(config: &CloudsConfig, overlay: f32) -> Vec4 {
    Vec4::new(config.clouds_bottom_height, config.clouds_top_height, config.planet_radius, overlay)
}

fn sun_disc(config: &CloudsConfig) -> Vec4 {
    config.sun_dir.truncate().extend(SUN_ANGULAR_RADIUS * config.sun_disc_scale.max(0.1))
}

// Far brighter than the sky around it, so it blooms and tonemaps to white.
fn sun_disc_radiance(config: &CloudsConfig) -> Vec4 {
    (config.sun_color.truncate() * 40.0).extend(1.0)
}

/// The disc follows the sun across the sky and takes its colour.
fn sync_sun_disc(config: Res<CloudsConfig>, mut materials: ResMut<Assets<CloudsMaterial>>) {
    let (sun, radiance) = (sun_disc(&config), sun_disc_radiance(&config));
    let stale: Vec<_> = materials
        .iter()
        .filter(|(_, material)| {
            material.sun != sun
                || material.sun_radiance != radiance
                || material.shell != cloud_shell(&config, material.shell.w)
        })
        .map(|(id, _)| id)
        .collect();
    for id in stale {
        if let Some(mut material) = materials.get_mut(id) {
            material.sun = sun;
            material.sun_radiance = radiance;
            material.shell = cloud_shell(&config, material.shell.w);
        }
    }
}
