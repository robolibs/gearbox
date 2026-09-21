//! GPU heightmap tile baking. Main world queues `TileBakeRequest`s; the render
//! world runs one compute dispatch per tile, writing FBM heights into a layer
//! of the `texture_2d_array` atlas.
//!
//! Bevy 0.19: pipelines init in `RenderStartup`, the render graph is an ECS
//! schedule (`RenderGraph`) whose systems take `RenderContext` as a param, and
//! per-dispatch params use a `DynamicUniformBuffer` with dynamic offsets
//! (wgpu 29 "immediates"/push constants are off by default).

use bevy::prelude::*;
use bevy::render::{
    extract_resource::{ExtractResource, ExtractResourcePlugin},
    render_asset::RenderAssets,
    render_resource::{
        binding_types::{texture_storage_2d_array, uniform_buffer},
        *,
    },
    renderer::{RenderContext, RenderDevice, RenderGraph, RenderGraphSystems, RenderQueue},
    texture::GpuImage,
    Render, RenderApp, RenderStartup, RenderSystems,
};
use std::borrow::Cow;

use crate::config::TILE_TEXELS;

/// Field order must match `TileBakeParams` in tile_bake.wgsl (encase lays this
/// out with the same std140 rules the shader uses).
#[derive(Clone, Copy, Debug, ShaderType)]
pub struct TileBakeRequest {
    /// Face-uv of the tile's node min corner.
    pub origin: Vec2,
    /// Node extent in face-uv units.
    pub scale: f32,
    pub face: u32,
    /// Per-planet noise domain offset.
    pub seed: Vec3,
    /// Destination atlas layer.
    pub layer: u32,
    /// Per-planet noise domain frequency.
    pub freq: f32,
}

/// Filled by the quadtree in the main world, cloned into the render world each
/// frame, drained (main side) in `First` of the next frame. Each request is
/// tagged with the destination atlas, so several planets can bake in one frame.
#[derive(Resource, Clone, Default, ExtractResource)]
pub struct TileBakeQueue {
    pub requests: Vec<(AssetId<Image>, TileBakeRequest)>,
}

fn drain_main_queue(mut queue: ResMut<TileBakeQueue>) {
    queue.requests.clear();
}

/// Render-world accumulator: requests survive here until the pipeline is
/// compiled, their atlas is prepared, and they are actually dispatched.
#[derive(Resource, Default)]
struct PendingBakes(Vec<(AssetId<Image>, TileBakeRequest)>);

#[derive(Resource)]
struct TileBakePipeline {
    layout: BindGroupLayoutDescriptor,
    pipeline: CachedComputePipelineId,
}

#[derive(Resource, Default)]
struct TileBakeBuffers {
    params: DynamicUniformBuffer<TileBakeRequest>,
    offsets: Vec<u32>,
}

/// One bind group per distinct destination atlas, plus the dynamic offsets of
/// the requests that target it. `deferred` holds pending indices whose atlas
/// isn't prepared yet (they stay queued instead of being dropped).
#[derive(Resource, Default)]
struct TileBakeBindGroups {
    groups: Vec<(BindGroup, Vec<u32>)>,
    deferred: Vec<usize>,
}

fn init_pipeline(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "tile_bake_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                texture_storage_2d_array(TextureFormat::R32Float, StorageTextureAccess::WriteOnly),
                uniform_buffer::<TileBakeRequest>(true),
            ),
        ),
    );
    let pipeline = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("tile_bake_pipeline".into()),
        layout: vec![layout.clone()],
        shader: asset_server.load("embedded://gearbox_planet/../shaders/tile_bake.wgsl"),
        entry_point: Some(Cow::from("bake")),
        ..default()
    });
    commands.insert_resource(TileBakePipeline { layout, pipeline });
}

fn accumulate_and_prepare(
    mut pending: ResMut<PendingBakes>,
    queue_res: Res<TileBakeQueue>,
    mut bufs: ResMut<TileBakeBuffers>,
    device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
) {
    pending.0.extend_from_slice(&queue_res.requests);
    bufs.offsets.clear();
    if pending.0.is_empty() {
        return;
    }
    bufs.params.clear();
    // offsets[i] corresponds to pending.0[i].
    for (_, req) in &pending.0 {
        let offset = bufs.params.push(req);
        bufs.offsets.push(offset);
    }
    bufs.params.write_buffer(&device, &render_queue);
}

fn prepare_bind_groups(
    mut groups: ResMut<TileBakeBindGroups>,
    pending: Res<PendingBakes>,
    pipeline: Res<TileBakePipeline>,
    pipeline_cache: Res<PipelineCache>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    bufs: Res<TileBakeBuffers>,
    device: Res<RenderDevice>,
) {
    groups.groups.clear();
    groups.deferred.clear();
    if pending.0.is_empty() {
        return;
    }
    let Some(params_binding) = bufs.params.binding() else {
        groups.deferred.extend(0..pending.0.len());
        return;
    };
    let layout = pipeline_cache.get_bind_group_layout(&pipeline.layout);
    // Group by destination atlas (first-seen order; a handful per frame).
    let mut atlases: Vec<AssetId<Image>> = Vec::new();
    for (atlas, _) in &pending.0 {
        if !atlases.contains(atlas) {
            atlases.push(*atlas);
        }
    }
    for atlas in atlases {
        let offsets: Vec<u32> = pending
            .0
            .iter()
            .enumerate()
            .filter(|(_, (a, _))| *a == atlas)
            .map(|(i, _)| bufs.offsets[i])
            .collect();
        let Some(gpu_atlas) = gpu_images.get(atlas) else {
            // Atlas not prepared yet — keep those requests queued.
            groups.deferred.extend(
                pending
                    .0
                    .iter()
                    .enumerate()
                    .filter(|(_, (a, _))| *a == atlas)
                    .map(|(i, _)| i),
            );
            continue;
        };
        let bind_group = device.create_bind_group(
            "tile_bake_bind_group",
            &layout,
            &BindGroupEntries::sequential((&gpu_atlas.texture_view, params_binding.clone())),
        );
        groups.groups.push((bind_group, offsets));
    }
}

fn dispatch_bakes(
    mut ctx: RenderContext,
    pipeline_cache: Res<PipelineCache>,
    pipeline: Res<TileBakePipeline>,
    groups: Res<TileBakeBindGroups>,
    mut pending: ResMut<PendingBakes>,
) {
    if pending.0.is_empty() {
        return;
    }
    let Some(compute) = pipeline_cache.get_compute_pipeline(pipeline.pipeline) else {
        // Still compiling: keep every request pending for a later frame.
        return;
    };
    let workgroups = TILE_TEXELS.div_ceil(8);
    {
        let mut pass = ctx
            .command_encoder()
            .begin_compute_pass(&ComputePassDescriptor {
                label: Some("tile_bake_pass"),
                ..default()
            });
        pass.set_pipeline(compute);
        for (bind_group, offsets) in &groups.groups {
            for offset in offsets {
                pass.set_bind_group(0, bind_group, &[*offset]);
                pass.dispatch_workgroups(workgroups, workgroups, 1);
            }
        }
    }
    // Retain only the requests whose atlas wasn't ready this frame.
    let deferred: Vec<_> = groups
        .deferred
        .iter()
        .map(|&i| pending.0[i])
        .collect();
    pending.0 = deferred;
}

pub struct TileBakePlugin;

impl Plugin for TileBakePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractResourcePlugin::<TileBakeQueue>::default())
            .add_systems(First, drain_main_queue);
        let render_app = app.sub_app_mut(RenderApp);
        render_app
            .init_resource::<PendingBakes>()
            .init_resource::<TileBakeBuffers>()
            .init_resource::<TileBakeBindGroups>()
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(
                Render,
                (
                    accumulate_and_prepare.in_set(RenderSystems::PrepareResources),
                    prepare_bind_groups.in_set(RenderSystems::PrepareBindGroups),
                ),
            )
            .add_systems(RenderGraph, dispatch_bakes.in_set(RenderGraphSystems::Begin));
    }
}
