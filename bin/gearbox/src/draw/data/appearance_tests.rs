use super::super::{
    test_allocations,
    tests::{fixture, stored},
};
use super::*;
use std::hint::black_box;

#[test]
fn inline_names_preserve_utf8_and_reject_invalid_names_without_truncation() {
    for value in [
        "x".into(),
        "a".repeat(63),
        format!("{}é", "a".repeat(61)),
        "温度".into(),
    ] {
        let name = Name::new(&value).unwrap();
        assert_eq!(&*name, value);
        assert_eq!(Name::new(&name).unwrap(), name);
    }
    for value in [
        "".into(),
        "a".repeat(64),
        format!("{}é", "a".repeat(62)),
        "bad\nname".into(),
        "bad\u{85}".into(),
    ] {
        assert!(Name::new(&value).is_err());
    }
    assert_eq!(size_of::<Name>(), 64);
    assert!(!std::mem::needs_drop::<AppearanceKey>());
}

#[test]
fn selected_and_radius_keys_allocate_nothing_before_admission() {
    let name = format!("{}é", "a".repeat(61));
    let colors = [
        ColorMode::Attribute {
            name: name.clone(),
            space: ColorSpace::Srgb,
        },
        ColorMode::Scalar {
            name: name.clone(),
            range: [0.0, 1.0],
            colormap: 0,
            missing: [0; 4],
        },
        ColorMode::Category {
            name,
            entries: vec![],
            missing: [0; 4],
        },
    ];
    let frame = DataFrame::default();
    let budget = CpuBudget::new(1024);
    let held = budget.reserve(1024).unwrap();
    for color in colors {
        let presentation = Presentation {
            color,
            radius_attribute: Some("半径".into()),
            ..Default::default()
        };
        let (_, allocations) = test_allocations::count(|| {
            for _ in 0..100 {
                let key = black_box(AppearanceKey::new(black_box(&presentation), &frame).unwrap());
                assert_eq!(black_box(key.clone()), key);
                assert_eq!(key.radius_attribute.as_deref(), Some("半径"));
            }
        });
        assert_eq!(allocations, 0);
    }
    drop(held);
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn key_identity_retains_full_names_and_ignores_style_only_changes() {
    let frame = DataFrame::default();
    let mut presentation = Presentation {
        color: ColorMode::Attribute {
            name: "temperature".into(),
            space: ColorSpace::Srgb,
        },
        radius_attribute: Some("radius".into()),
        ..Default::default()
    };
    let key = AppearanceKey::new(&presentation, &frame).unwrap();
    presentation.glow = 0.3;
    presentation.blur = 0.7;
    assert_eq!(AppearanceKey::new(&presentation, &frame).unwrap(), key);
    presentation.radius_attribute = Some("radius_2".into());
    assert_ne!(AppearanceKey::new(&presentation, &frame).unwrap(), key);
    presentation.radius_attribute = Some("radius".into());
    if let ColorMode::Attribute { name, .. } = &mut presentation.color {
        name.push('2');
    }
    assert_ne!(AppearanceKey::new(&presentation, &frame).unwrap(), key);
    presentation.radius_attribute = Some("x".repeat(64));
    assert!(AppearanceKey::new(&presentation, &frame).is_err());
}

#[test]
fn appearance_charges_inline_metadata_and_releases_exactly() {
    let budget = CpuBudget::new(1 << 20);
    let (schema, payload) = fixture(&[[0.0, 0.0, 1.0]], vec![]);
    let geometry = Geometry::prepare(
        &budget,
        &schema,
        stored(&budget, &schema, &payload, [0.0; 3]),
    )
    .unwrap();
    let before = budget.usage().committed();
    let appearance = Appearance::prepare(
        &budget,
        &schema,
        geometry.clone(),
        &Presentation::default(),
        &DataFrame::default(),
    )
    .unwrap();
    assert_eq!(appearance.attributes.layout.stride(), 0);
    assert_eq!(
        budget.usage().committed() - before,
        (size_of::<Appearance>() + 2 * size_of::<usize>() + size_of::<super::super::coarse::Samples>() + 16)
            as u64
            + appearance.attributes.layout.bytes()
            + appearance.coarse.attributes.layout.bytes()
    );
    drop(appearance);
    assert_eq!(budget.usage().committed(), before);
    drop(geometry);
    assert_eq!(budget.usage().committed(), 0);
}
