use wgpu::util::DeviceExt;

fn compute(inputs: &[[f32; 8]], body: &str) -> Vec<[f32; 8]> {
    let engine = molla_compute::Engine::new_gpu().unwrap();
    let device = engine.device().unwrap();
    let queue = engine.queue().unwrap();
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("cloud projection cases"),
        contents: bytemuck::cast_slice(inputs),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cloud projection results"),
        size: std::mem::size_of_val(inputs) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cloud projection readback"),
        size: output.size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let source = format!(
        "{}\nstruct Case {{ a: vec4<f32>, b: vec4<f32> }};\n@group(0) @binding(0) var<storage, read> cases: array<Case>;\n@group(0) @binding(1) var<storage, read_write> results: array<Case>;\n@compute @workgroup_size(64) fn check(@builtin(global_invocation_id) id: vec3<u32>) {{ if id.x >= arrayLength(&cases) {{ return; }} let item = cases[id.x]; {body} }}",
        super::definitions()
    );
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production cloud projection helpers"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("cloud projection proof"),
        layout: None,
        module: &shader,
        entry_point: Some("check"),
        compilation_options: Default::default(),
        cache: None,
    });
    let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cloud projection bindings"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: input.as_entire_binding(),
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
        pass.dispatch_workgroups((inputs.len() as u32).div_ceil(64), 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output.size());
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    let result = bytemuck::cast_slice(&readback.slice(..).get_mapped_range()).to_vec();
    readback.unmap();
    result
}

#[test]
#[ignore = "requires GPU; run via make test-draw-cloud-projection"]
fn gpu_radius_units_match_f64_without_intermediate_overflow() {
    let mut inputs = vec![[1e38, 0.0, 1.0, 1024.0, 1e38, 0.0, 0.0, 0.0]];
    for height in [64.0, 480.0, 1080.0, 16384.0] {
        for w in [0.1, 1.0, 10.0, 1000.0, 1e38] {
            for radius in [0.0, 0.125, 1.0, 1e20, 1e38] {
                inputs.push([radius, 7.25, 1.732, height, w, 0.0, 0.0, 0.0]);
            }
        }
    }
    inputs.extend([
        [1e38, 0.0, 1.0, 1024.0, 1e38, 0.0, 0.0, 0.0],
        [1e37, 0.0, 1e-30, 1080.0, 1e-8, 0.0, 0.0, 0.0],
        [f32::MIN_POSITIVE, 0.0, 1e30, 1080.0, 1e-8, 0.0, 0.0, 0.0],
        [f32::from_bits(1), 0.0, 1e30, 1080.0, 1e-8, 0.0, 0.0, 0.0],
        [0.0, f32::MAX, -1.0, 64.0, 1.0, 0.0, 0.0, 0.0],
    ]);
    let result = compute(
        &inputs,
        "results[id.x].a = vec4(cloud_radius_px(item.a.x, item.a.y, item.a.z, item.a.w, item.b.x), 0.0, 0.0, 0.0); results[id.x].b = vec4(0.0);",
    );
    for (case, actual) in inputs.iter().zip(result) {
        let expected = if case[0] > 0.0 {
            (f64::from(case[0]) * 0.5 * f64::from(case[3]) * f64::from(case[2]).abs()
                / f64::from(case[4]).abs().max(1e-8))
            .min(f64::from(f32::MAX))
        } else {
            f64::from(case[1])
        };
        assert!(
            actual[0].is_finite() && actual[0] >= 0.0,
            "nonfinite radius {actual:?} for {case:?}"
        );
        assert!(
            (f64::from(actual[0]) - expected).abs() <= expected.abs() * 3e-6 + 1e-6,
            "radius {} vs {expected} for {case:?}",
            actual[0]
        );
    }
    println!(
        "{{\"production_radius_cases\":{},\"f64_oracle\":true}}",
        inputs.len()
    );
}

#[test]
#[ignore = "requires GPU; run via make test-draw-cloud-projection"]
fn gpu_coverage_of_large_finite_discs_does_not_overflow_length() {
    let inputs = [
        [1e30, 7e29, 7e29, 0.0, 0.0, 0.0, 0.0, 0.0],
        [1e30, 9e29, 9e29, 0.0, 0.0, 0.0, 0.0, 0.0],
        [f32::MAX, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [10.0, 10.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    ];
    let result = compute(
        &inputs,
        "results[id.x].a = vec4(cloud_coverage(item.a.yz, item.a.x), 0.0, 0.0, 0.0); results[id.x].b = vec4(0.0);",
    );
    for (case, actual) in inputs.iter().zip(result) {
        let expected = (f64::from(case[0]) - f64::from(case[1]).hypot(f64::from(case[2])) + 0.5)
            .clamp(0.0, 1.0);
        assert!(
            (f64::from(actual[0]) - expected).abs() < 1e-5,
            "coverage {} vs {expected} for {case:?}",
            actual[0]
        );
    }
}

#[test]
#[ignore = "requires GPU; run via make test-draw-cloud-projection"]
fn gpu_production_vertices_preserve_clipped_circle_offsets_and_finite_depth() {
    let mut inputs = Vec::new();
    for viewport in [
        [1.0, 1.0],
        [640.0, 480.0],
        [480.0, 1080.0],
        [16384.0, 8192.0],
    ] {
        for w in [1e-20, 1.0, 1e20, 1e38] {
            for center in [[0.0, 0.0], [-1.5, 0.25], [1.5, -0.25], [0.1, 2.0]] {
                for radius in [0.125, 7.25, 1500.0, 1e30, f32::MAX] {
                    for vertex in 0..6 {
                        inputs.push([
                            center[0] * w,
                            center[1] * w,
                            0.5 * w,
                            w,
                            viewport[0],
                            viewport[1],
                            radius,
                            vertex as f32,
                        ]);
                    }
                }
            }
        }
    }
    let result = compute(
        &inputs,
        "let projected = cloud_project(item.a, item.b.xy, item.b.z, u32(item.b.w)); results[id.x].a = projected.position; results[id.x].b = vec4(projected.offset_px, projected.radius_px, cloud_coverage(projected.offset_px, projected.radius_px));",
    );
    let corners = [
        [-1.0, -1.0],
        [1.0, -1.0],
        [1.0, 1.0],
        [-1.0, -1.0],
        [1.0, 1.0],
        [-1.0, 1.0],
    ];
    for (case, actual) in inputs.iter().zip(result) {
        assert!(
            actual.iter().all(|value| value.is_finite()),
            "nonfinite vertex {actual:?} for {case:?}"
        );
        assert_eq!(actual[3], 1.0);
        assert!(
            (actual[2] - 0.5).abs() < 1e-5,
            "depth {actual:?} for {case:?}"
        );
        assert_eq!(actual[6], case[6]);
        assert!((0.0..=1.0).contains(&actual[7]));
        for axis in 0..2 {
            let half = f64::from(case[4 + axis]) * 0.5;
            let center = f64::from(case[axis]) / f64::from(case[3]) * half;
            let pixel = (center + corners[case[7] as usize][axis] * (f64::from(case[6]) + 1.0))
                .clamp(-half, half);
            assert!(
                (f64::from(actual[axis]) - pixel / half).abs() < 3e-5,
                "vertex {actual:?} for {case:?}"
            );
            assert!(
                (f64::from(actual[4 + axis]) - (pixel - center)).abs()
                    < (pixel - center).abs() * 3e-5 + 0.002,
                "circle offset {actual:?} for {case:?}"
            );
        }
    }
    println!(
        "{{\"production_vertex_cases\":{},\"finite_clip_coordinates\":true}}",
        inputs.len()
    );
}

#[test]
#[ignore = "requires GPU; run via make test-draw-cloud-projection"]
fn gpu_wide_projection_keeps_all_vertex_fields_finite() {
    let mut inputs = Vec::new();
    for viewport in [[64.0, 64.0], [16384.0, 8192.0]] {
        for center in [
            [-1e38, 0.0],
            [1e38, 0.0],
            [0.0, 1e38],
            [1e-3, 1e38],
            [1e38, -1e38],
        ] {
            for size in [1e-20, 1.0, f32::MAX, -5e37, -1e38, -2e38] {
                for vertex in 0..6 {
                    inputs.push([
                        center[0],
                        center[1],
                        0.5,
                        1.0,
                        viewport[0],
                        viewport[1],
                        size,
                        vertex as f32,
                    ]);
                }
            }
        }
    }
    let result = compute(
        &inputs,
        "let projected = cloud_project_point(item.a, item.b.xy, max(-item.b.z, 0.0), max(item.b.z, 0.0), 1.0, u32(item.b.w)); results[id.x].a = projected.position; results[id.x].b = vec4(projected.offset_px, projected.radius_px, f32(projected.linear_edge));",
    );
    for (case, actual) in inputs.iter().zip(result) {
        assert!(
            actual.iter().all(|value| value.is_finite()),
            "nonfinite wide vertex {actual:?} for {case:?}"
        );
        assert!((-1.0..=1.0).contains(&actual[0]) && (-1.0..=1.0).contains(&actual[1]));
        assert_eq!(actual[2], 0.5);
        assert_eq!(actual[3], 1.0);
        assert_eq!(actual[7], 1.0);
    }
    println!(
        "{{\"wide_projection_cases\":{},\"finite_vertices\":true}}",
        inputs.len()
    );
}

#[test]
#[ignore = "requires GPU; run via make test-draw-cloud-projection"]
fn gpu_projection_rejects_invalid_clip_depth_without_nonfinite_vertices() {
    let inputs = [
        [0.0, 0.0, 0.0, 0.0, 640.0, 480.0, 10.0, 0.0],
        [0.0, 0.0, 0.5, -1.0, 640.0, 480.0, 10.0, 0.0],
        [0.0, 0.0, -0.1, 1.0, 640.0, 480.0, 10.0, 0.0],
        [0.0, 0.0, 1.1, 1.0, 640.0, 480.0, 10.0, 0.0],
        [0.0, 0.0, 0.5, 1.0, 0.0, 480.0, 10.0, 0.0],
    ];
    let result = compute(
        &inputs,
        "let projected = cloud_project(item.a, item.b.xy, item.b.z, u32(item.b.w)); results[id.x].a = projected.position; results[id.x].b = vec4(projected.offset_px, projected.radius_px, 0.0);",
    );
    for actual in result {
        assert_eq!(actual, [2.0, 2.0, 2.0, 1.0, 0.0, 0.0, 0.0, 0.0]);
    }
}
