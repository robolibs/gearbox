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
/// Bytes of the threshold histogram, cleared every frame, then the chosen bin it keeps.
const DIFF_BYTES: u64 = 161 * 4;
const BINS_BYTES: u64 = 656;
/// Bytes of one page's node states, four bits per node, at storage offset alignment.
const STATE_BYTES: u64 = 256;
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

/// What one frame's LOD pass listed: the page it ran in each slot and the heads it left.
#[derive(Clone, Default)]
pub(in crate::draw::data) struct Listed {
    pub pages: Vec<(Entity, u32)>,
    pub heads: Vec<HeadCounts>,
}

impl Listed {
    /// The head of every page the pass ran.
    pub fn heads(&self) -> impl Iterator<Item = &HeadCounts> {
        self.pages.iter().filter_map(|(_, slot)| self.heads.get(*slot as usize))
    }
}

/// Node table entries per slot.
const NODES: usize = (TABLE_BYTES / 8) as usize;

/// Serial of the last node-table copy any view's LOD pass made.
static COPIES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A view's node tables as one frame's LOD pass left them, read back for picking.
pub(in crate::draw::data) struct Tables {
    pub serial: u64,
    pub listed: Listed,
    /// (first instance, refinement-order position) of every node, `NODES` per slot.
    pub entries: Vec<[u32; 2]>,
}

impl Tables {
    /// The ordinals the pass listed for `entity`'s page, read through its refinement `order`.
    pub fn ordinals<'a>(&'a self, entity: Entity, order: &'a [u32]) -> Option<impl Iterator<Item = u32> + 'a> {
        let (_, slot) = self.listed.pages.iter().find(|(page, _)| *page == entity)?;
        let head = self.listed.heads.get(*slot as usize)?;
        let first = *slot as usize * NODES;
        let table = self.entries.get(first..first + (head.nodes as usize).min(NODES))?;
        Some(table.iter().enumerate().flat_map(move |(node, &[offset, start])| {
            let next = table.get(node + 1).map_or(head.selected, |entry| entry[0]);
            (start..start + next.saturating_sub(offset)).filter_map(|at| order.get(at as usize).copied())
        }))
    }
}

/// A view's node tables copied for a pick, and where they go once read.
pub(in crate::draw::data) struct TablesCopy {
    pub buffer: OwnedBuffer,
    pub serial: u64,
    pub pool: std::sync::Arc<std::sync::Mutex<Option<OwnedBuffer>>>,
    pub last: std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<Tables>>>>,
}

/// Bytes of a page's accept dispatch arguments, each at its own offset-aligned slot.
const DISPATCH_BYTES: u64 = 16;

/// Threshold bins of the LOD pass: bin `b` is an error of 2^(b/4 - 8) px.
const BINS: u32 = 160;

/// First threshold bin at or above `error_px`, as the LOD pass bins thresholds.
fn bin_of(error_px: f32) -> u32 {
    if error_px <= (-8.0f32).exp2() {
        return 0;
    }
    ((error_px.log2() + 8.0) * 4.0).ceil().clamp(0.0, BINS as f32) as u32
}

/// The screen error in pixels a page's chosen threshold bin allows.
pub(in crate::draw::data) fn threshold_px(chosen: u32) -> f64 {
    (f64::from(chosen) / 4.0 - 8.0).exp2()
}

/// Whether the point budget made a view choose a coarser threshold than its error asks.
pub(in crate::draw::data) fn budget_limited(chosen: u32) -> bool {
    chosen > bin_of(ERROR_PX)
}

/// GPU bytes of one view's LOD buffers and readbacks with a slot for each of `pages`.
pub(in crate::draw::data) fn view_bytes(pages: usize) -> u64 {
    let slots = pages.max(1).checked_next_power_of_two().unwrap_or(usize::MAX) as u64;
    let per_slot = HEAD_STRIDE * (2 + READBACKS as u64) + STATE_BYTES + 2 * TABLE_BYTES;
    (LodView::min_size().get() + BINS_BYTES).saturating_add(slots.saturating_mul(per_slot))
}

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
                storage_buffer_sized(false, NonZeroU64::new(STATE_BYTES)),
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
    /// Each slot's node states from the frame before.
    pub states: OwnedBuffer,
    group: BindGroup,
    /// Mappable copies of `heads` free for the next frame.
    readbacks: std::sync::Arc<std::sync::Mutex<Vec<OwnedBuffer>>>,
    /// The pages and heads of the last frame read back.
    pub last: std::sync::Arc<std::sync::Mutex<Listed>>,
    /// A pick asked for the node tables; the free mappable copy and the last tables read.
    want_tables: std::sync::atomic::AtomicBool,
    pick_copy: std::sync::Arc<std::sync::Mutex<Option<OwnedBuffer>>>,
    tables_read: std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<Tables>>>>,
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
        let tables = allocate(device, queue, ledger, budget, u64::from(capacity) * TABLE_BYTES, BufferUsages::STORAGE | BufferUsages::COPY_SRC, "cloud lod tables")?;
        let dispatches = allocate(
            device,
            queue,
            ledger,
            budget,
            u64::from(capacity) * HEAD_STRIDE,
            BufferUsages::STORAGE | BufferUsages::INDIRECT,
            "cloud lod dispatches",
        )?;
        let states = allocate(
            device,
            queue,
            ledger,
            budget,
            u64::from(capacity) * STATE_BYTES,
            BufferUsages::STORAGE | BufferUsages::COPY_DST,
            "cloud lod states",
        )?;
        let readbacks = (0..READBACKS)
            .map(|_| {
                allocate(
                    device,
                    queue,
                    ledger,
                    budget,
                    heads.buffer.size(),
                    BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                    "cloud lod readback",
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let pick_copy = allocate(
            device,
            queue,
            ledger,
            budget,
            tables.buffer.size(),
            BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            "cloud lod pick tables",
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
            states,
            group,
            readbacks: std::sync::Arc::new(std::sync::Mutex::new(readbacks)),
            last: Default::default(),
            want_tables: Default::default(),
            pick_copy: std::sync::Arc::new(std::sync::Mutex::new(Some(pick_copy))),
            tables_read: Default::default(),
        })
    }

    /// GPU bytes of these buffers and all their readbacks, free or in flight.
    pub fn bytes(&self) -> u64 {
        [&self.uniform, &self.bins, &self.heads, &self.tables, &self.dispatches, &self.states]
            .iter()
            .map(|buffer| buffer.buffer.size())
            .sum::<u64>()
            + READBACKS as u64 * self.heads.buffer.size()
            + self.tables.buffer.size()
    }

    /// The node tables read back after the copy serial in `asked`, which the first call sets;
    /// until they arrive, asks the next LOD pass to copy them.
    pub fn pick_tables(&self, asked: &mut Option<u64>) -> Option<std::sync::Arc<Tables>> {
        use std::sync::atomic::Ordering;
        let after = *asked.get_or_insert_with(|| COPIES.load(Ordering::Acquire));
        let tables = self.tables_read.lock().unwrap().clone().filter(|tables| tables.serial > after);
        if tables.is_none() {
            self.want_tables.store(true, Ordering::Release);
        }
        tables
    }

    /// The slot's head, table, accept dispatch and node states, as the LOD pass and vertex
    /// stage bind them.
    pub fn slot(&self, slot: u32) -> [BufferBinding<'_>; 4] {
        let at = |buffer, stride, size| BufferBinding {
            buffer,
            offset: u64::from(slot) * stride,
            size: NonZeroU64::new(size),
        };
        [
            at(&self.heads.buffer, HEAD_STRIDE, HEAD_BYTES),
            at(&self.tables.buffer, TABLE_BYTES, TABLE_BYTES),
            at(&self.dispatches.buffer, HEAD_STRIDE, DISPATCH_BYTES),
            at(&self.states.buffer, STATE_BYTES, STATE_BYTES),
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
            let page_group = source.lod.as_ref()?;
            let fresh = page.fresh.swap(false, std::sync::atomic::Ordering::Relaxed);
            Some((slot, group, page_group, source.nodes, accept, fresh, page.entity))
        })
        .collect();
    if ready.is_empty() {
        return;
    }
    lod.write_view(&queue, view, shared.backend.limits().visible_points);
    let encoder = ctx.command_encoder();
    encoder.clear_buffer(&lod.bins.buffer, 0, Some(DIFF_BYTES));
    // A slot handed to another page starts from unrefined node states.
    for (slot, .., fresh, _) in &ready {
        if *fresh {
            encoder.clear_buffer(&lod.states.buffer, u64::from(*slot) * STATE_BYTES, Some(STATE_BYTES));
        }
    }
    let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
        label: Some("cloud lod"),
        timestamp_writes: None,
    });
    pass.set_bind_group(0, &lod.group, &[]);
    pass.set_pipeline(measure);
    for (_, slot_group, page_group, nodes, ..) in &ready {
        pass.set_bind_group(1, *page_group, &[]);
        pass.set_bind_group(2, *slot_group, &[]);
        pass.dispatch_workgroups(nodes.div_ceil(WORKGROUP).max(1), 1, 1);
    }
    pass.set_pipeline(choose);
    if let Some((_, slot_group, page_group, ..)) = ready.first() {
        pass.set_bind_group(1, *page_group, &[]);
        pass.set_bind_group(2, *slot_group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    pass.set_pipeline(emit);
    for (_, slot_group, page_group, ..) in &ready {
        pass.set_bind_group(1, *page_group, &[]);
        pass.set_bind_group(2, *slot_group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    pass.set_pipeline(accept);
    for (slot, _, page_group, _, accept_group, ..) in &ready {
        pass.set_bind_group(1, *page_group, &[]);
        pass.set_bind_group(2, *accept_group, &[]);
        pass.dispatch_workgroups_indirect(&lod.dispatches.buffer, u64::from(*slot) * HEAD_STRIDE);
    }
    drop(pass);
    let free = lod.readbacks.lock().unwrap().pop();
    if let Some(buffer) = free {
        encoder.copy_buffer_to_buffer(&lod.heads.buffer, 0, &buffer.buffer, 0, lod.heads.buffer.size());
        // A waiting pick also gets this frame's node tables.
        let wanted = lod.want_tables.load(std::sync::atomic::Ordering::Acquire);
        let copy = wanted.then(|| lod.pick_copy.lock().unwrap().take()).flatten();
        let tables = copy.map(|copy| {
            encoder.copy_buffer_to_buffer(&lod.tables.buffer, 0, &copy.buffer, 0, lod.tables.buffer.size());
            lod.want_tables.store(false, std::sync::atomic::Ordering::Release);
            TablesCopy {
                buffer: copy,
                serial: COPIES.fetch_add(1, std::sync::atomic::Ordering::AcqRel) + 1,
                pool: lod.pick_copy.clone(),
                last: lod.tables_read.clone(),
            }
        });
        receipts.lod_readback(super::super::receipts::LodReadback {
            view: view.retained_view_entity,
            buffer,
            pages: ready.iter().map(|(slot, .., entity)| (*entity, *slot)).collect(),
            pool: lod.readbacks.clone(),
            last: lod.last.clone(),
            tables,
        });
    }
}

pub(super) fn install(app: &mut App) {
    app.sub_app_mut(RenderApp)
        .add_systems(RenderStartup, init)
        .add_systems(Core3d, select.before(Core3dSystems::MainPass));
}
