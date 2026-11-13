use super::*;

#[test]
fn age_uniform_layout_is_charged_per_page_not_per_point() {
    use super::super::style::{PageUniform, Style, uniform_bytes};
    use gearbox_api::wire::draw_data::Presentation;
    let age = AgeUniform {
        anchor: UVec4::new(11, 22, 33, 44),
        mapping: Vec4::new(0.25, 0.5, 0.75, 1.0),
        palette: UVec4::new(55, 66, 77, 88),
    };
    let page = PageUniform {
        world_from_local: Mat4::IDENTITY,
        clip_rotation: Mat4::IDENTITY,
        clip_min: Vec4::ZERO,
        clip_max: Vec4::ZERO,
        age,
    };
    let encoded = uniform_bytes(&page);
    assert_eq!(size_of::<AgeUniform>(), 48);
    assert_eq!(uniform_bytes(&age), bytemuck::bytes_of(&age));
    assert_eq!(encoded.len(), 208);
    assert_eq!(&encoded[160..], bytemuck::bytes_of(&age));
    assert_eq!(
        uniform_bytes(&Style::new(&Presentation::default())).len(),
        5184
    );
}

#[test]
fn age_range_clock_and_palette_changes_keep_the_same_point_cache_key() {
    use super::super::appearance::AppearanceKey;
    use gearbox_api::wire::draw_data::{DataFrame, Presentation};
    let frame = DataFrame::default();
    let original = Presentation {
        color: color(AgeClock::Simulation, [0.0, 1.0]),
        ..Default::default()
    };
    let expected = AppearanceKey::new(&original, &frame).unwrap();
    for clock in [AgeClock::Simulation, AgeClock::Source] {
        for range in [[0.5, 0.5], [-f64::MAX, f64::MAX], [-1e-300, 1e-300]] {
            for colormap in super::super::color_tests::MAPS {
                let changed = Presentation {
                    color: ColorMode::Age {
                        clock,
                        range,
                        colormap,
                    },
                    ..original.clone()
                };
                assert_eq!(AppearanceKey::new(&changed, &frame).unwrap(), expected);
            }
        }
    }
}

fn color(clock: AgeClock, range: [f64; 2]) -> ColorMode {
    ColorMode::Age {
        clock,
        range,
        colormap: gearbox_api::colormap::GREY,
    }
}

fn capture(simulation_time_s: f64) -> CaptureStamp {
    CaptureStamp {
        simulation_time_s,
        source_time_ns: Some(0),
        source_clock: Some("clock".into()),
        sample: 0,
        scene_epoch: 1,
    }
}

fn coordinate(age: AgeUniform, offset: u64) -> f32 {
    let anchor = u64::from(age.anchor.x) | (u64::from(age.anchor.y) << 32);
    if age.anchor.z == 0 || offset == anchor {
        return age.mapping.x;
    }
    if age.anchor.z == 2 {
        return if offset < anchor { 1.0 } else { 0.0 };
    }
    let distance = offset.abs_diff(anchor);
    let magnitude = (distance >> 32) as f32 * 4294967296.0 + distance as u32 as f32;
    (age.mapping.x
        + if offset < anchor {
            magnitude
        } else {
            -magnitude
        } * age.mapping.y)
        .clamp(0.0, 1.0)
}

fn palette(age: AgeUniform, offset: u64) -> u32 {
    let seconds = offset / SECOND as u64;
    let remainder = (offset % SECOND as u64) as u32;
    let borrow = u32::from(remainder > age.palette.z);
    let reference = (u64::from(age.palette.x) | (u64::from(age.palette.y) << 32)) as i64;
    let rounded = reference - seconds as i64 - i64::from(borrow);
    let tie = u32::from(age.palette.w & 8 != 0 && remainder == age.palette.z && rounded <= 0);
    let rounded = rounded - i64::from(tie);
    let raw = rounded.clamp(0, 255) as u32;
    let map = if rounded < 0 { 255 } else { raw };
    let okabe = ((age.palette.w & 7) + 16 - (seconds as u32 & 7) - borrow - tie) & 7;
    map | (raw << 8) | (okabe << 16)
}

#[test]
fn binary_seconds_split_keeps_integer_nanoseconds_and_subnormal_fractions() {
    for (seconds, integer) in [
        (0.0, 0),
        (0.5, 500_000_000),
        (1.0, SECOND),
        (17179869184.5, 17179869184500000000),
    ] {
        let parts = Nanoseconds::seconds(seconds).unwrap();
        assert_eq!(parts.whole, integer);
        assert!(parts.fraction.zero());
    }
    for value in [f64::from_bits(1), 1e-300, 1e-9, 0.499999999, 0.500000001] {
        let positive = Nanoseconds::seconds(value).unwrap();
        let negative = Nanoseconds::seconds(-value).unwrap();
        assert_eq!(positive.whole, -negative.whole);
        assert_eq!(positive.fraction.value(), -negative.fraction.value());
        assert!(!positive.fraction.zero(), "{value}");
    }
    assert!(Nanoseconds::seconds(f64::MAX).is_none());
}

#[test]
fn finite_page_parameters_cover_the_full_f64_range_domain() {
    for clock in [AgeClock::Source, AgeClock::Simulation] {
        for now in [
            0.0,
            f64::from_bits(1),
            1e-300,
            0.5,
            1e10,
            1e30,
            1e300,
            f64::MAX,
        ] {
            for range in [
                [0.0, 0.0],
                [-1e-300, 1e-300],
                [f64::from_bits(1), f64::from_bits(2)],
                [-f64::MAX, f64::MAX],
                [1e300, f64::from_bits(1e300f64.to_bits() + 1)],
                [-1.0, 1.0],
                [0.0, 1e300],
            ] {
                let age = AgeUniform::new(&color(clock, range), Some(&capture(0.0)), now, u64::MAX);
                assert!(
                    age.mapping.is_finite(),
                    "{clock:?} {now} {range:?}: {age:?}"
                );
                for offset in [0, 1, 500_000_000, 1 << 53, u64::MAX] {
                    let value = coordinate(age, offset);
                    assert!(value.is_finite() && (0.0..=1.0).contains(&value), "{age:?}");
                }
            }
        }
    }
}

#[test]
fn tiny_age_windows_and_degenerate_ranges_do_not_become_nan_or_flat_f32_ranges() {
    let stamp = capture(0.0);
    for range in [[-1e-300, 1e-300], [-f64::from_bits(1), f64::from_bits(1)]] {
        let age = AgeUniform::new(&color(AgeClock::Source, range), Some(&stamp), 0.0, 0);
        assert_eq!(coordinate(age, 0), 0.5);
        assert_eq!(coordinate(age, 1), 0.0);
    }
    let age = AgeUniform::new(
        &color(AgeClock::Source, [0.0, 0.0]),
        Some(&stamp),
        0.0,
        u64::MAX,
    );
    for offset in [0, 1, u64::MAX] {
        assert_eq!(coordinate(age, offset), 0.5);
    }
    let base = 1u64 << 30;
    let age = AgeUniform::new(
        &color(
            AgeClock::Source,
            [base as f64, base as f64 + 2.0f64.powi(-20)],
        ),
        Some(&stamp),
        0.0,
        base * SECOND as u64 + 512,
    );
    for offset in [0, 1, 128, 511, 512, 513] {
        let expected =
            ((512i64 - offset as i64) as f64 / (2.0f64.powi(-20) * 1e9)).clamp(0.0, 1.0) as f32;
        assert!((coordinate(age, offset) - expected).abs() <= 6e-8);
    }
}

#[test]
fn simulation_subtraction_preserves_small_age_after_a_large_acquisition_offset() {
    let base = (1u64 << 34) * SECOND as u64;
    let range = [0.499999999, 0.500000001];
    for captured in [0.0, 1e-9] {
        let age = AgeUniform::new(
            &color(AgeClock::Simulation, range),
            Some(&capture(captured)),
            (1u64 << 34) as f64 + 0.5,
            0,
        );
        for ns in 0..=3 {
            let expected = super::super::appearance::display_coordinate(
                0.5 - captured - ns as f64 / 1e9,
                range,
            );
            assert!(
                (coordinate(age, base + ns) - expected).abs() < 8e-8,
                "capture {captured} offset {ns}: {age:?}"
            );
        }
        assert_eq!(
            palette(age, base),
            super::super::palette::indices(0.5 - captured)
        );
    }
}

#[test]
fn indexed_source_age_uses_integer_rounding_and_modulo_at_u64_max() {
    let stamp = capture(0.0);
    let age = AgeUniform::new(
        &color(AgeClock::Source, [-1.0, 1.0]),
        Some(&stamp),
        0.0,
        u64::MAX,
    );
    for offset in [
        0,
        1,
        u64::MAX - 500_000_001,
        u64::MAX - 500_000_000,
        u64::MAX - 499_999_999,
        u64::MAX,
    ] {
        let ns = u64::MAX - offset;
        let rounded = ns / SECOND as u64 + u64::from(ns % SECOND as u64 >= 500_000_000);
        let byte = rounded.min(255) as u32;
        assert_eq!(
            palette(age, offset),
            byte | byte << 8 | ((rounded % 8) as u32) << 16
        );
    }
    for now in [0, 1, 499_999_999, 500_000_000, 500_000_001] {
        let age = AgeUniform::new(
            &color(AgeClock::Source, [-1.0, 1.0]),
            Some(&stamp),
            0.0,
            now,
        );
        for offset in [0, 1, 500_000_000, 1_000_000_000, u64::MAX] {
            let ns = i128::from(now) - i128::from(offset);
            let magnitude = ns.unsigned_abs();
            let rounded = (magnitude / SECOND as u128
                + u128::from(magnitude % SECOND as u128 >= 500_000_000))
                as i128
                * ns.signum();
            let raw = rounded.clamp(0, 255) as u32;
            let map = if rounded < 0 { 255 } else { raw };
            assert_eq!(
                palette(age, offset),
                map | raw << 8 | (rounded.rem_euclid(8) as u32) << 16
            );
        }
    }
}

#[test]
fn huge_simulation_clock_keeps_fractional_capture_phase_in_indexed_palettes() {
    for now in [1e100, 1e300, f64::MAX] {
        let age = AgeUniform::new(
            &color(AgeClock::Simulation, [now, now]),
            Some(&capture(1.25)),
            now,
            0,
        );
        for offset in [0, 250_000_000, 250_000_001, 500_000_000, u64::MAX] {
            let rounded = (-((offset % 8_000_000_000) as f64) / 1e9 - 1.25 + 0.5).floor() as i64;
            let expected = 255 | 255 << 8 | (rounded.rem_euclid(8) as u32) << 16;
            assert_eq!(palette(age, offset), expected, "{now} {offset}");
            let future = AgeUniform::new(
                &color(AgeClock::Simulation, [0.0, 0.0]),
                Some(&capture(now)),
                1.25,
                0,
            );
            let rounded = (1.25 - (offset % 8_000_000_000) as f64 / 1e9 - 0.5).ceil() as i64;
            assert_eq!(
                palette(future, offset),
                255 | (rounded.rem_euclid(8) as u32) << 16
            );
        }
    }
}
