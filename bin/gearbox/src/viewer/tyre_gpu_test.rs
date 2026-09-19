use super::tyre_mesh::supported_vertex;
use molla_vehicle::real_tire::LoadedTireEnvelope;
use wgpu::util::DeviceExt;

#[test]
#[ignore = "requires a Vulkan adapter"]
fn gpu_envelope_matches_cpu_pressure_and_ground_sweep() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let mut input = Vec::<[f32; 4]>::new();
    let mut expected = Vec::new();
    for radius in [0.3_f32, 0.6912594, 0.8882051, 1.5] {
        for loaded in [0.4, 0.6, 0.8, 0.95, 1.0, 1.1] {
            for ground in [None, Some(radius * 0.7), Some(radius), Some(-0.1)] {
                for radial in [0.0, 0.4, 0.5, 0.7, 1.0, 1.05] {
                    for side in [-0.5, 0.0, 0.5] {
                        for angle in 0..72 {
                            let a = angle as f32 * std::f32::consts::TAU / 72.0;
                            let point = [
                                radius * radial * a.cos(),
                                radius * radial * a.sin(),
                                side * radius,
                                0.0,
                            ];
                            let envelope = [radius, radius * 0.5, radius * 0.6, radius * loaded];
                            input.extend([
                                point,
                                envelope,
                                [ground.unwrap_or(0.0), f32::from(ground.is_some()), 0.0, 0.0],
                            ]);
                            expected.push(supported_vertex(
                                LoadedTireEnvelope {
                                    radius: envelope[0] as f64,
                                    bead_radius: envelope[1] as f64,
                                    width: envelope[2] as f64,
                                    loaded_radius: envelope[3] as f64,
                                },
                                [point[0] as f64, point[1] as f64, point[2] as f64],
                                ground.map(f64::from),
                            ));
                        }
                    }
                }
            }
        }
    }
    let source = format!(
        "{}\n{}",
        include_str!("tyre_deform.wgsl"),
        r#"
@group(0) @binding(0) var<storage, read> cases: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> result: array<vec4<f32>>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= arrayLength(&result) { return; }
    let index = 3u * id.x;
    result[id.x] = vec4(deform_contact(cases[index].xyz, cases[index + 1u], cases[index + 2u]), 0.0);
}
"#
    );
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("tyre parity"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("tyre parity"),
        layout: None,
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let inputs = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&input),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let size = (expected.len() * 16) as u64;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: inputs.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bindings, &[]);
        pass.dispatch_workgroups((expected.len() as u32).div_ceil(64), 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    let mapped = readback.slice(..).get_mapped_range();
    let actual: &[[f32; 4]] = bytemuck::cast_slice(&mapped);
    let mut maximum = 0.0_f64;
    for (index, (actual, expected)) in actual.iter().zip(&expected).enumerate() {
        for axis in 0..3 {
            let error = (actual[axis] as f64 - expected[axis]).abs();
            maximum = maximum.max(error);
            assert!(
                error < 2e-5,
                "case {index} axis {axis}: GPU {actual:?} CPU {expected:?}"
            );
        }
    }
    eprintln!(
        "{} GPU/CPU tyre cases; maximum error {maximum} m",
        expected.len()
    );
}
