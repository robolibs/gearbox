use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::{render_resource::TextureViewId, texture::ManualTextureViews};
use mara::ui::mara_gpu::MaraRenderState;
use wgpu::util::DeviceExt;

const KEY: &str = "gearbox_draw_frost";

#[derive(Clone)]
struct Backdrop {
    textures: [egui::TextureId; 4],
    viewport: egui::Rect,
    uv: egui::Rect,
}

#[derive(Default)]
pub(super) struct DrawFrost {
    pipeline: Option<Pipeline>,
    targets: Option<Targets>,
}

struct Pipeline {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    passes: [wgpu::RenderPipeline; 5],
}

struct Targets {
    source: TextureViewId,
    size: UVec2,
    scale: f32,
    views: [wgpu::TextureView; 5],
    bind_groups: [wgpu::BindGroup; 2],
    textures: [egui::TextureId; 4],
    release: Box<dyn Fn() + Send + Sync>,
}

impl Drop for Targets {
    fn drop(&mut self) { (self.release)(); }
}

fn reduced_size(size: UVec2) -> UVec2 {
    UVec2::new(size.x.div_ceil(4).clamp(1, 1024), size.y.div_ceil(4).clamp(1, 1024))
}

fn cover_uv(size: UVec2, viewport: egui::Rect) -> egui::Rect {
    let texture_aspect = size.x as f32 / size.y.max(1) as f32;
    let view_aspect = viewport.width() / viewport.height().max(1.0);
    let visible = if view_aspect > texture_aspect { egui::vec2(1.0, texture_aspect / view_aspect) }
        else { egui::vec2(view_aspect / texture_aspect, 1.0) };
    egui::Rect::from_center_size(egui::pos2(0.5, 0.5), visible)
}

impl DrawFrost {
    pub(super) fn update(&mut self, ctx: &egui::Context, gpu: Option<MaraRenderState<'_>>, viewport: egui::Rect, world: &mut World, controls: bool) {
        ctx.data_mut(|data| data.remove::<Backdrop>(egui::Id::new(KEY)));
        let panels = world.get_resource::<crate::draw::DrawPanels>().is_some_and(|panels|
            panels.0.iter().any(|panel| panel.opacity > 0.0 && panel.chart.color[3] > 0));
        let labels = world.get_resource::<crate::draw::DrawLabels>().is_some_and(|labels|
            labels.0.iter().any(|label| (label.annotation.selected || (label.annotation.blur.is_some() && label.background[3] > 0))
                && label.opacity > 0.0 && label.color[3] > 0));
        if !(panels || labels || controls) || viewport.width() <= 0.0 || viewport.height() <= 0.0 {
            self.targets = None;
            return;
        }
        let Some(gpu) = gpu else { self.targets = None; return; };
        let target = world.query::<(&Camera, &RenderTarget)>().iter(world)
            .find_map(|(camera, target)| match target {
                RenderTarget::TextureView(handle) if camera.is_active => Some(*handle), _ => None,
            });
        let Some(source) = target.and_then(|handle| world.get_resource::<ManualTextureViews>()?.get(&handle).cloned()) else {
            self.targets = None;
            return;
        };
        let gpu = gpu.__internal_raw();
        let scale = ctx.pixels_per_point();
        let pipeline = self.pipeline.get_or_insert_with(|| Pipeline::new(&gpu.device));
        if self.targets.as_ref().is_none_or(|targets| targets.source != source.texture_view.id() || targets.size != source.size || targets.scale != scale) {
            self.targets = None;
            let (views, output_views, bind_groups) = pipeline.targets(&gpu.device, &source.texture_view, source.size, scale);
            let textures = output_views.map(|view| gpu.renderer.write().register_native_texture(&gpu.device, &view, wgpu::FilterMode::Linear));
            let renderer = gpu.renderer.clone();
            self.targets = Some(Targets { source: source.texture_view.id(), size: source.size, scale, views, bind_groups, textures,
                release: Box::new(move || { let mut renderer = renderer.write(); for id in textures { renderer.free_texture(&id); } }),
            });
        }
        let targets = self.targets.as_ref().unwrap();
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("draw frost") });
        pipeline.render(&mut encoder, &targets.views, &targets.bind_groups);
        gpu.queue.submit([encoder.finish()]);
        ctx.data_mut(|data| data.insert_temp(egui::Id::new(KEY), Backdrop {
            textures: targets.textures, viewport, uv: cover_uv(source.size, viewport),
        }));
    }
}

impl Pipeline {
    fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("draw frost"), source: wgpu::ShaderSource::Wgsl(include_str!("draw_frost.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("draw frost"), entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("draw frost"), bind_group_layouts: &[Some(&layout)], immediate_size: 0,
        });
        let passes = ["horizontal", "dark", "light", "dark_sharp", "light_sharp"].map(|entry| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(entry), layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vertex"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some(entry), compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: if entry == "horizontal" { wgpu::TextureFormat::Rgba16Float } else { wgpu::TextureFormat::Rgba8Unorm }, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
            primitive: Default::default(), depth_stencil: None, multisample: Default::default(), multiview_mask: None, cache: None,
        }));
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("draw frost"),
            mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        Self { layout, sampler, passes }
    }

    fn targets(&self, device: &wgpu::Device, source: &wgpu::TextureView, size: UVec2, scale: f32)
        -> ([wgpu::TextureView; 5], [wgpu::TextureView; 4], [wgpu::BindGroup; 2]) {
        let reduced = reduced_size(size);
        let views = std::array::from_fn(|i| device.create_texture(&wgpu::TextureDescriptor {
            label: Some("draw frost"), size: wgpu::Extent3d {
                width: if i < 3 { reduced.x } else { size.x.max(1) },
                height: if i < 3 { reduced.y } else { size.y.max(1) }, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: if i == 0 { wgpu::TextureFormat::Rgba16Float } else { wgpu::TextureFormat::Rgba8Unorm },
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        }).create_view(&Default::default()));
        let bind_groups = std::array::from_fn(|axis| {
            let step = if axis == 0 { [2.5 * scale / size.x.max(1) as f32, 0.0, 0.0, 0.0] }
                else { [0.0, 2.5 * scale / size.y.max(1) as f32, 0.0, 0.0] };
            let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("draw frost step"), contents: bytemuck::cast_slice(&step), usage: wgpu::BufferUsages::UNIFORM,
            });
            device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("draw frost"), layout: &self.layout, entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(if axis == 0 { source } else { &views[0] }) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: uniform.as_entire_binding() },
            ] })
        });
        let output_views = [views[1].clone(), views[2].clone(), views[3].clone(), views[4].clone()];
        (views, output_views, bind_groups)
    }

    fn render(&self, encoder: &mut wgpu::CommandEncoder, views: &[wgpu::TextureView; 5], bind_groups: &[wgpu::BindGroup; 2]) {
        for i in 0..5 {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("draw frost"), color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &views[i], depth_slice: None, resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })], depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
            });
            pass.set_pipeline(&self.passes[i]);
            pass.set_bind_group(0, &bind_groups[usize::from(i == 1 || i == 2)], &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

pub(super) fn paint(painter: &egui::Painter, points: &[egui::Pos2], opacity: f32, light: bool, blur: f32) -> bool {
    let Some(backdrop) = painter.ctx().data(|data| data.get_temp::<Backdrop>(egui::Id::new(KEY))) else { return false; };
    let opacity = opacity.clamp(0.0, 1.0);
    let soft = opacity * blur.clamp(0.0, 1.0);
    let sharp = (opacity - soft) / (1.0 - soft).max(1e-6);
    for (slot, alpha) in [(2 + usize::from(light), sharp), (usize::from(light), soft)] {
        if alpha <= 0.0 { continue; }
        let mut mesh = egui::Mesh::with_texture(backdrop.textures[slot]);
        let tint = egui::Color32::from_white_alpha((alpha * 255.0) as u8);
        for &pos in points {
            let point = (pos - backdrop.viewport.min) / backdrop.viewport.size();
            let uv = backdrop.uv.min + point * backdrop.uv.size();
            mesh.vertices.push(egui::epaint::Vertex { pos, uv, color: tint });
        }
        for i in 1..points.len().saturating_sub(1) { mesh.add_triangle(0, i as u32, i as u32 + 1); }
        painter.with_clip_rect(painter.clip_rect().intersect(backdrop.viewport)).add(egui::Shape::mesh(mesh));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_is_bounded_and_uv_matches_viewport_cover() {
        assert_eq!(reduced_size(UVec2::new(1226, 1348)), UVec2::new(307, 337));
        assert_eq!(reduced_size(UVec2::new(20000, 20000)), UVec2::splat(1024));
        assert_eq!(reduced_size(UVec2::ZERO), UVec2::ONE);
        let viewport = egui::Rect::from_min_size(egui::pos2(25.0, 50.0), egui::vec2(800.0, 400.0));
        assert_eq!(cover_uv(UVec2::new(800, 400), viewport), egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)));
        assert_eq!(cover_uv(UVec2::splat(400), viewport), egui::Rect::from_min_max(egui::pos2(0.0, 0.25), egui::pos2(1.0, 0.75)));
    }

    #[test]
    fn panel_polygon_samples_the_corresponding_scene_region() {
        let ctx = egui::Context::default();
        let viewport = egui::Rect::from_min_size(egui::pos2(20.0, 40.0), egui::vec2(800.0, 400.0));
        let rect = egui::Rect::from_min_max(egui::pos2(220.0, 140.0), egui::pos2(420.0, 240.0));
        let points = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
        for light in [false, true] {
            let output = crate::host::run_ui_clean(&ctx, egui::RawInput::default(), |ui| {
                ctx.data_mut(|data| data.remove::<Backdrop>(egui::Id::new(KEY)));
                assert!(!paint(ui.painter(), &points, 1.0, light, 1.0));
                ctx.data_mut(|data| data.insert_temp(egui::Id::new(KEY), Backdrop {
                    textures: [11, 12, 13, 14].map(egui::TextureId::User), viewport,
                    uv: cover_uv(UVec2::new(800, 400), viewport),
                }));
                assert!(paint(ui.painter(), &points, 0.5, light, 1.0));
            });
            let meshes: Vec<_> = output.shapes.iter().filter_map(|s| match &s.shape { egui::Shape::Mesh(m) => Some(m), _ => None }).collect();
            assert_eq!(meshes.len(), 1);
            assert_eq!(meshes[0].texture_id, egui::TextureId::User(if light { 12 } else { 11 }));
            assert_eq!(meshes[0].vertices[0].uv, egui::pos2(0.25, 0.25));
            assert_eq!(meshes[0].vertices[2].uv, egui::pos2(0.5, 0.5));
            assert!(meshes[0].vertices.iter().all(|v| v.color.a() == 127));
            assert_eq!(meshes[0].indices, [0, 1, 2, 0, 2, 3]);
        }
    }

    #[test]
    fn blur_mixes_clear_and_frost_without_changing_combined_opacity() {
        let ctx = egui::Context::default();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 400.0));
        let points = [viewport.left_top(), viewport.right_top(), viewport.right_bottom(), viewport.left_bottom()];
        for opacity in [0.25, 0.5, 1.0] {
            for blur in [0.0, 0.5, 1.0] {
                let output = crate::host::run_ui_clean(&ctx, egui::RawInput::default(), |ui| {
                    ctx.data_mut(|data| data.insert_temp(egui::Id::new(KEY), Backdrop {
                        textures: [11, 12, 13, 14].map(egui::TextureId::User), viewport,
                        uv: egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    }));
                    assert!(paint(ui.painter(), &points, opacity, false, blur));
                });
                let meshes: Vec<_> = output.shapes.iter().filter_map(|shape| match &shape.shape { egui::Shape::Mesh(mesh) => Some(mesh), _ => None }).collect();
                let alpha = meshes.iter().fold(0.0, |previous, mesh| {
                    let a = mesh.vertices[0].color.a() as f32 / 255.0;
                    a + previous * (1.0 - a)
                });
                assert!((alpha - opacity).abs() < 0.008);
                if blur == 0.0 { assert_eq!(meshes[0].texture_id, egui::TextureId::User(13)); }
                if blur == 1.0 { assert_eq!(meshes[0].texture_id, egui::TextureId::User(11)); }
            }
        }
    }

    #[test]
    #[ignore = "requires GPU; run via make test-draw-frost"]
    fn gpu_resize_and_disable_release_registered_backdrops() {
        use bevy::camera::ManualTextureViewHandle;
        use bevy::render::texture::ManualTextureView;
        let config = egui_wgpu::WgpuConfiguration::default();
        let instance = pollster::block_on(config.wgpu_setup.new_instance());
        let state = pollster::block_on(egui_wgpu::RenderState::create(&config, &instance, None, Default::default())).unwrap();
        let ctx = egui::Context::default();
        let mut world = World::new();
        world.init_resource::<ManualTextureViews>();
        let handle = ManualTextureViewHandle(941);
        let camera = world.spawn((Camera::default(), RenderTarget::TextureView(handle))).id();
        let mut frost = DrawFrost::default();
        let viewport = egui::Rect::from_min_size(egui::pos2(20.0, 30.0), egui::vec2(800.0, 500.0));
        let mut previous = Vec::new();
        for (width, height, scale) in [(256, 128, 1.0), (256, 128, 1.0), (128, 256, 1.0), (513, 127, 1.5), (256, 128, 2.0)] {
            let size = UVec2::new(width, height);
            let source = state.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("resized frost source"), size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[],
            });
            world.resource_mut::<ManualTextureViews>().insert(handle,
                ManualTextureView::with_default_format(source.create_view(&Default::default()).into(), size));
            ctx.set_pixels_per_point(scale);
            let _ = crate::host::run_ui_clean(&ctx, Default::default(), |_| {});
            frost.update(&ctx, Some(MaraRenderState::__internal_new(&state)), viewport, &mut world, true);
            let targets = frost.targets.as_ref().unwrap();
            let ids = targets.textures;
            assert_eq!(targets.scale, scale);
            assert_eq!(targets.views[0].texture().size().width, reduced_size(size).x);
            assert_eq!(targets.views[0].texture().size().height, reduced_size(size).y);
            assert!(ids.iter().all(|id| state.renderer.read().texture(id).is_some()));
            assert!(previous.iter().all(|id| state.renderer.read().texture(id).is_none()));
            let backdrop = ctx.data(|data| data.get_temp::<Backdrop>(egui::Id::new(KEY))).unwrap();
            assert_eq!(backdrop.textures, ids);
            assert_eq!(backdrop.uv, cover_uv(size, viewport));
            for _ in 0..8 {
                frost.update(&ctx, Some(MaraRenderState::__internal_new(&state)), viewport, &mut world, true);
                assert_eq!(frost.targets.as_ref().unwrap().textures, ids);
            }
            previous.extend(ids);
        }
        for reason in ["inactive-camera", "hidden", "no-gpu", "zero-size", "missing-source"] {
            frost.update(&ctx, Some(MaraRenderState::__internal_new(&state)), viewport, &mut world, true);
            let ids = frost.targets.as_ref().unwrap().textures;
            if reason == "inactive-camera" { world.get_mut::<Camera>(camera).unwrap().is_active = false; }
            if reason == "missing-source" { world.resource_mut::<ManualTextureViews>().remove(&handle); }
            let gpu = (reason != "no-gpu").then(|| MaraRenderState::__internal_new(&state));
            let area = if reason == "zero-size" { egui::Rect::ZERO } else { viewport };
            frost.update(&ctx, gpu, area, &mut world, reason != "hidden");
            assert!(frost.targets.is_none(), "{reason}");
            assert!(ids.iter().all(|id| state.renderer.read().texture(id).is_none()), "{reason}");
            assert!(ctx.data(|data| data.get_temp::<Backdrop>(egui::Id::new(KEY))).is_none());
            world.get_mut::<Camera>(camera).unwrap().is_active = true;
        }
        state.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    }

    #[test]
    #[ignore = "requires GPU; run via make test-draw-frost"]
    fn gpu_blurs_live_scene_without_flattening_transmitted_colors() {
        let engine = molla_compute::Engine::new_gpu().unwrap();
        let device = engine.device().unwrap();
        let queue = engine.queue().unwrap();
        let source = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("frost fixture"), size: wgpu::Extent3d { width: 256, height: 256, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[],
        });
        let pipeline = Pipeline::new(device);
        let (views, _, bindings) = pipeline.targets(device, &source.create_view(&Default::default()), UVec2::splat(256), 1.0);
        let bytes_per_output = 64 * 256;
        let readback = device.create_buffer(&wgpu::BufferDescriptor { label: Some("frost readback"),
            size: bytes_per_output * 2, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        for pattern in 0..3 {
            let pixels: Vec<u8> = (0..256).flat_map(|y| (0..256).flat_map(move |x| {
                if pattern == 2 { return if x < 128 { [0, 255, 0, 255] } else { [255, 0, 255, 255] }; }
                let value = if if pattern == 0 { (x / 2 + y / 2) % 2 == 0 } else { x >= 128 } { 255 } else { 0 };
                [value, value, value, 255]
            })).collect();
            queue.write_texture(source.as_image_copy(), &pixels, wgpu::TexelCopyBufferLayout {
                offset: 0, bytes_per_row: Some(1024), rows_per_image: Some(256),
            }, source.size());
            let mut encoder = device.create_command_encoder(&Default::default());
            pipeline.render(&mut encoder, &views, &bindings);
            for i in 0..2 {
                encoder.copy_texture_to_buffer(views[i + 1].texture().as_image_copy(), wgpu::TexelCopyBufferInfo {
                    buffer: &readback, layout: wgpu::TexelCopyBufferLayout {
                        offset: i as u64 * bytes_per_output, bytes_per_row: Some(256), rows_per_image: Some(64),
                    },
                }, views[i + 1].texture().size());
            }
            queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            readback.slice(..).map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            rx.recv().unwrap().unwrap();
            let bytes = readback.slice(..).get_mapped_range().expect("mapped GPU buffer range");
            for (i, output) in bytes.chunks_exact(bytes_per_output as usize).enumerate() {
                assert!(output.chunks_exact(4).all(|pixel| pixel[3] == 255));
                let at = |x: usize, y: usize| output[(y * 64 + x) * 4];
                if pattern == 0 {
                    let values: Vec<_> = (8..56).flat_map(|y| (8..56).map(move |x| at(x, y))).collect();
                    assert!(values.iter().max().unwrap() - values.iter().min().unwrap() < 5);
                } else if pattern == 1 {
                    assert!(at(10, 32) <= 2);
                    assert!(at(50, 32) >= if i == 0 { 190 } else { 253 });
                    assert!(at(31, 32) > at(10, 32) + 2 && at(31, 32) < at(50, 32) - 2);
                } else {
                    let green = &output[(32 * 64 + 10) * 4..][..4];
                    let magenta = &output[(32 * 64 + 50) * 4..][..4];
                    assert!(green[0] <= 2 && green[1] >= 190 && green[2] <= 2);
                    assert!(magenta[0] >= 190 && magenta[1] <= 2 && magenta[2] >= 190);
                }
            }
            drop(bytes);
            readback.unmap();
        }
    }
}
