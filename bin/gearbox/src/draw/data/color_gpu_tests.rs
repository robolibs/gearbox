use wgpu::util::DeviceExt;

use super::super::{
    age::AgeUniform,
    appearance::{Attributes, CATEGORY_VALID, COLOR_VALID, Layout, Name, Sample, Selector, VALUE_VALID},
    color_tests::{MAPS, expected, expected_at, prepared, presentation},
    style::{Style, uniform_bytes},
};
use gearbox_api::draw_data::CpuBudget;
use gearbox_api::wire::draw_data::*;

#[path = "age_gpu_tests.rs"]
mod age;

/// `CloudSample` as the cloud vertex shader assembles it from a page's attribute words.
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct GpuSample {
    kind: u32,
    valid: u32,
    words: [u32; 2],
}

#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct Input {
    sample: GpuSample,
    age: AgeUniform,
}

/// Reads every point back the way `draw_cloud.wesl`'s vertex stage does.
fn shader_samples(attributes: &Attributes) -> Vec<GpuSample> {
    let packing = attributes.layout.uniform();
    let words = &attributes.words.values;
    let colors = attributes.layout.colors as usize;
    (0..attributes.layout.count)
        .map(|index| {
            let base = index * packing.x as usize;
            let mut sample = GpuSample { kind: packing.y, valid: 1, words: [0; 2] };
            sample.words[..colors].copy_from_slice(&words[base..base + colors]);
            if packing.w != u32::MAX {
                sample.valid = words[packing.w as usize + index / 32] >> (index % 32) & 1;
            }
            sample
        })
        .collect()
}

fn packed(selector: &Selector, sample: &Sample) -> GpuSample {
    let budget = CpuBudget::new(1 << 10);
    let mut attributes = Attributes::new(&budget, Layout::new(selector, false, 1)).unwrap();
    attributes.set(0, sample);
    shader_samples(&attributes)[0]
}

fn colors(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &wgpu::ComputePipeline,
    style: &Style,
    input: &[Input],
) -> Vec<[f32; 4]> {
    let style = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("cloud color style"),
        contents: &uniform_bytes(style),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let input_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("cloud color inputs"),
        contents: bytemuck::cast_slice(input),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cloud color output"),
        size: input.len() as u64 * 32,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cloud color readback"),
        size: output.size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cloud color test"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: style.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: input_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: output.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bindings, &[]);
        pass.dispatch_workgroups((input.len() as u32).div_ceil(64), 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output.size());
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    let result: Vec<[u32; 4]> =
        bytemuck::cast_slice(&readback.slice(..).get_mapped_range().expect("mapped GPU buffer range")).to_vec();
    readback.unmap();
    for (item, pair) in input.iter().zip(result.chunks_exact(2)) {
        let ns = u64::from(item.sample.words[0]) | (u64::from(item.sample.words[1]) << 32);
        let seconds = ns / 1_000_000_000;
        assert_eq!(
            &pair[1][..3],
            &[
                seconds as u32,
                (seconds >> 32) as u32,
                (ns % 1_000_000_000) as u32
            ],
            "nanoseconds {ns}"
        );
        let expected = (ns as f64 / 1e9) as f32;
        assert!(
            pair[1][3].abs_diff(expected.to_bits()) <= 2,
            "nanoseconds {ns}"
        );
    }
    result
        .chunks_exact(2)
        .map(|pair| pair[0].map(f32::from_bits))
        .collect()
}

fn pipeline(device: &wgpu::Device) -> wgpu::ComputePipeline {
    let color_source = include_str!("../draw_cloud_color.wesl")
        .lines()
        .filter(|line| !line.starts_with("import "))
        .collect::<Vec<_>>()
        .join("\n")
        .replace("@group(3)", "@group(0)");
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production cloud color"),
        source: wgpu::ShaderSource::Wgsl(
            format!(
                "{}\n{}\n{}",
                super::definitions(),
                color_source,
                r#"
struct Input { sample: CloudSample, age: CloudAge };
@group(0) @binding(1) var<storage, read> inputs: array<Input>;
@group(0) @binding(2) var<storage, read_write> output: array<vec4<u32>>;
@compute @workgroup_size(64)
fn check(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= arrayLength(&inputs) { return; }
    let item = inputs[id.x];
    output[id.x * 2u] = bitcast<vec4<u32>>(cloud_sample_color(item.sample, item.age));
    let ns = item.sample.words;
    output[id.x * 2u + 1u] = vec4(cloud_seconds_parts(ns), bitcast<u32>(cloud_seconds(ns)));
}
"#
            )
            .into(),
        ),
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("production cloud color"),
        layout: None,
        module: &shader,
        entry_point: Some("check"),
        compilation_options: Default::default(),
        cache: None,
    })
}

#[test]
#[ignore = "requires GPU; run via make test-draw-cloud-gpu"]
fn gpu_production_color_selector_preserves_all_maps_raw_indices_missing_rgba_and_categories() {
    let engine = molla_compute::Engine::new_gpu().unwrap();
    let device = engine.device().unwrap();
    let queue = engine.queue().unwrap();
    let pipeline = pipeline(device);
    let mut values: Vec<_> = (-10..=260).map(f64::from).collect();
    values.extend([
        -0.5,
        0.5,
        98.5,
        255.5,
        -1e300,
        1e300,
        16_777_217.0,
        9_007_199_254_740_991.0,
    ]);
    let (budget, appearance) = prepared(&values);
    let samples = appearance.samples();
    let input: Vec<_> = shader_samples(&appearance.attributes)
        .into_iter()
        .map(|sample| Input {
            sample,
            ..Default::default()
        })
        .collect();
    let mut cases = 0;
    for map in MAPS {
        let p = presentation(map);
        let style = Style::new(&p);
        let output = colors(device, queue, &pipeline, &style, &input);
        for (index, actual) in output.iter().enumerate() {
            let expected = expected(&style, &samples[index]);
            for lane in 0..4 {
                assert!(
                    (actual[lane] - expected[lane]).abs() <= 2e-7,
                    "map {map}, sample {index}: {actual:?} vs {expected:?}"
                );
            }
        }
        cases += output.len();
        for clock in [AgeClock::Simulation, AgeClock::Source] {
            let age_presentation = Presentation {
                color: ColorMode::Age {
                    clock,
                    colormap: map,
                    range: [-10.0, 256.0],
                },
                ..p.clone()
            };
            let style = Style::new(&age_presentation);
            let ages: Vec<_> = (-512..=512).map(|value| value as f32 * 0.5).collect();
            let inputs: Vec<_> = ages
                .iter()
                .map(|age| {
                    let now = (1_000_000_000_000i64 + (f64::from(*age) * 1e9) as i64) as u64;
                    let capture = 999_000_000_000u64;
                    Input {
                        sample: packed(
                            &Selector::Age,
                            &Sample {
                                flags: VALUE_VALID,
                                acquisition_ns: [1_000_000_000, 0],
                                ..Default::default()
                            },
                        ),
                        age: AgeUniform::new(
                            &age_presentation.color,
                            Some(&CaptureStamp {
                                simulation_time_s: 999.0,
                                source_time_ns: Some(capture),
                                source_clock: Some("test".into()),
                                sample: 0,
                                scene_epoch: 1,
                            }),
                            1000.0 + f64::from(*age),
                            now,
                        ),
                    }
                })
                .collect();
            let output = colors(device, queue, &pipeline, &style, &inputs);
            for (index, age) in ages.into_iter().enumerate() {
                let sample = Sample {
                    flags: VALUE_VALID,
                    value: super::super::appearance::display_coordinate(
                        f64::from(age),
                        [-10.0, 256.0],
                    ),
                    palette_indices: super::super::palette::indices(f64::from(age)),
                    ..Default::default()
                };
                let expected = expected_at(&style, &sample, sample.value.clamp(0.0, 1.0) * 255.0);
                for lane in 0..4 {
                    assert!(
                        (output[index][lane] - expected[lane]).abs() <= 2e-6,
                        "age {age}, map {map}, clock {clock:?}: {:?} vs {expected:?}",
                        output[index]
                    );
                }
            }
            cases += output.len();
        }
    }
    let rgba = [28, 75, 180, 127];
    let sample = Sample {
        flags: COLOR_VALID | CATEGORY_VALID,
        rgba: u32::from_le_bytes(rgba),
        category: u32::MAX,
        ..Default::default()
    };
    for (selector, color) in [
        (Selector::Constant, ColorMode::Constant { rgba }),
        (
            Selector::Attribute(Name::new("rgba").unwrap()),
            ColorMode::Attribute {
                name: "rgba".into(),
                space: ColorSpace::Srgb,
            },
        ),
        (
            Selector::Category(Name::new("class").unwrap()),
            ColorMode::Category {
                name: "class".into(),
                entries: vec![PaletteEntry {
                    value: u32::MAX,
                    rgba,
                }],
                missing: [3; 4],
            },
        ),
    ] {
        let style = Style::new(&Presentation {
            color,
            ..Default::default()
        });
        let output = colors(
            device,
            queue,
            &pipeline,
            &style,
            &[Input {
                sample: packed(&selector, &sample),
                ..Default::default()
            }],
        );
        for lane in 0..4 {
            assert!((output[0][lane] - f32::from(rgba[lane]) / 255.0).abs() <= 1e-7);
        }
        cases += 1;
    }
    drop(appearance);
    assert_eq!(budget.usage().committed(), 0);
    let mut offsets = vec![
        0u64,
        1,
        499_999_999,
        500_000_000,
        500_000_001,
        999_999_999,
        1_000_000_000,
        4_294_967_295,
        4_294_967_296,
        (1 << 53) + 1,
        u64::MAX,
    ];
    let mut state = 0x64ec_1385_ebd2_0f17u64;
    for _ in 0..1024 {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        offsets.push(state);
    }
    let inputs: Vec<_> = offsets
        .iter()
        .map(|ns| Input {
            sample: packed(
                &Selector::Age,
                &Sample {
                    acquisition_ns: [*ns as u32, (*ns >> 32) as u32],
                    ..Default::default()
                },
            ),
            ..Default::default()
        })
        .collect();
    let output = colors(
        device,
        queue,
        &pipeline,
        &Style::new(&Presentation::default()),
        &inputs,
    );
    assert_eq!(output.len(), offsets.len());
    println!(
        "{{\"production_color_cases\":{cases},\"maps\":9,\"integer_time_cases\":{},\"value_sample_bytes\":4}}",
        offsets.len()
    );
}
