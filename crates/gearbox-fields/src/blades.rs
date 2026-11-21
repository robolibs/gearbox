//! Blades sown on the GPU. Per view, a compute pass visits every candidate
//! root of the blade layers' chunks, culls and shapes each blade once, and
//! appends a record; one indirect draw per kind then bends a strip from each
//! record. The per-chunk draw path only ever sees the other layers.

use super::profile::{BladeKind, FieldProfiles};
use super::render::{
    FieldBindGroups, NoVegetation, OPAQUE_FIRST, SetEmptyBindGroup, VegetationChunk,
    VegetationOffset, VegetationPipeline, chunk_in_view, view_frustum,
};
use bevy::core_pipeline::core_3d::{Transparent3d, TransparentSortingInfo3d};
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::ecs::query::QueryItem;
use bevy::ecs::system::{SystemParamItem, lifetimeless::*};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{
    MeshPipeline, MeshPipelineKey, SetMeshViewBindGroup, SetMeshViewBindingArrayBindGroup,
    ViewKeyCache,
};
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::globals::{GlobalsBuffer, GlobalsUniform};
use bevy::render::mesh::allocator::MeshAllocator;
use bevy::render::mesh::{RenderMesh, RenderMeshBufferInfo};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_phase::{
    AddRenderCommand, DrawFunctions, PhaseItem, PhaseItemExtraIndex, RenderCommand,
    RenderCommandResult, SetItemPipeline, TrackedRenderPass, ViewSortedRenderPhases,
};
use bevy::render::render_resource::binding_types::{
    storage_buffer_read_only_sized, storage_buffer_sized, uniform_buffer,
};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::sync_component::SyncComponent;
use bevy::render::sync_world::MainEntity;
use bevy::render::view::{
    ExtractedView, RetainedViewEntity, ViewUniform, ViewUniformOffset, ViewUniforms,
};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

const WORKGROUP: u32 = 64;

/// Words of the indirect arguments buffer: draw arguments, the append
/// counter, padding.
const ARGS_WORDS: usize = 8;

/// Frames an inactive view's GPU blade buffers stay warm before eviction, so
/// periodic sensor cameras (e.g. 5-30 Hz) reuse their buffers instead of
/// reallocating tens of megabytes every sample.
const VIEW_BUFFER_TTL_FRAMES: u32 = 180;

/// Scales a blade kind's record buffer capacity for small viewports (such as
/// 320x240 sensor cameras), where pixel-width thinning keeps blade counts far
/// below a full-HD viewer's ceiling.
fn blade_capacity_for_viewport(base: u32, viewport: UVec4) -> u32 {
    let pixels = viewport.z as f64 * viewport.w as f64;
    let full_hd = 1920.0 * 1080.0;
    if pixels <= 0.0 || pixels >= full_hd {
        return base;
    }
    let scale = (pixels / full_hd).sqrt().clamp(0.15, 1.0);
    ((base as f64 * scale).round() as u32).clamp(16_384, base)
}

/// The draw every view makes of one blade kind's records.
#[derive(Component, Clone)]
pub struct BladeDraw {
    pub kind: &'static BladeKind,
    pub template: Handle<Mesh>,
    pub cull: Handle<Shader>,
    pub draw: Handle<Shader>,
    /// Writes depth only, ahead of the lit draw.
    pub depth_only: bool,
}

/// `GEARBOX_GRASS_PREPASS=1`: blades lay their depth first and are lit only
/// where they are the nearest surface.
fn blade_prepass() -> bool {
    std::env::var("GEARBOX_GRASS_PREPASS").is_ok_and(|value| value == "1")
}

/// Which of the blade draws a pipeline serves.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BladePass {
    Lit,
    Depth,
    LitAfterDepth,
}

impl SyncComponent<bevy::render::RenderApp> for BladeDraw {
    type Target = Self;
}

impl ExtractComponent<bevy::render::RenderApp> for BladeDraw {
    type QueryData = &'static BladeDraw;
    type QueryFilter = ();
    type Out = Self;

    fn extract_component(item: QueryItem<'_, '_, Self::QueryData>) -> Option<Self> {
        Some(item.clone())
    }
}

pub struct BladePlugin;

impl Plugin for BladePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractComponentPlugin::<BladeDraw>::default())
            .add_systems(Update, spawn_blade_draws);
        app.sub_app_mut(RenderApp)
            .add_render_command::<Transparent3d, DrawSownBlades>()
            .init_resource::<SpecializedMeshPipelines<BladePipelines>>()
            .init_resource::<BladeJobs>()
            .init_resource::<BladeViews>()
            .add_systems(
                RenderStartup,
                init_blade_pipelines.after(super::render::init_vegetation_pipeline),
            )
            .add_systems(
                Render,
                (
                    queue_blades.in_set(RenderSystems::QueueMeshes),
                    (prepare_blade_jobs, prepare_blade_views)
                        .chain()
                        .in_set(RenderSystems::PrepareBindGroups)
                        .after(super::render::prepare_vegetation_bind_group),
                ),
            )
            .add_systems(Core3d, sow_blades.before(Core3dSystems::MainPass));
    }
}

/// One draw entity per blade kind any profile uses.
fn spawn_blade_draws(
    mut commands: Commands,
    profiles: Res<FieldProfiles>,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut spawned: Local<HashSet<&'static str>>,
) {
    for profile in profiles.0.values() {
        for layer in &profile.layers {
            let Some(blade) = layer.blade else {
                continue;
            };
            if !spawned.insert(blade.kind.cull) {
                continue;
            }
            let draw = BladeDraw {
                kind: blade.kind,
                template: meshes.add((blade.kind.template)()),
                cull: assets.load(blade.kind.cull),
                draw: assets.load(blade.kind.draw),
                depth_only: false,
            };
            if blade_prepass() {
                commands.spawn((
                    Name::new(format!("Blade depth {}", blade.kind.draw)),
                    BladeDraw { depth_only: true, ..draw.clone() },
                ));
            }
            commands.spawn((Name::new(format!("Blade draw {}", blade.kind.draw)), draw));
        }
    }
}

/// Per-dispatch parameters: the chunk's band, and counts.
#[derive(ShaderType, Clone, Copy, Default)]
struct BladeDispatch {
    /// Distance band (near, far) of the layer.
    band: Vec4,
    /// Instances issued, blades per instance, segments, record capacity.
    counts: UVec4,
}

/// One chunk's dispatch this frame.
struct BladeJob {
    kind: &'static str,
    field: (Entity, Option<AssetId<Image>>),
    field_offset: u32,
    dispatch_offset: u32,
    instances: u32,
    corner: Vec2,
    size: f32,
    height_span: Vec2,
    fade_end: f32,
    band: Vec2,
    centre: Vec3,
}

#[derive(Resource, Default)]
struct BladeJobs {
    uniforms: DynamicUniformBuffer<BladeDispatch>,
    jobs: Vec<BladeJob>,
}

/// A kind's arguments and bindings in one view.
struct ViewKind {
    args: Buffer,
    sow_group: BindGroup,
    draw_group: BindGroup,
}

struct ViewBlades {
    view_group: BindGroup,
    kinds: HashMap<&'static str, ViewKind>,
    /// Records, arguments and capacity per kind, kept warm across frames.
    buffers: HashMap<&'static str, (Buffer, Buffer, u32)>,
    idle_frames: u32,
}

#[derive(Resource, Default)]
struct BladeViews(HashMap<RetainedViewEntity, ViewBlades>);

#[derive(Resource)]
struct BladePipelines {
    view_layout: BindGroupLayoutDescriptor,
    sow_layout: BindGroupLayoutDescriptor,
    draw_layout: BindGroupLayoutDescriptor,
    empty_layout: BindGroupLayoutDescriptor,
    field_layout: BindGroupLayoutDescriptor,
    mesh_pipeline: MeshPipeline,
    /// Cull and finish pipelines per kind.
    sow: HashMap<&'static str, (CachedComputePipelineId, CachedComputePipelineId)>,
}

fn init_blade_pipelines(
    mut commands: Commands,
    mesh_pipeline: Res<MeshPipeline>,
    vegetation: Res<VegetationPipeline>,
) {
    let view_layout = BindGroupLayoutDescriptor::new(
        "blade view layout",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::COMPUTE,
            (
                (0, uniform_buffer::<ViewUniform>(true)),
                (11, uniform_buffer::<GlobalsUniform>(false)),
            ),
        ),
    );
    let sow_layout = BindGroupLayoutDescriptor::new(
        "blade sow layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                uniform_buffer::<BladeDispatch>(true),
                storage_buffer_sized(false, None),
                storage_buffer_sized(false, None),
            ),
        ),
    );
    let draw_layout = BindGroupLayoutDescriptor::new(
        "blade draw layout",
        &BindGroupLayoutEntries::single(ShaderStages::VERTEX, storage_buffer_read_only_sized(false, None)),
    );
    commands.insert_resource(BladePipelines {
        view_layout,
        sow_layout,
        draw_layout,
        empty_layout: vegetation.empty_layout.clone(),
        field_layout: vegetation.field_layout.clone(),
        mesh_pipeline: mesh_pipeline.clone(),
        sow: HashMap::default(),
    });
}

/// The frame's dispatches: every blade chunk issuing instances, with its
/// field bindings and parameters.
fn prepare_blade_jobs(
    chunks: Query<(&VegetationChunk, &VegetationOffset)>,
    jobs: ResMut<BladeJobs>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
) {
    let BladeJobs { uniforms, jobs } = jobs.into_inner();
    uniforms.clear();
    jobs.clear();
    for (chunk, offset) in &chunks {
        let Some(blade) = chunk.blade else {
            continue;
        };
        if chunk.instances == 0 {
            continue;
        }
        let dispatch_offset = uniforms.push(&BladeDispatch {
            band: chunk.band.extend(0.0).extend(0.0),
            counts: UVec4::new(chunk.instances, blade.twins, blade.segments, blade.kind.capacity),
        });
        let half = chunk.size * 0.5;
        jobs.push(BladeJob {
            kind: blade.kind.cull,
            field: (chunk.field_id, chunk.albedo.as_ref().map(Handle::id)),
            field_offset: offset.0,
            dispatch_offset,
            instances: chunk.instances,
            corner: chunk.corner,
            size: chunk.size,
            height_span: chunk.height_span,
            fade_end: chunk.fade_end,
            band: chunk.band,
            centre: Vec3::new(chunk.corner.x + half, 0.0, chunk.corner.y + half),
        });
    }
    uniforms.write_buffer(&render_device, &render_queue);
}

/// Buffers, bind groups and the frame's reset arguments for every view that
/// draws vegetation, and the sow pipelines of every kind.
#[allow(clippy::too_many_arguments)]
fn prepare_blade_views(
    views: Query<
        &ExtractedView,
        (With<ExtractedCamera>, With<ViewUniformOffset>, Without<NoVegetation>),
    >,
    phases: Res<ViewSortedRenderPhases<Transparent3d>>,
    draws: Query<&BladeDraw>,
    jobs: Res<BladeJobs>,
    mut state: ResMut<BladeViews>,
    mut pipelines: ResMut<BladePipelines>,
    pipeline_cache: Res<PipelineCache>,
    view_uniforms: Res<ViewUniforms>,
    globals: Res<GlobalsBuffer>,
    meshes: Res<RenderAssets<RenderMesh>>,
    mesh_allocator: Res<MeshAllocator>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
) {
    for draw in &draws {
        let layouts = [
            pipelines.view_layout.clone(),
            pipelines.sow_layout.clone(),
            pipelines.empty_layout.clone(),
            pipelines.field_layout.clone(),
        ];
        pipelines.sow.entry(draw.kind.cull).or_insert_with(|| {
            let pipeline = |entry: &'static str| {
                pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
                    label: Some(format!("blade {entry}").into()),
                    layout: layouts.to_vec(),
                    shader: draw.cull.clone(),
                    entry_point: Some(entry.into()),
                    zero_initialize_workgroup_memory: false,
                    ..default()
                })
            };
            (pipeline("sow_blades"), pipeline("finish"))
        });
    }
    let live: HashSet<RetainedViewEntity> = views
        .iter()
        .map(|extracted| extracted.retained_view_entity)
        .filter(|id| phases.contains_key(id))
        .collect();
    state.0.retain(|view, blades| {
        if live.contains(view) {
            blades.idle_frames = 0;
            true
        } else {
            blades.idle_frames += 1;
            blades.idle_frames <= VIEW_BUFFER_TTL_FRAMES
        }
    });
    // Without this frame's jobs nothing is sown, so nothing may be drawn from
    // the records of an earlier frame.
    for blades in state.0.values_mut() {
        blades.kinds.clear();
    }
    let (Some(view_binding), Some(globals_binding), Some(dispatch_binding)) = (
        view_uniforms.uniforms.binding(),
        globals.buffer.binding(),
        jobs.uniforms.binding(),
    ) else {
        return;
    };
    if jobs.jobs.is_empty() {
        return;
    }
    for extracted in &views {
        let view_id = extracted.retained_view_entity;
        if !phases.contains_key(&view_id) {
            continue;
        }
        let view_group = render_device.create_bind_group(
            "blade view",
            &pipeline_cache.get_bind_group_layout(&pipelines.view_layout),
            &BindGroupEntries::with_indices(((0, view_binding.clone()), (11, globals_binding.clone()))),
        );
        let blades = state.0.entry(view_id).or_insert_with(|| ViewBlades {
            view_group: view_group.clone(),
            kinds: HashMap::default(),
            buffers: HashMap::default(),
            idle_frames: 0,
        });
        blades.view_group = view_group;
        blades.idle_frames = 0;
        for draw in &draws {
            let (Some(mesh), Some(vertices), Some(indices)) = (
                meshes.get(draw.template.id()),
                mesh_allocator.mesh_vertex_slice(&draw.template.id()),
                mesh_allocator.mesh_index_slice(&draw.template.id()),
            ) else {
                continue;
            };
            let RenderMeshBufferInfo::Indexed { count, .. } = &mesh.buffer_info else {
                continue;
            };
            let wanted_cap = blade_capacity_for_viewport(draw.kind.capacity, extracted.viewport);
            let needs_alloc = blades
                .buffers
                .get(draw.kind.cull)
                .is_none_or(|(_, _, cap)| *cap < wanted_cap);
            if needs_alloc {
                let records = render_device.create_buffer(&BufferDescriptor {
                    label: Some("blade records"),
                    size: wanted_cap as u64 * draw.kind.stride as u64 * 4,
                    usage: BufferUsages::STORAGE,
                    mapped_at_creation: false,
                });
                let args = render_device.create_buffer(&BufferDescriptor {
                    label: Some("blade arguments"),
                    size: (ARGS_WORDS * 4) as u64,
                    usage: BufferUsages::STORAGE | BufferUsages::INDIRECT | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                blades.buffers.insert(draw.kind.cull, (records, args, wanted_cap));
            }
            let (records, args, _) = blades.buffers[draw.kind.cull].clone();
            let reset: [u32; ARGS_WORDS] = [
                *count,
                0,
                indices.range.start,
                vertices.range.start,
                0,
                0,
                0,
                0,
            ];
            render_queue.write_buffer(&args, 0, bytemuck::cast_slice(&reset));
            let sow_group = render_device.create_bind_group(
                "blade sow",
                &pipeline_cache.get_bind_group_layout(&pipelines.sow_layout),
                &BindGroupEntries::sequential((
                    dispatch_binding.clone(),
                    records.as_entire_binding(),
                    args.as_entire_binding(),
                )),
            );
            let draw_group = render_device.create_bind_group(
                "blade draw",
                &pipeline_cache.get_bind_group_layout(&pipelines.draw_layout),
                &BindGroupEntries::single(records.as_entire_binding()),
            );
            blades.kinds.insert(draw.kind.cull, ViewKind { args, sow_group, draw_group });
        }
    }
}

/// Sows every blade kind for the view being drawn, nearest chunk first, then
/// clamps each kind's count into its draw arguments.
#[allow(clippy::too_many_arguments)]
fn sow_blades(
    view: ViewQuery<(&ExtractedView, &ViewUniformOffset)>,
    state: Res<BladeViews>,
    jobs: Res<BladeJobs>,
    pipelines: Res<BladePipelines>,
    pipeline_cache: Res<PipelineCache>,
    fields: Option<Res<FieldBindGroups>>,
    mut ctx: RenderContext,
) {
    let (extracted, view_offset) = view.into_inner();
    let (Some(blades), Some(fields)) = (state.0.get(&extracted.retained_view_entity), fields) else {
        return;
    };
    if jobs.jobs.is_empty() {
        return;
    }
    let eye = extracted.world_from_view.translation();
    let frustum = view_frustum(extracted);
    let mut order: Vec<&BladeJob> = jobs
        .jobs
        .iter()
        .filter(|job| {
            chunk_in_view(
                job.corner,
                job.size,
                job.height_span,
                job.fade_end,
                Some(job.band),
                extracted,
                &frustum,
            )
        })
        .collect();
    if order.is_empty() {
        return;
    }
    order.sort_by(|a, b| a.centre.distance_squared(eye).total_cmp(&b.centre.distance_squared(eye)));

    let mut pass = ctx.command_encoder().begin_compute_pass(&ComputePassDescriptor {
        label: Some("blade sowing"),
        timestamp_writes: None,
    });
    pass.set_bind_group(0, &blades.view_group, &[view_offset.offset]);
    pass.set_bind_group(2, &fields.empty, &[]);
    for (kind, target) in &blades.kinds {
        let Some(&(sow, finish)) = pipelines.sow.get(kind) else {
            continue;
        };
        let (Some(sow), Some(finish)) =
            (pipeline_cache.get_compute_pipeline(sow), pipeline_cache.get_compute_pipeline(finish))
        else {
            continue;
        };
        let mut last = None;
        for job in order.iter().filter(|job| job.kind == *kind) {
            let Some(field) = fields.fields.get(&job.field) else {
                continue;
            };
            if last.is_none() {
                pass.set_pipeline(sow);
            }
            pass.set_bind_group(1, &target.sow_group, &[job.dispatch_offset]);
            pass.set_bind_group(3, field, &[job.field_offset]);
            pass.dispatch_workgroups(job.instances.div_ceil(WORKGROUP), 1, 1);
            last = Some(job.dispatch_offset);
        }
        if let Some(dispatch_offset) = last {
            pass.set_pipeline(finish);
            pass.set_bind_group(1, &target.sow_group, &[dispatch_offset]);
            pass.dispatch_workgroups(1, 1, 1);
        }
    }
}

/// Draws a kind's records with its draw shader, opaque, after the mesh
/// pipeline's view layouts.
impl SpecializedMeshPipeline for BladePipelines {
    type Key = (MeshPipelineKey, Handle<Shader>, BladePass);

    fn specialize(
        &self,
        (mesh_key, shader, pass): Self::Key,
        layout: &MeshVertexBufferLayoutRef,
    ) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let mut descriptor = self.mesh_pipeline.specialize(mesh_key, layout)?;
        descriptor.vertex.shader = shader.clone();
        let fragment = descriptor.fragment.as_mut().unwrap();
        fragment.shader = shader;
        fragment.entry_point = Some("fragment".into());
        super::render::back_light_defs(&mut fragment.shader_defs);
        match pass {
            BladePass::Lit => {}
            BladePass::Depth => {
                fragment.entry_point = Some("depth_only".into());
                for target in fragment.targets.iter_mut().flatten() {
                    target.write_mask = ColorWrites::empty();
                }
            }
            BladePass::LitAfterDepth => {
                if let Some(depth) = descriptor.depth_stencil.as_mut() {
                    depth.depth_write_enabled = Some(false);
                    depth.depth_compare = Some(CompareFunction::Equal);
                }
            }
        }
        if let Some(mesh_group) = descriptor.layout.get_mut(2) {
            *mesh_group = self.empty_layout.clone();
        }
        descriptor.layout.push(self.draw_layout.clone());
        descriptor.primitive.cull_mode = None;
        descriptor.multisample.alpha_to_coverage_enabled = false;
        Ok(descriptor)
    }
}

/// One item per kind per view, ahead of every chunk and blended item.
///
/// Queued unconditionally for every vegetation view because `QueueMeshes` runs
/// before `prepare_blade_views` (`PrepareBindGroups`): intermittently-active
/// views (such as sensor cameras) would otherwise miss their draw whenever
/// their state is prepared on the same frame. `DrawBladeRecords` skips any
/// kind that had no blades sown.
#[allow(clippy::too_many_arguments)]
fn queue_blades(
    draw_functions: Res<DrawFunctions<Transparent3d>>,
    pipelines: Res<BladePipelines>,
    mut specialized: ResMut<SpecializedMeshPipelines<BladePipelines>>,
    pipeline_cache: Res<PipelineCache>,
    meshes: Res<RenderAssets<RenderMesh>>,
    draws: Query<(Entity, &MainEntity, &BladeDraw)>,
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
    views: Query<&ExtractedView, (With<ExtractedCamera>, Without<NoVegetation>)>,
    view_key_cache: Res<ViewKeyCache>,
) {
    let draw_blades = draw_functions.read().id::<DrawSownBlades>();
    for view in &views {
        let (Some(phase), Some(&view_key)) = (
            phases.get_mut(&view.retained_view_entity),
            view_key_cache.get(&view.retained_view_entity),
        ) else {
            continue;
        };
        let view_from_world = view.world_from_view.affine().inverse();
        for (entity, main_entity, draw) in &draws {
            let Some(mesh) = meshes.get(draw.template.id()) else {
                continue;
            };
            let key = view_key
                | MeshPipelineKey::from_primitive_topology_and_strip_index(
                    mesh.primitive_topology(),
                    mesh.index_format(),
                );
            let pass = match (draw.depth_only, blade_prepass()) {
                (true, _) => BladePass::Depth,
                (false, true) => BladePass::LitAfterDepth,
                (false, false) => BladePass::Lit,
            };
            let Ok(pipeline) = specialized.specialize(
                &pipeline_cache,
                &pipelines,
                (key, draw.draw.clone(), pass),
                &mesh.layout,
            ) else {
                continue;
            };
            let at = Vec3::ZERO;
            let view_z = view_from_world.transform_point3(at).z;
            let order = if draw.depth_only { 2.0 } else { 1.0 };
            phase.add_retained(Transparent3d {
                sorting_info: TransparentSortingInfo3d::Sorted {
                    mesh_center: at,
                    depth_bias: OPAQUE_FIRST - order - view_z,
                },
                entity: (entity, *main_entity),
                pipeline,
                draw_function: draw_blades,
                distance: 0.0,
                batch_range: 0..1,
                extra_index: PhaseItemExtraIndex::None,
                indexed: true,
            });
        }
    }
}

type DrawSownBlades = (
    SetItemPipeline,
    SetMeshViewBindGroup<0>,
    SetMeshViewBindingArrayBindGroup<1>,
    SetEmptyBindGroup<2>,
    DrawBladeRecords,
);

struct DrawBladeRecords;

impl<P: PhaseItem> RenderCommand<P> for DrawBladeRecords {
    type Param = (SRes<BladeViews>, SRes<RenderAssets<RenderMesh>>, SRes<MeshAllocator>);
    type ViewQuery = &'static ExtractedView;
    type ItemQuery = Read<BladeDraw>;

    #[inline]
    fn render<'w>(
        _item: &P,
        view: &'w ExtractedView,
        draw: Option<&'w BladeDraw>,
        (state, meshes, mesh_allocator): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let (state, meshes, mesh_allocator) =
            (state.into_inner(), meshes.into_inner(), mesh_allocator.into_inner());
        let Some(draw) = draw else {
            return RenderCommandResult::Skip;
        };
        let Some(target) = state
            .0
            .get(&view.retained_view_entity)
            .and_then(|blades| blades.kinds.get(draw.kind.cull))
        else {
            return RenderCommandResult::Skip;
        };
        let id = draw.template.id();
        let (Some(mesh), Some(vertices), Some(indices)) = (
            meshes.get(id),
            mesh_allocator.mesh_vertex_slice(&id),
            mesh_allocator.mesh_index_slice(&id),
        ) else {
            return RenderCommandResult::Skip;
        };
        let RenderMeshBufferInfo::Indexed { index_format, .. } = &mesh.buffer_info else {
            return RenderCommandResult::Skip;
        };
        pass.set_bind_group(3, &target.draw_group, &[]);
        pass.set_vertex_buffer(0, vertices.buffer.slice(..));
        pass.set_index_buffer(indices.buffer.slice(..), *index_format);
        pass.draw_indexed_indirect(&target.args, 0);
        RenderCommandResult::Success
    }
}
