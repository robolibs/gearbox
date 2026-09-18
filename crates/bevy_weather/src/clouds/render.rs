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
    /// Direction towards the sun, with the disc's angular radius in `w`.
    #[uniform(107)]
    pub sun: Vec4,
    /// Radiance of the sun's disc.
    #[uniform(108)]
    pub sun_radiance: Vec4,
    /// The cloud shell: its base and top above the ground, the planet's
    /// radius, and 1 for the pass drawn over the scene, 0 for the sky behind.
    #[uniform(109)]
    pub shell: Vec4,
}

impl Material for CloudsMaterial {
    // Under the clouds they belong to the sky, behind everything. From above
    // them they lie over the land, so a second pass blends them in front.
    fn alpha_mode(&self) -> AlphaMode {
        if self.shell.w > 0.5 { AlphaMode::Premultiplied } else { AlphaMode::Opaque }
    }

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
