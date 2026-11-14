use super::super::super::frost::FrostView;
use bevy::camera::visibility::RenderLayers;
use bevy::core_pipeline::core_3d::{
    Opaque3d, Opaque3dBatchSetKey, Opaque3dBinKey, Transparent3d, TransparentSortingInfo3d,
};
use bevy::ecs::system::{SystemParamItem, lifetimeless::SRes};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{
    MeshPipeline, MeshPipelineKey, MeshPipelineSystems, SetMeshViewBindGroup,
    SetMeshViewBindingArrayBindGroup, ViewKeyCache,
};
use bevy::render::camera::ExtractedCamera;
use bevy::render::mesh::allocator::MeshSlabs;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_phase::{
    AddRenderCommand, BinnedRenderPhaseType, DrawFunctions, InputUniformIndex, PhaseItem,
    PhaseItemExtraIndex, RenderCommand, RenderCommandResult, SetItemPipeline, TrackedRenderPass,
    ViewBinnedRenderPhases, ViewSortedRenderPhases,
};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::sync_world::MainEntity;
use bevy::render::texture::GpuImage;
use bevy::render::view::{ExtractedView, RetainedViewEntity};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::ShaderDefVal;

use super::super::admission::Issue;
use super::super::owned::{List, Map};
use super::super::render::{OwnedBuffer, allocate, phase_lists, prune_phases, write};
#[cfg(feature = "draw-probes")]
use super::super::render::{Preparation, UploadFrame};
use super::super::runtime::Shared;
use super::super::style::owned_uniform;
use super::*;

enum Owner {
    Positions(Arc<RecordBuffer<Position>>),
    Samples(Arc<RecordBuffer<Sample>>),
}
impl Owner {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Positions(records) => bytemuck::cast_slice(&records.values),
            Self::Samples(records) => bytemuck::cast_slice(&records.values),
        }
    }
}
struct Storage {
    buffer: OwnedBuffer,
    owner: Owner,
    written: u64,
    _charge: Charge,
}
struct Page {
    uniform: OwnedBuffer,
    bytes: List<u8>,
    records: [usize; 2],
    points: BindGroup,
    style: BindGroup,
    ready: bool,
    count: u32,
    frost: bool,
    _charge: Charge,
}
#[derive(Resource, Default)]
struct Cache {
    storage: Map<usize, Storage>,
    pages: Map<Entity, Page>,
    failures: Map<Entity, Issue>,
    opaque: Map<RetainedViewEntity, List<(Entity, MainEntity)>>,
    transparent: Map<RetainedViewEntity, List<(Entity, MainEntity)>>,
}

pub(super) fn install(app: &mut App) {
    app.sub_app_mut(RenderApp)
        .init_resource::<Cache>()
        .init_resource::<SpecializedRenderPipelines<Pipeline>>()
        .add_render_command::<Opaque3d, DrawPoints>()
        .add_render_command::<Transparent3d, DrawPoints>()
        .add_systems(RenderStartup, init.after(MeshPipelineSystems))
        .add_systems(Render, (queue.in_set(RenderSystems::QueueMeshes),));
    #[cfg(feature = "draw-probes")]
    app.sub_app_mut(RenderApp).add_systems(
        Render,
        observe_allocations
            .after(Preparation::Dispatch)
            .in_set(RenderSystems::PrepareResources),
    );
}

#[derive(Resource)]
struct Pipeline {
    mesh: MeshPipeline,
    points: BindGroupLayoutDescriptor,
    style: BindGroupLayoutDescriptor,
    shader: Handle<Shader>,
}

fn init(mut commands: Commands, mesh: Res<MeshPipeline>, assets: Res<AssetServer>) {
    use binding_types::*;
    commands.insert_resource(Pipeline {
        mesh: mesh.clone(),
        shader: assets.load("embedded://gearbox/draw/draw_stroke.wesl"),
        points: BindGroupLayoutDescriptor::new(
            "compact legacy points",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX,
                (
                    storage_buffer_read_only_sized(false, None),
                    storage_buffer_read_only_sized(false, None),
                ),
            ),
        ),
        style: BindGroupLayoutDescriptor::new(
            "compact legacy style",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX_FRAGMENT,
                (
                    uniform_buffer::<Uniform>(false),
                    texture_3d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                ),
            ),
        ),
    });
}

impl SpecializedRenderPipeline for Pipeline {
    type Key = (MeshPipelineKey, bool, bool, bool, MeshVertexBufferLayoutRef);
    fn specialize(
        &self,
        (key, opaque, on_top, frost, layout): Self::Key,
    ) -> RenderPipelineDescriptor {
        let key = key
            | if opaque {
                MeshPipelineKey::BLEND_ALPHA_TO_COVERAGE
            } else {
                MeshPipelineKey::BLEND_PREMULTIPLIED_ALPHA
            };
        let mut out = self
            .mesh
            .specialize(key, &layout)
            .expect("compact point unit layout");
        out.vertex.shader = self.shader.clone();
        out.vertex.buffers.clear();
        out.vertex.shader_defs.extend([
            "DRAW_COMPACT_POINTS".into(),
            ShaderDefVal::UInt("MATERIAL_BIND_GROUP".into(), 3),
        ]);
        out.layout[2] = self.points.clone();
        out.layout.push(self.style.clone());
        out.primitive.cull_mode = None;
        let fragment = out.fragment.as_mut().unwrap();
        fragment.shader = self.shader.clone();
        fragment.shader_defs.extend([
            "DRAW_COMPACT_POINTS".into(),
            ShaderDefVal::UInt("MATERIAL_BIND_GROUP".into(), 3),
        ]);
        if opaque {
            fragment.shader_defs.push("DRAW_SPLAT".into());
        }
        if frost {
            fragment.shader_defs.push("DRAW_FROST".into());
        }
        if let Some(depth) = out.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(opaque);
            if on_top {
                depth.depth_compare = Some(CompareFunction::Always);
            }
        }
        out
    }
}

#[derive(bevy::ecs::system::SystemParam)]
pub(in crate::draw::data) struct Prepare<'w, 's> {
    cache: ResMut<'w, Cache>,
    shared: Option<Res<'w, Shared>>,
    nodes: Query<'w, 's, (Entity, &'static Node)>,
    device: Res<'w, RenderDevice>,
    queue: Res<'w, RenderQueue>,
    pipeline: Res<'w, Pipeline>,
    pipelines: Res<'w, PipelineCache>,
    images: Res<'w, RenderAssets<GpuImage>>,
}

impl Prepare<'_, '_> {
    pub fn count(&self) -> usize {
        self.nodes.iter().len()
    }
    pub fn jobs(&self) -> impl Iterator<Item = super::super::upload::Job> + '_ {
        self.nodes.iter().map(|(entity, _)| {
            super::super::upload::Job::new(
                super::super::upload::Key::Legacy(entity.to_bits()),
                entity.to_bits(),
            )
        })
    }
    pub fn prune(&mut self) {
        let Some(shared) = &self.shared else { return };
        let budget = shared.backend.budget();
        let cache = &mut self.cache;
        let nodes = &self.nodes;
        cache.storage.retain_keys(
            nodes.iter().flat_map(|(_, node)| {
                [
                    Arc::as_ptr(&node.points.positions) as usize,
                    Arc::as_ptr(&node.points.samples) as usize,
                ]
            }),
            |_, _| {},
        );
        cache.pages.retain(|entity, _| nodes.contains(*entity));
        cache.failures.retain(|entity, _| nodes.contains(*entity));
        let _ = cache.storage.compact(budget);
        let _ = cache.pages.compact(budget);
        let _ = cache.failures.compact(budget);
        let _ = cache.opaque.compact(budget);
        let _ = cache.transparent.compact(budget);
    }
    pub fn publish(&mut self, jobs: &[super::super::upload::Job]) {
        let Some(shared) = &self.shared else { return };
        let budget = shared.backend.budget();
        for job in jobs
            .iter()
            .filter(|job| matches!(job.key, super::super::upload::Key::Legacy(_)))
        {
            let entity = Entity::from_bits(job.item);
            if let Some(page) = self.cache.pages.get_mut(&entity) {
                page.ready = job.result.is_ok();
            }
            if let Some(issue) = job.result.err().filter(|issue| *issue != Issue::Deferred) {
                if self
                    .cache
                    .failures
                    .insert(budget, entity, issue)
                    .ok()
                    .flatten()
                    != Some(issue)
                {
                    warn!("compact legacy point admission: {issue:?}");
                }
            } else if job.result.is_ok() {
                self.cache.failures.remove(&entity);
            }
        }
    }
    pub fn step(
        &mut self,
        item: u64,
        remaining: &mut u64,
        staging: &mut List<(gearbox_api::draw_data::GpuCharge, Charge)>,
    ) -> Result<(), Issue> {
        let Self {
            cache,
            shared,
            nodes,
            device,
            queue,
            pipeline,
            pipelines,
            images,
        } = self;
        let shared = shared.as_deref().ok_or(Issue::Deferred)?;
        let device = &**device;
        let queue = &**queue;
        let pipeline = &**pipeline;
        let pipelines = &**pipelines;
        let lut = images.get(&shared.lut).ok_or(Issue::Deferred)?;
        let budget = shared.backend.budget();
        let ledger = shared.backend.gpu();
        let (entity, node) = nodes
            .get(Entity::from_bits(item))
            .map_err(|_| Issue::Deferred)?;
        let keys = [
            Arc::as_ptr(&node.points.positions) as usize,
            Arc::as_ptr(&node.points.samples) as usize,
        ];
        for (key, owner) in [
            (keys[0], Owner::Positions(node.points.positions.clone())),
            (keys[1], Owner::Samples(node.points.samples.clone())),
        ] {
            if !cache.storage.contains_key(&key) {
                if *remaining < 4 {
                    return Err(Issue::Deferred);
                }
                cache
                    .storage
                    .reserve(budget, 1)
                    .map_err(|_| Issue::CpuBudget)?;
                let metadata =
                    charge(budget, size_of::<Storage>() + 256).map_err(|_| Issue::CpuBudget)?;
                let buffer = allocate(
                    &device,
                    &queue,
                    ledger,
                    budget,
                    owner.bytes().len() as u64,
                    BufferUsages::STORAGE,
                    "legacy point records",
                )?;
                cache
                    .storage
                    .insert(
                        budget,
                        key,
                        Storage {
                            buffer,
                            owner,
                            written: 0,
                            _charge: metadata,
                        },
                    )
                    .map_err(|_| Issue::CpuBudget)?;
            }
            let storage = cache.storage.get_mut(&key).unwrap();
            let end =
                (storage.written + *remaining).min(storage.owner.bytes().len() as u64) / 4 * 4;
            if end > storage.written {
                write(
                    &queue,
                    ledger,
                    budget,
                    remaining,
                    &storage.buffer.buffer,
                    storage.written,
                    &storage.owner.bytes()[storage.written as usize..end as usize],
                    staging,
                )?;
                #[cfg(feature = "draw-probes")]
                if let Some(probe) = &shared.probe {
                    probe.legacy_uploaded(
                        if matches!(&storage.owner, Owner::Positions(_)) {
                            0
                        } else {
                            1
                        },
                        end - storage.written,
                    );
                }
                storage.written = end;
            }
            if storage.written != storage.owner.bytes().len() as u64 {
                return Err(Issue::Deferred);
            }
        }
        if cache
            .pages
            .get(&entity)
            .is_some_and(|page| page.records != keys)
        {
            cache.pages.remove(&entity);
        }
        if !cache.pages.contains_key(&entity) {
            cache
                .pages
                .reserve(budget, 1)
                .map_err(|_| Issue::CpuBudget)?;
            let metadata =
                charge(budget, size_of::<Page>() + 2048).map_err(|_| Issue::CpuBudget)?;
            let bytes = owned_uniform(budget, &node.uniform).map_err(|_| Issue::CpuBudget)?;
            if *remaining < bytes.len() as u64 {
                return Err(Issue::Deferred);
            }
            let uniform = allocate(
                &device,
                &queue,
                ledger,
                budget,
                bytes.len() as u64,
                BufferUsages::UNIFORM,
                "legacy point style",
            )?;
            let points = device.create_bind_group(
                "legacy point records",
                &pipelines.get_bind_group_layout(&pipeline.points),
                &BindGroupEntries::sequential((
                    cache.storage[&keys[0]].buffer.buffer.as_entire_binding(),
                    cache.storage[&keys[1]].buffer.buffer.as_entire_binding(),
                )),
            );
            let style = device.create_bind_group(
                "legacy point style",
                &pipelines.get_bind_group_layout(&pipeline.style),
                &BindGroupEntries::sequential((
                    uniform.buffer.as_entire_binding(),
                    &lut.texture_view,
                    &lut.sampler,
                )),
            );
            cache
                .pages
                .insert(
                    budget,
                    entity,
                    Page {
                        uniform,
                        bytes: List::default(),
                        records: keys,
                        points,
                        style,
                        ready: false,
                        count: node.points.positions.values.len() as u32,
                        frost: !node.points.opaque && node.uniform.unused.x > 0.0,
                        _charge: metadata,
                    },
                )
                .map_err(|_| Issue::CpuBudget)?;
        }
        let sizes = keys.map(|key| cache.storage[&key].buffer.buffer.size());
        let page = cache.pages.get_mut(&entity).unwrap();
        page.frost = !node.points.opaque && node.uniform.unused.x > 0.0;
        let bytes = owned_uniform(budget, &node.uniform).map_err(|_| Issue::CpuBudget)?;
        let uniform_size = bytes.len();
        let first = page.bytes.is_empty();
        if page.bytes.as_slice() != bytes.as_slice() {
            write(
                &queue,
                ledger,
                budget,
                remaining,
                &page.uniform.buffer,
                0,
                &bytes,
                staging,
            )?;
            #[cfg(feature = "draw-probes")]
            if let Some(probe) = &shared.probe {
                probe.legacy_uploaded(2, bytes.len() as u64);
            }
            page.bytes = bytes;
        }
        page.ready = true;
        if first {
            info!(
                "compact legacy ready entity={entity:?} points={} positions={} attributes={} uniform={}",
                page.count, sizes[0], sizes[1], uniform_size
            );
        }
        Ok(())
    }
}
#[cfg(feature = "draw-probes")]
fn observe_allocations(cache: Res<Cache>, shared: Option<Res<Shared>>, upload: Res<UploadFrame>) {
    let Some(shared) = shared else { return };
    if let Some(probe) = &shared.probe {
        use super::super::probe::LegacyReport;
        let usage = shared.backend.gpu().usage();
        probe.legacy(LegacyReport {
            points: cache.pages.values().map(|page| u64::from(page.count)).sum(),
            pages: cache.pages.len() as u64,
            ready: cache.pages.values().filter(|page| page.ready).count() as u64,
            positions: cache
                .storage
                .values()
                .filter(|s| matches!(&s.owner, Owner::Positions(_)))
                .map(|s| s.buffer.buffer.size())
                .sum(),
            attributes: cache
                .storage
                .values()
                .filter(|s| matches!(&s.owner, Owner::Samples(_)))
                .map(|s| s.buffer.buffer.size())
                .sum(),
            uniforms: cache
                .pages
                .values()
                .map(|page| page.uniform.buffer.size())
                .sum(),
            gpu_bytes: usage.bytes,
            auxiliary_gpu_bytes: usage.auxiliary,
            staging_gpu_bytes: usage.staging,
            retiring_gpu_bytes: usage.retiring,
            other_gpu_bytes: usage.other,
            charged_cpu_bytes: shared.backend.budget().usage().committed(),
            frame_upload: shared.backend.limits().upload_bytes_per_frame - upload.remaining,
            ..Default::default()
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn queue(
    mut cache: ResMut<Cache>,
    shared: Option<Res<Shared>>,
    nodes: Query<(Entity, &MainEntity, &Node)>,
    views: Query<(
        &ExtractedView,
        Option<&RenderLayers>,
        &ExtractedCamera,
        Option<&FrostView>,
    )>,
    keys: Res<ViewKeyCache>,
    pipeline: Res<Pipeline>,
    pipelines: Res<PipelineCache>,
    mut variants: ResMut<SpecializedRenderPipelines<Pipeline>>,
    mut opaque: ResMut<ViewBinnedRenderPhases<Opaque3d>>,
    mut transparent: ResMut<ViewSortedRenderPhases<Transparent3d>>,
    opaque_draw: Res<DrawFunctions<Opaque3d>>,
    transparent_draw: Res<DrawFunctions<Transparent3d>>,
) {
    let Some(shared) = shared else {
        return;
    };
    #[cfg(feature = "draw-probes")]
    if let Some(probe) = &shared.probe {
        probe.begin_legacy_queue();
    }
    let Cache {
        pages,
        opaque: old_opaque,
        transparent: old_transparent,
        ..
    } = &mut *cache;
    prune_phases(
        old_opaque,
        old_transparent,
        |entity| pages.contains_key(&entity),
        |retained| {
            views.iter().any(|(view, layers, _, _)| {
                view.retained_view_entity == *retained
                    && layers.is_some_and(|layers| {
                        layers.intersects(&RenderLayers::layer(super::super::super::DRAW_LAYER))
                    })
            })
        },
        &mut opaque,
        &mut transparent,
    );
    for (view, layers, camera, frost) in &views {
        let id = view.retained_view_entity;
        let Some((opaque, transparent)) = opaque.get_mut(&id).zip(transparent.get_mut(&id)) else {
            continue;
        };
        if !layers.is_some_and(|layers| {
            layers.intersects(&RenderLayers::layer(super::super::super::DRAW_LAYER))
        }) {
            continue;
        }
        let Some(key) = keys.get(&id).copied() else {
            continue;
        };
        let key = key
            | MeshPipelineKey::from_primitive_topology_and_strip_index(
                PrimitiveTopology::TriangleList,
                None,
            );
        let phase_counts = std::array::from_fn(|pass| {
            nodes
                .iter()
                .filter(|(entity, _, node)| {
                    node.visible
                        && cache.pages.get(entity).is_some_and(|page| page.ready)
                        && usize::from(!node.points.opaque) == pass
                })
                .count()
        });
        let Cache {
            opaque: old_opaque,
            transparent: old_transparent,
            ..
        } = &mut *cache;
        let Ok((mut own_opaque, mut own_transparent)) = phase_lists(
            shared.backend.budget(),
            phase_counts,
            id,
            old_opaque,
            old_transparent,
            opaque,
            transparent,
        ) else {
            continue;
        };
        for (entity, main, node) in &nodes {
            if !node.visible || !cache.pages.get(&entity).is_some_and(|page| page.ready) {
                continue;
            }
            let points = &node.points;
            if !points.opaque
                && node.uniform.unused.x > 0.0
                && !frost.is_some_and(|frost| {
                    frost.ready(camera.physical_target_size, view.target_format)
                })
            {
                continue;
            }
            let pipeline_id = variants.specialize(
                &pipelines,
                &pipeline,
                (
                    key,
                    points.opaque,
                    points.on_top,
                    !points.opaque && node.uniform.unused.x > 0.0,
                    shared.layout.clone(),
                ),
            );
            if pipelines.get_render_pipeline(pipeline_id).is_none() {
                continue;
            }
            if points.opaque {
                opaque.add(
                    Opaque3dBatchSetKey {
                        draw_function: opaque_draw.read().id::<DrawPoints>(),
                        pipeline: pipeline_id,
                        material_bind_group_index: None,
                        lightmap_slab: None,
                        slabs: MeshSlabs::default(),
                    },
                    Opaque3dBinKey {
                        asset_id: shared.mesh.id().untyped(),
                    },
                    (entity, *main),
                    InputUniformIndex::default(),
                    BinnedRenderPhaseType::NonMesh,
                );
                own_opaque
                    .push((entity, *main))
                    .expect("reserved legacy opaque phase slot");
            } else {
                transparent.add_retained(Transparent3d {
                    sorting_info: TransparentSortingInfo3d::Sorted {
                        mesh_center: points.sort_center(node.uniform.world_from_local),
                        depth_bias: 0.5 + if points.on_top { 1.0e7 } else { 0.0 },
                    },
                    distance: 0.0,
                    entity: (entity, *main),
                    pipeline: pipeline_id,
                    draw_function: transparent_draw.read().id::<DrawPoints>(),
                    batch_range: 0..1,
                    extra_index: PhaseItemExtraIndex::None,
                    indexed: false,
                });
                own_transparent
                    .push((entity, *main))
                    .expect("reserved legacy transparent phase slot");
            }
        }
        #[cfg(feature = "draw-probes")]
        if let Some(probe) = &shared.probe {
            probe.legacy_queued(id, own_opaque.len() + own_transparent.len());
        }
        if !own_opaque.is_empty() {
            cache
                .opaque
                .insert(shared.backend.budget(), id, own_opaque)
                .expect("reserved legacy opaque view slot");
        }
        if !own_transparent.is_empty() {
            cache
                .transparent
                .insert(shared.backend.budget(), id, own_transparent)
                .expect("reserved legacy transparent view slot");
        }
    }
}

type DrawPoints = (
    SetItemPipeline,
    SetMeshViewBindGroup<0>,
    SetMeshViewBindingArrayBindGroup<1>,
    DrawPage,
);
struct DrawPage;
impl<P: PhaseItem> RenderCommand<P> for DrawPage {
    type Param = SRes<Cache>;
    type ViewQuery = Option<&'static FrostView>;
    type ItemQuery = ();
    fn render<'w>(
        item: &P,
        frost: Option<&'w FrostView>,
        _: Option<()>,
        cache: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let Some(page) = cache
            .into_inner()
            .pages
            .get(&item.entity())
            .filter(|page| page.ready)
        else {
            return RenderCommandResult::Skip;
        };
        if page.frost && frost.is_none() {
            return RenderCommandResult::Skip;
        }
        pass.set_bind_group(2, &page.points, &[]);
        pass.set_bind_group(3, &page.style, &[]);
        pass.draw(0..6, 0..page.count);
        RenderCommandResult::Success
    }
}
