use bevy::math::DVec3;
use gearbox_globe::{Body, Datum, Geodetic, Terrain};
use wgpu::util::DeviceExt;

#[test]
#[ignore = "requires Vulkan hardware via oslo make test-planet-gpu"]
fn globe_bake_matches_local_crater_heights_on_hardware() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    })).unwrap();
    assert_ne!(adapter.get_info().device_type, wgpu::DeviceType::Cpu);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let source = format!("{}\n{}", include_str!("../shaders/tile_bake.wgsl"), r#"
@group(1) @binding(0) var<storage,read> probes: array<vec4<f32>>;
@group(1) @binding(1) var<storage,read_write> measured: array<f32>;
@compute @workgroup_size(64)
fn probe(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x < arrayLength(&probes) {
        measured[id.x] = terrain_height(normalize(probes[id.x].xyz), params.seed, params.freq) * params.height_amp;
    }
}
"#);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production planetary height bake"), source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None, layout: None, module: &module, entry_point: Some("probe"),
        compilation_options: Default::default(), cache: None,
    });
    for body in [Body::Earth, Body::Mars, Body::Moon] {
        let (lat, lon) = body.home();
        let home = Datum::on(body, lat, lon);
        let terrain = Terrain::new(&home);
        let mut positions = Vec::new();
        for i in 0..256 {
            let local = home.to_ecef(DVec3::new((i % 16) as f64 * 41.3 - 300.0, 0.0, (i / 16) as f64 * 37.7 - 280.0));
            positions.push(local.normalize() * body.radius_m());
        }
        for i in 0..128 {
            positions.push(Geodetic::new(-88.0 + i as f64 * 1.38, -179.0 + i as f64 * 2.71, 0.0).ecef_on(body));
        }
        let points: Vec<[f32;4]> = positions.iter().map(|p| [p.x as f32,p.y as f32,p.z as f32,0.0]).collect();
        let mut words = [0u32;16];
        words[8] = 3.0f32.to_bits(); words[9] = (body.radius_m() as f32).to_bits();
        words[10] = 2400.0f32.to_bits(); words[11] = body as u32;
        for (i,v) in terrain.bake_home().into_iter().enumerate() { words[12+i] = v.to_bits(); }
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None, contents: bytemuck::cast_slice(&words), usage: wgpu::BufferUsages::UNIFORM,
        });
        let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None, contents: bytemuck::cast_slice(&points), usage: wgpu::BufferUsages::STORAGE,
        });
        let size = points.len() as u64 * 4;
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: None, size, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None, size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false,
        });
        let globals = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None, layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry { binding: 1, resource: uniform.as_entire_binding() }],
        });
        let buffers = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None, layout: &pipeline.get_bind_group_layout(1), entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: input.as_entire_binding() },
                wpu_output(&output),
            ],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline); pass.set_bind_group(0,&globals,&[]); pass.set_bind_group(1,&buffers,&[]);
            pass.dispatch_workgroups((points.len() as u32).div_ceil(64),1,1);
        }
        encoder.copy_buffer_to_buffer(&output,0,&readback,0,size);
        queue.submit([encoder.finish()]);
        let (send, recv) = std::sync::mpsc::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |result| send.send(result).unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        recv.recv().unwrap().unwrap();
        let bytes = readback.slice(..).get_mapped_range();
        let measured: &[f32] = bytemuck::cast_slice(&bytes);
        let mut worst = 0.0f32;
        for (i, (&actual, p)) in measured.iter().zip(&positions).enumerate() {
            let expected = if body == Body::Earth { 0.0 } else { terrain.height(*p) };
            let error = (actual-expected).abs();
            assert!(actual.is_finite() && error < 0.75, "{body:?} probe {i}: GPU {actual}, CPU {expected}, error {error}");
            worst = worst.max(error);
        }
        println!("{body:?}: {} production-shader probes, max CPU/GPU height error {worst} m", points.len());
    }
}

fn wpu_output(buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding: 1, resource: buffer.as_entire_binding() }
}

#[test]
#[ignore = "requires Vulkan hardware via oslo make test-planet-gpu"]
fn production_height_filter_interpolates_r32float_without_filtering_features() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    assert_ne!(adapter.get_info().device_type, wgpu::DeviceType::Cpu);
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let shader = include_str!("../shaders/terrain.wgsl");
    let start = shader.find("fn sample_height(").unwrap();
    let end = start + shader[start..].find("\n}").unwrap() + 2;
    let helper = &shader[start..end];
    assert!(include_str!("../shaders/water.wgsl").contains(helper));
    let source = format!(r#"
const TILE_TEXELS: f32 = 132.0;
const BORDER: f32 = 1.0;
@group(0) @binding(0) var heightmaps: texture_2d_array<f32>;
@group(0) @binding(1) var<storage,read> probes: array<vec4<f32>>;
@group(0) @binding(2) var<storage,read_write> measured: array<f32>;
{helper}
@compute @workgroup_size(64)
fn probe(@builtin(global_invocation_id) id: vec3<u32>) {{
    if id.x < arrayLength(&probes) {{
        measured[id.x] = sample_height(u32(probes[id.x].z), probes[id.x].xy);
    }}
}}
"#);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production manual height filter"), source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None, layout: None, module: &module, entry_point: Some("probe"),
        compilation_options: Default::default(), cache: None,
    });
    let size = wgpu::Extent3d { width: 132, height: 132, depth_or_array_layers: 2 };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None, size, mip_level_count: 1, sample_count: 1,
        dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[],
    });
    let height = |x: usize, y: usize, layer: usize| (x*x + y*3 + layer*1000) as f32 / 100.0;
    let values: Vec<f32> = (0..2).flat_map(|layer| (0..132).flat_map(move |y| (0..132).map(move |x| height(x,y,layer)))).collect();
    queue.write_texture(wgpu::TexelCopyTextureInfo {
        texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All,
    }, bytemuck::cast_slice(&values), wgpu::TexelCopyBufferLayout {
        offset: 0, bytes_per_row: Some(132*4), rows_per_image: Some(132),
    }, size);
    let coordinates = [-2.0, -1.0, -0.75, 0.0, 0.125, 0.5, 17.375, 63.9, 127.25, 128.0, 129.5, 131.0];
    let points: Vec<[f32;4]> = (0..2).flat_map(|layer| coordinates.into_iter().flat_map(move |y| coordinates.into_iter().map(move |x| [x,y,layer as f32,0.0]))).collect();
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None, contents: bytemuck::cast_slice(&points), usage: wgpu::BufferUsages::STORAGE,
    });
    let bytes = points.len() as u64 * 4;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None, size: bytes, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None, size: bytes, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false,
    });
    let view = texture.create_view(&Default::default());
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None, layout: &pipeline.get_bind_group_layout(0), entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 1, resource: input.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: output.as_entire_binding() },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline); pass.set_bind_group(0,&bind,&[]);
        pass.dispatch_workgroups((points.len() as u32).div_ceil(64),1,1);
    }
    encoder.copy_buffer_to_buffer(&output,0,&readback,0,bytes);
    queue.submit([encoder.finish()]);
    let (send, recv) = std::sync::mpsc::channel();
    readback.slice(..).map_async(wgpu::MapMode::Read, move |r| send.send(r).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    recv.recv().unwrap().unwrap();
    let mapped = readback.slice(..).get_mapped_range();
    let measured: &[f32] = bytemuck::cast_slice(&mapped);
    for (p, actual) in points.iter().zip(measured) {
        let x = (p[0]+1.0).clamp(0.0,131.0); let y = (p[1]+1.0).clamp(0.0,131.0);
        let ix = x.floor() as usize; let iy = y.floor() as usize; let layer = p[2] as usize;
        let a = height(ix,iy,layer); let b = height((ix+1).min(131),iy,layer);
        let c = height(ix,(iy+1).min(131),layer); let d = height((ix+1).min(131),(iy+1).min(131),layer);
        let low = a+(b-a)*x.fract(); let high = c+(d-c)*x.fract();
        let expected = low+(high-low)*y.fract();
        assert!((actual-expected).abs() < 0.0001, "probe {p:?}: {actual} != {expected}");
    }
    println!("{} production R32Float filter probes passed, including both atlas layers and borders", points.len());
}
