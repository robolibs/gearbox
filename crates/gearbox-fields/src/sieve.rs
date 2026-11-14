//! Sieved chunks: per view, a compute pass runs a layer's cheapest tests once
//! per candidate and lists the ones that may show; the chunk then draws only
//! those, indirectly, through its usual vertex shader.

use super::render::{FieldBindGroups, NoVegetation, VegetationChunk, VegetationOffset, VegetationPipeline};
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::render::globals::{GlobalsBuffer, GlobalsUniform};
use bevy::render::mesh::allocator::MeshAllocator;
use bevy::render::mesh::{RenderMesh, RenderMeshBufferInfo};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{storage_buffer_sized, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::view::{ExtractedView, ViewUniform, ViewUniformOffset, ViewUniforms};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use std::num::NonZeroU64;

const WORKGROUP: u32 = 64;

/// Bytes of one chunk's indirect arguments as the sieve writes them.
const ARGS_BYTES: u64 = 20;

pub struct SievePlugin;

impl Plugin for SievePlugin {
    fn build(&self, app: &mut App) {
        app.sub_app_mut(RenderApp)
            .init_resource::<SieveJobs>()
            .init_resource::<SieveViews>()
            .add_systems(
                RenderStartup,
                init_sieve_pipelines.after(super::render::init_vegetation_pipeline),
            )
            .add_systems(
                Render,
                (prepare_sieve_jobs, prepare_sieve_views)
                    .chain()
                    .in_set(RenderSystems::PrepareBindGroups)
                    .after(super::render::prepare_vegetation_bind_group),
            )
            .add_systems(Core3d, sieve_chunks.before(Core3dSystems::MainPass));
    }
}

#[derive(ShaderType, Clone, Copy, Default)]
struct SieveDispatch {
    /// Candidates issued.
    counts: UVec4,
}

/// Where a sieved chunk's survivors and arguments sit in a view's buffers.
#[derive(Clone, Copy)]
pub(crate) struct SieveSlot {
    /// Byte offset of the chunk's survivor list.
    pub(crate) region: u32,
    /// Byte offset of the chunk's indirect arguments.
    pub(crate) args: u32,
}

struct SieveJob {
    shader: Handle<Shader>,
    field: (Entity, Option<AssetId<Image>>),
    field_offset: u32,
    dispatch_offset: u32,
    instances: u32,
    slot: SieveSlot,
    /// Index count, first index and base vertex of the chunk's mesh.
    mesh: [u32; 3],
}

#[derive(Resource, Default)]
struct SieveJobs {
    uniforms: DynamicUniformBuffer<SieveDispatch>,
    jobs: Vec<SieveJob>,
    /// Bytes every view's survivor buffer needs, and the widest one region.
    survivor_bytes: u64,
    widest: u64,
    args_bytes: u64,
}

struct SieveView {
    survivors: Buffer,
    args: Buffer,
    view_group: BindGroup,
    sieve_group: BindGroup,
    draw_group: BindGroup,
}

#[derive(Resource, Default)]
pub(crate) struct SieveViews {
    views: HashMap<Entity, SieveView>,
    slots: HashMap<Entity, SieveSlot>,
}

impl SieveViews {
    /// The view's survivor group and the chunk's slot in it.
    pub(crate) fn slot(&self, view: Entity, chunk: Entity) -> Option<(&BindGroup, SieveSlot)> {
        Some((&self.views.get(&view)?.draw_group, *self.slots.get(&chunk)?))
    }

    /// The view's arguments buffer and the chunk's offset in it.
    pub(crate) fn args(&self, view: Entity, chunk: Entity) -> Option<(&Buffer, u64)> {
        Some((&self.views.get(&view)?.args, self.slots.get(&chunk)?.args as u64))
    }
}

#[derive(Resource)]
struct SievePipelines {
    view_layout: BindGroupLayoutDescriptor,
    sieve_layout: BindGroupLayoutDescriptor,
    draw_layout: BindGroupLayoutDescriptor,
    empty_layout: BindGroupLayoutDescriptor,
    field_layout: BindGroupLayoutDescriptor,
    pipelines: HashMap<AssetId<Shader>, CachedComputePipelineId>,
}

fn init_sieve_pipelines(mut commands: Commands, vegetation: Res<VegetationPipeline>) {
    let view_layout = BindGroupLayoutDescriptor::new(
        "sieve view layout",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::COMPUTE,
            (
                (0, uniform_buffer::<ViewUniform>(true)),
                (11, uniform_buffer::<GlobalsUniform>(false)),
            ),
        ),
    );
    let sieve_layout = BindGroupLayoutDescriptor::new(
        "sieve layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                uniform_buffer::<SieveDispatch>(true),
                storage_buffer_sized(true, None),
                storage_buffer_sized(true, NonZeroU64::new(ARGS_BYTES)),
            ),
        ),
    );
    commands.insert_resource(SievePipelines {
        view_layout,
        sieve_layout,
        draw_layout: vegetation.sieve_layout.clone(),
        empty_layout: vegetation.empty_layout.clone(),
        field_layout: vegetation.field_layout.clone(),
        pipelines: HashMap::default(),
    });
}

/// The frame's sieved chunks, each given a survivor region as long as its
/// candidate list and its own arguments, both at storage offset alignment.
#[allow(clippy::too_many_arguments)]
fn prepare_sieve_jobs(
    chunks: Query<(Entity, &VegetationChunk, &VegetationOffset)>,
    jobs: ResMut<SieveJobs>,
    mut views: ResMut<SieveViews>,
    mut pipelines: ResMut<SievePipelines>,
    pipeline_cache: Res<PipelineCache>,
    meshes: Res<RenderAssets<RenderMesh>>,
    mesh_allocator: Res<MeshAllocator>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
) {
    let align = render_device.limits().min_storage_buffer_offset_alignment as u64;
    let SieveJobs { uniforms, jobs, survivor_bytes, widest, args_bytes } = jobs.into_inner();
    uniforms.clear();
    jobs.clear();
    views.slots.clear();
    let (mut survivors, mut args, mut wide) = (0u64, 0u64, 4u64);
    for (entity, chunk, offset) in &chunks {
        let Some(shader) = chunk.sieve.as_ref() else {
            continue;
        };
        if chunk.instances == 0 {
            continue;
        }
        let id = chunk.mesh.id();
        let (Some(mesh), Some(vertices), Some(indices)) = (
            meshes.get(id),
            mesh_allocator.mesh_vertex_slice(&id),
            mesh_allocator.mesh_index_slice(&id),
        ) else {
            continue;
        };
        let RenderMeshBufferInfo::Indexed { count, .. } = &mesh.buffer_info else {
            continue;
        };
        let layouts = [
            pipelines.view_layout.clone(),
            pipelines.sieve_layout.clone(),
            pipelines.empty_layout.clone(),
            pipelines.field_layout.clone(),
        ];
        pipelines.pipelines.entry(shader.id()).or_insert_with(|| {
            pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
                label: Some("vegetation sieve".into()),
                layout: layouts.to_vec(),
                shader: shader.clone(),
                entry_point: None,
                zero_initialize_workgroup_memory: false,
                ..default()
            })
        });
        let region_bytes = (chunk.instances as u64 * 4).next_multiple_of(align);
        let slot = SieveSlot { region: survivors as u32, args: args as u32 };
        survivors += region_bytes;
        args += ARGS_BYTES.next_multiple_of(align);
        wide = wide.max(region_bytes);
        let dispatch_offset = uniforms.push(&SieveDispatch {
            counts: UVec4::new(chunk.instances, 0, 0, 0),
        });
        views.slots.insert(entity, slot);
        jobs.push(SieveJob {
            shader: shader.clone(),
            field: (chunk.field_id, chunk.albedo.as_ref().map(Handle::id)),
            field_offset: offset.0,
            dispatch_offset,
            instances: chunk.instances,
            slot,
            mesh: [*count, indices.range.start, vertices.range.start],
        });
    }
    *survivor_bytes = survivors + wide;
    *widest = wide;
    *args_bytes = args.max(align);
    uniforms.write_buffer(&render_device, &render_queue);
}

/// Every vegetation view's survivor and argument buffers, grown to the frame's
/// jobs, the arguments reset to no instances, and the bind groups over them.
#[allow(clippy::too_many_arguments)]
fn prepare_sieve_views(
    views: Query<Entity, (With<ExtractedView>, With<ViewUniformOffset>, Without<NoVegetation>)>,
    jobs: Res<SieveJobs>,
    mut state: ResMut<SieveViews>,
    pipelines: Res<SievePipelines>,
    pipeline_cache: Res<PipelineCache>,
    view_uniforms: Res<ViewUniforms>,
    globals: Res<GlobalsBuffer>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
) {
    let live: HashSet<Entity> = views.iter().collect();
    state.views.retain(|view, _| live.contains(view));
    if jobs.jobs.is_empty() {
        return;
    }
    let (Some(view_binding), Some(globals_binding), Some(dispatch_binding)) = (
        view_uniforms.uniforms.binding(),
        globals.buffer.binding(),
        jobs.uniforms.binding(),
    ) else {
        return;
    };
    let mut reset = vec![0u32; (jobs.args_bytes / 4) as usize];
    for job in &jobs.jobs {
        let at = (job.slot.args / 4) as usize;
        reset[at..at + 5].copy_from_slice(&[job.mesh[0], 0, job.mesh[1], job.mesh[2], 0]);
    }
    for view in &views {
        let fits = state.views.get(&view).is_some_and(|known| {
            known.survivors.size() >= jobs.survivor_bytes && known.args.size() >= jobs.args_bytes
        });
        let (survivors, args) = match state.views.get(&view) {
            Some(known) if fits => (known.survivors.clone(), known.args.clone()),
            _ => (
                render_device.create_buffer(&BufferDescriptor {
                    label: Some("vegetation sieve survivors"),
                    size: (jobs.survivor_bytes * 3 / 2).next_multiple_of(4),
                    usage: BufferUsages::STORAGE,
                    mapped_at_creation: false,
                }),
                render_device.create_buffer(&BufferDescriptor {
                    label: Some("vegetation sieve arguments"),
                    size: jobs.args_bytes * 3 / 2 + 256,
                    usage: BufferUsages::STORAGE | BufferUsages::INDIRECT | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
            ),
        };
        render_queue.write_buffer(&args, 0, bytemuck::cast_slice(&reset));
        let region = BufferBinding { buffer: &survivors, offset: 0, size: NonZeroU64::new(jobs.widest) };
        let view_group = render_device.create_bind_group(
            "sieve view",
            &pipeline_cache.get_bind_group_layout(&pipelines.view_layout),
            &BindGroupEntries::with_indices(((0, view_binding.clone()), (11, globals_binding.clone()))),
        );
        let sieve_group = render_device.create_bind_group(
            "sieve",
            &pipeline_cache.get_bind_group_layout(&pipelines.sieve_layout),
            &BindGroupEntries::sequential((
                dispatch_binding.clone(),
                region.clone(),
                BufferBinding { buffer: &args, offset: 0, size: NonZeroU64::new(ARGS_BYTES) },
            )),
        );
        let draw_group = render_device.create_bind_group(
            "sieve survivors",
            &pipeline_cache.get_bind_group_layout(&pipelines.draw_layout),
            &BindGroupEntries::single(region),
        );
        state.views.insert(view, SieveView { survivors, args, view_group, sieve_group, draw_group });
    }
}

/// Sieves every chunk of the frame for the view being drawn.
fn sieve_chunks(
    view: ViewQuery<&ViewUniformOffset>,
    state: Res<SieveViews>,
    jobs: Res<SieveJobs>,
    pipelines: Res<SievePipelines>,
    pipeline_cache: Res<PipelineCache>,
    fields: Option<Res<FieldBindGroups>>,
    mut ctx: RenderContext,
) {
    let entity = view.entity();
    let view_offset = view.into_inner();
    let (Some(target), Some(fields)) = (state.views.get(&entity), fields) else {
        return;
    };
    if jobs.jobs.is_empty() {
        return;
    }
    let mut pass = ctx.command_encoder().begin_compute_pass(&ComputePassDescriptor {
        label: Some("vegetation sieve"),
        timestamp_writes: None,
    });
    pass.set_bind_group(0, &target.view_group, &[view_offset.offset]);
    pass.set_bind_group(2, &fields.empty, &[]);
    for job in &jobs.jobs {
        let (Some(pipeline), Some(field)) = (
            pipelines.pipelines.get(&job.shader.id()).and_then(|id| pipeline_cache.get_compute_pipeline(*id)),
            fields.fields.get(&job.field),
        ) else {
            continue;
        };
        pass.set_pipeline(pipeline);
        pass.set_bind_group(1, &target.sieve_group, &[job.dispatch_offset, job.slot.region, job.slot.args]);
        pass.set_bind_group(3, field, &[job.field_offset]);
        pass.dispatch_workgroups(job.instances.div_ceil(WORKGROUP), 1, 1);
    }
}
