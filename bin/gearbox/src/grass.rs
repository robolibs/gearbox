//! GPU-generated grass. A chunk entity is one instanced draw of a blade
//! template; the vertex shader derives every blade from its instance index,
//! reads the terrain heightmap for its footing, and fades density with
//! distance so no blade data ever crosses the CPU.

use bevy::core_pipeline::core_3d::{Transparent3d, TransparentSortingInfo3d};
use bevy::ecs::query::QueryItem;
use bevy::ecs::system::{SystemParamItem, lifetimeless::*};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{
    self, MeshInputUniform, MeshPipeline, MeshPipelineKey, MeshPipelineSystems, MeshUniform,
    RenderMeshInstances, SetMeshBindGroup, SetMeshViewBindGroup, SetMeshViewBindingArrayBindGroup,
    ViewKeyCache,
};
use bevy::prelude::*;
use bevy::render::batching::gpu_preprocessing::BatchedInstanceBuffers;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::mesh::allocator::MeshAllocator;
use bevy::render::mesh::{RenderMesh, RenderMeshBufferInfo};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_phase::{
    AddRenderCommand, DrawFunctions, PhaseItem, PhaseItemExtraIndex, RenderCommand,
    RenderCommandResult, SetItemPipeline, TrackedRenderPass, ViewSortedRenderPhases,
};
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::sync_component::SyncComponent;
use bevy::render::sync_world::MainEntity;
use bevy::render::texture::GpuImage;
use bevy::render::view::ExtractedView;
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

const SHADER: &str = "embedded://gearbox_sim/../assets/shaders/grass_instanced.wgsl";

/// Per-chunk draw: the chunk corner and how many blade instances this frame.
#[derive(Component, Clone, Copy)]
pub struct GrassChunkDraw {
    pub corner: Vec2,
    pub instances: u32,
}

impl SyncComponent for GrassChunkDraw {
    type Target = Self;
}

impl ExtractComponent for GrassChunkDraw {
    type QueryData = &'static GrassChunkDraw;
    type QueryFilter = ();
    type Out = Self;

    fn extract_component(item: QueryItem<'_, '_, Self::QueryData>) -> Option<Self> {
        Some(*item)
    }
}

/// The terrain heightmap (RGBA32F: height, normal x, normal z), the trample
/// map wheels write into (RG16Uint: press time, roll direction), and the
/// numbers the shader needs to place blades on them.
#[derive(Resource, ExtractResource, Clone)]
pub struct GrassField {
    pub heightmap: Handle<Image>,
    pub trample: Handle<Image>,
    pub params: GrassParams,
}

/// One wheel on the ground this frame: where, which way it rolls, how wide.
#[derive(Clone, Copy, Debug)]
pub struct WheelContact {
    pub position: Vec3,
    pub direction: Vec2,
    pub width: f32,
}

/// Wheel contacts collected by the controllers during one frame, stamped
/// into the trample map by the render world.
#[derive(Resource, ExtractResource, Clone, Default)]
pub struct WheelContacts {
    pub contacts: Vec<WheelContact>,
    /// `Time::elapsed_secs_wrapped` this frame, the clock the stamps carry.
    pub now: f32,
}

/// Wheel contacts are stamped into a wrapped-time clock with this period;
/// it has to match `Time`'s wrap period.
pub const TRAMPLE_CLOCK_S: f32 = 3600.0;

/// Encodes a stamp: time on the wrapped clock and roll direction as an angle;
/// 0 in the angle channel means never stamped.
pub fn trample_texel(now: f32, direction: Vec2) -> [u16; 2] {
    let time = ((now.rem_euclid(TRAMPLE_CLOCK_S) / TRAMPLE_CLOCK_S) * 65535.0).round() as u16;
    let angle = (direction.y.atan2(direction.x) + std::f32::consts::PI) / std::f32::consts::TAU;
    let angle = ((angle * 65534.0).round() as u16).saturating_add(1);
    [time, angle]
}

fn begin_wheel_contacts(mut contacts: ResMut<WheelContacts>, time: Res<Time>) {
    contacts.contacts.clear();
    contacts.now = time.elapsed_secs_wrapped();
}

#[derive(ShaderType, Clone, Copy, Debug, Default)]
pub struct GrassParams {
    /// World XZ of the chunk corner; filled in per chunk.
    pub corner: Vec2,
    /// World XZ of texel (0, 0).
    pub origin: Vec2,
    /// Texels per metre.
    pub texels_per_metre: f32,
    /// Chunk side in metres; the chunk entity's translation is its corner.
    pub chunk_size: f32,
    /// Full density holds to `fade_start` and reaches zero at `fade_end`.
    pub fade_start: f32,
    pub fade_end: f32,
    /// Blades per chunk at full density.
    pub blades_per_chunk: f32,
    pub texel_count: f32,
    pub trample_texels_per_metre: f32,
    pub trample_texel_count: f32,
}

pub struct GrassPlugin;

impl Plugin for GrassPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "../assets/shaders/grass_instanced.wgsl");
        app.add_plugins((
            ExtractComponentPlugin::<GrassChunkDraw>::default(),
            ExtractResourcePlugin::<GrassField>::default(),
            ExtractResourcePlugin::<WheelContacts>::default(),
        ))
        .init_resource::<WheelContacts>()
        .add_systems(First, begin_wheel_contacts);
        app.sub_app_mut(RenderApp)
            .add_render_command::<Transparent3d, DrawGrass>()
            .init_resource::<SpecializedMeshPipelines<GrassPipeline>>()
            .add_systems(RenderStartup, init_grass_pipeline.after(MeshPipelineSystems))
            .init_resource::<GrassUniforms>()
            .add_systems(
                Render,
                (
                    queue_grass.in_set(RenderSystems::QueueMeshes),
                    prepare_grass_uniforms.in_set(RenderSystems::PrepareResources),
                    stamp_wheel_contacts.in_set(RenderSystems::PrepareResources),
                    prepare_grass_bind_group.in_set(RenderSystems::PrepareBindGroups),
                ),
            );
    }
}

#[allow(clippy::too_many_arguments)]
fn queue_grass(
    draw_functions: Res<DrawFunctions<Transparent3d>>,
    grass_pipeline: Res<GrassPipeline>,
    mut pipelines: ResMut<SpecializedMeshPipelines<GrassPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    meshes: Res<RenderAssets<RenderMesh>>,
    render_mesh_instances: Res<RenderMeshInstances>,
    batched: Option<Res<BatchedInstanceBuffers<MeshUniform, MeshInputUniform>>>,
    chunks: Query<(Entity, &MainEntity, &GrassChunkDraw)>,
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
    views: Query<&ExtractedView>,
    view_key_cache: Res<ViewKeyCache>,
    field: Option<Res<GrassField>>,
) {
    if field.is_none() {
        return;
    }
    let draw_grass = draw_functions.read().id::<DrawGrass>();
    for view in &views {
        let Some(phase) = phases.get_mut(&view.retained_view_entity) else {
            continue;
        };
        let Some(&view_key) = view_key_cache.get(&view.retained_view_entity) else {
            continue;
        };
        for (entity, main_entity, draw) in &chunks {
            if draw.instances == 0 {
                continue;
            }
            let Some(mesh_instance) = render_mesh_instances.render_mesh_queue_data(*main_entity)
            else {
                continue;
            };
            let Some(mesh) = meshes.get(mesh_instance.mesh_asset_id()) else {
                continue;
            };
            let key = view_key
                | MeshPipelineKey::from_primitive_topology_and_strip_index(
                    mesh.primitive_topology(),
                    mesh.index_format(),
                );
            let Ok(pipeline) =
                pipelines.specialize(&pipeline_cache, &grass_pipeline, key, &mesh.layout)
            else {
                continue;
            };
            let mesh_center = pbr::get_mesh_instance_world_from_local(
                *main_entity,
                mesh_instance.current_uniform_index,
                &render_mesh_instances,
                batched.as_deref(),
            )
            .transform_point3(mesh.aabb_center);
            phase.add_retained(Transparent3d {
                sorting_info: TransparentSortingInfo3d::Sorted { mesh_center, depth_bias: 0.0 },
                entity: (entity, *main_entity),
                pipeline,
                draw_function: draw_grass,
                distance: 0.0,
                batch_range: 0..1,
                extra_index: PhaseItemExtraIndex::None,
                indexed: true,
            });
        }
    }
}

/// One uniform slot per chunk, rewritten every frame.
#[derive(Resource, Default)]
struct GrassUniforms(DynamicUniformBuffer<GrassParams>);

#[derive(Component)]
struct GrassChunkOffset(u32);

fn prepare_grass_uniforms(
    mut commands: Commands,
    field: Option<Res<GrassField>>,
    chunks: Query<(Entity, &GrassChunkDraw)>,
    mut uniforms: ResMut<GrassUniforms>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
) {
    let Some(field) = field else {
        return;
    };
    uniforms.0.clear();
    for (entity, draw) in &chunks {
        let params = GrassParams { corner: draw.corner, ..field.params };
        let offset = uniforms.0.push(&params);
        commands.entity(entity).insert(GrassChunkOffset(offset));
    }
    uniforms.0.write_buffer(&render_device, &render_queue);
}

#[derive(Resource)]
struct GrassBindGroup(BindGroup);

/// Rebuilt every frame: the uniform buffer may have grown.
fn prepare_grass_bind_group(
    mut commands: Commands,
    field: Option<Res<GrassField>>,
    images: Res<RenderAssets<GpuImage>>,
    pipeline: Res<GrassPipeline>,
    pipeline_cache: Res<PipelineCache>,
    render_device: Res<RenderDevice>,
    uniforms: Res<GrassUniforms>,
) {
    let Some(field) = field else {
        commands.remove_resource::<GrassBindGroup>();
        return;
    };
    let (Some(image), Some(trample), Some(binding)) = (
        images.get(&field.heightmap),
        images.get(&field.trample),
        uniforms.0.binding(),
    ) else {
        commands.remove_resource::<GrassBindGroup>();
        return;
    };
    let bind_group = render_device.create_bind_group(
        "grass field",
        &pipeline_cache.get_bind_group_layout(&pipeline.field_layout),
        &BindGroupEntries::sequential((
            &image.texture_view,
            &pipeline.sampler,
            binding,
            &trample.texture_view,
        )),
    );
    commands.insert_resource(GrassBindGroup(bind_group));
}

/// Writes each wheel's footprint into the trample map: a small square of
/// texels carrying this frame's clock and the roll direction.
fn stamp_wheel_contacts(
    field: Option<Res<GrassField>>,
    contacts: Option<Res<WheelContacts>>,
    images: Res<RenderAssets<GpuImage>>,
    render_queue: Res<RenderQueue>,
) {
    let (Some(field), Some(contacts)) = (field, contacts) else {
        return;
    };
    if contacts.contacts.is_empty() {
        return;
    }
    let Some(trample) = images.get(&field.trample) else {
        return;
    };
    let tpm = field.params.trample_texels_per_metre;
    let count = field.params.trample_texel_count as i32;
    for contact in &contacts.contacts {
        let centre_x = ((contact.position.x - field.params.origin.x) * tpm).round() as i32;
        let centre_z = ((contact.position.z - field.params.origin.y) * tpm).round() as i32;
        let half = ((contact.width * 0.5 * tpm).ceil() as i32).max(1);
        let x0 = (centre_x - half).clamp(0, count - 1);
        let x1 = (centre_x + half).clamp(0, count - 1);
        let z0 = (centre_z - half).clamp(0, count - 1);
        let z1 = (centre_z + half).clamp(0, count - 1);
        if x1 < x0 || z1 < z0 {
            continue;
        }
        let (width, height) = ((x1 - x0 + 1) as u32, (z1 - z0 + 1) as u32);
        let texel = trample_texel(contacts.now, contact.direction);
        let data: Vec<u16> = texel.iter().copied().cycle().take((width * height * 2) as usize).collect();
        let mut target = trample.texture.as_image_copy();
        target.origin = Origin3d { x: x0 as u32, y: z0 as u32, z: 0 };
        render_queue.write_texture(
            target,
            bytemuck::cast_slice(&data),
            TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 4), rows_per_image: None },
            Extent3d { width, height, depth_or_array_layers: 1 },
        );
    }
}

#[derive(Resource)]
struct GrassPipeline {
    shader: Handle<Shader>,
    mesh_pipeline: MeshPipeline,
    field_layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
}

fn init_grass_pipeline(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mesh_pipeline: Res<MeshPipeline>,
    render_device: Res<RenderDevice>,
) {
    let field_layout = BindGroupLayoutDescriptor::new(
        "grass field layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::VERTEX_FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: false }),
                sampler(SamplerBindingType::NonFiltering),
                uniform_buffer::<GrassParams>(true),
                texture_2d(TextureSampleType::Uint),
            ),
        ),
    );
    let sampler = render_device.create_sampler(&SamplerDescriptor {
        label: Some("grass heightmap sampler"),
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Nearest,
        min_filter: FilterMode::Nearest,
        ..default()
    });
    commands.insert_resource(GrassPipeline {
        shader: asset_server.load(SHADER),
        mesh_pipeline: mesh_pipeline.clone(),
        field_layout,
        sampler,
    });
}

impl SpecializedMeshPipeline for GrassPipeline {
    type Key = MeshPipelineKey;

    fn specialize(
        &self,
        key: Self::Key,
        layout: &MeshVertexBufferLayoutRef,
    ) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let mut descriptor = self.mesh_pipeline.specialize(key, layout)?;
        descriptor.vertex.shader = self.shader.clone();
        descriptor.fragment.as_mut().unwrap().shader = self.shader.clone();
        descriptor.layout.push(self.field_layout.clone());
        descriptor.primitive.cull_mode = None;
        Ok(descriptor)
    }
}

type DrawGrass = (
    SetItemPipeline,
    SetMeshViewBindGroup<0>,
    SetMeshViewBindingArrayBindGroup<1>,
    SetMeshBindGroup<2>,
    SetGrassFieldBindGroup<3>,
    DrawBlades,
);

struct SetGrassFieldBindGroup<const I: usize>;

impl<P: PhaseItem, const I: usize> RenderCommand<P> for SetGrassFieldBindGroup<I> {
    type Param = Option<SRes<GrassBindGroup>>;
    type ViewQuery = ();
    type ItemQuery = Read<GrassChunkOffset>;

    #[inline]
    fn render<'w>(
        _item: &P,
        _view: (),
        offset: Option<&'w GrassChunkOffset>,
        field: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let (Some(field), Some(offset)) = (field, offset) else {
            return RenderCommandResult::Skip;
        };
        pass.set_bind_group(I, &field.into_inner().0, &[offset.0]);
        RenderCommandResult::Success
    }
}

struct DrawBlades;

impl<P: PhaseItem> RenderCommand<P> for DrawBlades {
    type Param = (SRes<RenderAssets<RenderMesh>>, SRes<RenderMeshInstances>, SRes<MeshAllocator>);
    type ViewQuery = ();
    type ItemQuery = Read<GrassChunkDraw>;

    #[inline]
    fn render<'w>(
        item: &P,
        _view: (),
        draw: Option<&'w GrassChunkDraw>,
        (meshes, render_mesh_instances, mesh_allocator): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let mesh_allocator = mesh_allocator.into_inner();
        let Some(draw) = draw else {
            return RenderCommandResult::Skip;
        };
        let Some(mesh_instance) = render_mesh_instances.render_mesh_queue_data(item.main_entity())
        else {
            return RenderCommandResult::Skip;
        };
        let Some(gpu_mesh) = meshes.into_inner().get(mesh_instance.mesh_asset_id()) else {
            return RenderCommandResult::Skip;
        };
        let Some(vertex_slice) = mesh_allocator.mesh_vertex_slice(&mesh_instance.mesh_asset_id())
        else {
            return RenderCommandResult::Skip;
        };
        pass.set_vertex_buffer(0, vertex_slice.buffer.slice(..));
        match &gpu_mesh.buffer_info {
            RenderMeshBufferInfo::Indexed { index_format, count } => {
                let Some(index_slice) =
                    mesh_allocator.mesh_index_slice(&mesh_instance.mesh_asset_id())
                else {
                    return RenderCommandResult::Skip;
                };
                pass.set_index_buffer(index_slice.buffer.slice(..), *index_format);
                pass.draw_indexed(
                    index_slice.range.start..(index_slice.range.start + count),
                    vertex_slice.range.start as i32,
                    0..draw.instances,
                );
            }
            RenderMeshBufferInfo::NonIndexed => {
                pass.draw(vertex_slice.range, 0..draw.instances);
            }
        }
        RenderCommandResult::Success
    }
}
