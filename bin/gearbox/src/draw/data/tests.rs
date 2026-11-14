use std::sync::Arc;

use gearbox_api::draw_data::{CpuBudget, PageUpload, StoredPage};
use gearbox_api::wire::{colormap, draw_data::*};

use super::appearance::*;
use super::cloud::*;

#[test]
fn radius_units_reject_nonfinite_or_nonpositive_values_without_an_upper_api_cap() {
    for radius in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, -0.0, 0.0] {
        for size in [PointSize::Metres { radius }, PointSize::Pixels { radius }] {
            let presentation = Presentation {
                size,
                ..Default::default()
            };
            assert!(presentation.validate(None, None, false).is_err());
        }
    }
    for radius in [f32::from_bits(1), f32::MIN_POSITIVE, 0.125, 1e30, f32::MAX] {
        for size in [PointSize::Metres { radius }, PointSize::Pixels { radius }] {
            let presentation = Presentation {
                size,
                ..Default::default()
            };
            presentation.validate(None, None, false).unwrap();
        }
    }
}

pub(super) fn attribute(
    name: &str,
    scalar: ScalarType,
    components: u8,
    semantic: AttributeSemantic,
    units: &str,
    nullable: bool,
) -> Attribute {
    Attribute {
        name: name.into(),
        scalar,
        components,
        semantic,
        units: units.into(),
        normalization: Normalization::None,
        nullable,
    }
}

pub(super) fn fixture(points: &[[f64; 3]], mask: Vec<u8>) -> (CloudSchema, CloudPage) {
    let schema = CloudSchema {
        revision: 1,
        attributes: vec![
            attribute(
                "xyz",
                ScalarType::F64,
                3,
                AttributeSemantic::Position,
                "m",
                true,
            ),
            attribute(
                "id",
                ScalarType::U64,
                1,
                AttributeSemantic::SourceId,
                "",
                false,
            ),
        ],
    };
    let columns = vec![
        ColumnData {
            values: points
                .iter()
                .flatten()
                .flat_map(|v| v.to_le_bytes())
                .collect(),
            validity: mask,
        },
        ColumnData {
            values: (0..points.len())
                .flat_map(|i| (u64::MAX - i as u64).to_le_bytes())
                .collect(),
            validity: vec![],
        },
    ];
    (
        schema,
        CloudPage {
            count: points.len() as u32,
            columns,
        },
    )
}

pub(super) fn stored(
    budget: &CpuBudget,
    schema: &CloudSchema,
    page: &CloudPage,
    origin_m: [f64; 3],
) -> Arc<StoredPage> {
    stored_address(budget, schema, page, origin_m, 0)
}

pub(super) fn stored_address(
    budget: &CpuBudget,
    schema: &CloudSchema,
    page: &CloudPage,
    origin_m: [f64; 3],
    page_id: u64,
) -> Arc<StoredPage> {
    let bytes = page.encode(schema).unwrap();
    let descriptor = PageDescriptor {
        address: PageAddress {
            scan_key: 0,
            page_id,
        },
        encoding: PageEncoding::CloudColumnsV1,
        items: page.count,
        byte_len: bytes.len() as u64,
        crc32: page_crc32(&bytes),
        origin_m,
    };
    let mut upload = PageUpload::new(budget, descriptor).unwrap();
    for (index, chunk) in bytes.chunks(DATA_CHUNK_BYTES).enumerate() {
        upload
            .push((index * DATA_CHUNK_BYTES) as u64, chunk)
            .unwrap();
    }
    upload.finish_cloud(schema).unwrap().into_page()
}

pub(super) fn add(
    schema: &mut CloudSchema,
    page: &mut CloudPage,
    field: Attribute,
    values: Vec<u8>,
    validity: Vec<u8>,
) {
    schema.attributes.push(field);
    page.columns.push(ColumnData { values, validity });
}

#[test]
fn compact_records_store_only_the_selected_colour_words() {
    assert_eq!(size_of::<Position>(), 12);
    let selectors = [
        (Selector::Constant, 0),
        (Selector::Attribute(name("rgba")), 4),
        (Selector::Scalar(name("scalar"), [0.0, 1.0]), 4),
        (Selector::Axis(2, [0.0, 1.0]), 4),
        (Selector::Range([0.0; 3], [0.0, 1.0]), 4),
        (Selector::Category(name("class")), 4),
        (Selector::Age, 8),
    ];
    for (selector, bytes) in selectors {
        for radius in [false, true] {
            let layout = Layout::new(&selector, radius, 1_000_000);
            let mask = if layout.mask { 1_000_000u64.div_ceil(32) * 4 } else { 0 };
            assert_eq!(
                layout.bytes(),
                (bytes + 4 * u64::from(radius)) * 1_000_000 + mask,
                "{selector:?} radius {radius}"
            );
            assert_eq!(layout.mask, bytes > 0 && !matches!(selector, Selector::Attribute(_)));
        }
    }
}

#[test]
fn packed_words_round_trip_every_selector_validity_and_radius() {
    let budget = CpuBudget::new(1 << 20);
    let value = Sample {
        value: 0.5,
        palette_indices: 255 | 3 << 16,
        flags: VALUE_VALID | RADIUS_VALID,
        radius_m: 0.25,
        ..Default::default()
    };
    let cases = [
        (Selector::Attribute(name("rgba")), Sample { rgba: 0x7f50_1ec8, flags: COLOR_VALID, ..Default::default() }),
        (Selector::Scalar(name("scalar"), [0.0, 1.0]), value),
        (Selector::Category(name("class")), Sample { category: u32::MAX, flags: CATEGORY_VALID, ..Default::default() }),
        (Selector::Age, Sample { acquisition_ns: [7, u32::MAX], flags: VALUE_VALID, ..Default::default() }),
    ];
    for (selector, sample) in cases {
        let mut attributes = Attributes::new(&budget, Layout::new(&selector, true, 40)).unwrap();
        attributes.set(33, &sample);
        attributes.set(34, &Sample::default());
        let expected = Sample { flags: sample.flags & !RADIUS_VALID, radius_m: 0.0, ..sample };
        let radius = Sample { radius_m: 0.25, flags: expected.flags | RADIUS_VALID, ..expected };
        let stored = if sample.flags & RADIUS_VALID != 0 { radius } else { expected };
        assert_eq!(attributes.sample(33), Some(stored), "{selector:?}");
        assert_eq!(attributes.sample(34), Some(Sample::default()), "{selector:?}");
        assert_eq!(attributes.sample(40), None);
        let coarse = attributes.gather(&budget, &[34, 33]).unwrap();
        assert_eq!(coarse.sample(1), Some(stored));
        assert_eq!(coarse.sample(0), Some(Sample::default()));
    }
    assert_eq!(super::appearance::value_parts(value_word(0.5, 255 | 3 << 16)), (0.5, 255 | 3 << 16));
    assert_eq!(value_word(1.0, 0) & 0xf_ffff, 255 * 4096);
    assert_eq!(budget.usage().committed(), 0);
}

fn name(value: &str) -> Name {
    Name::new(value).unwrap()
}

#[test]
fn validity_compaction_preserves_exact_source_ids_and_original_ordinals() {
    let budget = CpuBudget::new(1 << 20);
    let (schema, page) = fixture(&[[1.0, 2.0, 3.0], [0.0; 3], [4.0, 5.0, 6.0]], vec![0b101]);
    let source = stored(&budget, &schema, &page, [0.0; 3]);
    let geometry = Geometry::prepare(&budget, &schema, source).unwrap();
    assert_eq!(
        (0..3).map(|i| geometry.source_row(i)).collect::<Vec<_>>(),
        [Some(0), Some(2), None]
    );
    assert_eq!(geometry.source_id(0), Some(u64::MAX));
    assert_eq!(geometry.source_id(1), Some(u64::MAX - 2));
    assert_eq!(geometry.source_id(2), None);
    assert_eq!(geometry.source_position(1).unwrap(), [4.0, 5.0, 6.0]);
    assert_eq!(geometry.bounds_m, Some([[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]));
}

#[test]
fn f64_rebasing_keeps_small_clusters_at_large_offsets() {
    let budget = CpuBudget::new(1 << 20);
    let (schema, page) = fixture(
        &[[1e15; 3], [1e15 + 0.125, 1e15 + 0.25, 1e15 - 0.5]],
        vec![],
    );
    let source = stored(&budget, &schema, &page, [1e20; 3]);
    let geometry = Geometry::prepare(&budget, &schema, source).unwrap();
    assert_eq!(geometry.rebase_m, [1e15; 3]);
    assert_eq!(geometry.positions.values[1].xyz, [0.125, 0.25, -0.5]);
    assert_eq!(geometry.max_error_m, 0.0);
}

#[test]
fn unrepresentable_extent_fails_without_leaking_prepared_records() {
    let budget = CpuBudget::new(1 << 20);
    for point in [[1e100; 3], [100_000.123456; 3], [65_536.0008; 3]] {
        let (schema, page) = fixture(&[[0.0; 3], point], vec![]);
        let source = stored(&budget, &schema, &page, [0.0; 3]);
        let before = budget.usage().committed();
        assert!(Geometry::prepare(&budget, &schema, source.clone()).is_err());
        assert_eq!(budget.usage().committed(), before);
    }
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn all_invalid_and_empty_pages_have_no_invented_geometry() {
    for (points, mask) in [(vec![[0.0; 3]; 3], vec![0]), (vec![], vec![])] {
        let budget = CpuBudget::new(1 << 20);
        let (schema, page) = fixture(&points, mask);
        let geometry =
            Geometry::prepare(&budget, &schema, stored(&budget, &schema, &page, [0.0; 3])).unwrap();
        assert!(geometry.positions.values.is_empty());
        assert_eq!(geometry.bounds_m, None);
        assert_eq!(geometry.max_error_m, 0.0);
    }
}

#[test]
fn display_ranges_handle_constants_extremes_and_clamping_without_overflow() {
    assert_eq!(display_coordinate(2.0, [2.0, 2.0]), 0.5);
    assert_eq!(display_coordinate(0.0, [-f64::MAX, f64::MAX]), 0.5);
    assert_eq!(display_coordinate(-5.0, [0.0, 1.0]), 0.0);
    assert_eq!(display_coordinate(5.0, [0.0, 1.0]), 1.0);
    assert_eq!(display_coordinate(1e15 + 0.25, [1e15, 1e15 + 0.5]), 0.5);
    assert_eq!(
        display_coordinate(f64::MIN_POSITIVE, [0.0, f64::MIN_POSITIVE * 2.0]),
        0.5
    );
}

#[test]
fn scalar_decode_and_normalization_cover_every_declared_type() {
    let cases = [
        (ScalarType::U8, vec![255], 255.0),
        (
            ScalarType::U16,
            u16::MAX.to_le_bytes().to_vec(),
            u16::MAX as f64,
        ),
        (
            ScalarType::U32,
            u32::MAX.to_le_bytes().to_vec(),
            u32::MAX as f64,
        ),
        (
            ScalarType::U64,
            u64::MAX.to_le_bytes().to_vec(),
            u64::MAX as f64,
        ),
        (
            ScalarType::I16,
            i16::MIN.to_le_bytes().to_vec(),
            i16::MIN as f64,
        ),
        (
            ScalarType::I32,
            i32::MIN.to_le_bytes().to_vec(),
            i32::MIN as f64,
        ),
        (ScalarType::F32, 0.125f32.to_le_bytes().to_vec(), 0.125),
        (ScalarType::F64, 0.125f64.to_le_bytes().to_vec(), 0.125),
    ];
    for (scalar, bytes, expected) in cases {
        let column = ColumnView {
            values: &bytes,
            validity: &[],
        };
        let mut field = attribute("value", scalar, 1, AttributeSemantic::Intensity, "", true);
        assert_eq!(normalized(column, &field, 0).unwrap(), Some(expected));
        if !scalar.is_float() {
            field.normalization = if expected < 0.0 {
                Normalization::Snorm
            } else {
                Normalization::Unorm
            };
            assert_eq!(
                normalized(column, &field, 0).unwrap(),
                Some(expected.signum())
            );
        }
        assert_eq!(
            normalized(
                ColumnView {
                    validity: &[0],
                    ..column
                },
                &field,
                0
            )
            .unwrap(),
            None
        );
        assert!(scalar_value(&bytes, scalar, usize::MAX).is_err());
        assert!(scalar_value(&[], scalar, 0).is_err());
    }
}

#[test]
fn palette_blur_uniform_radius_and_placement_do_not_change_attribute_cache_keys() {
    let frame = DataFrame::default();
    let a = Presentation {
        color: ColorMode::Scalar {
            name: "intensity".into(),
            colormap: colormap::VIRIDIS,
            range: [0.0, 1.0],
            missing: [1; 4],
        },
        ..Default::default()
    };
    let mut b = a.clone();
    b.blur = 1.0;
    b.opacity = 0.2;
    b.glow = 0.1;
    b.size = PointSize::Metres { radius: 0.05 };
    if let ColorMode::Scalar {
        colormap, missing, ..
    } = &mut b.color
    {
        *colormap = colormap::TURBO;
        *missing = [2; 4];
    }
    let mut moved = frame.clone();
    moved.pose.translation_m = [1e12; 3];
    assert_eq!(
        AppearanceKey::new(&a, &frame).unwrap(),
        AppearanceKey::new(&b, &moved).unwrap()
    );
    if let ColorMode::Scalar { range, .. } = &mut b.color {
        *range = [-1.0, 1.0];
    }
    assert_ne!(
        AppearanceKey::new(&a, &frame).unwrap(),
        AppearanceKey::new(&b, &frame).unwrap()
    );
}

#[test]
fn rgba_category_and_metric_radius_keep_validity_and_exact_integer_values() {
    let budget = CpuBudget::new(1 << 20);
    let (mut schema, mut page) = fixture(&[[0.0; 3], [1.0; 3]], vec![]);
    let mut color = attribute(
        "rgba",
        ScalarType::U8,
        4,
        AttributeSemantic::Color,
        "",
        true,
    );
    color.normalization = Normalization::Unorm;
    add(
        &mut schema,
        &mut page,
        color,
        vec![200, 30, 80, 127, 0, 0, 0, 0],
        vec![1],
    );
    add(
        &mut schema,
        &mut page,
        attribute(
            "class",
            ScalarType::U32,
            1,
            AttributeSemantic::Category,
            "",
            true,
        ),
        [u32::MAX, 0]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect(),
        vec![1],
    );
    add(
        &mut schema,
        &mut page,
        attribute(
            "radius",
            ScalarType::F64,
            1,
            AttributeSemantic::Radius,
            "m",
            true,
        ),
        [0.125f64, 0.0]
            .into_iter()
            .flat_map(f64::to_le_bytes)
            .collect(),
        vec![1],
    );
    let geometry =
        Geometry::prepare(&budget, &schema, stored(&budget, &schema, &page, [0.0; 3])).unwrap();
    let mut presentation = Presentation {
        color: ColorMode::Attribute {
            name: "rgba".into(),
            space: ColorSpace::Srgb,
        },
        radius_attribute: Some("radius".into()),
        ..Default::default()
    };
    let colors = Appearance::prepare(
        &budget,
        &schema,
        geometry.clone(),
        &presentation,
        &DataFrame::default(),
    )
    .unwrap();
    assert_eq!(
        colors.samples()[0].rgba.to_le_bytes(),
        [200, 30, 80, 127]
    );
    assert_eq!(colors.samples()[0].flags, COLOR_VALID | RADIUS_VALID);
    assert_eq!(colors.samples()[1].flags, 0);
    presentation.color = ColorMode::Category {
        name: "class".into(),
        entries: vec![],
        missing: [50; 4],
    };
    let categories = Appearance::prepare(
        &budget,
        &schema,
        geometry.clone(),
        &presentation,
        &DataFrame::default(),
    )
    .unwrap();
    assert_eq!(categories.samples()[0].category, u32::MAX);
    assert_eq!(
        categories.samples()[0].flags,
        CATEGORY_VALID | RADIUS_VALID
    );
    assert_eq!(categories.samples()[1].flags, 0);
    assert!(Arc::ptr_eq(&colors.geometry, &categories.geometry));
    assert_eq!(categories.samples()[0].radius_m, 0.125);
}

#[test]
fn axis_and_range_colors_use_source_pose_and_both_origins_once() {
    let budget = CpuBudget::new(1 << 20);
    let (schema, page) = fixture(&[[1.0, 0.0, 0.0]], vec![]);
    let geometry = Geometry::prepare(
        &budget,
        &schema,
        stored(&budget, &schema, &page, [2.0, 0.0, 0.0]),
    )
    .unwrap();
    let s = 0.5f64.sqrt();
    let frame = DataFrame {
        origin_m: [3.0, 0.0, 0.0],
        pose: RigidPose {
            translation_m: [10.0, 20.0, 30.0],
            rotation_wxyz: [s, 0.0, 0.0, s],
        },
        ..Default::default()
    };
    let mut p = Presentation {
        color: ColorMode::Axis {
            axis: 1,
            colormap: colormap::VIRIDIS,
            range: [20.0, 32.0],
        },
        ..Default::default()
    };
    let a = Appearance::prepare(&budget, &schema, geometry.clone(), &p, &frame).unwrap();
    assert!((a.samples()[0].value - 0.5).abs() < 1e-6);
    p.color = ColorMode::Range {
        origin_m: [10.0, 20.0, 30.0],
        colormap: colormap::TURBO,
        range: [0.0, 12.0],
    };
    let b = Appearance::prepare(&budget, &schema, geometry, &p, &frame).unwrap();
    assert!((b.samples()[0].value - 0.5).abs() < 1e-6);
}

#[test]
fn prepared_ownership_is_charged_until_last_reader_and_budget_failure_is_atomic() {
    let budget = CpuBudget::new(1 << 20);
    let (schema, page) = fixture(&[[0.0; 3]; 8], vec![]);
    let source = stored(&budget, &schema, &page, [0.0; 3]);
    let baseline = budget.usage().committed();
    let geometry = Geometry::prepare(&budget, &schema, source.clone()).unwrap();
    let a = Appearance::prepare(
        &budget,
        &schema,
        geometry.clone(),
        &Presentation::default(),
        &DataFrame::default(),
    )
    .unwrap();
    assert!(budget.usage().scratch_bytes >= 8 * size_of::<Position>() as u64);
    let before = budget.usage().committed();
    let full = budget.reserve(budget.usage().limit - before).unwrap();
    assert!(
        Appearance::prepare(
            &budget,
            &schema,
            geometry.clone(),
            &Presentation::default(),
            &DataFrame::default()
        )
        .is_err()
    );
    drop(full);
    assert_eq!(budget.usage().committed(), before);
    let reader = a.clone();
    drop(a);
    drop(geometry);
    assert_eq!(budget.usage().committed(), before);
    drop(reader);
    assert_eq!(budget.usage().committed(), baseline);
    drop(source);
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn acquisition_offsets_are_nullable_and_ambiguous_columns_are_explicit_errors() {
    let budget = CpuBudget::new(1 << 20);
    let (mut schema, mut page) = fixture(&[[0.0; 3]; 2], vec![]);
    add(
        &mut schema,
        &mut page,
        attribute(
            "offset",
            ScalarType::U64,
            1,
            AttributeSemantic::AcquisitionOffset,
            "ns",
            true,
        ),
        [500_000_000u64, 0]
            .into_iter()
            .flat_map(u64::to_le_bytes)
            .collect(),
        vec![1],
    );
    let frame = DataFrame {
        capture: Some(CaptureStamp {
            simulation_time_s: 3.0,
            source_time_ns: Some(u64::MAX - 1_000_000_000),
            source_clock: Some("sensor".into()),
            sample: 1,
            scene_epoch: 1,
        }),
        ..Default::default()
    };
    let p = Presentation {
        color: ColorMode::Age {
            clock: AgeClock::Source,
            colormap: colormap::VIRIDIS,
            range: [0.0, 2.0],
        },
        ..Default::default()
    };
    let geometry =
        Geometry::prepare(&budget, &schema, stored(&budget, &schema, &page, [0.0; 3])).unwrap();
    let a = Appearance::prepare(&budget, &schema, geometry, &p, &frame).unwrap();
    assert_eq!(a.samples()[0].acquisition_ns, [500_000_000, 0]);
    assert_eq!(a.samples()[1].flags, 0);
    add(
        &mut schema,
        &mut page,
        attribute(
            "offset2",
            ScalarType::U64,
            1,
            AttributeSemantic::AcquisitionOffset,
            "ns",
            true,
        ),
        [0u64, 0].into_iter().flat_map(u64::to_le_bytes).collect(),
        vec![],
    );
    let geometry =
        Geometry::prepare(&budget, &schema, stored(&budget, &schema, &page, [0.0; 3])).unwrap();
    assert!(Appearance::prepare(&budget, &schema, geometry, &p, &frame).is_err());
}

#[test]
fn source_acquisition_time_overflow_is_rejected_before_gpu_conversion() {
    let budget = CpuBudget::new(1 << 20);
    let (mut schema, mut page) = fixture(&[[0.0; 3]], vec![]);
    add(
        &mut schema,
        &mut page,
        attribute(
            "offset",
            ScalarType::U64,
            1,
            AttributeSemantic::AcquisitionOffset,
            "ns",
            false,
        ),
        2u64.to_le_bytes().to_vec(),
        vec![],
    );
    let frame = DataFrame {
        capture: Some(CaptureStamp {
            simulation_time_s: 1.0,
            source_time_ns: Some(u64::MAX - 1),
            source_clock: Some("sensor".into()),
            sample: 1,
            scene_epoch: 1,
        }),
        ..Default::default()
    };
    let p = Presentation {
        color: ColorMode::Age {
            clock: AgeClock::Source,
            colormap: colormap::VIRIDIS,
            range: [0.0, 1.0],
        },
        ..Default::default()
    };
    let geometry =
        Geometry::prepare(&budget, &schema, stored(&budget, &schema, &page, [0.0; 3])).unwrap();
    let before = budget.usage().committed();
    assert!(Appearance::prepare(&budget, &schema, geometry.clone(), &p, &frame).is_err());
    assert_eq!(budget.usage().committed(), before);
    assert_eq!(
        exact_u64(
            ColumnView {
                values: &u64::MAX.to_le_bytes(),
                validity: &[]
            },
            0
        )
        .unwrap(),
        Some(u64::MAX)
    );
    assert!(exact_u64(ColumnView::default(), usize::MAX).is_err());
}

/// A live dataset lifetime for preparing generations outside a renderer.
pub(super) fn lifetime() -> gearbox_api::draw_data::DatasetLifetime {
    let mut registry = gearbox_api::draw_data::DrawingStore::new(1, 1).unwrap();
    let key = DataKey::new("draw-test", "lifetime").unwrap();
    let permit = registry.begin_dataset(&key, DatasetKind::Cloud, None).unwrap();
    registry.commit_dataset(permit, Arc::new(())).unwrap();
    registry.get(&(key.namespace, key.id)).unwrap().lifetime.clone()
}

/// A resident generation of `descriptor` over `pages`.
pub(super) fn generation(
    budget: &CpuBudget,
    descriptor: &DataDescriptor,
    pages: &[Arc<StoredPage>],
) -> Arc<gearbox_api::draw_data::DatasetStorage> {
    use gearbox_api::draw_data::{DatasetMetadata, DatasetStorage, Generation};
    let generation = Generation::build(budget, DatasetMetadata::new(budget, descriptor).unwrap(), &[], pages).unwrap();
    DatasetStorage::from_resident(budget, generation).unwrap()
}
