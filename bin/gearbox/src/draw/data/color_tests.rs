use std::sync::Arc;

use gearbox_api::{
    draw_data::CpuBudget,
    wire::{colormap, draw_data::*, draw_geom},
};

use super::{
    appearance::{Appearance, Sample, VALUE_VALID},
    cloud::Geometry,
    palette::{attribute_indices, indices},
    style::Style,
    tests::{add, attribute, fixture, stored},
};

pub(super) const MAPS: [u32; 9] = [
    colormap::TURBO,
    colormap::VIRIDIS,
    colormap::RDYLGN,
    colormap::RAINBOW,
    colormap::GREY,
    colormap::COSTMAP,
    colormap::MAP,
    colormap::RAW,
    colormap::OKABE,
];

pub(super) fn presentation(map: u32) -> Presentation {
    Presentation {
        color: ColorMode::Scalar {
            name: "scalar".into(),
            colormap: map,
            range: [-10.0, 256.0],
            missing: [9, 23, 71, 127],
        },
        ..Default::default()
    }
}

pub(super) fn prepared(values: &[f64]) -> (CpuBudget, Arc<Appearance>) {
    let budget = CpuBudget::new(1 << 20);
    let points = vec![[0.0; 3]; values.len() + 1];
    let (mut schema, mut page) = fixture(&points, vec![]);
    let mut validity = vec![0u8; points.len().div_ceil(8)];
    for i in 0..values.len() {
        validity[i / 8] |= 1 << (i % 8);
    }
    add(
        &mut schema,
        &mut page,
        attribute(
            "scalar",
            ScalarType::F64,
            1,
            AttributeSemantic::Custom,
            "",
            true,
        ),
        values
            .iter()
            .copied()
            .chain([0.0])
            .flat_map(f64::to_le_bytes)
            .collect(),
        validity,
    );
    let geometry =
        Geometry::prepare(&budget, &schema, stored(&budget, &schema, &page, [0.0; 3])).unwrap();
    let appearance = Appearance::prepare(
        &budget,
        &schema,
        geometry,
        &presentation(colormap::VIRIDIS),
        &DataFrame::default(),
    )
    .unwrap();
    (budget, appearance)
}

/// The colour of a stored value sample: its ramp position is the value word's 8.12 fixed point.
pub(super) fn expected(style: &Style, sample: &Sample) -> [f32; 4] {
    let at = (super::appearance::value_word(sample.value, 0) & 0xf_ffff) as f32 / 4096.0;
    expected_at(style, sample, at)
}

pub(super) fn expected_at(style: &Style, sample: &Sample, at: f32) -> [f32; 4] {
    if sample.flags & VALUE_VALID == 0 {
        return style.missing.to_array();
    }
    if style.controls.y != 0 {
        return style.ramp
            [((sample.palette_indices >> ((style.controls.y - 1) * 8)) & 255) as usize]
            .to_array();
    }
    let low = at.floor() as usize;
    style.ramp[low]
        .lerp(style.ramp[(low + 1).min(255)], at.fract())
        .to_array()
}

#[test]
fn indexed_palettes_keep_rounding_negative_unknown_and_large_value_semantics() {
    for (value, bytes) in [
        (-1e300, [255, 0, 0]),
        (-8.5, [255, 0, 7]),
        (-1.0, [255, 0, 7]),
        (-0.5, [255, 0, 7]),
        (-0.499, [0, 0, 0]),
        (0.0, [0, 0, 0]),
        (0.5, [1, 1, 1]),
        (98.5, [99, 99, 3]),
        (255.0, [255, 255, 7]),
        (256.0, [255, 255, 0]),
        (16_777_217.0, [255, 255, 1]),
        (9_007_199_254_740_991.0, [255, 255, 7]),
        (1e300, [255, 255, 0]),
    ] {
        let packed = indices(value);
        assert_eq!(
            [packed & 255, (packed >> 8) & 255, packed >> 16],
            bytes,
            "{value}"
        );
    }
}

#[test]
fn unnormalized_u64_palette_indices_never_pass_through_float() {
    let field = attribute(
        "scalar",
        ScalarType::U64,
        1,
        AttributeSemantic::Custom,
        "",
        false,
    );
    for value in [0u64, 1, 255, 256, (1 << 53) + 1, u64::MAX - 1, u64::MAX] {
        let bytes = value.to_le_bytes();
        let actual = attribute_indices(
            ColumnView {
                values: &bytes,
                validity: &[],
            },
            &field,
            0,
            value as f64,
        )
        .unwrap();
        assert_eq!(actual & 255, value.min(255) as u32);
        assert_eq!((actual >> 8) & 255, value.min(255) as u32);
        assert_eq!(actual >> 16, (value % 8) as u32);
    }
    let mut field = field;
    field.normalization = Normalization::Unorm;
    assert!(field.validate().is_err());
    assert!(
        attribute_indices(
            ColumnView {
                values: &[],
                validity: &[]
            },
            &Attribute {
                normalization: Normalization::None,
                ..field
            },
            0,
            1.0
        )
        .is_err()
    );
}

#[test]
fn attribute_preparation_applies_normalization_before_palette_indexing() {
    let cases = [
        (
            ScalarType::U8,
            Normalization::None,
            vec![255],
            indices(255.0),
        ),
        (
            ScalarType::U8,
            Normalization::Unorm,
            vec![255],
            indices(1.0),
        ),
        (
            ScalarType::U16,
            Normalization::None,
            257u16.to_le_bytes().to_vec(),
            indices(257.0),
        ),
        (
            ScalarType::U32,
            Normalization::None,
            u32::MAX.to_le_bytes().to_vec(),
            indices(f64::from(u32::MAX)),
        ),
        (
            ScalarType::U64,
            Normalization::None,
            ((1u64 << 53) + 1).to_le_bytes().to_vec(),
            255 | (255 << 8) | (1 << 16),
        ),
        (
            ScalarType::U16,
            Normalization::Unorm,
            u16::MAX.to_le_bytes().to_vec(),
            indices(1.0),
        ),
        (
            ScalarType::I16,
            Normalization::None,
            (-9i16).to_le_bytes().to_vec(),
            indices(-9.0),
        ),
        (
            ScalarType::I16,
            Normalization::Snorm,
            i16::MIN.to_le_bytes().to_vec(),
            indices(-1.0),
        ),
        (
            ScalarType::I32,
            Normalization::None,
            i32::MIN.to_le_bytes().to_vec(),
            indices(f64::from(i32::MIN)),
        ),
        (
            ScalarType::F32,
            Normalization::None,
            98.5f32.to_le_bytes().to_vec(),
            indices(98.5),
        ),
        (
            ScalarType::F64,
            Normalization::None,
            16_777_217f64.to_le_bytes().to_vec(),
            indices(16_777_217.0),
        ),
    ];
    let budget = CpuBudget::new(1 << 20);
    for (scalar, normalization, bytes, expected) in cases {
        let (mut schema, mut page) = fixture(&[[0.0; 3]], vec![]);
        let mut field = attribute("scalar", scalar, 1, AttributeSemantic::Custom, "", false);
        field.normalization = normalization;
        add(&mut schema, &mut page, field, bytes, vec![]);
        let geometry =
            Geometry::prepare(&budget, &schema, stored(&budget, &schema, &page, [0.0; 3])).unwrap();
        let appearance = Appearance::prepare(
            &budget,
            &schema,
            geometry,
            &presentation(colormap::OKABE),
            &DataFrame::default(),
        )
        .unwrap();
        assert_eq!(
            appearance.samples()[0].palette_indices, expected,
            "{scalar:?}/{normalization:?}"
        );
    }
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn prepared_scalar_preserves_measurements_missing_values_and_raw_indices() {
    let values = [-1.0, 0.0, 99.0, 100.0, 255.0, 16_777_217.0, 1e300];
    let (budget, appearance) = prepared(&values);
    let samples = appearance.samples();
    for (index, value) in values.into_iter().enumerate() {
        let sample = samples[index];
        assert_eq!(sample.flags, VALUE_VALID);
        assert_eq!(sample.palette_indices, indices(value));
        let column = appearance.geometry.source().unwrap().column(2).unwrap();
        assert_eq!(
            &column.values[index * 8..index * 8 + 8],
            &value.to_le_bytes()
        );
        assert_eq!(
            appearance.geometry.source_id(index),
            Some(u64::MAX - index as u64)
        );
    }
    assert_eq!(samples.last().unwrap().flags, 0);
    assert_eq!(appearance.attributes.layout.stride(), 1);
    assert_eq!(appearance.attributes.bytes().len(), samples.len() * 4 + 4);
    drop(appearance);
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn indexed_lookup_matches_named_api_palettes_without_interpolation() {
    let values = [
        -9.5, -1.0, -0.5, 0.0, 0.5, 50.0, 98.5, 99.0, 100.0, 127.0, 128.0, 254.0, 255.0, 256.0,
    ];
    let (_, appearance) = prepared(&values);
    let samples = appearance.samples();
    for map in MAPS {
        let style = Style::new(&presentation(map));
        for (index, value) in values.into_iter().enumerate() {
            let sample = &samples[index];
            if colormap::is_palette(map) {
                let bytes = draw_geom::palette(map, value as f32);
                assert_eq!(
                    expected(&style, sample),
                    bytes.map(|v| f32::from(v) / 255.0),
                    "map {map}, value {value}"
                );
            } else {
                assert_eq!(style.controls.y, 0);
                for (i, color) in style.ramp.iter().enumerate() {
                    assert_eq!(
                        color.to_array(),
                        draw_geom::ramp(map, i as f32 / 255.0).map(|v| f32::from(v) / 255.0)
                    );
                }
            }
        }
        assert_eq!(
            expected(&style, samples.last().unwrap()),
            style.missing.to_array()
        );
    }
}

#[test]
fn binary_alpha_is_cutout_but_fractional_alpha_remains_transmitting() {
    use super::alpha::{AlphaStyle, OPAQUE, Summary, TRANSMITTING};
    let summary = Summary {
        valid_value: true,
        missing_value: true,
        valid_category: true,
        missing_category: true,
        ..Default::default()
    };
    let passes = |style: &Style| AlphaStyle::new(style).required(summary, 4);
    let mut p = presentation(colormap::COSTMAP);
    if let ColorMode::Scalar { missing, .. } = &mut p.color {
        *missing = [20, 30, 40, 0];
    }
    let style = Style::new(&p);
    assert_eq!(style.ramp[0].w, 0.0);
    assert_eq!(passes(&style), OPAQUE);
    p.opacity = 0.9;
    assert_eq!(passes(&Style::new(&p)), TRANSMITTING);
    p.opacity = 1.0;
    if let ColorMode::Scalar { missing, .. } = &mut p.color {
        missing[3] = 127;
    }
    assert_eq!(passes(&Style::new(&p)), OPAQUE | TRANSMITTING);
    p.color = ColorMode::Category {
        name: "class".into(),
        entries: vec![PaletteEntry {
            value: 3,
            rgba: [30, 40, 50, 0],
        }],
        missing: [0, 0, 0, 255],
    };
    assert_eq!(passes(&Style::new(&p)), OPAQUE);
    if let ColorMode::Category { entries, .. } = &mut p.color {
        entries[0].rgba[3] = 127;
    }
    assert_eq!(passes(&Style::new(&p)), OPAQUE | TRANSMITTING);
}
