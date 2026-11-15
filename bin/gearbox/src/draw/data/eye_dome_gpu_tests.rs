use super::super::{
    alpha::{OPAQUE, RasterState},
    render::dome::{FORMAT, LEVELS, multiply, scene_depth},
    style::{Style, uniform_bytes},
};
use super::raster::{Point, point, render, shader};
use bevy::math::{Mat4, Vec4};
use gearbox_api::wire::draw_data::Presentation;
use wgpu::util::DeviceExt;

const SIZE: i32 = 32;
const NEAR: f32 = 0.1;
const SURFACE_SLOPE: f32 = 0.01;
const RING: [(i32, i32); 8] = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)];

/// Bevy's infinite reverse-depth perspective: hardware depth is `NEAR / distance`.
fn depth(distance: f32) -> f32 {
    NEAR / distance
}

fn log_distance(depth: f32) -> f32 {
    (NEAR / depth).log2()
}

fn view_from_clip() -> Mat4 {
    Mat4::from_cols(
        Vec4::new(1.0, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 1.0, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 0.0, -1.0),
        Vec4::new(0.0, 0.0, NEAR, 0.0),
    )
    .inverse()
}

/// One dome level: hardware depth and strength per texel, row by row.
struct Level {
    width: i32,
    height: i32,
    texels: Vec<[f32; 2]>,
}

impl Level {
    fn at(&self, x: i32, y: i32) -> [f32; 2] {
        self.texels[(y.clamp(0, self.height - 1) * self.width + x.clamp(0, self.width - 1)) as usize]
    }

    /// The nearest splat of each 2x2 block; the last row and column take an odd remainder.
    fn pull(&self) -> Level {
        let (width, height) = (self.width / 2, self.height / 2);
        let span = |t: i32, last: i32, fine: i32| 2 * t..=if t == last { fine - 1 } else { 2 * t + 1 };
        let texels = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|(x, y)| {
                let mut nearest = [0.0; 2];
                for fy in span(y, height - 1, self.height) {
                    for fx in span(x, width - 1, self.width) {
                        let splat = self.at(fx, fy);
                        if splat[0] > nearest[0] {
                            nearest = splat;
                        }
                    }
                }
                nearest
            })
            .collect();
        Level { width, height, texels }
    }

    /// The published Eye-Dome response (Boucheny; Potree's `edl.fs`) over the 8-texel ring at
    /// `scale` pixels per texel, and the share of the ring holding splats.
    fn ring(&self, x: i32, y: i32, scale: f32, strength: f32) -> (f32, f32) {
        let own = log_distance(self.at(x, y)[0]);
        let filled: Vec<f32> = RING
            .iter()
            .map(|(dx, dy)| (x + dx, y + dy))
            .filter(|(x, y)| (0..self.width).contains(x) && (0..self.height).contains(y))
            .map(|(x, y)| self.at(x, y)[0])
            .filter(|n| *n > 0.0)
            .collect();
        let response: f32 = filled.iter().map(|n| (own - log_distance(*n)).max(0.0)).sum();
        ((-response / (8.0 * scale) * 300.0 * strength).exp(), filled.len() as f32 / 8.0)
    }
}

fn pyramid(dome: &[[f32; 2]]) -> Vec<Level> {
    let mut levels = vec![Level { width: SIZE, height: SIZE, texels: dome.to_vec() }];
    while levels.len() < LEVELS as usize {
        levels.push(levels.last().unwrap().pull());
    }
    levels
}

/// CloudCompare's full, half and quarter scale mix (weights 1, 1/2, 1/4): coarse levels are
/// sampled bilinearly, and every tap is weighted by its ring's fill and its depth agreement.
fn expected(levels: &[Level], x: i32, y: i32) -> f32 {
    let [centre, strength] = levels[0].at(x, y);
    if centre <= 0.0 || strength <= 0.0 {
        return 1.0;
    }
    let own = log_distance(centre);
    let (shade, filled) = levels[0].ring(x, y, 1.0, strength);
    let (mut dark, mut weight) = (filled * (1.0 - shade), filled);
    let bilinear = |t: f32, corner: i32| if corner == 1 { t - t.floor() } else { 1.0 - (t - t.floor()) };
    for (index, level) in levels.iter().enumerate().skip(1) {
        let scale = (1 << index) as f32;
        let at = [(x as f32 + 0.5) / scale - 0.5, (y as f32 + 0.5) / scale - 0.5];
        for (cx, cy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            let tx = (at[0].floor() as i32 + cx).clamp(0, level.width - 1);
            let ty = (at[1].floor() as i32 + cy).clamp(0, level.height - 1);
            let tap = level.at(tx, ty)[0];
            if tap <= 0.0 {
                continue;
            }
            let gap = (own - log_distance(tap)) / (SURFACE_SLOPE * scale);
            let share = bilinear(at[0], cx) * bilinear(at[1], cy) * (-0.5 * gap * gap).exp() / scale;
            let (shade, filled) = level.ring(tx, ty, scale, strength);
            dark += share * filled * (1.0 - shade);
            weight += share * filled;
        }
    }
    if weight <= 0.0 { 1.0 } else { 1.0 - dark / weight }
}

/// The production dome shader with a test `View` and a full-screen triangle.
fn production(device: &wgpu::Device) -> wgpu::ShaderModule {
    let body: String = include_str!("../draw_cloud_dome.wesl")
        .lines()
        .filter(|line| !line.trim_start().starts_with("import "))
        .map(|line| format!("{line}\n"))
        .collect();
    let source = format!(
        "struct View {{ view_from_clip: mat4x4<f32> }};\n{body}
@vertex
fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {{
    let uv = vec2(f32(index >> 1u), f32(index & 1u)) * 2.0;
    return vec4(uv * vec2(2.0, -2.0) + vec2(-1.0, 1.0), 0.0, 1.0);
}}"
    );
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production eye dome"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

fn fullscreen(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    entry: &str,
    target: wgpu::ColorTargetState,
    depth_stencil: Option<wgpu::DepthStencilState>,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry),
        layout: None,
        vertex: wgpu::VertexState { module, entry_point: Some("vertex"), compilation_options: Default::default(), buffers: &[] },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            targets: &[Some(target)],
        }),
        primitive: Default::default(),
        depth_stencil,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn attachment(view: &wgpu::TextureView, load: wgpu::LoadOp<wgpu::Color>) -> Option<wgpu::RenderPassColorAttachment<'_>> {
    Some(wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
    })
}

/// `dome` as level 0 of a mip chain whose coarser levels the production `pull` fills.
fn dome_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    module: &wgpu::ShaderModule,
    encoder: &mut wgpu::CommandEncoder,
    (width, height): (u32, u32),
    dome: &[[f32; 2]],
) -> wgpu::Texture {
    let extent = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("eye dome test"),
        size: extent,
        mip_level_count: LEVELS,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        bytemuck::cast_slice(dome),
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 8), rows_per_image: Some(height) },
        extent,
    );
    let pull = fullscreen(
        device,
        module,
        "pull",
        wgpu::ColorTargetState { format: FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL },
        None,
    );
    let level = |base_mip_level| {
        texture.create_view(&wgpu::TextureViewDescriptor { base_mip_level, mip_level_count: Some(1), ..Default::default() })
    };
    for coarse in 1..LEVELS {
        let (source, target) = (level(coarse - 1), level(coarse));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("eye dome pull"),
            layout: &pull.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&source) }],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[attachment(&target, wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT))],
            ..Default::default()
        });
        pass.set_pipeline(&pull);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
    texture
}

/// A white scene at hardware depth `scene`, multiplied by the production composite over `dome`.
fn composite(device: &wgpu::Device, queue: &wgpu::Queue, dome: &[[f32; 2]], scene: f32) -> Vec<f32> {
    let module = production(device);
    let mut encoder = device.create_command_encoder(&Default::default());
    let input = dome_texture(device, queue, &module, &mut encoder, (SIZE as u32, SIZE as u32), dome);
    let extent = wgpu::Extent3d { width: SIZE as u32, height: SIZE as u32, depth_or_array_layers: 1 };
    let texture = |format, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("eye dome test"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let color = texture(
        wgpu::TextureFormat::Rgba16Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let depth = texture(wgpu::TextureFormat::Depth32Float, wgpu::TextureUsages::RENDER_ATTACHMENT);
    let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("eye dome view"),
        contents: bytemuck::cast_slice(&view_from_clip().to_cols_array()),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let pipeline = fullscreen(
        device,
        &module,
        "fragment",
        wgpu::ColorTargetState {
            format: wgpu::TextureFormat::Rgba16Float,
            blend: Some(multiply()),
            write_mask: wgpu::ColorWrites::ALL,
        },
        Some(scene_depth()),
    );
    let input = input.create_view(&Default::default());
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("eye dome test"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&input) },
        ],
    });
    {
        let color = color.create_view(&Default::default());
        let depth = depth.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[attachment(&color, wgpu::LoadOp::Clear(wgpu::Color::WHITE))],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(scene), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
    let pixels: Vec<[f32; 4]> = read(device, queue, encoder, &color, 0, 8);
    pixels
        .iter()
        .map(|pixel| {
            assert!((pixel[0] - pixel[1]).abs() < 1e-6 && (pixel[0] - pixel[2]).abs() < 1e-6, "grey shade {pixel:?}");
            assert_eq!(pixel[3], 1.0, "the scene alpha is kept");
            pixel[0]
        })
        .collect()
}

/// Copies one mip `level` of `texture` out and decodes each pixel: Rgba16Float as `[f32; 4]`,
/// Rg32Float as `[f32; 2]` padded.
fn read(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    mut encoder: wgpu::CommandEncoder,
    texture: &wgpu::Texture,
    level: u32,
    pixel_bytes: u32,
) -> Vec<[f32; 4]> {
    let extent = texture.size().mip_level_size(level, wgpu::TextureDimension::D2);
    let row = (extent.width * pixel_bytes).next_multiple_of(256);
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("eye dome readback"),
        size: u64::from(row * extent.height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture, mip_level: level, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(extent.height) },
        },
        extent,
    );
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback.slice(..).map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    let bytes = readback.slice(..).get_mapped_range().expect("mapped GPU buffer range").to_vec();
    readback.unmap();
    let float = texture.format() == FORMAT;
    (0..extent.height as usize)
        .flat_map(|y| (0..extent.width as usize).map(move |x| (y, x)))
        .map(|(y, x)| {
            let at = y * row as usize + x * pixel_bytes as usize;
            let pixel = &bytes[at..at + pixel_bytes as usize];
            if float {
                let [depth, strength] = bytemuck::pod_read_unaligned::<[f32; 2]>(pixel);
                [depth, strength, 0.0, 0.0]
            } else {
                bytemuck::pod_read_unaligned::<[u16; 4]>(pixel).map(|v| half::f16::from_bits(v).to_f32())
            }
        })
        .collect()
}

fn scene(texel: impl Fn(i32, i32) -> [f32; 2]) -> Vec<[f32; 2]> {
    (0..SIZE * SIZE).map(|index| texel(index % SIZE, index / SIZE)).collect()
}

/// A far wall at 20 m with a square 1% nearer on its left half; the right half of the wall
/// is unshaded and the bottom row is empty.
fn wall() -> Vec<[f32; 2]> {
    scene(|x, y| {
        if y == SIZE - 1 {
            [0.0, 0.0]
        } else if (8..16).contains(&x) && (8..24).contains(&y) {
            [depth(19.8), 0.5]
        } else {
            [depth(20.0), if x < 16 { 0.5 } else { 0.0 }]
        }
    })
}

/// Lone points every 4 pixels, too sparse for any pixel's ring to hold one: a wall at 20 m
/// with its right half on a surface at 16 m.
fn sparse() -> Vec<[f32; 2]> {
    scene(|x, y| match (x % 4, y % 4) {
        (0, 0) => [depth(if x < 16 { 20.0 } else { 16.0 }), 1.0],
        _ => [0.0, 0.0],
    })
}

fn assert_matches_reference(dome: &[[f32; 2]], shaded: &[f32]) {
    let levels = pyramid(dome);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (got, want) = (shaded[(y * SIZE + x) as usize], expected(&levels, x, y));
            assert!((got - want).abs() < 2e-3, "pixel ({x}, {y}): {got} against {want}");
        }
    }
}

#[test]
#[ignore = "requires GPU; run via make test-draw-cloud-gpu"]
fn gpu_eye_dome_darkens_points_only_behind_nearer_neighbours() {
    let engine = molla_compute::Engine::new_gpu().unwrap();
    let (device, queue) = (engine.device().unwrap(), engine.queue().unwrap());
    let dome = wall();
    let shaded = composite(device, queue, &dome, depth(20.0));
    assert_matches_reference(&dome, &shaded);
    let at = |x: i32, y: i32| shaded[(y * SIZE + x) as usize];
    assert!(at(7, 16) < 0.7, "the wall beside the square darkens: {}", at(7, 16));
    assert!(at(7, 7) > at(7, 16), "the wall off the square's corner darkens less than beside it");
    assert!(at(7, 16) < at(5, 16) && at(5, 16) < 1.0, "the coarse levels spread the outline: {}", at(5, 16));
    for (x, y, what) in [
        (12, 16, "inside the square"),
        (17, 16, "unshaded wall beside the square"),
        (1, 1, "flat wall"),
        (2, 30, "wall above empty pixels"),
        (2, 31, "empty pixels"),
    ] {
        assert_eq!(at(x, y), 1.0, "{what} keeps its colour");
    }
}

#[test]
#[ignore = "requires GPU; run via make test-draw-cloud-gpu"]
fn gpu_eye_dome_shades_sparse_points_through_the_coarse_levels() {
    let engine = molla_compute::Engine::new_gpu().unwrap();
    let (device, queue) = (engine.device().unwrap(), engine.queue().unwrap());
    let dome = sparse();
    let shaded = composite(device, queue, &dome, 0.0);
    assert_matches_reference(&dome, &shaded);
    let at = |x: i32, y: i32| shaded[(y * SIZE + x) as usize];
    for y in (0..SIZE).step_by(4) {
        assert!(at(12, y) < 0.5, "points beside the nearer surface darken: ({}, {y}) {}", 12, at(12, y));
        assert!(at(16, y) > 0.999, "the nearer surface takes no shade from the far one: {}", at(16, y));
        assert_eq!(at(8, y), 1.0, "points away from the step keep their colour");
        assert_eq!(at(13, y), 1.0, "empty pixels keep their colour");
    }
}

#[test]
#[ignore = "requires GPU; run via make test-draw-cloud-gpu"]
fn gpu_eye_dome_leaves_samples_drawn_nearer_than_the_point() {
    let engine = molla_compute::Engine::new_gpu().unwrap();
    let (device, queue) = (engine.device().unwrap(), engine.queue().unwrap());
    let dome = wall();
    let open = composite(device, queue, &dome, 0.0);
    for scene in [19.9, 1.0] {
        let hidden = composite(device, queue, &dome, depth(scene));
        for (index, (got, [point, _])) in hidden.iter().zip(&dome).enumerate() {
            let (want, what) = if *point >= depth(scene) { (open[index], "shades as unhidden") } else { (1.0, "keeps its colour") };
            assert_eq!(*got, want, "pixel {index} behind a surface at {scene} m {what}");
        }
    }
    assert!(open.iter().any(|shade| *shade < 1.0), "the wall is shaded where nothing hides it");
}

#[test]
#[ignore = "requires GPU; run via make test-draw-cloud-gpu"]
fn gpu_dome_pull_keeps_the_nearest_splat_of_each_block() {
    let engine = molla_compute::Engine::new_gpu().unwrap();
    let (device, queue) = (engine.device().unwrap(), engine.queue().unwrap());
    let (width, height) = (9, 7);
    let fine = Level {
        width,
        height,
        texels: (0..width * height)
            .map(|index| {
                let (x, y) = (index % width, index / width);
                match (x, y) {
                    (2..=3, 2..=3) => [0.0, 0.0],
                    (8, 5) => [0.9, 0.25],
                    _ => [0.01 + 0.001 * ((index * 37) % 23) as f32, 0.5 + 0.01 * (index % 7) as f32],
                }
            })
            .collect(),
    };
    let module = production(device);
    let mut levels = vec![fine];
    for _ in 1..LEVELS {
        levels.push(levels.last().unwrap().pull());
    }
    for (index, want) in levels.iter().enumerate().skip(1) {
        let mut encoder = device.create_command_encoder(&Default::default());
        let texture = dome_texture(device, queue, &module, &mut encoder, (width as u32, height as u32), &levels[0].texels);
        let got: Vec<[f32; 2]> = read(device, queue, encoder, &texture, index as u32, 8).iter().map(|t| [t[0], t[1]]).collect();
        assert_eq!(got, want.texels, "level {index}");
    }
    assert_eq!((levels[1].width, levels[1].height), (4, 3));
    assert_eq!(levels[1].at(1, 1), [0.0, 0.0], "an empty block stays empty");
    assert_eq!(levels[1].at(3, 2), [0.9, 0.25], "the last texel takes the odd column and row");
    assert_eq!(levels[2].at(1, 0), [0.9, 0.25], "the nearest splat carries its strength up");
}

/// The production `dome` entry over `points`, as the Eye-Dome depth redraw renders it.
fn redraw(device: &wgpu::Device, queue: &wgpu::Queue, points: &[Point], samples: u32, strength: f32) -> Vec<[f32; 2]> {
    let style = Style::new(&Presentation { eye_dome: strength, ..Default::default() });
    let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("dome style"),
        contents: &uniform_bytes(&style),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let records = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("dome points"),
        contents: bytemuck::cast_slice(points),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production cloud dome"),
        source: wgpu::ShaderSource::Wgsl(shader(true, samples).into()),
    });
    let state = RasterState { opaque: true, samples: 1 };
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("cloud dome redraw"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vertex"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("dome"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format: FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
        }),
        primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
        depth_stencil: Some(state.depth(wgpu::TextureFormat::Depth32Float)),
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    let extent = wgpu::Extent3d { width: 64, height: 64, depth_or_array_layers: 1 };
    let texture = |format, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("dome redraw"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let target = texture(FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
    let depth = texture(wgpu::TextureFormat::Depth32Float, wgpu::TextureUsages::RENDER_ATTACHMENT);
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("dome redraw"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: records.as_entire_binding() },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let view = target.create_view(&Default::default());
        let depth = depth.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[attachment(&view, wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT))],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..6, 0..points.len() as u32);
    }
    read(device, queue, encoder, &target, 0, 8).into_iter().map(|[depth, strength, ..]| [depth, strength]).collect()
}

#[test]
#[ignore = "requires GPU; run via make test-draw-cloud-gpu"]
fn gpu_dome_redraw_covers_the_pixels_the_opaque_pass_draws() {
    let engine = molla_compute::Engine::new_gpu().unwrap();
    let (device, queue) = (engine.device().unwrap(), engine.queue().unwrap());
    let mut near = point(0.6, [1.0, 1.0, 1.0, 1.0]);
    near.center[0] = 26.0;
    let far = point(0.3, [1.0, 1.0, 1.0, 1.0]);
    for samples in [1, 4] {
        let points = [far, near];
        let drawn = render(device, queue, &points, samples, 1.0, 0.0, 0.0, OPAQUE);
        let dome = redraw(device, queue, &points, samples, 0.75);
        let mut covered = 0;
        for (index, ([depth, strength], pixel)) in dome.iter().zip(&drawn).enumerate() {
            if *depth > 0.0 {
                covered += 1;
                assert!(pixel[0] > 0.0, "pixel {index} has a dome depth the opaque pass left empty, {samples}x");
                assert_eq!(*strength, 0.75);
                let (x, y) = ((index % 64) as f32 + 0.5, (index / 64) as f32 + 0.5);
                if (x - 26.0).hypot(y - 32.0) < 12.0 {
                    assert!((depth - 0.6).abs() < 1e-6, "the nearer splat wins at pixel {index}: {depth}");
                }
            }
            if samples == 1 {
                assert_eq!(*depth > 0.0, pixel[0] > 0.0, "single-sample pixel {index} matches the opaque pass");
            } else if pixel[0] >= 0.999 {
                assert!(*depth > 0.0, "fully covered pixel {index} has a dome depth, {samples}x");
            }
        }
        assert!(covered > 500, "the splats cover the dome: {covered}");
    }
    let mut translucent = point(0.5, [1.0, 1.0, 1.0, 0.5]);
    translucent.center[0] = 30.0;
    assert!(redraw(device, queue, &[translucent], 1, 0.75).iter().all(|[depth, _]| *depth == 0.0));
}
