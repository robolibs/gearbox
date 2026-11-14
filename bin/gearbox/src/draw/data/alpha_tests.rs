use super::*;
use gearbox_api::wire::{colormap, draw_data::*};

fn required(color: ColorMode, records: &[Sample], opacity: f32, samples: u32) -> u8 {
    AlphaStyle::new(&Style::new(&Presentation {
        color,
        opacity,
        ..Default::default()
    }))
    .required(Summary::new(records), samples)
}

#[test]
fn rgba_pages_route_opaque_and_fractional_samples_separately() {
    let records = [0u8, 127, 255].map(|alpha| Sample {
        rgba: u32::from_le_bytes([20, 70, 130, alpha]),
        flags: COLOR_VALID,
        ..Default::default()
    });
    let color = ColorMode::Attribute {
        name: "rgba".into(),
        space: ColorSpace::Srgb,
    };
    assert_eq!(
        required(color.clone(), &records, 1.0, 4),
        OPAQUE | TRANSMITTING
    );
    assert_eq!(
        required(color.clone(), &records[1..2], 1.0, 4),
        TRANSMITTING
    );
    assert_eq!(required(color.clone(), &records[2..], 1.0, 4), OPAQUE);
    assert_eq!(required(color.clone(), &records, 0.5, 4), TRANSMITTING);
    assert_eq!(required(color.clone(), &records, 0.0, 4), OPAQUE);
    assert_eq!(
        required(
            color,
            &[Sample {
                rgba: u32::MAX,
                ..Default::default()
            }],
            1.0,
            4
        ),
        OPAQUE
    );
}

#[test]
fn unused_missing_color_does_not_remove_depth_from_valid_ages() {
    let color = ColorMode::Age {
        clock: AgeClock::Simulation,
        colormap: colormap::VIRIDIS,
        range: [0.0, 1.0],
    };
    let valid = Sample {
        flags: VALUE_VALID,
        ..Default::default()
    };
    assert_eq!(required(color.clone(), &[valid], 1.0, 4), OPAQUE);
    assert_eq!(
        required(color.clone(), &[valid, Sample::default()], 1.0, 4),
        OPAQUE | TRANSMITTING
    );
    assert_eq!(required(color, &[Sample::default()], 1.0, 4), TRANSMITTING);
}

#[test]
fn single_sample_opaque_edges_require_a_second_pass_but_transmission_does_not() {
    for alpha in [0, 127, 255] {
        let color = ColorMode::Constant {
            rgba: [20, 60, 90, alpha],
        };
        assert_eq!(
            required(color, &[], 1.0, 1),
            match alpha {
                0 => OPAQUE,
                255 => OPAQUE | TRANSMITTING,
                _ => TRANSMITTING,
            }
        );
    }
    for samples in [1, 2, 4, 8] {
        for opaque in [false, true] {
            let state = RasterState { opaque, samples };
            assert_eq!(
                state.multisample().alpha_to_coverage_enabled,
                opaque && samples > 1
            );
            assert_eq!(
                state.depth(TextureFormat::Depth32Float).depth_write_enabled,
                Some(opaque)
            );
            assert_eq!(state.blend().is_some(), !opaque);
        }
    }
}

#[test]
fn glow_uniform_preserves_the_full_finite_api_domain() {
    for glow in [0.0, 0.22, 4.0, 8.0, 1e10, f32::MAX] {
        let presentation = Presentation {
            glow,
            ..Default::default()
        };
        presentation.validate_intrinsic().unwrap();
        assert_eq!(Style::new(&presentation).effects.y, glow);
    }
}
