use super::super::{
    alpha::{OPAQUE, RasterState, TRANSMITTING},
    style::{Style, uniform_bytes},
};
use gearbox_api::wire::draw_data::Presentation;
use wgpu::util::DeviceExt;

#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub(super) struct Point {
    pub center: [f32; 4],
    rgba: [f32; 4],
    projection: [f32; 4],
    clip_mode: [u32; 4],
}

pub(super) fn point(depth: f32, rgba: [f32; 4]) -> Point {
    Point {
        center: [32.0, 32.0, depth, 12.4],
        rgba,
        projection: [1.0, 1.0, 64.0, 64.0],
        clip_mode: [0; 4],
    }
}

/// Resolves the `@if((A) && (!(B)))` guards of our WESL sources: the statement after
/// each guard is kept or dropped whole, and imports are removed.
fn preprocess(source: &str, opaque: bool, samples: u32) -> String {
    let flag = |name: &str| match name {
        "CLOUD_OPAQUE" => opaque,
        "CLOUD_SINGLE_SAMPLE" => samples == 1,
        "DRAW_FROST" | "VIEW_TRANSMISSION_TEXTURE" => true,
        _ => panic!("unknown shader flag {name}"),
    };
    let mut output = String::new();
    let mut lines = source.lines();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("import ") || (trimmed.starts_with("@if(") && trimmed.contains(") import ")) {
            continue;
        }
        let Some(guard) = line.trim().strip_prefix("@if(").and_then(|g| g.strip_suffix(')')) else {
            output.push_str(line);
            output.push('\n');
            continue;
        };
        let keep = guard.split("&&").all(|term| {
            let term = term.trim().trim_start_matches('(').trim_end_matches(')');
            match term.strip_prefix('!') {
                Some(name) => !flag(name.trim_start_matches('(')),
                None => flag(term),
            }
        });
        let (mut statement, mut depth) = (String::new(), 0i32);
        for part in lines.by_ref() {
            depth += part.matches(['{', '(']).count() as i32 - part.matches(['}', ')']).count() as i32;
            statement.push_str(part);
            statement.push('\n');
            if depth == 0 && part.trim_end().ends_with([';', '}', ',']) {
                break;
            }
        }
        if keep {
            output.push_str(&preprocess(&statement, opaque, samples));
        }
    }
    output
}

pub(super) fn shader(opaque: bool, samples: u32) -> String {
    let production = include_str!("../draw_cloud.wesl");
    let output = production
        .split_once("struct CloudOutput")
        .unwrap()
        .1
        .split_once("@vertex")
        .unwrap()
        .0;
    let fragment = production.split_once("@fragment").unwrap().1;
    let color = preprocess(include_str!("../draw_cloud_color.wesl"), opaque, samples)
        .replace("@group(3)", "@group(0)");
    format!(
        "{}\n{}\nstruct CloudOutput{}\n{}\n{}\n@fragment{}",
        super::definitions(),
        color,
        output,
        preprocess(include_str!("../draw_frost.wesl"), opaque, samples),
        r#"
struct RasterPoint { center: vec4<f32>, rgba: vec4<f32>, projection: vec4<f32>, clip_mode: vec4<u32> };
@group(0) @binding(1) var<storage, read> points: array<RasterPoint>;
@group(1) @binding(0) var view_transmission_texture: texture_2d<f32>;
@group(1) @binding(1) var view_transmission_sampler: sampler;
@vertex
fn vertex(@builtin(vertex_index) vertex: u32, @builtin(instance_index) index: u32) -> CloudOutput {
    let point = points[index];
    let w = point.projection.x;
    var clip = vec4((point.center.xy / 32.0 - 1.0) * vec2(1.0, -1.0) * w, point.center.z * w, w);
    if point.clip_mode.x != 0u { clip = vec4(point.center.xyz, w); }
    let projected = cloud_project_point(clip, point.projection.zw, max(-point.center.w, 0.0),
        max(point.center.w, 0.0), point.projection.y, vertex);
    var out: CloudOutput;
    out.position = projected.position;
    out.offset_px = projected.offset_px;
    out.radius_px = projected.radius_px;
    out.linear_edge = projected.linear_edge;
    out.rgba = point.rgba;
    return out;
}
"#,
        preprocess(fragment, opaque, samples)
    )
}

pub(super) fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    points: &[Point],
    samples: u32,
    opacity: f32,
    glow: f32,
    blur: f32,
    passes: u8,
) -> Vec<[f32; 4]> {
    let style = Style::new(&Presentation {
        opacity,
        glow,
        blur,
        ..Default::default()
    });
    let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("raster style"),
        contents: &uniform_bytes(&style),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let records = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("raster points"),
        contents: bytemuck::cast_slice(points),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let data_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("raster inputs"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let frost_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("raster frost"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("cloud raster test"),
        bind_group_layouts: &[Some(&data_layout), Some(&frost_layout)],
        immediate_size: 0,
    });
    let pipelines = [true, false].map(|opaque| {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("production cloud fragment"),
            source: wgpu::ShaderSource::Wgsl(shader(opaque, samples).into()),
        });
        let state = RasterState { opaque, samples };
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("production cloud raster state"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend: state.blend(),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(state.depth(wgpu::TextureFormat::Depth32Float)),
            multisample: state.multisample(),
            multiview_mask: None,
            cache: None,
        })
    });
    let extent = wgpu::Extent3d {
        width: 64,
        height: 64,
        depth_or_array_layers: 1,
    };
    let texture = |format, samples, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cloud raster target"),
            size: extent,
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let output = texture(
        wgpu::TextureFormat::Rgba16Float,
        1,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let target = texture(
        wgpu::TextureFormat::Rgba16Float,
        samples,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let depth = texture(
        wgpu::TextureFormat::Depth32Float,
        samples,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let background = texture(
        wgpu::TextureFormat::Rgba8Unorm,
        1,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    let pixels: Vec<u8> = (0..4096)
        .flat_map(|i| {
            if (i / 64 / 2 + i % 64 / 2) % 2 == 0 {
                [0, 255, 0, 255]
            } else {
                [0, 0, 255, 255]
            }
        })
        .collect();
    queue.write_texture(
        background.as_image_copy(),
        &pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(256),
            rows_per_image: Some(64),
        },
        extent,
    );
    let output = output.create_view(&Default::default());
    let target = target.create_view(&Default::default());
    let depth = depth.create_view(&Default::default());
    let background = background.create_view(&Default::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let data = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cloud raster data"),
        layout: &data_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: records.as_entire_binding(),
            },
        ],
    });
    let frost = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cloud raster backdrop"),
        layout: &frost_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&background),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cloud raster readback"),
        size: 64 * 64 * 8,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("cloud raster passes"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: if samples == 1 { &output } else { &target },
                depth_slice: None,
                resolve_target: (samples > 1).then_some(&output),
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_bind_group(0, &data, &[]);
        pass.set_bind_group(1, &frost, &[]);
        for (index, pipeline) in pipelines.iter().enumerate() {
            if passes & (1 << index) == 0 {
                continue;
            }
            pass.set_pipeline(pipeline);
            pass.draw(0..6, 0..points.len() as u32);
        }
    }
    encoder.copy_texture_to_buffer(
        output.texture().as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(512),
                rows_per_image: Some(64),
            },
        },
        extent,
    );
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    let result = bytemuck::cast_slice::<u8, [u16; 4]>(&readback.slice(..).get_mapped_range().expect("mapped GPU buffer range"))
        .iter()
        .map(|pixel| pixel.map(|v| half::f16::from_bits(v).to_f32()))
        .collect::<Vec<_>>();
    readback.unmap();
    assert!(
        result
            .iter()
            .flatten()
            .all(|v| v.is_finite() && *v >= 0.0 && *v <= 65504.0)
    );
    result
}

#[test]
#[ignore = "requires GPU; run via make test-draw-cloud-gpu"]
fn gpu_large_offscreen_discs_keep_projection_ratios_beyond_f32_pixels() {
    let engine = molla_compute::Engine::new_gpu().unwrap();
    let device = engine.device().unwrap();
    let queue = engine.queue().unwrap();
    for samples in [1, 4] {
        for sign in [-1.0, 1.0] {
            for ratio in [0.5, 1.0, 2.0] {
                let mut value = point(0.5, [1.0, 0.0, 0.0, 1.0]);
                value.center = [sign * 1e38, 0.0, 0.5, -ratio * 1e38];
                value.clip_mode[0] = 1;
                let image = render(
                    device,
                    queue,
                    &[value],
                    samples,
                    1.0,
                    0.0,
                    0.0,
                    OPAQUE | TRANSMITTING,
                );
                for (index, pixel) in image.iter().enumerate() {
                    let inside =
                        ratio > 1.0 || ratio == 1.0 && ((index % 64 >= 32) == (sign > 0.0));
                    assert!(
                        (pixel[0] - if inside { 1.0 } else { 0.0 }).abs() < 0.01,
                        "large offscreen coverage at {index}: {pixel:?}, ratio {ratio}, sign {sign}, samples {samples}"
                    );
                }
            }
        }
    }
}

#[test]
#[ignore = "requires GPU; run via make test-draw-cloud-gpu"]
fn gpu_projected_disc_sizes_edges_and_large_radii_match_pixel_oracles() {
    let engine = molla_compute::Engine::new_gpu().unwrap();
    let device = engine.device().unwrap();
    let queue = engine.queue().unwrap();
    let mut cases = 0;
    for samples in [1, 4] {
        for (center, size, w, projection, expected_radius) in [
            ([32.0, 32.0], 8.0, 1.0, 1.0, 8.0),
            ([32.0, 32.0], 8.0, 100.0, 4.0, 8.0),
            ([32.0, 32.0], -0.25, 1.0, 1.0, 8.0),
            ([32.0, 32.0], -0.25, 2.0, 1.0, 4.0),
            ([32.0, 32.0], -0.25, 1.0, -2.0, 16.0),
            ([-4.0, 32.0], 12.0, 1.0, 1.0, 12.0),
            ([68.0, 32.0], 12.0, 1.0, 1.0, 12.0),
            ([32.0, -4.0], 12.0, 1.0, 1.0, 12.0),
            ([32.0, 68.0], 12.0, 1.0, 1.0, 12.0),
            ([100.0, 32.0], 80.0, 1.0, 1.0, 80.0),
            ([32.0, 32.0], f32::MAX, 1.0, 1.0, f32::MAX),
            ([32.0, 32.0], -1e38, 1e38, 1.0, 32.0),
        ] {
            let mut value = point(0.5, [1.0, 0.0, 0.0, 1.0]);
            value.center = [center[0], center[1], 0.5, size];
            value.projection = [w, projection, 64.0, 64.0];
            let image = render(
                device,
                queue,
                &[value],
                samples,
                1.0,
                0.0,
                0.0,
                OPAQUE | TRANSMITTING,
            );
            for (index, pixel) in image.iter().enumerate() {
                let x = (index % 64) as f64 + 0.5 - f64::from(center[0]);
                let y = (index / 64) as f64 + 0.5 - f64::from(center[1]);
                let distance = x.hypot(y);
                if distance < f64::from(expected_radius) - 1.0 {
                    assert!(
                        pixel[0] > 0.99,
                        "missing interior at {index}: {pixel:?}, center {center:?}, radius {expected_radius}"
                    );
                } else if distance > f64::from(expected_radius) + 1.0 {
                    assert!(
                        pixel[0] < 0.01,
                        "excess coverage at {index}: {pixel:?}, center {center:?}, radius {expected_radius}"
                    );
                }
            }
            cases += 1;
        }
    }
    println!(
        "{{\"projected_disc_raster_cases\":{cases},\"sample_counts\":[1,4],\"pixel_oracles\":true}}"
    );
}

#[test]
#[ignore = "requires GPU; run via make test-draw-cloud-gpu"]
fn gpu_production_fragments_preserve_depth_smooth_edges_transmission_and_glow() {
    let engine = molla_compute::Engine::new_gpu().unwrap();
    let device = engine.device().unwrap();
    let queue = engine.queue().unwrap();
    let center = |pixels: &[[f32; 4]]| pixels[32 * 64 + 32];
    let check = |pixel: [f32; 4], rgb: [f32; 3]| {
        for i in 0..3 {
            assert!((pixel[i] - rgb[i]).abs() < 0.003, "{pixel:?} vs {rgb:?}");
        }
    };
    let mut cases = 0;
    for samples in [1, 4] {
        let both = OPAQUE | TRANSMITTING;
        for fractional in [false, true] {
            let near = point(0.8, [1.0, 0.0, 0.0, 1.0]);
            let far = point(0.3, [0.0, 0.0, 1.0, if fractional { 0.5 } else { 1.0 }]);
            for points in [[near, far], [far, near]] {
                check(
                    center(&render(
                        device, queue, &points, samples, 1.0, 0.0, 0.0, both,
                    )),
                    [1.0, 0.0, 0.0],
                );
                cases += 1;
            }
        }
        check(
            center(&render(
                device,
                queue,
                &[
                    point(0.8, [1.0, 0.0, 0.0, 0.25]),
                    point(0.3, [0.0, 0.0, 1.0, 1.0]),
                ],
                samples,
                1.0,
                0.0,
                0.0,
                both,
            )),
            [0.25, 0.0, 0.75],
        );
        cases += 1;
        check(
            center(&render(
                device,
                queue,
                &[
                    point(0.8, [1.0, 0.0, 0.0, 0.0]),
                    point(0.3, [0.0, 0.0, 1.0, 1.0]),
                ],
                samples,
                1.0,
                0.0,
                0.0,
                both,
            )),
            [0.0, 0.0, 1.0],
        );
        cases += 1;
        let single = [point(0.5, [1.0, 0.0, 0.0, 1.0])];
        let sharp = render(device, queue, &single, samples, 1.0, 0.0, 0.0, both);
        assert!(sharp.iter().filter(|p| p[0] > 0.01 && p[0] < 0.99).count() > 4);
        let frost = render(device, queue, &single, samples, 1.0, 0.0, 1.0, both);
        assert_eq!(
            sharp, frost,
            "opaque coverage edges must not frost the background"
        );
        cases += 2;
        let partial = render(device, queue, &single, samples, 0.5, 0.0, 0.0, both);
        check(center(&partial), [0.5, 0.0, 0.0]);
        let frosted = render(device, queue, &single, samples, 0.5, 0.0, 1.0, both);
        assert!(center(&frosted)[1] + center(&frosted)[2] > 0.1);
        cases += 2;
        let low = render(device, queue, &single, samples, 1.0, 4.0, 0.0, both);
        let high = render(device, queue, &single, samples, 1.0, 8.0, 0.0, both);
        assert!(low.iter().zip(&high).any(|(a, b)| b[0] > a[0] + 0.05));
        cases += 2;
        let huge = render(device, queue, &single, samples, 1.0, f32::MAX, 0.0, both);
        assert!(huge.iter().any(|p| p[0] > 1000.0));
        cases += 1;
        let hidden = render(device, queue, &single, samples, 0.0, f32::MAX, 1.0, both);
        assert!(hidden.iter().all(|p| p[..3] == [0.0; 3]));
        cases += 1;
    }
    println!(
        "{{\"production_raster_cases\":{cases},\"samples\":[1,4],\"hdr_readback\":true,\"opaque_depth_and_fractional_edges\":true}}"
    );
}
