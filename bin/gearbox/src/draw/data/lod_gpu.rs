//! The per-view GPU level-of-detail pass of compact clouds: its buffers, pipelines and the
//! dispatches that list each full page's points before the main pass draws them indirectly.

use std::num::NonZeroU64;

use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::render::render_resource::binding_types::{
    storage_buffer_read_only_sized, storage_buffer_sized, uniform_buffer,
};
use bevy::render::renderer::{RenderContext, ViewQuery};

use super::*;

/// Stride of one page's head in a view's heads buffer, at storage offset alignment.
pub(in crate::draw::data) const HEAD_STRIDE: u64 = 256;
/// Head copies a view keeps in flight for receipts before it skips a frame's counts.
const READBACKS: usize = 4;
/// Bytes of a head: indirect draw and accept dispatch arguments, then what was selected.
const HEAD_BYTES: u64 = 64;
/// Bytes of one page's node table: (first instance, refinement-order position) per node.
pub(super) const TABLE_BYTES: u64 = 4096;
const BINS_BYTES: u64 = 656;
const WORKGROUP: u32 = 256;
/// Screen error a node may show before it is refined.
const ERROR_PX: f32 = 1.5;

#[derive(Clone, Copy, ShaderType)]
struct LodView {
    clip_from_world: Mat4,
    viewport: Vec2,
    error_px: f32,
    budget: u32,
}

/// A page's head as the LOD pass leaves it: its indirect draw, then what it selected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub(in crate::draw::data) struct HeadCounts {
    pub draw: [u32; 4],
    pub nodes: u32,
    pub selected: u32,
    pub represented: u32,
    pub accepted: u32,
    pub chosen: u32,
    pub pad: [u32; 7],
}

/// Bytes of a page's accept dispatch arguments, each at its own offset-aligned slot.
const DISPATCH_BYTES: u64 = 16;

#[derive(Resource)]
pub(in crate::draw::data) struct LodPipelines {
    view: BindGroupLayoutDescriptor,
    pub(super) page: BindGroupLayoutDescriptor,
    pub(super) slot: BindGroupLayoutDescriptor,
    pub(super) accept_slot: BindGroupLayoutDescriptor,
    measure: CachedComputePipelineId,
    choose: CachedComputePipelineId,
    emit: CachedComputePipelineId,
    accept: CachedComputePipelineId,
}

pub(super) fn init(mut commands: Commands, assets: Res<AssetServer>, cache: Res<PipelineCache>) {
    let view = BindGroupLayoutDescriptor::new(
        "cloud lod view",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                uniform_buffer::<LodView>(false),
                storage_buffer_sized(false, NonZeroU64::new(BINS_BYTES)),
            ),
        ),
    );
    let page = BindGroupLayoutDescriptor::new(
        "cloud lod page",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                uniform_buffer::<PageUniform>(false),
                storage_buffer_read_only_sized(false, None),
                storage_buffer_read_only_sized(false, None),
                storage_buffer_read_only_sized(false, None),
            ),
        ),
    );
    let slot = BindGroupLayoutDescriptor::new(
        "cloud lod slot",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                storage_buffer_sized(false, NonZeroU64::new(HEAD_BYTES)),
                storage_buffer_sized(false, NonZeroU64::new(TABLE_BYTES)),
                storage_buffer_sized(false, NonZeroU64::new(DISPATCH_BYTES)),
            ),
        ),
    );
    // Accept reads its dispatch arguments indirectly, so it binds the slot without them.
    let accept_slot = BindGroupLayoutDescriptor::new(
        "cloud lod accept slot",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                storage_buffer_sized(false, NonZeroU64::new(HEAD_BYTES)),
                storage_buffer_sized(false, NonZeroU64::new(TABLE_BYTES)),
            ),
        ),
    );
    let shader = assets.load("embedded://gearbox/draw/draw_cloud_lod.wesl");
    let pipeline = |entry: &'static str, slot: &BindGroupLayoutDescriptor| {
        cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: Some(format!("cloud lod {entry}").into()),
            layout: vec![view.clone(), page.clone(), slot.clone()],
            shader: shader.clone(),
            entry_point: Some(entry.into()),
            zero_initialize_workgroup_memory: true,
            ..default()
        })
    };
    commands.insert_resource(LodPipelines {
        measure: pipeline("measure_nodes", &slot),
        choose: pipeline("choose", &slot),
        emit: pipeline("emit", &slot),
        accept: pipeline("accept", &accept_slot),
        view,
        page,
        slot,
        accept_slot,
    });
}

/// One view's level-of-detail buffers: a slot per full page it draws.
pub(in crate::draw::data) struct ViewLod {
    pub capacity: u32,
    uniform: OwnedBuffer,
    bins: OwnedBuffer,
    pub heads: OwnedBuffer,
    pub tables: OwnedBuffer,
    dispatches: OwnedBuffer,
    group: BindGroup,
    /// Mappable copies of `heads` free for the next frame, and how many exist.
    readbacks: std::sync::Arc<std::sync::Mutex<Vec<OwnedBuffer>>>,
    created: std::sync::atomic::AtomicUsize,
    /// Heads as the last resolved frame read them back, per slot.
    pub last: std::sync::Arc<std::sync::Mutex<Vec<HeadCounts>>>,
}

impl ViewLod {
    pub fn new(
        device: &RenderDevice,
        queue: &RenderQueue,
        shared: &Shared,
        pipelines: &LodPipelines,
        cache: &PipelineCache,
        capacity: u32,
    ) -> Result<Self, Issue> {
        let budget = shared.backend.budget();
        let ledger = shared.backend.gpu();
        let capacity = capacity.max(1).next_power_of_two();
        let uniform = allocate(device, queue, ledger, budget, LodView::min_size().get(), BufferUsages::UNIFORM | BufferUsages::COPY_DST, "cloud lod view")?;
        let bins = allocate(device, queue, ledger, budget, BINS_BYTES, BufferUsages::STORAGE | BufferUsages::COPY_DST, "cloud lod bins")?;
        let heads = allocate(
            device,
            queue,
            ledger,
            budget,
            u64::from(capacity) * HEAD_STRIDE,
            BufferUsages::STORAGE | BufferUsages::INDIRECT | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
            "cloud lod heads",
        )?;
        let tables = allocate(device, queue, ledger, budget, u64::from(capacity) * TABLE_BYTES, BufferUsages::STORAGE, "cloud lod tables")?;
        let dispatches = allocate(
            device,
            queue,
            ledger,
            budget,
            u64::from(capacity) * HEAD_STRIDE,
            BufferUsages::STORAGE | BufferUsages::INDIRECT,
            "cloud lod dispatches",
        )?;
        let group = device.create_bind_group(
            "cloud lod view",
            &cache.get_bind_group_layout(&pipelines.view),
            &BindGroupEntries::sequential((uniform.buffer.as_entire_binding(), bins.buffer.as_entire_binding())),
        );
        Ok(Self {
            capacity,
            uniform,
            bins,
            heads,
            tables,
            dispatches,
            group,
            readbacks: Default::default(),
            created: Default::default(),
            last: Default::default(),
        })
    }

    /// A free mappable copy of `heads`, allocating one while fewer than `READBACKS` exist.
    fn readback(&self, device: &RenderDevice, queue: &RenderQueue, shared: &Shared) -> Option<OwnedBuffer> {
        use std::sync::atomic::Ordering;
        if let Some(buffer) = self.readbacks.lock().unwrap().pop() {
            return Some(buffer);
        }
        if self.created.fetch_add(1, Ordering::AcqRel) >= READBACKS {
            self.created.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        allocate(
            device,
            queue,
            shared.backend.gpu(),
            shared.backend.budget(),
            u64::from(self.capacity) * HEAD_STRIDE,
            BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            "cloud lod readback",
        )
        .ok()
    }

    /// The slot's head, table and accept dispatch, as the LOD pass and vertex stage bind them.
    pub fn slot(&self, slot: u32) -> [BufferBinding<'_>; 3] {
        let at = |buffer, stride, size| BufferBinding {
            buffer,
            offset: u64::from(slot) * stride,
            size: NonZeroU64::new(size),
        };
        [
            at(&self.heads.buffer, HEAD_STRIDE, HEAD_BYTES),
            at(&self.tables.buffer, TABLE_BYTES, TABLE_BYTES),
            at(&self.dispatches.buffer, HEAD_STRIDE, DISPATCH_BYTES),
        ]
    }

    pub fn write_view(&self, queue: &RenderQueue, view: &ExtractedView, budget: u32) {
        let clip_from_world = view.clip_from_world.unwrap_or_else(|| {
            view.clip_from_view * Mat4::from(view.world_from_view.affine().inverse())
        });
        let value = LodView {
            clip_from_world,
            viewport: Vec2::new(view.viewport.z as f32, view.viewport.w as f32),
            error_px: ERROR_PX,
            budget,
        };
        let mut bytes = bevy::render::render_resource::encase::UniformBuffer::new(Vec::new());
        bytes.write(&value).expect("cloud lod view layout");
        queue.write_buffer(&self.uniform.buffer, 0, bytes.as_ref());
    }
}

/// Selects every full page the view draws: measure all, choose a threshold, list, count.
#[allow(clippy::too_many_arguments)]
pub(super) fn select(
    view: ViewQuery<&ExtractedView>,
    selections: Res<selection::Views>,
    cache: Res<Cache>,
    pipelines: Res<LodPipelines>,
    pipeline_cache: Res<PipelineCache>,
    queue: Res<RenderQueue>,
    device: Res<RenderDevice>,
    shared: Option<Res<Shared>>,
    receipts: Res<super::super::receipts::Receipts>,
    mut ctx: RenderContext,
) {
    let view = view.into_inner();
    let (Some((lod, pages)), Some(shared)) = (selections.lod_pages(view.retained_view_entity), shared) else {
        return;
    };
    let (Some(measure), Some(choose), Some(emit), Some(accept)) = (
        pipeline_cache.get_compute_pipeline(pipelines.measure),
        pipeline_cache.get_compute_pipeline(pipelines.choose),
        pipeline_cache.get_compute_pipeline(pipelines.emit),
        pipeline_cache.get_compute_pipeline(pipelines.accept),
    ) else {
        return;
    };
    let ready: Vec<_> = pages
        .filter_map(|page| {
            let slot = page.slot?;
            let group = page.lod_group.as_ref()?;
            let accept = page.accept_group.as_ref()?;
            let source = cache.pages.get(&(page.entity, Tier::Full))?;
            Some((slot, group, source.lod.as_ref()?, source.nodes, accept))
        })
        .collect();
    if ready.is_empty() {
        return;
    }
    lod.write_view(&queue, view, shared.backend.limits().visible_points);
    let encoder = ctx.command_encoder();
    encoder.clear_buffer(&lod.bins.buffer, 0, None);
    let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
        label: Some("cloud lod"),
        timestamp_writes: None,
    });
    pass.set_bind_group(0, &lod.group, &[]);
    pass.set_pipeline(measure);
    for (_, slot_group, page_group, nodes, _) in &ready {
        pass.set_bind_group(1, *page_group, &[]);
        pass.set_bind_group(2, *slot_group, &[]);
        pass.dispatch_workgroups(nodes.div_ceil(WORKGROUP).max(1), 1, 1);
    }
    pass.set_pipeline(choose);
    if let Some((_, slot_group, page_group, _, _)) = ready.first() {
        pass.set_bind_group(1, *page_group, &[]);
        pass.set_bind_group(2, *slot_group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    pass.set_pipeline(emit);
    for (_, slot_group, page_group, _, _) in &ready {
        pass.set_bind_group(1, *page_group, &[]);
        pass.set_bind_group(2, *slot_group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    pass.set_pipeline(accept);
    for (slot, _, page_group, _, accept_group) in &ready {
        pass.set_bind_group(1, *page_group, &[]);
        pass.set_bind_group(2, *accept_group, &[]);
        pass.dispatch_workgroups_indirect(&lod.dispatches.buffer, u64::from(*slot) * HEAD_STRIDE);
    }
    drop(pass);
    if let Some(buffer) = lod.readback(&device, &queue, &shared) {
        encoder.copy_buffer_to_buffer(&lod.heads.buffer, 0, &buffer.buffer, 0, lod.heads.buffer.size());
        receipts.lod_readback(super::super::receipts::LodReadback {
            view: view.retained_view_entity,
            buffer,
            pool: lod.readbacks.clone(),
            last: lod.last.clone(),
        });
    }
}

pub(super) fn install(app: &mut App) {
    app.sub_app_mut(RenderApp)
        .add_systems(RenderStartup, init)
        .add_systems(Core3d, select.before(Core3dSystems::MainPass));
}
