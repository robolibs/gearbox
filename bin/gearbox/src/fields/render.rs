//! Shared instanced vegetation renderer and per-field wheel-map uploads.

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
use bevy::render::texture::{FallbackImage, GpuImage};
use bevy::render::view::ExtractedView;
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

use super::contacts::{WheelContacts, trample_texel};
use super::profile::WheelMapParams;
use bevy::platform::collections::HashMap;
use std::ops::Range;

/// Per-chunk draw: the chunk corner and how many blade instances this frame.
#[derive(Component, Clone)]
pub struct VegetationChunk {
    pub corner: Vec2,
    pub instances: u32,
    pub capacity: f32,
    pub field_id: Entity,
    pub shader: Handle<Shader>,
    pub fade_start: f32,
    pub fade_end: f32,
    pub inverse_square_thinning: bool,
    /// Albedo of an asset clump layer; procedural layers bind the fallback.
    pub albedo: Option<Handle<Image>>,
    /// Index ranges of a clump mesh's variants, one draw each; empty draws it whole.
    pub variants: Vec<Range<u32>>,
}

impl SyncComponent for VegetationChunk {
    type Target = Self;
}

impl ExtractComponent for VegetationChunk {
    type QueryData = &'static VegetationChunk;
    type QueryFilter = ();
    type Out = Self;

    fn extract_component(item: QueryItem<'_, '_, Self::QueryData>) -> Option<Self> {
        Some(item.clone())
    }
}

/// The terrain heightmap (RGBA32F: height, normal x, normal z), the trample
/// map wheels write into (RG16Uint: press time, roll direction), and the
/// numbers the shader needs to place blades on them.
#[derive(Clone)]
pub struct FieldGpu {
    pub heightmap: Handle<Image>,
    pub trample: Handle<Image>,
    pub params: VegetationParams,
    pub footprint_length: f32,
}

#[derive(Resource, ExtractResource, Clone, Default)]
pub struct RenderFields(pub HashMap<Entity, FieldGpu>);

#[derive(ShaderType, Clone, Copy, Debug, Default)]
pub struct VegetationParams {
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
    pub inverse_square_thinning: u32,
    pub bounds: Vec4,
    pub wheels: WheelMapParams,
}

pub struct VegetationPlugin;

impl Plugin for VegetationPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            ExtractComponentPlugin::<VegetationChunk>::default(),
            ExtractResourcePlugin::<RenderFields>::default(),
        ));
        app.sub_app_mut(RenderApp)
            .add_render_command::<Transparent3d, DrawVegetation>()
            .init_resource::<SpecializedMeshPipelines<VegetationPipeline>>()
            .add_systems(
                RenderStartup,
                init_vegetation_pipeline.after(MeshPipelineSystems),
            )
            .init_resource::<VegetationUniforms>()
            .add_systems(
                Render,
                (
                    queue_vegetation.in_set(RenderSystems::QueueMeshes),
                    prepare_vegetation_uniforms.in_set(RenderSystems::PrepareResources),
                    stamp_wheel_contacts.in_set(RenderSystems::PrepareResources),
                    prepare_vegetation_bind_group.in_set(RenderSystems::PrepareBindGroups),
                ),
            );
    }
}

#[allow(clippy::too_many_arguments)]
fn queue_vegetation(
    draw_functions: Res<DrawFunctions<Transparent3d>>,
    vegetation_pipeline: Res<VegetationPipeline>,
    mut pipelines: ResMut<SpecializedMeshPipelines<VegetationPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    meshes: Res<RenderAssets<RenderMesh>>,
    render_mesh_instances: Res<RenderMeshInstances>,
    batched: Option<Res<BatchedInstanceBuffers<MeshUniform, MeshInputUniform>>>,
    chunks: Query<(Entity, &MainEntity, &VegetationChunk)>,
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
    views: Query<&ExtractedView>,
    view_key_cache: Res<ViewKeyCache>,
    fields: Res<RenderFields>,
    mut warned: Local<bool>,
) {
    if fields.0.is_empty() {
        return;
    }
    let draw_vegetation = draw_functions.read().id::<DrawVegetation>();
    for view in &views {
        let Some(phase) = phases.get_mut(&view.retained_view_entity) else {
            continue;
        };
        let Some(&view_key) = view_key_cache.get(&view.retained_view_entity) else {
            continue;
        };
        for (entity, main_entity, draw) in &chunks {
            if draw.instances == 0 || !fields.0.contains_key(&draw.field_id) {
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
            let pipeline = match pipelines.specialize(
                &pipeline_cache,
                &vegetation_pipeline,
                (key, draw.shader.clone()),
                &mesh.layout,
            ) {
                Ok(pipeline) => pipeline,
                Err(error) => {
                    if !*warned {
                        warn!("vegetation pipeline for {:?}: {error}", draw.shader.path());
                        *warned = true;
                    }
                    continue;
                }
            };

            let mesh_center = pbr::get_mesh_instance_world_from_local(
                *main_entity,
                mesh_instance.current_uniform_index,
                &render_mesh_instances,
                batched.as_deref(),
            )
            .transform_point3(mesh.aabb_center);
            phase.add_retained(Transparent3d {
                sorting_info: TransparentSortingInfo3d::Sorted {
                    mesh_center,
                    depth_bias: 0.0,
                },
                entity: (entity, *main_entity),
                pipeline,
                draw_function: draw_vegetation,
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
struct VegetationUniforms(DynamicUniformBuffer<VegetationParams>);

#[derive(Component)]
struct VegetationOffset(u32);

fn prepare_vegetation_uniforms(
    mut commands: Commands,
    fields: Res<RenderFields>,
    chunks: Query<(Entity, &VegetationChunk)>,
    mut uniforms: ResMut<VegetationUniforms>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
) {
    uniforms.0.clear();
    for (entity, draw) in &chunks {
        let Some(field) = fields.0.get(&draw.field_id) else {
            continue;
        };
        let params = VegetationParams {
            corner: draw.corner,
            blades_per_chunk: draw.capacity,
            fade_start: draw.fade_start,
            fade_end: draw.fade_end,
            inverse_square_thinning: u32::from(draw.inverse_square_thinning),
            ..field.params
        };
        let offset = uniforms.0.push(&params);
        commands.entity(entity).insert(VegetationOffset(offset));
    }
    uniforms.0.write_buffer(&render_device, &render_queue);
}

#[derive(Resource)]
struct FieldBindGroups(HashMap<(Entity, Option<AssetId<Image>>), BindGroup>);

/// Field bindings rebuilt against the current dynamic uniform buffer, one per
/// field and albedo in use.
#[allow(clippy::too_many_arguments)]
fn prepare_vegetation_bind_group(
    mut commands: Commands,
    fields: Res<RenderFields>,
    chunks: Query<&VegetationChunk>,
    images: Res<RenderAssets<GpuImage>>,
    fallback: Res<FallbackImage>,
    pipeline: Res<VegetationPipeline>,
    pipeline_cache: Res<PipelineCache>,
    render_device: Res<RenderDevice>,
    uniforms: Res<VegetationUniforms>,
) {
    let mut groups = HashMap::default();
    if let Some(binding) = uniforms.0.binding() {
        for draw in &chunks {
            let key = (draw.field_id, draw.albedo.as_ref().map(Handle::id));
            if groups.contains_key(&key) {
                continue;
            }
            let Some(field) = fields.0.get(&draw.field_id) else {
                continue;
            };
            let albedo = match key.1 {
                Some(id) => images.get(id),
                None => Some(&fallback.d2),
            };
            let (Some(image), Some(trample), Some(albedo)) =
                (images.get(&field.heightmap), images.get(&field.trample), albedo)
            else {
                continue;
            };
            let group = render_device.create_bind_group(
                "field vegetation",
                &pipeline_cache.get_bind_group_layout(&pipeline.field_layout),
                &BindGroupEntries::sequential((
                    &image.texture_view,
                    &pipeline.sampler,
                    binding.clone(),
                    &trample.texture_view,
                    &albedo.texture_view,
                    &albedo.sampler,
                )),
            );
            groups.insert(key, group);
        }
    }
    commands.insert_resource(FieldBindGroups(groups));
}

/// Writes oriented wheel footprints with a timestamp and roll direction.
fn stamp_wheel_contacts(
    fields: Res<RenderFields>,
    contacts: Option<Res<WheelContacts>>,
    images: Res<RenderAssets<GpuImage>>,
    render_queue: Res<RenderQueue>,
    mut stamped: Local<bevy::platform::collections::HashSet<Entity>>,
) {
    stamped.retain(|entity| fields.0.contains_key(entity));
    let Some(contacts) = contacts else {
        return;
    };
    if contacts.contacts.is_empty() {
        return;
    }
    for (&entity, field) in &fields.0 {
        let Some(trample) = images.get(&field.trample) else {
            continue;
        };
        let tpm = field.params.wheels.texels_per_metre;
        let width = field.params.wheels.width as i32;
        let height = field.params.wheels.height as i32;
        for contact in &contacts.contacts {
            // The footprint is a rectangle in the wheel's own frame: tyre width
            // across the axle, a short contact patch along the roll. Texel
            // centres sit on integer coordinates; a texel is stamped only when
            // its centre lies inside that rectangle, row by row.
            let centre = Vec2::new(
                (contact.position.x - field.params.wheels.origin.x) * tpm,
                (contact.position.z - field.params.wheels.origin.y) * tpm,
            );
            let roll = contact.direction.normalize_or(Vec2::X);
            let axle = roll.perp();
            let half_width = (contact.width * 0.5 * tpm).max(0.5);
            let half_length = (field.footprint_length * 0.5 * tpm).max(0.5);
            let reach = half_width.hypot(half_length);
            let z0 = ((centre.y - reach).ceil() as i32).clamp(0, height - 1);
            let z1 = ((centre.y + reach).floor() as i32).clamp(0, height - 1);
            let x_lo = ((centre.x - reach).ceil() as i32).clamp(0, width - 1);
            let x_hi = ((centre.x + reach).floor() as i32).clamp(0, width - 1);
            let texel = trample_texel(contacts.now, roll);
            for z in z0..=z1 {
                let inside = |x: i32| {
                    let d = Vec2::new(x as f32, z as f32) - centre;
                    d.dot(axle).abs() <= half_width && d.dot(roll).abs() <= half_length
                };
                let Some(x0) = (x_lo..=x_hi).find(|x| inside(*x)) else {
                    continue;
                };
                let x1 = (x0..=x_hi).take_while(|x| inside(*x)).last().unwrap_or(x0);
                let width = (x1 - x0 + 1) as u32;
                let data: Vec<u16> = texel
                    .iter()
                    .copied()
                    .cycle()
                    .take((width * 2) as usize)
                    .collect();
                let mut target = trample.texture.as_image_copy();
                target.origin = Origin3d {
                    x: x0 as u32,
                    y: z as u32,
                    z: 0,
                };
                render_queue.write_texture(
                    target,
                    bytemuck::cast_slice(&data),
                    TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(width * 4),
                        rows_per_image: None,
                    },
                    Extent3d {
                        width,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                );
                if stamped.insert(entity) {
                    info!(
                        "field {entity}: first wheel stamp at {:?}, clock {:.2}, map origin {:?}",
                        contact.position, contacts.now, field.params.wheels.origin
                    );
                }
            }
        }
    }
}

#[derive(Resource)]
struct VegetationPipeline {
    mesh_pipeline: MeshPipeline,
    field_layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
}

fn init_vegetation_pipeline(
    mut commands: Commands,
    mesh_pipeline: Res<MeshPipeline>,
    render_device: Res<RenderDevice>,
) {
    let field_layout = BindGroupLayoutDescriptor::new(
        "vegetation field layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::VERTEX_FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: false }),
                sampler(SamplerBindingType::NonFiltering),
                uniform_buffer::<VegetationParams>(true),
                texture_2d(TextureSampleType::Uint),
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    );
    let sampler = render_device.create_sampler(&SamplerDescriptor {
        label: Some("vegetation heightmap sampler"),
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Nearest,
        min_filter: FilterMode::Nearest,
        ..default()
    });
    commands.insert_resource(VegetationPipeline {
        mesh_pipeline: mesh_pipeline.clone(),
        field_layout,
        sampler,
    });
}

impl SpecializedMeshPipeline for VegetationPipeline {
    type Key = (MeshPipelineKey, Handle<Shader>);

    fn specialize(
        &self,
        key: Self::Key,
        layout: &MeshVertexBufferLayoutRef,
    ) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let mut descriptor = self.mesh_pipeline.specialize(key.0, layout)?;
        descriptor.vertex.shader = key.1.clone();
        let fragment = descriptor.fragment.as_mut().unwrap();
        fragment.shader = key.1;
        fragment
            .shader_defs
            .push("STANDARD_MATERIAL_DIFFUSE_TRANSMISSION".into());
        fragment
            .shader_defs
            .push("STANDARD_MATERIAL_DIFFUSE_OR_SPECULAR_TRANSMISSION".into());
        descriptor.layout.push(self.field_layout.clone());
        descriptor.primitive.cull_mode = None;
        descriptor.multisample.alpha_to_coverage_enabled = descriptor.multisample.count > 1;
        Ok(descriptor)
    }
}

type DrawVegetation = (
    SetItemPipeline,
    SetMeshViewBindGroup<0>,
    SetMeshViewBindingArrayBindGroup<1>,
    SetMeshBindGroup<2>,
    SetFieldBindGroup<3>,
    DrawBlades,
);

struct SetFieldBindGroup<const I: usize>;

impl<P: PhaseItem, const I: usize> RenderCommand<P> for SetFieldBindGroup<I> {
    type Param = Option<SRes<FieldBindGroups>>;
    type ViewQuery = ();
    type ItemQuery = (Read<VegetationOffset>, Read<VegetationChunk>);

    #[inline]
    fn render<'w>(
        _item: &P,
        _view: (),
        offset: Option<(&'w VegetationOffset, &'w VegetationChunk)>,
        field: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let (Some(field), Some((offset, draw))) = (field, offset) else {
            return RenderCommandResult::Skip;
        };
        let key = (draw.field_id, draw.albedo.as_ref().map(Handle::id));
        let Some(group) = field.into_inner().0.get(&key) else {
            return RenderCommandResult::Skip;
        };
        pass.set_bind_group(I, group, &[offset.0]);
        RenderCommandResult::Success
    }
}

struct DrawBlades;

impl<P: PhaseItem> RenderCommand<P> for DrawBlades {
    type Param = (
        SRes<RenderAssets<RenderMesh>>,
        SRes<RenderMeshInstances>,
        SRes<MeshAllocator>,
    );
    type ViewQuery = ();
    type ItemQuery = Read<VegetationChunk>;

    #[inline]
    fn render<'w>(
        item: &P,
        _view: (),
        draw: Option<&'w VegetationChunk>,
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
            RenderMeshBufferInfo::Indexed {
                index_format,
                count,
            } => {
                let Some(index_slice) =
                    mesh_allocator.mesh_index_slice(&mesh_instance.mesh_asset_id())
                else {
                    return RenderCommandResult::Skip;
                };
                pass.set_index_buffer(index_slice.buffer.slice(..), *index_format);
                let start = index_slice.range.start;
                if draw.variants.is_empty() {
                    pass.draw_indexed(
                        start..(start + count),
                        vertex_slice.range.start as i32,
                        0..draw.instances,
                    );
                }
                // One draw per clump variant; the first instance tells the
                // shader the variant count and which variant this is.
                let variants = draw.variants.len() as u32;
                for (variant, range) in draw.variants.iter().enumerate() {
                    let variant = variant as u32;
                    let instances = (draw.instances + variants - 1 - variant) / variants;
                    let first = (variants << 28) | (variant << 24);
                    pass.draw_indexed(
                        (start + range.start)..(start + range.end),
                        vertex_slice.range.start as i32,
                        first..first + instances,
                    );
                }
            }
            RenderMeshBufferInfo::NonIndexed => {
                pass.draw(vertex_slice.range, 0..draw.instances);
            }
        }
        RenderCommandResult::Success
    }
}
