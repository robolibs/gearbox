use super::appearance::{Attributes, KIND_AGE, Layout, RADIUS_VALID, Sample, Selector, VALUE_VALID};
use super::cloud::Position;

#[path = "color_gpu_tests.rs"]
mod color;
#[path = "raster_gpu_tests.rs"]
mod raster;
#[path = "projection_gpu_tests.rs"]
mod projection;

fn definitions() -> &'static str {
    include_str!("../draw_cloud_types.wesl")
}

#[test]
#[ignore = "requires GPU; run via make test-draw-cloud-gpu"]
fn gpu_clip_rotation_and_inclusive_boundaries_match_cpu_selected_counts() {
    use super::{
        clip::{Clip, rotated},
        cloud::Geometry,
        tests::{fixture, stored},
    };
    use bevy::math::{DQuat, DVec3, Mat3, Vec3};
    use gearbox_api::{
        draw_data::CpuBudget,
        wire::draw_data::{ClipBounds, DataFrame},
    };
    let engine = molla_compute::Engine::new_gpu().unwrap();
    let device = engine.device().unwrap();
    let queue = engine.queue().unwrap();
    let budget = CpuBudget::new(1 << 20);
    let mut input = Vec::<f32>::new();
    let mut expected = Vec::<u32>::new();
    for scenario in 0..8 {
        let points: Vec<_> = (0..129)
            .map(|i| [-2.0 + f64::from(i) / 32.0, f64::from(i % 7) / 8.0, 1.0])
            .collect();
        let (schema, payload) = fixture(&points, vec![]);
        let geometry = Geometry::prepare(
            &budget,
            &schema,
            stored(&budget, &schema, &payload, [0.0; 3]),
        )
        .unwrap();
        let mut frame = DataFrame::default();
        let q = if scenario == 5 {
            DQuat::from_euler(bevy::math::EulerRot::XYZ, 0.4, -0.2, 1.1)
        } else {
            DQuat::from_rotation_z(if scenario == 0 { 0.0 } else { 0.4 })
        };
        frame.pose.rotation_wxyz = [q.w, q.x, q.y, q.z];
        if scenario == 1 {
            frame.origin_m = [1e12, -1e12, 10.0];
        }
        let center = q * (DVec3::from_array(frame.origin_m) + DVec3::new(0.0, 0.0, 1.0));
        let bounds = match scenario {
            2 => None,
            3 => Some(ClipBounds {
                min_m: [1e300; 3],
                max_m: [2e300; 3],
                ..Default::default()
            }),
            4 => Some(ClipBounds {
                min_m: [-1e300; 3],
                max_m: [1e300; 3],
                ..Default::default()
            }),
            6 => Some(ClipBounds {
                min_m: (center - DVec3::ONE).to_array(),
                max_m: (center + DVec3::ONE).to_array(),
                invert: true,
                ..Default::default()
            }),
            7 => {
                let turn = DQuat::from_rotation_z(0.7);
                let local = turn.inverse() * center;
                Some(ClipBounds {
                    min_m: (local - DVec3::new(5.0, 0.06, 5.0)).to_array(),
                    max_m: (local + DVec3::new(5.0, 0.06, 5.0)).to_array(),
                    rotation_wxyz: Some([turn.w, turn.x, turn.y, turn.z]),
                    invert: false,
                })
            }
            _ => Some(ClipBounds {
                min_m: (center - DVec3::ONE).to_array(),
                max_m: (center + DVec3::ONE).to_array(),
                ..Default::default()
            }),
        };
        let clip = Clip::prepare(&geometry, &frame, bounds).unwrap();
        let rotation = Mat3::from_mat4(clip.rotation);
        let mut selected = 0;
        for point in &geometry.positions.values {
            input.extend(clip.rotation.to_cols_array());
            input.extend(clip.min.to_array());
            input.extend(clip.max.to_array());
            input.extend([point.xyz[0], point.xyz[1], point.xyz[2], 0.0]);
            let value = rotated(rotation, Vec3::from_array(point.xyz));
            let inside = value.cmpge(clip.min.truncate()).all() && value.cmple(clip.max.truncate()).all();
            let accepted = clip.min.w == 0.0
                || clip.min.w == 1.0 && inside
                || clip.min.w == super::clip::INVERTED && !inside;
            selected += u64::from(accepted);
            expected.extend(value.to_array().map(f32::to_bits));
            expected.push(u32::from(accepted));
        }
        assert_eq!(clip.selected, selected);
    }
    assert_eq!(budget.usage().committed(), 0);
    let count = expected.len() as u32 / 4;
    let input_bytes = bytemuck::cast_slice(&input);
    let inputs = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("clip input"),
        size: input_bytes.len() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&inputs, 0, input_bytes);
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("clip output"),
        size: expected.len() as u64 * 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("clip readback"),
        size: output.size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("clip parity"),
        source: wgpu::ShaderSource::Wgsl(
            format!(
                "{}\n{}",
                definitions(),
                r#"
struct ClipInput { rotation: mat4x4<f32>, low: vec4<f32>, high: vec4<f32>, point: vec4<f32> };
@group(0) @binding(0) var<storage, read> inputs: array<ClipInput>;
@group(0) @binding(1) var<storage, read_write> output: array<vec4<u32>>;
@compute @workgroup_size(64)
fn check(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= arrayLength(&inputs) { return; }
    let item = inputs[id.x];
    let position = cloud_clip_position(item.rotation, item.point.xyz);
    let selected = cloud_clip_accept(item.rotation, item.low, item.high, item.point.xyz);
    output[id.x] = vec4(bitcast<vec3<u32>>(position), select(0u, 1u, selected));
}
"#
            )
            .into(),
        ),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("clip parity"),
        layout: None,
        module: &shader,
        entry_point: Some("check"),
        compilation_options: Default::default(),
        cache: None,
    });
    let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("clip parity"),
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
        pass.dispatch_workgroups(count.div_ceil(64), 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output.size());
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    assert_eq!(
        &*readback.slice(..).get_mapped_range().expect("mapped GPU buffer range"),
        bytemuck::cast_slice::<u32, u8>(&expected)
    );
    readback.unmap();
    println!("{{\"clip_cases\":{count},\"cpu_gpu_bits_and_selection_match\":true}}");
}

#[test]
#[ignore = "requires GPU; run via make test-draw-cloud-gpu"]
fn gpu_compact_records_match_wgsl_offsets_in_actual_million_sample_allocations() {
    let engine = molla_compute::Engine::new_gpu().unwrap();
    let device = engine.device().unwrap();
    let queue = engine.queue().unwrap();
    use wgpu::util::DeviceExt;
    let count = 1_000_000usize;
    let budget = gearbox_api::draw_data::CpuBudget::new(64 << 20);
    let layout = Layout::new(&Selector::Age, true, count);
    let mut attributes = Attributes::new(&budget, layout).unwrap();
    let position = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cloud position records"),
        size: (count * size_of::<Position>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let samples = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cloud attribute records"),
        size: layout.bytes(),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    assert_eq!(position.size() + samples.size(), 24_125_000);
    let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("cloud layout uniform"),
        contents: bytemuck::cast_slice(&layout.uniform().to_array()),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cloud layout output"),
        size: 144,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cloud layout readback"),
        size: output.size(),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut expected = Vec::<u32>::new();
    let rows = [0usize, 1, 65_535, 999_999];
    for (row, index) in rows.into_iter().enumerate() {
        let point = Position {
            xyz: [row as f32 + 0.125, -2.5, 0.75],
        };
        attributes.set(
            index,
            &Sample {
                radius_m: 0.0625,
                flags: if row % 2 == 0 { VALUE_VALID | RADIUS_VALID } else { 0 },
                acquisition_ns: [17 + row as u32, u32::MAX - row as u32],
                ..Default::default()
            },
        );
        queue.write_buffer(&position, (index * size_of::<Position>()) as u64, bytemuck::bytes_of(&point));
        expected.extend(point.xyz.map(f32::to_bits));
    }
    queue.write_buffer(&samples, 0, attributes.bytes());
    let mask = layout.uniform().w as usize;
    let mut fields = Vec::new();
    for (row, index) in rows.into_iter().enumerate() {
        let words = &attributes.words.values[index * 3..index * 3 + 3];
        let valid = attributes.words.values[mask + index / 32] >> (index % 32) & 1;
        assert_eq!(valid, u32::from(row % 2 == 0));
        assert_eq!(attributes.sample(index).unwrap().flags & VALUE_VALID != 0, valid != 0);
        fields.extend(expected[row * 3..row * 3 + 3].iter().copied());
        fields.extend([KIND_AGE, valid, words[0], words[1], words[2]]);
    }
    fields.extend([1.0f32, 0.0, -1.0, 1.0].map(f32::to_bits));
    let expected: Vec<u8> = bytemuck::cast_slice(&fields).to_vec();
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("cloud layout probe"),
        source: wgpu::ShaderSource::Wgsl(
            format!(
                "{}\n{}",
                definitions(),
                r#"
@group(0) @binding(0) var<storage, read> positions: array<CloudPosition>;
@group(0) @binding(1) var<storage, read> attributes: array<u32>;
@group(0) @binding(2) var<storage, read_write> result: array<u32>;
@group(0) @binding(3) var<uniform> packing: vec4<u32>;
@compute @workgroup_size(4)
fn check(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let row = invocation.x;
    let index = array<u32, 4>(0u, 1u, 65535u, 999999u)[row];
    let p = positions[index];
    let base = index * packing.x;
    let colors = packing.x - packing.z;
    var sample = CloudSample(packing.y, 1u, vec2(0u));
    if colors > 0u { sample.words.x = attributes[base]; }
    if colors > 1u { sample.words.y = attributes[base + 1u]; }
    if packing.w != CLOUD_ALL_VALID { sample.valid = (attributes[packing.w + index / 32u] >> (index % 32u)) & 1u; }
    var radius = 0u;
    if packing.z != 0u { radius = attributes[base + colors]; }
    let fields = array<u32, 8>(bitcast<u32>(p.x), bitcast<u32>(p.y), bitcast<u32>(p.z),
        sample.kind, sample.valid, sample.words.x, sample.words.y, radius);
    for (var field = 0u; field < 8u; field += 1u) { result[row * 8u + field] = fields[field]; }
    var age = cloud_source_age_ns(vec2(0xfffffffeu, 0xffffffffu), vec2(0xfffffffcu, 0xffffffffu), array<u32, 2>(row + 1u, 0u));
    if row == 3u { age = cloud_source_age_ns(vec2(2u, 1u), vec2(0xfffffffeu, 0u), array<u32, 2>(3u, 0u)); }
    result[32u + row] = bitcast<u32>(age);
}
"#
            )
            .into(),
        ),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("cloud layout probe"),
        layout: None,
        module: &shader,
        entry_point: Some("check"),
        compilation_options: Default::default(),
        cache: None,
    });
    let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cloud layout probe"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: position.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: samples.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: output.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: uniform.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bindings, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output.size());
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    assert_eq!(&*readback.slice(..).get_mapped_range().expect("mapped GPU buffer range"), expected);
    readback.unmap();
    println!(
        "{{\"samples\":1000000,\"base_gpu_bytes\":{},\"probe_gpu_bytes\":{},\"readback_gpu_bytes\":{},\"probe_upload_bytes\":{},\"scope\":\"actual layout allocations; not integrated rendering or residency acceptance\"}}",
        position.size() + samples.size(),
        output.size(),
        readback.size(),
        4 * size_of::<Position>() + attributes.bytes().len()
    );
}
