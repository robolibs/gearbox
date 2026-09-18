use bevy::{
    asset::{AssetPath, embedded_asset, embedded_path},
    prelude::*,
    reflect::TypePath,
    render::render_resource::AsBindGroup,
    shader::{ShaderRef, load_shader_library},
};

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(crate) struct CloudsMaterial {
    #[texture(100)]
    #[sampler(101)]
    pub cloud_render_image: Handle<Image>,
    #[texture(106)]
    pub sky_image: Handle<Image>,
}

impl Material for CloudsMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("shaders/clouds.wgsl")).with_source("embedded"),
        )
    }
}

pub(crate) struct CloudsShaderPlugin;

impl Plugin for CloudsShaderPlugin {
    fn build(&self, app: &mut App) {
        load_shader_library!(app, "shaders/common.wgsl");
        embedded_asset!(app, "shaders/clouds.wgsl");
        embedded_asset!(app, "shaders/clouds_compute.wgsl");
        app.add_plugins(MaterialPlugin::<CloudsMaterial>::default());
    }
}
