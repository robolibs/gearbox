use super::super::super::{appearance::display_coordinate, palette::indices};
use super::*;

struct Case {
    input: Input,
    coordinate: f32,
    indices: u32,
    label: String,
}

fn integer_indices(ns: i128) -> u32 {
    let magnitude = ns.unsigned_abs();
    let rounded = (magnitude / 1_000_000_000 + u128::from(magnitude % 1_000_000_000 >= 500_000_000))
        as i128
        * ns.signum();
    let raw = rounded.clamp(0, 255) as u32;
    let map = if rounded < 0 { 255 } else { raw };
    map | raw << 8 | (rounded.rem_euclid(8) as u32) << 16
}

fn case(
    clock: AgeClock,
    range: [f64; 2],
    capture_s: f64,
    now_s: f64,
    source_ns: u64,
    offset: u64,
    coordinate: f32,
    indices: u32,
) -> Case {
    let capture = CaptureStamp {
        simulation_time_s: capture_s,
        source_time_ns: Some(0),
        source_clock: Some("precision".into()),
        sample: 0,
        scene_epoch: 1,
    };
    Case {
        input: Input {
            sample: Sample {
                acquisition_ns: [offset as u32, (offset >> 32) as u32],
                flags: VALUE_VALID,
                ..Default::default()
            },
            age: AgeUniform::new(
                &ColorMode::Age {
                    clock,
                    range,
                    colormap: gearbox_api::colormap::GREY,
                },
                Some(&capture),
                now_s,
                source_ns,
            ),
        },
        coordinate,
        indices,
        label: format!(
            "{clock:?} range={range:?} capture={capture_s} now={now_s}/{source_ns} offset={offset}"
        ),
    }
}

#[test]
#[ignore = "requires GPU; run via make test-draw-cloud-gpu"]
fn gpu_age_normalization_and_indexed_palettes_preserve_extreme_and_cancelling_times() {
    let engine = molla_compute::Engine::new_gpu().unwrap();
    let device = engine.device().unwrap();
    let queue = engine.queue().unwrap();
    let pipeline = pipeline(device);
    let mut cases = Vec::new();
    let base = 1u64 << 30;
    let source = base * 1_000_000_000 + 512;
    let range = [base as f64, base as f64 + 2.0f64.powi(-20)];
    for offset in 0..=1024 {
        cases.push(case(
            AgeClock::Source,
            range,
            0.0,
            0.0,
            source,
            offset,
            (((512i64 - offset as i64) as f64) * 2.0f64.powi(20) / 1e9).clamp(0.0, 1.0) as f32,
            integer_indices(i128::from(source) - i128::from(offset)),
        ));
    }
    let base = (1u64 << 34) * 1_000_000_000;
    for range in [
        [0.499999999, 0.500000001],
        [-1.0, 1.0],
        [0.5, 0.5],
        [-1e300, 1e300],
    ] {
        for capture_s in [0.0, 1e-9, 0.25, 0.5, 0.500000001, 1.0] {
            for extra in [0, 1, 2, 500_000_000, 999_999_999] {
                let age = 0.5 - capture_s - extra as f64 / 1e9;
                cases.push(case(
                    AgeClock::Simulation,
                    range,
                    capture_s,
                    (1u64 << 34) as f64 + 0.5,
                    0,
                    base + extra,
                    display_coordinate(age, range),
                    indices(age),
                ));
            }
        }
    }
    for range in [
        [-1e-300, 1e-300],
        [-f64::from_bits(1), f64::from_bits(1)],
        [0.0, 0.0],
        [-f64::MAX, f64::MAX],
    ] {
        for source in [0, 1, 500_000_000, u64::MAX] {
            for offset in [0, 1, 500_000_000, u64::MAX] {
                let ns = i128::from(source) - i128::from(offset);
                cases.push(case(
                    AgeClock::Source,
                    range,
                    0.0,
                    0.0,
                    source,
                    offset,
                    display_coordinate(ns as f64 / 1e9, range),
                    integer_indices(ns),
                ));
            }
        }
    }
    for (now, capture_s, index) in [
        (0.5, f64::from_bits(1), 0),
        (f64::from_bits(1), 0.5, 0),
        (0.5, 0.0, 1 | 1 << 8 | 1 << 16),
        (0.0, 0.5, 255 | 7 << 16),
    ] {
        cases.push(case(
            AgeClock::Simulation,
            [-1.0, 1.0],
            capture_s,
            now,
            0,
            0,
            display_coordinate(now - capture_s, [-1.0, 1.0]),
            index,
        ));
    }
    for now in [1e100, 1e300, f64::MAX] {
        for offset in [0, 250_000_000, 250_000_001, 500_000_000, u64::MAX] {
            let rounded = (-((offset % 8_000_000_000) as f64) / 1e9 - 1.25 + 0.5).floor() as i64;
            cases.push(case(
                AgeClock::Simulation,
                [now, now],
                1.25,
                now,
                0,
                offset,
                0.5,
                255 | 255 << 8 | (rounded.rem_euclid(8) as u32) << 16,
            ));
            let rounded = (1.25 - (offset % 8_000_000_000) as f64 / 1e9 - 0.5).ceil() as i64;
            cases.push(case(
                AgeClock::Simulation,
                [0.0, 0.0],
                now,
                1.25,
                0,
                offset,
                0.5,
                255 | (rounded.rem_euclid(8) as u32) << 16,
            ));
        }
    }
    let mut random = 0xabfd_1147_0123_84abu64;
    for _ in 0..1024 {
        random = random
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let source = random;
        random = random
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let ns = i128::from(source) - i128::from(random);
        let range = [-2e10, 2e10];
        cases.push(case(
            AgeClock::Source,
            range,
            0.0,
            0.0,
            source,
            random,
            display_coordinate(ns as f64 / 1e9, range),
            integer_indices(ns),
        ));
    }
    let inputs: Vec<_> = cases.iter().map(|case| case.input).collect();
    for map in MAPS {
        let style = Style::new(&Presentation {
            color: ColorMode::Age {
                clock: AgeClock::Source,
                colormap: map,
                range: [0.0, 1.0],
            },
            ..Default::default()
        });
        let output = colors(device, queue, &pipeline, &style, &inputs);
        for (case, actual) in cases.iter().zip(output) {
            let target = expected(
                &style,
                &Sample {
                    value: case.coordinate,
                    palette_indices: case.indices,
                    flags: VALUE_VALID,
                    ..Default::default()
                },
            );
            for lane in 0..4 {
                assert!(
                    actual[lane].is_finite() && (actual[lane] - target[lane]).abs() <= 3e-6,
                    "map {map} {}: {actual:?} vs {target:?}; params {:?}",
                    case.label,
                    case.input.age
                );
            }
        }
    }
    println!(
        "{{\"age_precision_color_cases\":{},\"maps\":9,\"integer_offset_anchor\":true}}",
        cases.len() * MAPS.len()
    );
}
