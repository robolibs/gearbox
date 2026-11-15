//! Multi-scale Eye-Dome Lighting for opaque cloud points. After the opaque pass, each view
//! that draws a shaded cloud redraws its opaque pages depth-only into level 0 of a dome
//! texture, pulls the nearest splat of each 2x2 block into the half and quarter levels,
//! then a full-screen pass multiplies the scene by every point pixel's mixed shade.

use std::sync::Arc;

use bevy::camera::{MainPassResolutionOverride, Viewport};
use bevy::core_pipeline::{
    FullscreenShader,
    core_3d::{CORE_3D_DEPTH_FORMAT, main_opaque_pass_3d},
    schedule::{Core3d, Core3dSystems},
};
use bevy::pbr::{MeshPipelineKey, MeshViewBindGroup, ViewKeyCache, main_transmissive_pass_3d};
use bevy::prelude::*;
use bevy::render::{
    Render, RenderApp, RenderStartup, RenderSystems,
    camera::ExtractedCamera,
    render_resource::*,
    renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery},
    view::{
        ExtractedView, ViewDepthStencilTexture, ViewTarget, ViewUniform, ViewUniformOffset,
        ViewUniforms,
    },
};
use gearbox_api::draw_data::{Charge, DrawingBackend, GpuCharge, GpuClass, MemoryClass};

use super::super::super::frost::FrostView;
use super::{Cache, PageDraw, Pipeline, Shared, Variant, selection};

/// Hardware depth and Eye-Dome strength of the opaque splat drawn at each pixel.
pub(in crate::draw::data) const FORMAT: TextureFormat = TextureFormat::Rg32Float;
/// Full, half and quarter resolution.
pub(in crate::draw::data) const LEVELS: u32 = 3;
const TEXEL_BYTES: u64 = 8;
const DEPTH_BYTES: u64 = 4;

pub(super) fn install(app: &mut App) {
    app.sub_app_mut(RenderApp)
        .init_resource::<SpecializedRenderPipelines<Composite>>()
        .add_systems(RenderStartup, init)
        .add_systems(Render, prepare.in_set(RenderSystems::PrepareBindGroups))
        .add_systems(
            Core3d,
            shade
                .after(main_opaque_pass_3d)
                .before(main_transmissive_pass_3d)
                .in_set(Core3dSystems::MainPass),
        );
}

#[derive(Resource)]
pub(super) struct Composite {
    layout: BindGroupLayoutDescriptor,
    pull_layout: BindGroupLayoutDescriptor,
    pull: CachedRenderPipelineId,
    shader: Handle<Shader>,
    fullscreen: FullscreenShader,
}

fn init(
    mut commands: Commands,
    assets: Res<AssetServer>,
    fullscreen: Res<FullscreenShader>,
    pipelines: Res<PipelineCache>,
) {
    use binding_types::*;
    let texels = || texture_2d(TextureSampleType::Float { filterable: false });
    let shader = assets.load("embedded://gearbox/draw/draw_cloud_dome.wesl");
    let pull_layout = BindGroupLayoutDescriptor::new(
        "cloud eye dome pull",
        &BindGroupLayoutEntries::with_indices(ShaderStages::FRAGMENT, ((1, texels()),)),
    );
    let pull = pipelines.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("cloud eye dome pull".into()),
        layout: vec![pull_layout.clone()],
        vertex: fullscreen.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: shader.clone(),
            entry_point: Some("pull".into()),
            targets: vec![Some(ColorTargetState {
                format: FORMAT,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
            ..default()
        }),
        ..default()
    });
    commands.insert_resource(Composite {
        layout: BindGroupLayoutDescriptor::new(
            "cloud eye dome",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (uniform_buffer::<ViewUniform>(true), texels()),
            ),
        ),
        pull_layout,
        pull,
        shader,
        fullscreen: fullscreen.clone(),
    });
}

impl SpecializedRenderPipeline for Composite {
    type Key = (TextureFormat, u32);
    fn specialize(&self, (format, samples): Self::Key) -> RenderPipelineDescriptor {
        RenderPipelineDescriptor {
            label: Some("cloud eye dome".into()),
            layout: vec![self.layout.clone()],
            vertex: self.fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                entry_point: Some("fragment".into()),
                targets: vec![Some(ColorTargetState {
                    format,
                    blend: Some(multiply()),
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            depth_stencil: Some(scene_depth()),
            multisample: MultisampleState {
                count: samples,
                ..default()
            },
            ..default()
        }
    }
}

/// Scales the scene colour by the shade and keeps its alpha.
pub(in crate::draw::data) fn multiply() -> BlendState {
    BlendState {
        color: BlendComponent {
            src_factor: BlendFactor::Zero,
            dst_factor: BlendFactor::Src,
            operation: BlendOperation::Add,
        },
        alpha: BlendComponent {
            src_factor: BlendFactor::Zero,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::Add,
        },
    }
}

/// Shades a sample only where nothing nearer than the point was drawn.
pub(in crate::draw::data) fn scene_depth() -> DepthStencilState {
    DepthStencilState {
        format: CORE_3D_DEPTH_FORMAT,
        depth_write_enabled: Some(false),
        depth_compare: Some(CompareFunction::GreaterEqual),
        stencil: default(),
        bias: default(),
    }
}

/// A view's dome levels and the depth buffer its redraw resolves overlapping splats with.
struct Targets {
    dome: Texture,
    /// Every level, as the composite reads them.
    pyramid: TextureView,
    /// One level each, as the redraw and pulls write and read them.
    levels: [TextureView; LEVELS as usize],
    depth: Texture,
    depth_view: TextureView,
    queue: RenderQueue,
    gpu: Option<GpuCharge>,
    _cpu: Charge,
}

impl Targets {
    fn new(
        device: &RenderDevice,
        queue: &RenderQueue,
        backend: &DrawingBackend,
        size: UVec2,
    ) -> Option<Arc<Self>> {
        let limit = device.limits().max_texture_dimension_2d;
        let least = 1 << (LEVELS - 1);
        if size.x < least || size.y < least || size.x > limit || size.y > limit {
            return None;
        }
        let texels = |level: u32| u64::from((size.x >> level) * (size.y >> level));
        let bytes = (0..LEVELS).map(texels).sum::<u64>() * TEXEL_BYTES + texels(0) * DEPTH_BYTES;
        let metadata = (size_of::<Self>() + size_of::<Arc<Self>>() + 512) as u64;
        let cpu = backend
            .budget()
            .reserve(metadata)
            .and_then(|mut r| r.charge(metadata, MemoryClass::Metadata))
            .ok()?;
        let gpu = backend.gpu().reserve_class(bytes, GpuClass::Auxiliary)?;
        let texture = |label, format, usage, mip_level_count| {
            device.create_texture(&TextureDescriptor {
                label: Some(label),
                size: Extent3d {
                    width: size.x,
                    height: size.y,
                    depth_or_array_layers: 1,
                },
                mip_level_count,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let dome = texture(
            "cloud eye dome",
            FORMAT,
            TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            LEVELS,
        );
        let depth = texture(
            "cloud eye dome depth",
            CORE_3D_DEPTH_FORMAT,
            TextureUsages::RENDER_ATTACHMENT,
            1,
        );
        Some(Arc::new(Self {
            pyramid: dome.create_view(&TextureViewDescriptor::default()),
            levels: std::array::from_fn(|level| {
                dome.create_view(&TextureViewDescriptor {
                    base_mip_level: level as u32,
                    mip_level_count: Some(1),
                    ..default()
                })
            }),
            depth_view: depth.create_view(&TextureViewDescriptor::default()),
            dome,
            depth,
            queue: queue.clone(),
            gpu: Some(gpu),
            _cpu: cpu,
        }))
    }

    fn matches(&self, size: UVec2) -> bool {
        size == UVec2::new(self.dome.width(), self.dome.height())
    }
}

impl Drop for Targets {
    fn drop(&mut self) {
        if let Some(mut gpu) = self.gpu.take() {
            gpu.retire();
            let textures = (self.dome.clone(), self.depth.clone());
            self.queue.submit([]);
            self.queue
                .on_submitted_work_done(move || drop((gpu, textures)));
        }
    }
}

#[derive(Component)]
struct DomeView {
    targets: Arc<Targets>,
    /// The depth redraw for plain and GPU-selected pages.
    depth: [CachedRenderPipelineId; 2],
    shade: CachedRenderPipelineId,
    group: BindGroup,
    /// Each coarse level's finer source.
    pulls: [BindGroup; LEVELS as usize - 1],
}

#[allow(clippy::too_many_arguments)]
fn prepare(
    mut commands: Commands,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    shared: Option<Res<Shared>>,
    cache: Res<Cache>,
    selections: Res<selection::Views>,
    keys: Res<ViewKeyCache>,
    pipeline: Res<Pipeline>,
    composite: Res<Composite>,
    pipelines: Res<PipelineCache>,
    mut clouds: ResMut<SpecializedRenderPipelines<Pipeline>>,
    mut shades: ResMut<SpecializedRenderPipelines<Composite>>,
    uniforms: Res<ViewUniforms>,
    views: Query<(Entity, &ExtractedView, &ExtractedCamera, &Msaa, Option<&DomeView>)>,
) {
    for (entity, view, camera, msaa, previous) in &views {
        let retained = view.retained_view_entity;
        let ready = (
            shared.as_ref(),
            camera.physical_target_size,
            keys.get(&retained),
            uniforms.uniforms.binding(),
        );
        let (Some(shared), Some(size), Some(key), Some(uniform)) = ready else {
            if previous.is_some() {
                commands.entity(entity).remove::<DomeView>();
            }
            continue;
        };
        let targets = super::shaded(&cache, &selections, retained)
            .then(|| {
                previous
                    .map(|old| &old.targets)
                    .filter(|targets| targets.matches(size))
                    .cloned()
                    .or_else(|| Targets::new(&device, &queue, &shared.backend, size))
            })
            .flatten();
        let Some(targets) = targets else {
            if previous.is_some() {
                commands.entity(entity).remove::<DomeView>();
            }
            continue;
        };
        let key = *key
            | MeshPipelineKey::from_primitive_topology_and_strip_index(
                PrimitiveTopology::TriangleList,
                None,
            );
        let depth = [false, true].map(|selected| {
            let variant = Variant {
                opaque: true,
                frost: false,
                selected,
                dome: true,
            };
            clouds.specialize(&pipelines, &pipeline, (key, variant, shared.layout.clone()))
        });
        let shade = shades.specialize(&pipelines, &composite, (view.target_format, msaa.samples()));
        let group = device.create_bind_group(
            "cloud eye dome",
            &pipelines.get_bind_group_layout(&composite.layout),
            &BindGroupEntries::sequential((uniform, &targets.pyramid)),
        );
        let pull_layout = pipelines.get_bind_group_layout(&composite.pull_layout);
        let pulls = std::array::from_fn(|level| {
            device.create_bind_group(
                "cloud eye dome pull",
                &pull_layout,
                &BindGroupEntries::with_indices(((1, &targets.levels[level]),)),
            )
        });
        commands.entity(entity).insert(DomeView {
            targets,
            depth,
            shade,
            group,
            pulls,
        });
    }
}

#[allow(clippy::type_complexity)]
fn shade(
    view: ViewQuery<(
        &ExtractedView,
        &ExtractedCamera,
        &Msaa,
        &ViewTarget,
        &ViewDepthStencilTexture,
        &ViewUniformOffset,
        &MeshViewBindGroup,
        Option<&FrostView>,
        Option<&MainPassResolutionOverride>,
        Option<&DomeView>,
    )>,
    cache: Res<Cache>,
    selections: Res<selection::Views>,
    composite: Res<Composite>,
    pipelines: Res<PipelineCache>,
    mut context: RenderContext,
) {
    let (view, camera, msaa, target, depth, offset, groups, frost, resolution, dome) =
        view.into_inner();
    let Some(dome) = dome else {
        return;
    };
    let (Some(plain), Some(selected), Some(pull), Some(shade)) = (
        pipelines.get_render_pipeline(dome.depth[0]),
        pipelines.get_render_pipeline(dome.depth[1]),
        pipelines.get_render_pipeline(composite.pull),
        pipelines.get_render_pipeline(dome.shade),
    ) else {
        return;
    };
    let (cache, selections) = (cache.into_inner(), selections.into_inner());
    let viewport = Viewport::from_viewport_and_override(camera.viewport.as_ref(), resolution);
    {
        let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("cloud eye dome depth"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &dome.targets.levels[0],
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(Default::default()),
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: &dome.targets.depth_view,
                depth_ops: Some(Operations {
                    load: LoadOp::Clear(0.0),
                    store: StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some(viewport) = &viewport {
            pass.set_camera_viewport(viewport);
        }
        pass.set_bind_group(0, &groups.main, &groups.main_offsets);
        pass.set_bind_group(1, &groups.binding_array, &[]);
        let retained = view.retained_view_entity;
        for (entity, _) in cache.opaque.get(&retained).into_iter().flat_map(|items| items.iter()) {
            let Some(draw) =
                PageDraw::find(cache, selections, retained, msaa.samples(), frost.is_some(), *entity)
            else {
                continue;
            };
            pass.set_render_pipeline(if draw.selected() { selected } else { plain });
            draw.issue(&mut pass);
        }
    }
    for (level, source) in dome.pulls.iter().enumerate() {
        let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("cloud eye dome pull"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &dome.targets.levels[level + 1],
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(Default::default()),
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_render_pipeline(pull);
        pass.set_bind_group(0, source, &[]);
        pass.draw(0..3, 0..1);
    }
    let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("cloud eye dome shade"),
        color_attachments: &[Some(target.get_color_attachment())],
        depth_stencil_attachment: Some(depth.get_attachment(StoreOp::Store)),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    if let Some(viewport) = &viewport {
        pass.set_camera_viewport(viewport);
    }
    pass.set_render_pipeline(shade);
    pass.set_bind_group(0, &dome.group, &[offset.offset]);
    pass.draw(0..3, 0..1);
}
