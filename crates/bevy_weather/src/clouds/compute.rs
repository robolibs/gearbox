use bevy::{
    asset::load_embedded_asset,
    ecs::system::ResMut,
    prelude::*,
    render::{
        Extract, GpuResourceAppExt, Render, RenderApp, RenderSystems,
        extract_resource::ExtractResourcePlugin,
        render_asset::RenderAssets,
        render_resource::{
            AsBindGroup, BindGroup, BindGroupEntries, BindGroupLayoutDescriptor,
            BindGroupLayoutEntries, CachedComputePipelineId, ComputePassDescriptor,
            ComputePipelineDescriptor, PipelineCache, ShaderStages, binding_types::uniform_buffer,
        },
        renderer::{RenderContext, RenderDevice, RenderGraph, RenderGraphSystems, RenderQueue},
        texture::GpuImage,
    },
};
/// Controls the compute shader which renders the volumetric clouds.
use std::borrow::Cow;

use super::config::CloudsConfig;

use super::{
    images::{GROUND_SHADOW_SIZE, IMAGE_SIZE},
    uniforms::{CloudsImage, CloudsUniform, CloudsUniformBuffer},
};

const WORKGROUP_SIZE: u32 = 8;

#[derive(Resource, Clone, Copy, PartialEq)]
pub(crate) struct CameraMatrices {
    pub translation: Vec3,
    pub inverse_camera_view: Mat4,
    pub inverse_camera_projection: Mat4,
}

#[derive(Resource)]
struct CloudsUniformBindGroup(BindGroup);

#[derive(Resource)]
struct CloudsImageBindGroup(BindGroup);

#[derive(Resource, Default)]
struct CloudsHistory(Option<(CameraMatrices, CloudsConfig, AssetId<Image>)>);

#[expect(clippy::too_many_arguments)]
fn prepare_uniforms_bind_group(
    mut commands: Commands,
    pipeline: Res<CloudsPipeline>,
    pipeline_cache: Res<PipelineCache>,
    render_queue: Res<RenderQueue>,
    mut clouds_uniform_buffer: ResMut<CloudsUniformBuffer>,
    camera: ResMut<CameraMatrices>,
    clouds_config: Res<CloudsConfig>,
    render_device: Res<RenderDevice>,
    time: Res<Time<Real>>,
    history: Res<CloudsHistory>,
    fields: Res<CloudsImage>,
    state: Res<CloudsState>,
) {
    let buffer = clouds_uniform_buffer.buffer.get_mut();

    buffer.clouds_raymarch_steps_count = clouds_config.clouds_raymarch_steps_count;
    buffer.planet_radius = clouds_config.planet_radius;
    buffer.clouds_bottom_height = clouds_config.clouds_bottom_height;
    buffer.clouds_top_height = clouds_config.clouds_top_height;
    buffer.clouds_coverage = clouds_config.clouds_coverage;
    buffer.clouds_detail_strength = clouds_config.clouds_detail_strength;
    buffer.clouds_base_edge_softness = clouds_config.clouds_base_edge_softness;
    buffer.clouds_bottom_softness = clouds_config.clouds_bottom_softness;
    buffer.clouds_density = clouds_config.clouds_density;
    buffer.clouds_shadow_raymarch_steps_count = clouds_config.clouds_shadow_raymarch_steps_count;
    buffer.clouds_shadow_raymarch_step_size = clouds_config.clouds_shadow_raymarch_step_size;
    buffer.clouds_shadow_raymarch_step_multiply =
        clouds_config.clouds_shadow_raymarch_step_multiply;
    buffer.forward_scattering_g = clouds_config.forward_scattering_g;
    buffer.backward_scattering_g = clouds_config.backward_scattering_g;
    buffer.scattering_lerp = clouds_config.scattering_lerp;
    buffer.clouds_ambient_color_top = clouds_config.clouds_ambient_color_top;
    buffer.clouds_ambient_color_bottom = clouds_config.clouds_ambient_color_bottom;
    buffer.clouds_min_transmittance = clouds_config.clouds_min_transmittance;
    buffer.clouds_base_scale = clouds_config.clouds_base_scale;
    buffer.clouds_detail_scale = clouds_config.clouds_detail_scale;
    buffer.sun_dir = clouds_config.sun_dir;
    buffer.sun_color = clouds_config.sun_color;
    buffer.sky_zenith_color = clouds_config.sky_zenith_color;
    buffer.sky_horizon_color = clouds_config.sky_horizon_color;
    buffer.camera_translation = camera.translation;
    buffer.time = time.elapsed_secs();
    let current = (*camera, *clouds_config, fields.cloud_render_image.id());
    buffer.reprojection_strength =
        if matches!(*state, CloudsState::Update) && history.0.as_ref() == Some(&current) {
            clouds_config
                .reprojection_strength
                .powf(time.delta_secs() * 60.0)
        } else {
            0.0
        };
    buffer.render_resolution = clouds_config.render_resolution;
    buffer.inverse_camera_view = camera.inverse_camera_view;
    buffer.inverse_camera_projection = camera.inverse_camera_projection;
    buffer.wind_displacement += time.delta_secs() * clouds_config.wind_velocity;
    let frame = super::sun_rotation(clouds_config.sun_dir.truncate());
    buffer.shadow_right = frame * Vec3::X;
    buffer.shadow_up = frame * Vec3::Y;
    buffer.shadow_half_extent = 0.5 * clouds_config.cloud_shadow_extent;
    buffer.shadow_opacity = clouds_config.cloud_shadow_opacity.clamp(0.0, 1.0);
    buffer.shadow_center = clouds_config.cloud_shadow_center;

    clouds_uniform_buffer
        .buffer
        .write_buffer(&render_device, &render_queue);

    let bind_group_uniforms = render_device.create_bind_group(
        None,
        &pipeline_cache.get_bind_group_layout(&pipeline.uniform_bind_group_layout),
        &BindGroupEntries::single(clouds_uniform_buffer.buffer.binding().unwrap().clone()),
    );
    commands.insert_resource(CloudsUniformBindGroup(bind_group_uniforms));
}

fn prepare_textures_bind_group(
    mut commands: Commands,
    pipeline: Res<CloudsPipeline>,
    pipeline_cache: Res<PipelineCache>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    clouds_image: Res<CloudsImage>,
    config: Res<CloudsConfig>,
    render_device: Res<RenderDevice>,
) {
    let (
        Some(cloud_render_view),
        Some(cloud_atlas_view),
        Some(cloud_worley_view),
        Some(sky_view),
        Some(history),
        Some(ground_shadow),
    ) = (
        gpu_images.get(&clouds_image.cloud_render_image),
        gpu_images.get(&clouds_image.cloud_atlas_image),
        gpu_images.get(&clouds_image.cloud_worley_image),
        gpu_images.get(&clouds_image.sky_image),
        gpu_images.get(&clouds_image.history_image),
        gpu_images.get(&clouds_image.ground_shadow_image),
    )
    else {
        commands.remove_resource::<CloudsImageBindGroup>();
        return;
    };

    let expected = config.render_resolution.as_uvec2();
    if [cloud_render_view, sky_view, history].iter().any(|image| {
        let size = image.texture_descriptor.size;
        UVec2::new(size.width, size.height) != expected
    }) {
        commands.remove_resource::<CloudsImageBindGroup>();
        return;
    }

    let bind_group = render_device.create_bind_group(
        None,
        &pipeline_cache.get_bind_group_layout(&pipeline.texture_bind_group_layout),
        &BindGroupEntries::sequential((
            &cloud_render_view.texture_view,
            &cloud_atlas_view.texture_view,
            &cloud_worley_view.texture_view,
            &sky_view.texture_view,
            &history.texture_view,
            &ground_shadow.texture_view,
        )),
    );
    commands.insert_resource(CloudsImageBindGroup(bind_group));
}

/// The compute shading pipeline
///
/// Note that the compute shader is loaded in [`CloudsShaderPlugin`] so this resource depends on
/// that plugin.
#[derive(Resource)]
struct CloudsPipeline {
    texture_bind_group_layout: BindGroupLayoutDescriptor,
    uniform_bind_group_layout: BindGroupLayoutDescriptor,
    init_pipeline: CachedComputePipelineId,
    update_pipeline: CachedComputePipelineId,
    ground_shadow_pipeline: CachedComputePipelineId,
}

impl FromWorld for CloudsPipeline {
    fn from_world(world: &mut World) -> Self {
        let render_device = world.resource::<RenderDevice>();
        let texture_bind_group_layout = CloudsImage::bind_group_layout_descriptor(render_device);
        let shader = load_embedded_asset!(world, "shaders/clouds_compute.wgsl");
        let pipeline_cache = world.resource::<PipelineCache>();

        let entries = BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (uniform_buffer::<CloudsUniform>(false),),
        );

        let uniform_bind_group_layout =
            BindGroupLayoutDescriptor::new("uniform_bind_group_layout", &entries);

        let init_pipeline = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
            zero_initialize_workgroup_memory: false,
            label: None,
            layout: vec![
                uniform_bind_group_layout.clone(),
                texture_bind_group_layout.clone(),
            ],
            immediate_size: 0,
            shader: shader.clone(),
            shader_defs: vec![],
            entry_point: Some(Cow::from("init")),
        });
        let update_pipeline = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
            zero_initialize_workgroup_memory: false,
            label: None,
            layout: vec![
                uniform_bind_group_layout.clone(),
                texture_bind_group_layout.clone(),
            ],
            immediate_size: 0,
            shader: shader.clone(),
            shader_defs: vec![],
            entry_point: Some(Cow::from("update")),
        });
        let ground_shadow_pipeline =
            pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
                zero_initialize_workgroup_memory: false,
                label: None,
                layout: vec![
                    uniform_bind_group_layout.clone(),
                    texture_bind_group_layout.clone(),
                ],
                immediate_size: 0,
                shader,
                shader_defs: vec![],
                entry_point: Some(Cow::from("ground_shadow")),
            });

        CloudsPipeline {
            texture_bind_group_layout,
            uniform_bind_group_layout,
            init_pipeline,
            update_pipeline,
            ground_shadow_pipeline,
        }
    }
}

#[derive(Resource, Default)]
enum CloudsState {
    #[default]
    Loading,
    Init,
    Update,
}

fn run_clouds_compute_pass(
    mut state: ResMut<CloudsState>,
    pipeline_cache: Res<PipelineCache>,
    pipeline: Res<CloudsPipeline>,
    textures: Option<Res<CloudsImageBindGroup>>,
    uniforms: Option<Res<CloudsUniformBindGroup>>,
    fields: Res<CloudsImage>,
    images: Res<RenderAssets<GpuImage>>,
    camera: Res<CameraMatrices>,
    config: Res<CloudsConfig>,
    mut history_state: ResMut<CloudsHistory>,
    mut context: RenderContext,
) {
    let (Some(textures), Some(uniforms)) = (textures, uniforms) else {
        return;
    };
    let (Some(output), Some(history)) = (
        images.get(&fields.cloud_render_image),
        images.get(&fields.history_image),
    ) else {
        return;
    };
    let initialize = matches!(*state, CloudsState::Loading);
    let id = if initialize {
        pipeline.init_pipeline
    } else {
        pipeline.update_pipeline
    };
    let Some(compute) = pipeline_cache.get_compute_pipeline(id) else {
        return;
    };
    let size = output.texture_descriptor.size;
    if !initialize {
        context.command_encoder().copy_texture_to_texture(
            output.texture.as_image_copy(),
            history.texture.as_image_copy(),
            size,
        );
    }
    let mut pass = context
        .command_encoder()
        .begin_compute_pass(&ComputePassDescriptor::default());
    pass.set_bind_group(0, &uniforms.0, &[]);
    pass.set_bind_group(1, &textures.0, &[]);
    pass.set_pipeline(compute);
    let (width, height) = if initialize {
        (IMAGE_SIZE, IMAGE_SIZE)
    } else {
        (size.width, size.height)
    };
    pass.dispatch_workgroups(
        width.div_ceil(WORKGROUP_SIZE),
        height.div_ceil(WORKGROUP_SIZE),
        1,
    );
    // The ground shadow map rides the same pass once the noise is baked.
    if !initialize
        && let Some(shadow) = pipeline_cache.get_compute_pipeline(pipeline.ground_shadow_pipeline)
    {
        pass.set_pipeline(shadow);
        let groups = GROUND_SHADOW_SIZE.div_ceil(WORKGROUP_SIZE);
        pass.dispatch_workgroups(groups, groups, 1);
    }
    if !initialize {
        history_state.0 = Some((*camera, *config, fields.cloud_render_image.id()));
    }
    *state = if initialize {
        CloudsState::Init
    } else {
        CloudsState::Update
    };
}

/// A plugin for the compute shader which renders clouds.
pub(crate) struct CloudsComputePlugin;

impl Plugin for CloudsComputePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractResourcePlugin::<CloudsImage>::default());
        app.add_plugins(ExtractResourcePlugin::<CloudsUniform>::default());

        let render_app = app.sub_app_mut(RenderApp);
        render_app
            .init_gpu_resource::<CloudsPipeline>()
            .init_resource::<CloudsUniformBuffer>()
            .init_resource::<CloudsHistory>()
            .init_resource::<CloudsState>();
        render_app.add_systems(
            Render,
            prepare_textures_bind_group.in_set(RenderSystems::PrepareResources),
        );
        render_app.add_systems(
            Render,
            prepare_uniforms_bind_group.in_set(RenderSystems::PrepareResources),
        );
        render_app.add_systems(
            RenderGraph,
            run_clouds_compute_pass.in_set(RenderGraphSystems::Begin),
        );

        render_app.add_systems(
            ExtractSchedule,
            (extract_clouds_config, extract_time, extract_camera_matrices),
        );
    }
}

fn extract_clouds_config(mut commands: Commands, config: Extract<Res<CloudsConfig>>) {
    commands.insert_resource(**config);
}

fn extract_time(mut commands: Commands, time: Extract<Res<Time<Real>>>) {
    commands.insert_resource(**time);
}

fn extract_camera_matrices(mut commands: Commands, camera: Extract<Res<CameraMatrices>>) {
    commands.insert_resource(**camera);
}
