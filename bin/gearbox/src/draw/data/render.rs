use std::sync::Arc;

use super::super::frost::FrostView;
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
use bevy::prelude::*;
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
use gearbox_api::draw_data::{Charge, CpuBudget, GpuBudget, GpuCharge, GpuClass, MemoryClass};

use super::{
    admission::{Issue, Results},
    appearance::Appearance,
    cloud::Geometry,
    coarse::Tier,
    frontier::{DatasetSet, Frontiers, Offered, Selection, ViewFrontier},
    owned::{List, Map},
    runtime::{Node, Shared},
    style::{PageUniform, Style, owned_uniform},
    worker::Dataset,
};

#[path = "lod_render.rs"]
pub(in crate::draw::data) mod selection;

pub(super) struct OwnedBuffer {
    pub buffer: Buffer,
    queue: RenderQueue,
    lease: Option<GpuCharge>,
    cpu: Option<Charge>,
}
impl Drop for OwnedBuffer {
    fn drop(&mut self) {
        if let Some(mut lease) = self.lease.take() {
            lease.retire();
            let cpu = self.cpu.take();
            let buffer = self.buffer.clone();
            self.queue.submit([]);
            self.queue
                .on_submitted_work_done(move || drop((lease, cpu, buffer)));
        }
    }
}

enum Owner {
    Geometry(Arc<Geometry>),
    Appearance(Arc<Appearance>),
    CoarseGeometry(Arc<super::cloud::SpatialMetadata>),
    CoarseAppearance(Arc<super::coarse::Samples>),
}
impl Owner {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Geometry(value) => bytemuck::cast_slice(&value.positions.values),
            Self::Appearance(value) => value.attributes.bytes(),
            Self::CoarseGeometry(value) => bytemuck::cast_slice(&value.coarse.positions.values),
            Self::CoarseAppearance(value) => value.attributes.bytes(),
        }
    }
}
struct Storage {
    buffer: OwnedBuffer,
    written: u64,
    owner: Owner,
}
struct StyleBuffer {
    buffer: OwnedBuffer,
    style: Style,
    generation: Arc<Dataset>,
    group: Option<BindGroup>,
    _cpu: Charge,
}
struct PageBuffer {
    buffer: OwnedBuffer,
    previous: List<u8>,
    group: BindGroup,
    geometry: (usize, Tier),
    /// None when the page has no attribute words and binds its positions in their place.
    appearance: Option<(usize, Tier)>,
    style: usize,
    count: u32,
}

#[derive(Resource, Default)]
struct Cache {
    page_loads: u64,
    evicted_pages: u64,
    geometry: Map<(usize, Tier), Storage>,
    appearance: Map<(usize, Tier), Storage>,
    styles: Map<usize, StyleBuffer>,
    pages: Map<(Entity, Tier), PageBuffer>,
    opaque: Map<RetainedViewEntity, List<(Entity, MainEntity)>>,
    transparent: Map<RetainedViewEntity, List<(Entity, MainEntity)>>,
}

pub(super) fn allocate(
    device: &RenderDevice,
    queue: &RenderQueue,
    ledger: &GpuBudget,
    budget: &CpuBudget,
    bytes: u64,
    usage: BufferUsages,
    label: &'static str,
) -> Result<OwnedBuffer, Issue> {
    let bytes = bytes.max(4);
    check_buffer_limits(&device.limits(), bytes, usage)?;
    let class = if usage.contains(BufferUsages::UNIFORM) {
        GpuClass::Uniforms
    } else {
        GpuClass::Records
    };
    let cpu = super::worker::charge(
        budget,
        size_of::<OwnedBuffer>() + size_of::<GpuCharge>() + 256,
    )
    .map_err(|_| Issue::CpuBudget)?;
    let lease = ledger
        .reserve_class(bytes, class)
        .ok_or(Issue::GpuAllocation)?;
    let buffer = device.create_buffer(&BufferDescriptor {
        label: Some(label),
        size: bytes,
        usage: usage | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    Ok(OwnedBuffer {
        buffer,
        queue: queue.clone(),
        lease: Some(lease),
        cpu: Some(cpu),
    })
}

fn check_buffer_limits(
    limits: &bevy::render::settings::WgpuLimits,
    bytes: u64,
    usage: BufferUsages,
) -> Result<(), Issue> {
    if bytes > limits.max_buffer_size
        || (usage.contains(BufferUsages::STORAGE)
            && bytes > u64::from(limits.max_storage_buffer_binding_size))
        || (usage.contains(BufferUsages::UNIFORM)
            && bytes > u64::from(limits.max_uniform_buffer_binding_size))
    {
        return Err(Issue::DeviceLimit);
    }
    Ok(())
}

#[cfg(test)]
mod admission_tests {
    use super::*;

    #[test]
    fn storage_and_uniform_binding_limits_are_checked_before_allocation() {
        let limits = WgpuLimits {
            max_buffer_size: 1024,
            max_storage_buffer_binding_size: 512,
            max_uniform_buffer_binding_size: 256,
            ..Default::default()
        };
        for (usage, limit) in [
            (BufferUsages::STORAGE, 512),
            (BufferUsages::UNIFORM, 256),
            (BufferUsages::COPY_DST, 1024),
        ] {
            assert_eq!(check_buffer_limits(&limits, limit, usage), Ok(()));
            assert_eq!(
                check_buffer_limits(&limits, limit + 4, usage),
                Err(Issue::DeviceLimit)
            );
        }
        assert_eq!(
            check_buffer_limits(&limits, u64::MAX, BufferUsages::STORAGE),
            Err(Issue::DeviceLimit)
        );
    }
}

pub(super) fn write(
    queue: &RenderQueue,
    ledger: &GpuBudget,
    budget: &CpuBudget,
    remaining: &mut u64,
    buffer: &Buffer,
    offset: u64,
    bytes: &[u8],
    charges: &mut List<(GpuCharge, Charge)>,
) -> Result<(), Issue> {
    if bytes.len() as u64 > *remaining {
        return Err(Issue::Deferred);
    }
    if charges.remaining() == 0 {
        return Err(Issue::CpuBudget);
    }
    let gpu = ledger
        .reserve_class(bytes.len() as u64, GpuClass::Staging)
        .ok_or(Issue::GpuStaging)?;
    let cpu = budget
        .reserve(bytes.len() as u64)
        .and_then(|mut reservation| reservation.charge(bytes.len() as u64, MemoryClass::Scratch))
        .map_err(|_| Issue::CpuBudget)?;
    queue.write_buffer(buffer, offset, bytes);
    *remaining -= bytes.len() as u64;
    ledger.record_upload(bytes.len() as u64);
    charges.push((gpu, cpu)).map_err(|_| Issue::CpuBudget)?;
    Ok(())
}

#[derive(Resource, Default)]
pub(super) struct UploadFrame {
    pub remaining: u64,
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Preparation {
    Reset,
    Dispatch,
}

fn reset_uploads(shared: Option<Res<Shared>>, mut upload: ResMut<UploadFrame>) {
    upload.remaining = shared.map_or(0, |shared| shared.backend.limits().upload_bytes_per_frame);
}

pub(super) fn install(app: &mut App) {
    app.sub_app_mut(RenderApp)
        .init_resource::<Cache>()
        .init_resource::<selection::Views>()
        .init_resource::<UploadFrame>()
        .configure_sets(
            Render,
            (Preparation::Reset, Preparation::Dispatch)
                .chain()
                .in_set(RenderSystems::PrepareResources),
        )
        .add_systems(Render, reset_uploads.in_set(Preparation::Reset))
        .init_resource::<Frontiers>()
        .init_resource::<SpecializedRenderPipelines<Pipeline>>()
        .add_render_command::<Opaque3d, DrawCloud<true>>()
        .add_render_command::<Transparent3d, DrawCloud<false>>()
        .add_systems(RenderStartup, init.after(MeshPipelineSystems))
        .add_systems(
            Render,
            ((
                super::receipts::begin,
                queue,
                super::receipts::prepare_frame,
            )
                .chain()
                .in_set(RenderSystems::QueueMeshes),),
        );
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
    page: BindGroupLayoutDescriptor,
    page_selected: BindGroupLayoutDescriptor,
    style: BindGroupLayoutDescriptor,
    shader: Handle<Shader>,
}

fn init(mut commands: Commands, mesh: Res<MeshPipeline>, assets: Res<AssetServer>) {
    use binding_types::*;
    commands.insert_resource(Pipeline {
        mesh: mesh.clone(),
        shader: assets.load("embedded://gearbox/draw/draw_cloud.wesl"),
        page: BindGroupLayoutDescriptor::new(
            "cloud page",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX_FRAGMENT,
                (
                    uniform_buffer::<PageUniform>(false),
                    storage_buffer_read_only_sized(false, None),
                    storage_buffer_read_only_sized(false, None),
                ),
            ),
        ),
        style: BindGroupLayoutDescriptor::new(
            "cloud style",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX_FRAGMENT,
                (
                    uniform_buffer::<Style>(false),
                    texture_3d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                ),
            ),
        ),
        page_selected: BindGroupLayoutDescriptor::new(
            "selected cloud page",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX_FRAGMENT,
                (
                    uniform_buffer::<PageUniform>(false),
                    storage_buffer_read_only_sized(false, None),
                    storage_buffer_read_only_sized(false, None),
                    storage_buffer_read_only_sized(false, None),
                ),
            ),
        ),
    });
}

impl SpecializedRenderPipeline for Pipeline {
    type Key = (MeshPipelineKey, bool, bool, bool, MeshVertexBufferLayoutRef);
    fn specialize(
        &self,
        (key, opaque, frost, selected, layout): Self::Key,
    ) -> RenderPipelineDescriptor {
        let mut descriptor = self
            .mesh
            .specialize(key, &layout)
            .expect("unit rectangle cloud layout");
        descriptor.vertex.shader = self.shader.clone();
        descriptor.vertex.buffers.clear();
        descriptor.layout[2] = if selected {
            self.page_selected.clone()
        } else {
            self.page.clone()
        };
        descriptor.layout.push(self.style.clone());
        descriptor.primitive.cull_mode = None;
        let state = super::alpha::RasterState {
            opaque,
            samples: descriptor.multisample.count,
        };
        descriptor.multisample = state.multisample();
        let fragment = descriptor.fragment.as_mut().unwrap();
        fragment.shader = self.shader.clone();
        if selected {
            descriptor.vertex.shader_defs.push("CLOUD_SELECTED".into());
            fragment.shader_defs.push("CLOUD_SELECTED".into());
        }
        if opaque {
            fragment.shader_defs.push("CLOUD_OPAQUE".into());
        }
        if frost {
            fragment.shader_defs.push("DRAW_FROST".into());
        }
        if state.samples == 1 {
            fragment.shader_defs.push("CLOUD_SINGLE_SAMPLE".into());
        }
        for target in fragment.targets.iter_mut().flatten() {
            target.blend = state.blend();
        }
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            *depth = state.depth(depth.format);
        }
        descriptor
    }
}

#[derive(bevy::ecs::system::SystemParam)]
pub(super) struct Prepare<'w, 's> {
    cache: ResMut<'w, Cache>,
    selections: ResMut<'w, selection::Views>,
    offered: Option<Res<'w, Offered>>,
    receipts: Res<'w, super::receipts::Receipts>,
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
            .filter(|(entity, _)| self.selections.needs(*entity))
            .count()
            + self.selections.count()
    }
    pub fn jobs(&self) -> impl Iterator<Item = super::upload::Job> + '_ {
        self.nodes
            .iter()
            .filter(|(entity, _)| self.selections.needs(*entity))
            .map(|(entity, node)| {
                super::upload::Job::new(
                    super::upload::Key::Typed(node.dataset.revision.incarnation),
                    entity.to_bits(),
                )
            })
            .chain(self.selections.jobs())
    }
    pub fn deny(&self) {
        if let Some(shared) = &self.shared {
            for (_, node) in &self.nodes {
                Issue::CpuBudget.report(&shared.backend, &node.dataset);
            }
        }
    }
    pub fn publish<'a>(&'a self, jobs: &[super::upload::Job], mut results: Results<'a>) {
        let Some(shared) = &self.shared else { return };
        for job in jobs {
            if matches!(job.key, super::upload::Key::Typed(_))
                && let Ok((_, node)) = self.nodes.get(Entity::from_bits(job.item))
            {
                results.record(&node.dataset, job.result);
                if job.result == Err(Issue::GpuAllocation) {
                    self.selections.starve(Entity::from_bits(job.item));
                }
            }
        }
        results.publish(&shared.backend);
        for job in jobs {
            if let Err(issue) = job.result {
                self.selections.report(job.key, job.item, issue, shared);
            }
        }
    }
    pub fn prune(&mut self) {
        let Some(shared) = &self.shared else { return };
        let budget = shared.backend.budget();
        self.selections
            .prune(budget, self.receipts.views(), self.offered.as_deref());
        let cache = &mut self.cache;
        let nodes = &self.nodes;
        let selections = &self.selections;
        let mut evicted = 0;
        cache.pages.retain_keys(
            selections
                .sources()
                .filter(|(entity, _)| nodes.contains(*entity)),
            |(entity, tier), _| {
                if *tier == Tier::Full
                    && nodes
                        .get(*entity)
                        .is_ok_and(|(_, node)| node.dataset.lifetime.is_live())
                {
                    evicted += 1;
                }
            },
        );
        cache.evicted_pages += evicted;
        cache.geometry.retain_keys(
            selections
                .sources()
                .filter_map(|(entity, tier)| nodes.get(entity).ok().map(|(_, node)| (node, tier)))
                .map(|(node, tier)| (Arc::as_ptr(&node.page.appearance.geometry) as usize, tier)),
            |_, _| {},
        );
        cache.appearance.retain_keys(
            selections
                .sources()
                .filter_map(|(entity, tier)| nodes.get(entity).ok().map(|(_, node)| (node, tier)))
                .map(|(node, tier)| (Arc::as_ptr(&node.page.appearance) as usize, tier)),
            |_, _| {},
        );
        cache.styles.retain_keys(
            selections
                .needed()
                .filter_map(|entity| nodes.get(entity).ok())
                .map(|(_, node)| Arc::as_ptr(&node.dataset) as usize),
            |_, _| {},
        );
        let _ = cache.pages.compact(budget);
        let _ = cache.geometry.compact(budget);
        let _ = cache.appearance.compact(budget);
        let _ = cache.styles.compact(budget);
        let _ = cache.opaque.compact(budget);
        let _ = cache.transparent.compact(budget);
    }
    pub fn step(
        &mut self,
        item: u64,
        remaining: &mut u64,
        staging: &mut List<(GpuCharge, Charge)>,
    ) -> Result<(), Issue> {
        let entity = Entity::from_bits(item);
        for tier in [Tier::Coarse, Tier::Full] {
            if self
                .selections
                .sources()
                .any(|source| source == (entity, tier))
            {
                self.step_tier(entity, tier, remaining, staging)?;
            }
        }
        Ok(())
    }

    fn step_tier(
        &mut self,
        entity: Entity,
        tier: Tier,
        remaining: &mut u64,
        staging: &mut List<(GpuCharge, Charge)>,
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
            ..
        } = self;
        let shared = shared.as_deref().ok_or(Issue::Deferred)?;
        let device = &**device;
        let queue = &**queue;
        let pipeline = &**pipeline;
        let pipelines = &**pipelines;
        let lut = images.get(&shared.lut).ok_or(Issue::Deferred)?;
        let budget = shared.backend.budget();
        let ledger = shared.backend.gpu().clone();
        let (entity, node) = nodes.get(entity).map_err(|_| Issue::Deferred)?;
        let page_key = (entity, tier);
        let geometry = (Arc::as_ptr(&node.page.appearance.geometry) as usize, tier);
        let appearance = (Arc::as_ptr(&node.page.appearance) as usize, tier);
        let style = Arc::as_ptr(&node.dataset) as usize;
        let layout = match tier {
            Tier::Full => node.page.appearance.attributes.layout,
            Tier::Coarse => node.page.appearance.coarse.attributes.layout,
        };
        // Pages without attribute words bind their positions in the unread attribute slot.
        let attributes_bound = layout.words() > 0;
        for kind in 0..2 {
            if kind == 1 && !attributes_bound {
                continue;
            }
            let (key, owner) = if kind == 0 {
                (
                    geometry,
                    match tier {
                        Tier::Full => Owner::Geometry(node.page.appearance.geometry.clone()),
                        Tier::Coarse => {
                            Owner::CoarseGeometry(node.page.appearance.geometry.metadata.clone())
                        }
                    },
                )
            } else {
                (
                    appearance,
                    match tier {
                        Tier::Full => Owner::Appearance(node.page.appearance.clone()),
                        Tier::Coarse => {
                            Owner::CoarseAppearance(node.page.appearance.coarse.clone())
                        }
                    },
                )
            };
            let table = if kind == 0 {
                &mut cache.geometry
            } else {
                &mut cache.appearance
            };
            if !table.contains_key(&key) {
                if *remaining < 4 {
                    return Err(Issue::Deferred);
                }
                table.reserve(budget, 1).map_err(|_| Issue::CpuBudget)?;
                let buffer = allocate(
                    &device,
                    &queue,
                    &ledger,
                    budget,
                    owner.bytes().len() as u64,
                    BufferUsages::STORAGE,
                    "cloud records",
                )?;
                table
                    .insert(
                        budget,
                        key,
                        Storage {
                            buffer,
                            written: 0,
                            owner,
                        },
                    )
                    .map_err(|_| Issue::CpuBudget)?;
            }
            let storage = table.get_mut(&key).unwrap();
            let end =
                (storage.written + *remaining).min(storage.owner.bytes().len() as u64) / 4 * 4;
            if end > storage.written {
                write(
                    &queue,
                    &ledger,
                    budget,
                    remaining,
                    &storage.buffer.buffer,
                    storage.written,
                    &storage.owner.bytes()[storage.written as usize..end as usize],
                    staging,
                )?;
                #[cfg(feature = "draw-probes")]
                if let Some(probe) = &shared.probe {
                    probe.uploaded(
                        match (kind, tier) {
                            (0, Tier::Full) => super::probe::UploadKind::Positions,
                            (_, Tier::Full) => super::probe::UploadKind::Attributes,
                            (0, Tier::Coarse) => super::probe::UploadKind::CoarsePositions,
                            (_, Tier::Coarse) => super::probe::UploadKind::CoarseAttributes,
                        },
                        (end - storage.written) as usize,
                    );
                }
                storage.written = end;
            }
            if storage.written != storage.owner.bytes().len() as u64 {
                return Err(Issue::Deferred);
            }
        }
        if !cache.styles.contains_key(&style) {
            cache
                .styles
                .reserve(budget, 1)
                .map_err(|_| Issue::CpuBudget)?;
            let value = Style::new(&node.dataset.generation.descriptor().presentation);
            let bytes = owned_uniform(budget, &value).map_err(|_| Issue::CpuBudget)?;
            if *remaining < bytes.len() as u64 {
                return Err(Issue::Deferred);
            }
            let cpu = super::worker::charge(budget, size_of::<StyleBuffer>() + bytes.len() + 512)
                .map_err(|_| Issue::CpuBudget)?;
            let buffer = allocate(
                &device,
                &queue,
                &ledger,
                budget,
                bytes.len() as u64,
                BufferUsages::UNIFORM,
                "cloud style",
            )?;
            write(
                &queue,
                &ledger,
                budget,
                remaining,
                &buffer.buffer,
                0,
                &bytes,
                staging,
            )?;
            #[cfg(feature = "draw-probes")]
            if let Some(probe) = &shared.probe {
                probe.uploaded(super::probe::UploadKind::Styles, bytes.len());
            }
            let group = device.create_bind_group(
                "cloud style",
                &pipelines.get_bind_group_layout(&pipeline.style),
                &BindGroupEntries::sequential((
                    buffer.buffer.as_entire_binding(),
                    &lut.texture_view,
                    &lut.sampler,
                )),
            );
            cache
                .styles
                .insert(
                    budget,
                    style,
                    StyleBuffer {
                        buffer,
                        style: value,
                        generation: node.dataset.clone(),
                        group: Some(group),
                        _cpu: cpu,
                    },
                )
                .map_err(|_| Issue::CpuBudget)?;
        }
        if !cache.pages.contains_key(&page_key) {
            cache
                .pages
                .reserve(budget, 1)
                .map_err(|_| Issue::CpuBudget)?;
        }
        let geometry_buffer = cache
            .geometry
            .get(&geometry)
            .filter(|storage| storage.written == storage.owner.bytes().len() as u64);
        let appearance_buffer = if attributes_bound {
            cache
                .appearance
                .get(&appearance)
                .filter(|storage| storage.written == storage.owner.bytes().len() as u64)
        } else {
            geometry_buffer
        };
        let (Some(geometry_buffer), Some(appearance_buffer)) = (geometry_buffer, appearance_buffer)
        else {
            return Err(Issue::Deferred);
        };
        let frame = &node.page.scan.value().frame;
        let capture_ns = frame
            .capture
            .as_ref()
            .and_then(|capture| capture.source_time_ns)
            .unwrap_or(0);
        let current_ns = node
            .dataset
            .source_clocks
            .time_ns(frame.capture.as_ref())
            .unwrap_or(capture_ns);
        let age = super::age::AgeUniform::new(
            &node.dataset.generation.descriptor().presentation.color,
            frame.capture.as_ref(),
            node.simulation_s,
            current_ns,
        );
        let coverage = match tier {
            Tier::Full => 0.0,
            Tier::Coarse => super::coarse::coverage_radius_m(&node.page.appearance.geometry.metadata),
        };
        let value = PageUniform {
            world_from_local: node.world_from_local,
            clip_rotation: node.page.clip.rotation,
            clip_min: node.page.clip.min,
            clip_max: node.page.clip.max,
            age,
            coverage: Vec4::new(coverage, 0.0, 0.0, 0.0),
            attributes: layout.uniform(),
        };
        let bytes = owned_uniform(budget, &value).map_err(|_| Issue::CpuBudget)?;
        if !cache.pages.contains_key(&page_key) {
            if *remaining < bytes.len() as u64 {
                return Err(Issue::Deferred);
            }
            let buffer = allocate(
                &device,
                &queue,
                &ledger,
                budget,
                bytes.len() as u64,
                BufferUsages::UNIFORM,
                "cloud placement",
            )?;
            let group = device.create_bind_group(
                "cloud page",
                &pipelines.get_bind_group_layout(&pipeline.page),
                &BindGroupEntries::sequential((
                    buffer.buffer.as_entire_binding(),
                    geometry_buffer.buffer.buffer.as_entire_binding(),
                    appearance_buffer.buffer.buffer.as_entire_binding(),
                )),
            );
            cache
                .pages
                .insert(
                    budget,
                    page_key,
                    PageBuffer {
                        buffer,
                        previous: List::default(),
                        group,
                        geometry,
                        appearance: attributes_bound.then_some(appearance),
                        style,
                        count: match tier {
                            Tier::Full => node.page.appearance.geometry.positions.values.len(),
                            Tier::Coarse => {
                                node.page.appearance.geometry.coarse.positions.values.len()
                            }
                        } as u32,
                    },
                )
                .map_err(|_| Issue::CpuBudget)?;
            cache.page_loads += u64::from(tier == Tier::Full);
        }
        let page = cache.pages.get_mut(&page_key).unwrap();
        if page.previous.as_slice() != bytes.as_slice() {
            write(
                &queue,
                &ledger,
                budget,
                remaining,
                &page.buffer.buffer,
                0,
                &bytes,
                staging,
            )?;
            #[cfg(feature = "draw-probes")]
            if let Some(probe) = &shared.probe {
                probe.uploaded(super::probe::UploadKind::Placements, bytes.len());
            }
            page.previous = bytes;
        }
        Ok(())
    }

    pub fn step_selection(
        &mut self,
        key: super::upload::Key,
        item: u64,
        remaining: &mut u64,
        staging: &mut List<(GpuCharge, Charge)>,
    ) -> Result<(), Issue> {
        let shared = self.shared.as_deref().ok_or(Issue::Deferred)?;
        self.selections.upload(
            key,
            item,
            &self.cache,
            shared,
            &self.device,
            &self.queue,
            &self.pipeline,
            &self.pipelines,
            remaining,
            staging,
        )
    }
}
#[cfg(feature = "draw-probes")]
fn observe_allocations(
    cache: Res<Cache>,
    selections: Res<selection::Views>,
    shared: Option<Res<Shared>>,
    upload: Res<UploadFrame>,
) {
    let Some(shared) = shared else { return };
    if let Some(probe) = &shared.probe {
        selections.observe(probe);
        let usage = shared.backend.gpu().usage();
        probe.allocations(super::probe::Allocations {
            demanded_position_bytes: selections.demanded_position_bytes(),
            page_loads: cache.page_loads,
            evicted_pages: cache.evicted_pages,
            indices: selections.index_bytes(),
            selected_points: selections.selected_points(),
            positions: cache
                .geometry
                .values()
                .map(|storage| storage.buffer.buffer.size())
                .sum(),
            attributes: cache
                .appearance
                .values()
                .map(|storage| storage.buffer.buffer.size())
                .sum(),
            coarse_positions: cache
                .geometry
                .values()
                .filter(|storage| matches!(storage.owner, Owner::CoarseGeometry(_)))
                .map(|storage| storage.buffer.buffer.size())
                .sum(),
            coarse_attributes: cache
                .appearance
                .values()
                .filter(|storage| matches!(storage.owner, Owner::CoarseAppearance(_)))
                .map(|storage| storage.buffer.buffer.size())
                .sum(),
            coarse_pages: cache
                .pages
                .values()
                .filter(|page| page.geometry.1 == Tier::Coarse)
                .count() as u64,
            coarse_points: cache
                .pages
                .values()
                .filter(|page| page.geometry.1 == Tier::Coarse)
                .map(|page| u64::from(page.count))
                .sum(),
            styles: cache
                .styles
                .values()
                .map(|style| style.buffer.buffer.size())
                .sum(),
            placements: cache
                .pages
                .values()
                .map(|page| page.buffer.buffer.size())
                .sum(),
            points: cache.pages.values().map(|page| u64::from(page.count)).sum(),
            pages: cache.pages.len() as u64,
            gpu_bytes: usage.bytes,
            auxiliary_gpu_bytes: usage.auxiliary,
            staging_gpu_bytes: usage.staging,
            retiring_gpu_bytes: usage.retiring,
            other_gpu_bytes: usage.other,
            gpu_peak: usage.peak,
            uploaded: usage.uploaded,
            frame_upload: shared.backend.limits().upload_bytes_per_frame - upload.remaining,
            max_frame_upload: 0,
        });
        probe.motion_frame(
            shared.backend.limits(),
            shared.backend.budget().usage().committed(),
        );
    }
}

struct PreparedPage {
    entity: Entity,
    coarse: bool,
    full: bool,
    main: MainEntity,
    dataset: usize,
    center: Vec3,
    ids: [Option<CachedRenderPipelineId>; 2],
    indexed_ids: [Option<CachedRenderPipelineId>; 2],
}

fn placed(nodes: &Query<(Entity, &MainEntity, &Node)>, dataset: &Dataset) -> bool {
    nodes
        .iter()
        .filter(|(_, _, node)| std::ptr::eq(node.dataset.as_ref(), dataset) && node.placed)
        .count()
        == dataset.render_pages()
}

pub(super) fn prune_phases(
    old_opaque: &mut Map<RetainedViewEntity, List<(Entity, MainEntity)>>,
    old_transparent: &mut Map<RetainedViewEntity, List<(Entity, MainEntity)>>,
    live_page: impl Fn(Entity) -> bool,
    live_view: impl Fn(&RetainedViewEntity) -> bool,
    opaque: &mut ViewBinnedRenderPhases<Opaque3d>,
    transparent: &mut ViewSortedRenderPhases<Transparent3d>,
) {
    old_opaque.retain(|view, items| {
        let Some(phase) = opaque.get_mut(view) else {
            return false;
        };
        items.retain(|(entity, main)| {
            if live_view(view) && live_page(*entity) {
                true
            } else {
                phase.remove(*main);
                false
            }
        });
        !items.is_empty()
    });
    old_transparent.retain(|view, items| {
        let Some(phase) = transparent.get_mut(view) else {
            return false;
        };
        items.retain(|(entity, main)| {
            if live_view(view) && live_page(*entity) {
                true
            } else {
                phase.remove(*entity, *main);
                false
            }
        });
        !items.is_empty()
    });
}

pub(super) fn phase_lists(
    budget: &CpuBudget,
    count: [usize; 2],
    key: RetainedViewEntity,
    old_opaque: &mut Map<RetainedViewEntity, List<(Entity, MainEntity)>>,
    old_transparent: &mut Map<RetainedViewEntity, List<(Entity, MainEntity)>>,
    opaque: &mut bevy::render::render_phase::BinnedRenderPhase<Opaque3d>,
    transparent: &mut bevy::render::render_phase::SortedRenderPhase<Transparent3d>,
) -> Result<
    (List<(Entity, MainEntity)>, List<(Entity, MainEntity)>),
    gearbox_api::draw_data::StorageError,
> {
    if count[0] > 0 && !old_opaque.contains_key(&key) {
        old_opaque.reserve(budget, 1)?;
    }
    if count[1] > 0 && !old_transparent.contains_key(&key) {
        old_transparent.reserve(budget, 1)?;
    }
    let new_opaque = phase_capacity(
        budget,
        old_opaque.get(&key).map_or(0, List::capacity),
        count[0],
    )?;
    let new_transparent = phase_capacity(
        budget,
        old_transparent.get(&key).map_or(0, List::capacity),
        count[1],
    )?;
    let mut own_opaque = old_opaque.take_retaining_capacity(&key).unwrap_or_default();
    let mut own_transparent = old_transparent
        .take_retaining_capacity(&key)
        .unwrap_or_default();
    for (_, main) in &own_opaque {
        opaque.remove(*main);
    }
    for (entity, main) in &own_transparent {
        transparent.remove(*entity, *main);
    }
    own_opaque.clear();
    own_transparent.clear();
    Ok((
        new_opaque.unwrap_or(own_opaque),
        new_transparent.unwrap_or(own_transparent),
    ))
}

fn phase_capacity(
    budget: &CpuBudget,
    current: usize,
    needed: usize,
) -> Result<Option<List<(Entity, MainEntity)>>, gearbox_api::draw_data::StorageError> {
    if needed == current {
        Ok(None)
    } else if needed > current {
        List::new(budget, needed, MemoryClass::Metadata).map(Some)
    } else {
        Ok(List::new(budget, needed, MemoryClass::Metadata).ok())
    }
}

#[allow(clippy::too_many_arguments)]
fn queue(
    mut cache: ResMut<Cache>,
    mut selections: ResMut<selection::Views>,
    shared: Option<Res<Shared>>,
    nodes: Query<(Entity, &MainEntity, &Node)>,
    views: Query<(
        &ExtractedView,
        Option<&RenderLayers>,
        &Msaa,
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
    offered: Option<Res<Offered>>,
    receipts: Res<super::receipts::Receipts>,
    mut frontiers: ResMut<Frontiers>,
) {
    let (Some(shared), Some(offered)) = (shared, offered) else {
        return;
    };
    offered.prune();
    if let Some(selection) = &frontiers.0 {
        selection.prune();
    }
    let cpu_failure = || {
        for data in offered.candidates().iter() {
            Issue::CpuBudget.report(&shared.backend, data);
        }
    };
    let Cache {
        pages,
        opaque: old_opaque,
        transparent: old_transparent,
        ..
    } = &mut *cache;
    prune_phases(
        old_opaque,
        old_transparent,
        |entity| {
            pages.contains_key(&(entity, Tier::Full)) || pages.contains_key(&(entity, Tier::Coarse))
        },
        |view| {
            receipts
                .views()
                .iter()
                .any(|registered| registered.retained == *view)
        },
        &mut opaque,
        &mut transparent,
    );
    let Ok(mut next) = Selection::new(shared.backend.budget(), receipts.views().len()) else {
        cpu_failure();
        return;
    };
    let count = nodes.iter().count();
    let Ok(mut prepared) = List::new(shared.backend.budget(), count, MemoryClass::Scratch) else {
        cpu_failure();
        return;
    };
    let admission = selection::admission(
        &shared,
        receipts.views().iter().filter_map(|registered| {
            views
                .iter()
                .find(|(view, layers, _, _, _)| {
                    view.retained_view_entity == registered.retained
                        && layers.is_some_and(|layers| {
                            layers.intersects(&RenderLayers::layer(super::super::DRAW_LAYER))
                        })
                })
                .map(|(view, _, _, _, _)| (registered.identity, view))
        }),
        receipts.views().len(),
        &nodes,
        &offered,
    );
    let admission = match admission {
        Ok(plan) => {
            let published = offered.retention.demand(plan.wanted_stamp(), || {
                super::worker::Demand::new(shared.backend.budget(), plan.wanted())
            });
            if published.is_err() {
                for data in offered.candidates().iter() {
                    super::admission::Issue::CpuBudget.report(&shared.backend, data);
                }
            }
            Some(plan)
        }
        Err(issue) => {
            for data in offered.candidates().iter() {
                issue.report(&shared.backend, data);
            }
            None
        }
    };
    for registered in receipts.views() {
        prepared.clear();
        let previous = frontiers
            .0
            .as_ref()
            .and_then(|selection| selection.find(registered.retained, registered.identity));
        let camera = views.iter().find(|(view, layers, _, _, _)| {
            view.retained_view_entity == registered.retained
                && layers.is_some_and(|layers| {
                    layers.intersects(&RenderLayers::layer(super::super::DRAW_LAYER))
                })
        });
        let phases = opaque
            .get_mut(&registered.retained)
            .zip(transparent.get_mut(&registered.retained));
        let key = keys.get(&registered.retained).copied();
        let (
            Some((view, _, msaa, camera, frost)),
            Some((opaque_phase, transparent_phase)),
            Some(key),
        ) = (camera, phases, key)
        else {
            if let Some(datasets) = DatasetSet::select(
                shared.backend.budget(),
                previous,
                &[],
                |data| offered.live(data),
                |_| false,
            )
            .ok()
            .or_else(|| previous.cloned())
            {
                next.views.push(ViewFrontier {
                    retained: registered.retained,
                    identity: registered.identity,
                    datasets,
                    preparing: selections.pending_set(registered.retained, registered.identity),
                });
            }
            continue;
        };
        #[cfg(feature = "draw-probes")]
        if let Some(probe) = &shared.probe {
            probe.view(registered.retained, registered.identity, msaa.samples());
            probe.camera(registered.retained, selection::view_stamp(view));
        }
        for (entity, main, node) in &nodes {
            if !node.placed || !offered.live(&node.dataset) {
                continue;
            }
            if node.dataset.needs_frost(msaa.samples())
                && !frost.is_some_and(|frost| {
                    frost.ready(camera.physical_target_size, view.target_format)
                })
            {
                continue;
            }
            #[cfg(feature = "draw-probes")]
            if shared
                .probe
                .as_ref()
                .is_some_and(|probe| !probe.allow(registered.retained, &node.dataset, &node.page))
            {
                continue;
            }
            let full = cache
                .pages
                .get(&(entity, Tier::Full))
                .filter(|page| !page.previous.is_empty());
            let coarse = cache
                .pages
                .get(&(entity, Tier::Coarse))
                .filter(|page| !page.previous.is_empty());
            let Some(page) = full.or(coarse) else {
                continue;
            };
            let Some(style) = cache
                .styles
                .get(&page.style)
                .filter(|style| style.group.is_some())
            else {
                continue;
            };
            let required = node
                .dataset
                .alpha
                .required(node.page.appearance.alpha, msaa.samples());
            let key = key
                | MeshPipelineKey::from_primitive_topology_and_strip_index(
                    PrimitiveTopology::TriangleList,
                    None,
                );
            let mut ids = [None; 2];
            let mut indexed_ids = [None; 2];
            for (index, id) in ids.iter_mut().enumerate() {
                if required & (1 << index) == 0 {
                    continue;
                }
                let is_opaque = index == 0;
                let frost = !is_opaque && style.style.effects.z > 0.0;
                let pipeline_id = variants.specialize(
                    &pipelines,
                    &pipeline,
                    (key, is_opaque, frost, false, shared.layout.clone()),
                );
                if pipelines.get_render_pipeline(pipeline_id).is_some() {
                    *id = Some(pipeline_id);
                }
                let indexed = variants.specialize(
                    &pipelines,
                    &pipeline,
                    (key, is_opaque, frost, true, shared.layout.clone()),
                );
                if pipelines.get_render_pipeline(indexed).is_some() {
                    indexed_ids[index] = Some(indexed);
                }
            }
            if ids.iter().flatten().count() != required.count_ones() as usize {
                continue;
            }
            prepared
                .push(PreparedPage {
                    entity,
                    full: full.is_some(),
                    coarse: coarse.is_some(),
                    main: *main,
                    dataset: Arc::as_ptr(&node.dataset) as usize,
                    center: node.world_from_local.transform_point3(Vec3::ZERO),
                    ids,
                    indexed_ids,
                })
                .expect("reserved prepared page slot");
        }
        prepared.sort_unstable_by_key(|page| page.dataset);
        let datasets = match DatasetSet::select(
            shared.backend.budget(),
            previous,
            &offered.candidates(),
            |data| offered.live(data),
            |data| placed(&nodes, data),
        ) {
            Ok(datasets) => datasets,
            Err(error) => {
                warn!("cloud frontier admission: {error}");
                let Some(previous) = previous else {
                    continue;
                };
                previous.clone()
            }
        };
        let phase_counts = std::array::from_fn(|pass| {
            prepared
                .iter()
                .filter(|page| page.ids[pass].is_some())
                .count()
        });
        let Cache {
            opaque: old_opaque,
            transparent: old_transparent,
            ..
        } = &mut *cache;
        let (mut own_opaque, mut own_transparent) = match phase_lists(
            shared.backend.budget(),
            phase_counts,
            view.retained_view_entity,
            old_opaque,
            old_transparent,
            opaque_phase,
            transparent_phase,
        ) {
            Ok(lists) => lists,
            Err(_) => {
                cpu_failure();
                if let Some(previous) = previous {
                    next.views.push(ViewFrontier {
                        retained: registered.retained,
                        identity: registered.identity,
                        datasets: previous.clone(),
                        preparing: selections.pending_set(registered.retained, registered.identity),
                    });
                }
                continue;
            }
        };
        if let Some(admission) = &admission
            && let Err(issue) = selections.update(
                &shared, registered, view, datasets, &nodes, &prepared, admission,
            )
        {
            for data in offered.candidates().iter() {
                issue.report(&shared.backend, data);
            }
        }
        let Some(datasets) = selections.active_set(registered.retained, registered.identity) else {
            if let Some(preparing) =
                selections.pending_set(registered.retained, registered.identity)
                && let Ok(datasets) = DatasetSet::select(
                    shared.backend.budget(),
                    previous,
                    &[],
                    |data| offered.live(data),
                    |_| false,
                )
            {
                next.views.push(ViewFrontier {
                    retained: registered.retained,
                    identity: registered.identity,
                    datasets,
                    preparing: Some(preparing),
                });
            } else {
                selections.cancel_unpublished(registered.retained);
                cpu_failure();
            }
            continue;
        };
        let selected_data = datasets.datasets.read();
        for data in selected_data.iter() {
            if previous.is_none_or(|old| {
                !Arc::ptr_eq(old, &datasets)
                    && old
                        .datasets
                        .read()
                        .binary_search_by_key(&(Arc::as_ptr(data) as usize), |data| {
                            Arc::as_ptr(data) as usize
                        })
                        .is_err()
            }) {
                info!(
                    "typed cloud activated {}/{} incarnation={} generation={} points={} view={} epoch={}",
                    data.key().namespace,
                    data.key().id,
                    data.revision.incarnation,
                    data.revision.generation,
                    data.generation.counts().valid_samples,
                    registered.identity.id,
                    registered.identity.epoch
                );
            }
        }
        for page in &prepared {
            let Ok(_) = selected_data
                .binary_search_by_key(&page.dataset, |data| Arc::as_ptr(data) as usize)
            else {
                continue;
            };
            let Some(selected_page) = selections.page(registered.retained, page.entity) else {
                continue;
            };
            let ids = if selected_page.indexed() {
                page.indexed_ids
            } else {
                page.ids
            };
            let entity = page.entity;
            let main = page.main;
            if let Some(id) = ids[0] {
                opaque_phase.add(
                    Opaque3dBatchSetKey {
                        draw_function: opaque_draw.read().id::<DrawCloud<true>>(),
                        pipeline: id,
                        material_bind_group_index: None,
                        lightmap_slab: None,
                        slabs: MeshSlabs::default(),
                    },
                    Opaque3dBinKey {
                        asset_id: shared.mesh.id().untyped(),
                    },
                    (entity, main),
                    InputUniformIndex::default(),
                    BinnedRenderPhaseType::NonMesh,
                );
                own_opaque
                    .push((entity, main))
                    .expect("reserved opaque phase slot");
            }
            if let Some(id) = ids[1] {
                transparent_phase.add_retained(Transparent3d {
                    sorting_info: TransparentSortingInfo3d::Sorted {
                        mesh_center: page.center,
                        depth_bias: 0.0,
                    },
                    distance: 0.0,
                    entity: (entity, main),
                    pipeline: id,
                    draw_function: transparent_draw.read().id::<DrawCloud<false>>(),
                    batch_range: 0..1,
                    extra_index: PhaseItemExtraIndex::None,
                    indexed: false,
                });
                own_transparent
                    .push((entity, main))
                    .expect("reserved transparent phase slot");
            }
        }
        drop(selected_data);
        next.views.push(ViewFrontier {
            retained: registered.retained,
            identity: registered.identity,
            datasets,
            preparing: selections.pending_set(registered.retained, registered.identity),
        });
        if !own_opaque.is_empty() {
            cache
                .opaque
                .insert(
                    shared.backend.budget(),
                    view.retained_view_entity,
                    own_opaque,
                )
                .expect("reserved opaque view slot");
        }
        if !own_transparent.is_empty() {
            cache
                .transparent
                .insert(
                    shared.backend.budget(),
                    view.retained_view_entity,
                    own_transparent,
                )
                .expect("reserved transparent view slot");
        }
    }
    let selection = Arc::new(next);
    offered.retention.select(Some(selection.clone()));
    frontiers.0 = Some(selection);
}

type DrawCloud<const OPAQUE: bool> = (
    SetItemPipeline,
    SetMeshViewBindGroup<0>,
    SetMeshViewBindingArrayBindGroup<1>,
    DrawPage<OPAQUE>,
);
struct DrawPage<const OPAQUE: bool>;
impl<P: PhaseItem, const OPAQUE: bool> RenderCommand<P> for DrawPage<OPAQUE> {
    type Param = (
        SRes<Cache>,
        SRes<super::receipts::Receipts>,
        SRes<selection::Views>,
    );
    type ViewQuery = (
        &'static ExtractedView,
        &'static Msaa,
        Option<&'static FrostView>,
    );
    type ItemQuery = ();
    fn render<'w>(
        item: &P,
        (view, msaa, frost): (&'w ExtractedView, &'w Msaa, Option<&'w FrostView>),
        _: Option<()>,
        (cache, receipts, selections): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let cache = cache.into_inner();
        let Some(selected) = selections
            .into_inner()
            .page(view.retained_view_entity, item.entity())
        else {
            return RenderCommandResult::Skip;
        };
        let Some(page) = cache
            .pages
            .get(&(item.entity(), selected.tier))
            .filter(|page| page.count > 0)
        else {
            return RenderCommandResult::Skip;
        };
        let Some(style) = cache.styles.get(&page.style) else {
            return RenderCommandResult::Skip;
        };
        if !style.generation.lifetime.is_live() {
            return RenderCommandResult::Skip;
        }
        if style.generation.needs_frost(msaa.samples()) && frost.is_none() {
            return RenderCommandResult::Skip;
        }
        let Some(group) = style.group.as_ref() else {
            return RenderCommandResult::Skip;
        };
        if !selected.dataset.lifetime.is_live() {
            return RenderCommandResult::Skip;
        }
        pass.set_bind_group(2, selected.group.as_ref().unwrap_or(&page.group), &[]);
        pass.set_bind_group(3, group, &[]);
        pass.draw(0..6, selected.range.clone());
        receipts.into_inner().drawn(
            view.retained_view_entity,
            item.entity(),
            if OPAQUE {
                super::alpha::OPAQUE
            } else {
                super::alpha::TRANSMITTING
            },
        );
        RenderCommandResult::Success
    }
}
