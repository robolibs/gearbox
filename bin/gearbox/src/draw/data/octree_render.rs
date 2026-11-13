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
use super::super::runtime::Shared;
use super::super::style::owned_uniform;
use super::*;

/// What a storage buffer holds: a page's records or its overriding colours.
enum Owner {
    Cells(Arc<CellBuffer>),
    Colors(Arc<ColorBuffer>),
}

struct Storage {
    buffer: OwnedBuffer,
    owner: Owner,
    written: u64,
    _charge: Charge,
}

impl Storage {
    fn bytes(&self) -> &[u8] {
        match &self.owner {
            Owner::Cells(cells) => bytemuck::cast_slice(&cells.records.values),
            Owner::Colors(colors) => bytemuck::cast_slice(&colors.values.values),
        }
    }
}

struct Page {
    uniform: OwnedBuffer,
    bytes: List<u8>,
    records: usize,
    /// The colour buffer key, zero when records carry their own colour.
    colors: usize,
    cells: BindGroup,
    style: BindGroup,
    count: u32,
    opaque: bool,
    shape: Shape,
    _charge: Charge,
}

#[derive(Resource, Default)]
struct Cache {
    storage: Map<usize, Storage>,
    /// Pages currently drawn; replaced only once their successor is complete.
    pages: Map<Entity, Page>,
    staged: Map<Entity, Page>,
    opaque: Map<RetainedViewEntity, List<(Entity, MainEntity)>>,
    transparent: Map<RetainedViewEntity, List<(Entity, MainEntity)>>,
}

pub(super) fn install(app: &mut App) {
    app.sub_app_mut(RenderApp)
        .init_resource::<Cache>()
        .init_resource::<SpecializedRenderPipelines<Pipeline>>()
        .add_render_command::<Opaque3d, DrawOctree>()
        .add_render_command::<Transparent3d, DrawOctree>()
        .add_systems(RenderStartup, init.after(MeshPipelineSystems))
        .add_systems(Render, queue.in_set(RenderSystems::QueueMeshes));
}

#[derive(Resource)]
struct Pipeline {
    mesh: MeshPipeline,
    cells: BindGroupLayoutDescriptor,
    style: BindGroupLayoutDescriptor,
    shader: Handle<Shader>,
}

fn init(mut commands: Commands, mesh: Res<MeshPipeline>, assets: Res<AssetServer>) {
    use binding_types::*;
    commands.insert_resource(Pipeline {
        mesh: mesh.clone(),
        shader: assets.load("embedded://gearbox/draw/draw_octree.wgsl"),
        cells: BindGroupLayoutDescriptor::new(
            "octree cells",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX,
                (
                    storage_buffer_read_only_sized(false, None),
                    storage_buffer_read_only_sized(false, None),
                ),
            ),
        ),
        style: BindGroupLayoutDescriptor::new(
            "octree style",
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
    type Key = (MeshPipelineKey, bool, bool, Shape, MeshVertexBufferLayoutRef);
    fn specialize(&self, (key, opaque, frost, shape, layout): Self::Key) -> RenderPipelineDescriptor {
        let key = key
            | if opaque {
                MeshPipelineKey::empty()
            } else {
                MeshPipelineKey::BLEND_PREMULTIPLIED_ALPHA
            };
        let mut out = self
            .mesh
            .specialize(key, &layout)
            .expect("octree unit layout");
        out.vertex.shader = self.shader.clone();
        out.vertex.buffers.clear();
        out.vertex
            .shader_defs
            .push(ShaderDefVal::UInt("MATERIAL_BIND_GROUP".into(), 3));
        let define = match shape {
            Shape::Boxes => None,
            Shape::Discs => Some("OCTREE_DISCS"),
            Shape::Triangles => Some("OCTREE_TRIANGLES"),
        };
        if let Some(define) = define {
            out.vertex.shader_defs.push(define.into());
        }
        out.layout[2] = self.cells.clone();
        out.layout.push(self.style.clone());
        out.primitive.cull_mode = None;
        let fragment = out.fragment.as_mut().unwrap();
        fragment.shader = self.shader.clone();
        fragment
            .shader_defs
            .push(ShaderDefVal::UInt("MATERIAL_BIND_GROUP".into(), 3));
        if let Some(define) = define {
            fragment.shader_defs.push(define.into());
        }
        if opaque {
            fragment.shader_defs.push("OCTREE_OPAQUE".into());
        }
        if frost {
            fragment.shader_defs.push("DRAW_FROST".into());
        }
        if let Some(depth) = out.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(opaque);
            // Cells are absent from the depth prepass, so an Equal test would reject them.
            depth.depth_compare = Some(CompareFunction::GreaterEqual);
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
        self.nodes
            .iter()
            .filter(|(_, node)| !node.page.cells.records.values.is_empty())
            .count()
    }

    pub fn jobs(&self) -> impl Iterator<Item = super::super::upload::Job> + '_ {
        self.nodes
            .iter()
            .filter(|(_, node)| !node.page.cells.records.values.is_empty())
            .map(|(entity, _)| {
                super::super::upload::Job::new(
                    super::super::upload::Key::Octree(entity.to_bits()),
                    entity.to_bits(),
                )
            })
    }

    pub fn prune(&mut self) {
        let Some(shared) = &self.shared else { return };
        let budget = shared.backend.budget();
        let Cache {
            storage,
            pages,
            staged,
            opaque,
            transparent,
        } = &mut *self.cache;
        let nodes = &self.nodes;
        pages.retain(|entity, _| nodes.contains(*entity));
        staged.retain(|entity, _| nodes.contains(*entity));
        let live: Vec<usize> = nodes
            .iter()
            .flat_map(|(_, node)| [Some(Arc::as_ptr(&node.page.cells) as usize), node.page.colors.as_ref().map(|c| Arc::as_ptr(c) as usize)])
            .flatten()
            .chain(pages.values().flat_map(|page| [page.records, page.colors]).filter(|key| *key != 0))
            .collect();
        storage.retain_keys(live.into_iter(), |_, _| {});
        let _ = storage.compact(budget);
        let _ = pages.compact(budget);
        let _ = staged.compact(budget);
        let _ = opaque.compact(budget);
        let _ = transparent.compact(budget);
    }

    pub fn publish(&self, jobs: &[super::super::upload::Job]) {
        for job in jobs
            .iter()
            .filter(|job| matches!(job.key, super::super::upload::Key::Octree(_)))
        {
            if let Some(issue) = job.result.err().filter(|issue| *issue != Issue::Deferred) {
                warn!(
                    "octree admission {:?}: {issue:?}",
                    Entity::from_bits(job.item)
                );
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
        let lut = images.get(&shared.lut).ok_or(Issue::Deferred)?;
        let budget = shared.backend.budget();
        let ledger = shared.backend.gpu();
        let (entity, node) = nodes
            .get(Entity::from_bits(item))
            .map_err(|_| Issue::Deferred)?;
        let key = Arc::as_ptr(&node.page.cells) as usize;
        let upload = Upload { device, queue, ledger, budget };
        upload.stream(cache, remaining, staging, key, || Owner::Cells(node.page.cells.clone()))?;
        let colors = node.page.colors.as_ref().map_or(0, |colors| Arc::as_ptr(colors) as usize);
        if let Some(buffer) = &node.page.colors {
            upload.stream(cache, remaining, staging, colors, || Owner::Colors(buffer.clone()))?;
        }
        let total = cache.storage[&key].bytes().len() as u64;
        let current = cache
            .pages
            .get(&entity)
            .is_some_and(|page| page.records == key && page.colors == colors);
        if !current
            && !cache
                .staged
                .get(&entity)
                .is_some_and(|page| page.records == key && page.colors == colors)
        {
            cache.staged.remove(&entity);
            cache
                .staged
                .reserve(budget, 1)
                .map_err(|_| Issue::CpuBudget)?;
            let metadata =
                charge(budget, size_of::<Page>() + 2048).map_err(|_| Issue::CpuBudget)?;
            let bytes = owned_uniform(budget, &node.uniform).map_err(|_| Issue::CpuBudget)?;
            let uniform = allocate(
                device,
                queue,
                ledger,
                budget,
                bytes.len() as u64,
                BufferUsages::UNIFORM,
                "octree style",
            )?;
            let cells = device.create_bind_group(
                "octree cells",
                &pipelines.get_bind_group_layout(&pipeline.cells),
                &BindGroupEntries::sequential((
                    cache.storage[&key].buffer.buffer.as_entire_binding(),
                    cache.storage[&if colors == 0 { key } else { colors }].buffer.buffer.as_entire_binding(),
                )),
            );
            let style = device.create_bind_group(
                "octree style",
                &pipelines.get_bind_group_layout(&pipeline.style),
                &BindGroupEntries::sequential((
                    uniform.buffer.as_entire_binding(),
                    &lut.texture_view,
                    &lut.sampler,
                )),
            );
            cache
                .staged
                .insert(
                    budget,
                    entity,
                    Page {
                        uniform,
                        bytes: List::default(),
                        records: key,
                        colors,
                        cells,
                        style,
                        count: node.page.cells.records.values.len() as u32,
                        opaque: node.page.opaque,
                        shape: node.page.shape,
                        _charge: metadata,
                    },
                )
                .map_err(|_| Issue::CpuBudget)?;
        }
        let page = if current {
            cache.pages.get_mut(&entity).unwrap()
        } else {
            cache.staged.get_mut(&entity).unwrap()
        };
        let bytes = owned_uniform(budget, &node.uniform).map_err(|_| Issue::CpuBudget)?;
        if page.bytes.as_slice() != bytes.as_slice() {
            write(
                queue,
                ledger,
                budget,
                remaining,
                &page.uniform.buffer,
                0,
                &bytes,
                staging,
            )?;
            page.bytes = bytes;
        }
        if !current {
            let page = cache.staged.remove(&entity).unwrap();
            info!(
                "octree page ready entity={entity:?} cells={} bytes={total}",
                page.count
            );
            cache
                .pages
                .reserve(budget, 1)
                .map_err(|_| Issue::CpuBudget)?;
            cache
                .pages
                .insert(budget, entity, page)
                .map_err(|_| Issue::CpuBudget)?;
        }
        Ok(())
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
    let Some(shared) = shared else { return };
    let demand: u64 = nodes
        .iter()
        .map(|(_, _, node)| {
            let colors = node.page.colors.as_ref().map_or(0, |colors| colors.values.values.len() * 4);
            (node.page.cells.records.values.len() * size_of::<Cell>() + colors + 512) as u64
        })
        .sum();
    shared.compact_demand.store(demand, std::sync::atomic::Ordering::Relaxed);
    let layer = RenderLayers::layer(super::super::super::DRAW_LAYER);
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
                    && layers.is_some_and(|layers| layers.intersects(&layer))
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
        if !layers.is_some_and(|layers| layers.intersects(&layer)) {
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
        let drawable =
            |entity: &Entity, node: &Node| node.visible && cache.pages.contains_key(entity);
        let phase_counts = std::array::from_fn(|pass| {
            nodes
                .iter()
                .filter(|(entity, _, node)| {
                    drawable(entity, node) && usize::from(!cache.pages[entity].opaque) == pass
                })
                .count()
        });
        let Cache {
            pages,
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
            let Some(page) = pages.get(&entity).filter(|_| node.visible) else {
                continue;
            };
            let frosted = !page.opaque && node.uniform.extra.x > 0.0;
            if frosted
                && !frost.is_some_and(|frost| {
                    frost.ready(camera.physical_target_size, view.target_format)
                })
            {
                continue;
            }
            let pipeline_id = variants.specialize(
                &pipelines,
                &pipeline,
                (key, page.opaque, frosted, page.shape, shared.layout.clone()),
            );
            if pipelines.get_render_pipeline(pipeline_id).is_none() {
                continue;
            }
            if page.opaque {
                opaque.add(
                    Opaque3dBatchSetKey {
                        draw_function: opaque_draw.read().id::<DrawOctree>(),
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
                    .expect("reserved octree opaque phase slot");
            } else {
                transparent.add_retained(Transparent3d {
                    sorting_info: TransparentSortingInfo3d::Sorted {
                        mesh_center: node.uniform.world_from_local.transform_point3(Vec3::ZERO),
                        depth_bias: 0.5,
                    },
                    distance: 0.0,
                    entity: (entity, *main),
                    pipeline: pipeline_id,
                    draw_function: transparent_draw.read().id::<DrawOctree>(),
                    batch_range: 0..1,
                    extra_index: PhaseItemExtraIndex::None,
                    indexed: false,
                });
                own_transparent
                    .push((entity, *main))
                    .expect("reserved octree transparent phase slot");
            }
        }
        if !own_opaque.is_empty() {
            cache
                .opaque
                .insert(shared.backend.budget(), id, own_opaque)
                .expect("reserved octree opaque view slot");
        }
        if !own_transparent.is_empty() {
            cache
                .transparent
                .insert(shared.backend.budget(), id, own_transparent)
                .expect("reserved octree transparent view slot");
        }
    }
}

type DrawOctree = (
    SetItemPipeline,
    SetMeshViewBindGroup<0>,
    SetMeshViewBindingArrayBindGroup<1>,
    DrawCells,
);

struct DrawCells;

impl<P: PhaseItem> RenderCommand<P> for DrawCells {
    type Param = (SRes<Cache>, SRes<super::super::receipts::Receipts>);
    type ViewQuery = &'static ExtractedView;
    type ItemQuery = ();
    fn render<'w>(
        item: &P,
        view: &'w ExtractedView,
        _: Option<()>,
        (cache, receipts): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let Some(page) = cache.into_inner().pages.get(&item.entity()) else {
            return RenderCommandResult::Skip;
        };
        receipts.drawn_octree(view.retained_view_entity, item.entity());
        pass.set_bind_group(2, &page.cells, &[]);
        pass.set_bind_group(3, &page.style, &[]);
        match page.shape {
            Shape::Boxes => pass.draw(0..36, 0..page.count),
            Shape::Discs => pass.draw(0..6, 0..page.count),
            Shape::Triangles => pass.draw(0..page.count, 0..1),
        }
        RenderCommandResult::Success
    }
}

/// What streaming one storage buffer needs from the frame.
struct Upload<'a> {
    device: &'a RenderDevice,
    queue: &'a RenderQueue,
    ledger: &'a gearbox_api::draw_data::GpuBudget,
    budget: &'a CpuBudget,
}

impl Upload<'_> {
    /// Allocates the buffer for `key` once, then writes what this frame's upload
    /// allowance permits; `Deferred` until every byte is on the GPU.
    fn stream(
        &self,
        cache: &mut Cache,
        remaining: &mut u64,
        staging: &mut List<(gearbox_api::draw_data::GpuCharge, Charge)>,
        key: usize,
        owner: impl FnOnce() -> Owner,
    ) -> Result<(), Issue> {
        if !cache.storage.contains_key(&key) {
            if *remaining < 4 {
                return Err(Issue::Deferred);
            }
            cache.storage.reserve(self.budget, 1).map_err(|_| Issue::CpuBudget)?;
            let metadata = charge(self.budget, size_of::<Storage>() + 256).map_err(|_| Issue::CpuBudget)?;
            let owner = owner();
            let (label, bytes) = match &owner {
                Owner::Cells(cells) => ("octree cells", size_of_val::<[Cell]>(&cells.records.values)),
                Owner::Colors(colors) => ("octree colors", size_of_val::<[u32]>(&colors.values.values)),
            };
            let buffer = allocate(self.device, self.queue, self.ledger, self.budget, bytes as u64, BufferUsages::STORAGE, label)?;
            cache
                .storage
                .insert(self.budget, key, Storage { buffer, owner, written: 0, _charge: metadata })
                .map_err(|_| Issue::CpuBudget)?;
        }
        let storage = cache.storage.get_mut(&key).unwrap();
        let total = storage.bytes().len() as u64;
        let end = (storage.written + *remaining).min(total) / 4 * 4;
        if end > storage.written {
            let range = storage.written as usize..end as usize;
            write(self.queue, self.ledger, self.budget, remaining, &storage.buffer.buffer, storage.written, &storage.bytes()[range], staging)?;
            storage.written = end;
        }
        if storage.written != total {
            return Err(Issue::Deferred);
        }
        Ok(())
    }
}
