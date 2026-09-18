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

/// Renders upstream volumetric clouds for the selected scene camera.
pub struct CloudsPlugin;

impl Plugin for CloudsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CloudsConfig>()
            .add_plugins((CloudsComputePlugin, CloudsShaderPlugin))
            .add_systems(Startup, clouds_setup)
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
    meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<CloudsMaterial>>,
    config: Res<CloudsConfig>,
) {
    let fields = build_images(&mut images, config.render_resolution.as_uvec2());
    let material = materials.add(CloudsMaterial {
        cloud_render_image: fields.cloud_render_image.clone(),
        sky_image: fields.sky_image.clone(),
    });
    init_skybox_mesh(
        &mut commands,
        meshes,
        SkyboxMaterials::from_one_material(MeshMaterial3d(material)),
    );
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
