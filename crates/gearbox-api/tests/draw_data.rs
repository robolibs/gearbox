use gearbox_api::wire::draw_data::*;

fn attribute(
    name: &str,
    scalar: ScalarType,
    components: u8,
    semantic: AttributeSemantic,
    units: &str,
) -> Attribute {
    Attribute {
        name: name.into(),
        scalar,
        components,
        semantic,
        units: units.into(),
        normalization: Normalization::None,
        nullable: false,
    }
}

fn schema() -> CloudSchema {
    CloudSchema {
        revision: SCHEMA_REVISION,
        attributes: vec![
            attribute(
                "position",
                ScalarType::F32,
                3,
                AttributeSemantic::Position,
                "m",
            ),
            attribute(
                "source_id",
                ScalarType::U64,
                1,
                AttributeSemantic::SourceId,
                "",
            ),
        ],
    }
}

fn floats(values: &[f32]) -> ColumnData {
    ColumnData {
        values: values.iter().flat_map(|v| v.to_le_bytes()).collect(),
        validity: Vec::new(),
    }
}

fn page() -> CloudPage {
    CloudPage {
        count: 2,
        columns: vec![
            floats(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]),
            ColumnData {
                values: [0u64, u64::MAX]
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect(),
                validity: Vec::new(),
            },
        ],
    }
}

#[test]
fn typed_schema_roundtrips_without_changing_data_values() {
    let schema = schema();
    let bytes = schema.to_json().unwrap();
    assert!(bytes.len() < MAX_SCHEMA_BYTES);
    assert_eq!(CloudSchema::from_json(&bytes).unwrap(), schema);
    assert!(page().validate(&schema).is_ok());
    assert_eq!(&page().columns[0].values[0..4], &1.0f32.to_le_bytes());
    assert_eq!(&page().columns[1].values[8..16], &u64::MAX.to_le_bytes());
}

#[test]
fn every_scalar_type_has_an_explicit_width_and_checked_byte_count() {
    for (scalar, bytes) in [
        (ScalarType::U8, 1),
        (ScalarType::U16, 2),
        (ScalarType::U32, 4),
        (ScalarType::U64, 8),
        (ScalarType::I16, 2),
        (ScalarType::I32, 4),
        (ScalarType::F32, 4),
        (ScalarType::F64, 8),
    ] {
        let field = attribute("custom", scalar, 4, AttributeSemantic::Custom, "unit");
        assert_eq!(scalar.size(), bytes);
        assert_eq!(field.byte_len(5).unwrap(), bytes * 4 * 5);
        assert!(field.byte_len(u64::MAX).is_err());
        assert!(
            ColumnData {
                values: vec![0; bytes * 4 * 5],
                validity: vec![]
            }
            .validate(&field, 5)
            .is_ok()
        );
    }
}

#[test]
fn schema_requires_unique_names_and_exactly_one_position_and_id() {
    let mut s = schema();
    s.attributes.push(s.attributes[0].clone());
    assert!(s.validate().unwrap_err().contains("duplicate"));
    s.attributes.last_mut().unwrap().name = "second_position".into();
    assert!(s.validate().unwrap_err().contains("exactly one"));
    s.attributes.truncate(1);
    assert!(s.validate().unwrap_err().contains("SourceId"));
    s.attributes.clear();
    assert!(s.validate().is_err());
}

#[test]
fn unknown_revision_fields_and_oversized_schema_fail_closed() {
    let mut s = schema();
    s.revision += 1;
    assert!(s.to_json().is_err());
    assert!(CloudSchema::from_json(&vec![b' '; MAX_SCHEMA_BYTES + 1]).is_err());
    assert!(CloudSchema::from_json(b"\xff").is_err());
    let mut value = serde_json::to_value(schema()).unwrap();
    value["unexpected"] = true.into();
    assert!(CloudSchema::from_json(&serde_json::to_vec(&value).unwrap()).is_err());
    value.as_object_mut().unwrap().remove("unexpected");
    value["attributes"][0]["scalar"] = "f16".into();
    assert!(CloudSchema::from_json(&serde_json::to_vec(&value).unwrap()).is_err());
}

#[test]
fn field_names_components_units_and_normalization_are_bounded() {
    let mut a = schema().attributes.remove(0);
    a.name = "é".repeat(32);
    assert!(a.validate().is_err());
    a.name = "bad\0name".into();
    assert!(a.validate().is_err());
    a.name = "position".into();
    a.components = 0;
    assert!(a.validate().is_err());
    a.components = 5;
    assert!(a.validate().is_err());
    a.components = 3;
    a.units = "pixels".into();
    assert!(a.validate().is_err());
    a.units = "m".into();
    a.normalization = Normalization::Unorm;
    assert!(a.validate().is_err());
    let mut s = schema();
    for i in 0..MAX_ATTRIBUTES {
        s.attributes.push(attribute(
            &format!("a{i}"),
            ScalarType::U8,
            1,
            AttributeSemantic::Custom,
            "",
        ));
    }
    assert!(s.validate().unwrap_err().contains("32 attributes"));
}

#[test]
fn invalid_coordinates_have_an_explicit_mask_and_canonical_payload() {
    let mut s = schema();
    s.attributes[0].nullable = true;
    let mut p = page();
    p.columns[0].validity = vec![1];
    assert!(p.validate(&s).unwrap_err().contains("canonical zero"));
    p.columns[0].values[12..].fill(0);
    assert!(p.validate(&s).is_ok());
    p.columns[0].validity[0] = 0b1000_0001;
    assert!(p.validate(&s).unwrap_err().contains("padding"));
    p.columns[0].validity = vec![1, 0];
    assert!(p.validate(&s).unwrap_err().contains("mask length"));
    s.attributes[0].nullable = false;
    p.columns[0].validity = vec![3];
    assert!(p.validate(&s).is_err());
}

#[test]
fn valid_float_samples_reject_nan_and_infinity() {
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut p = page();
        p.columns[0].values[..4].copy_from_slice(&value.to_le_bytes());
        assert!(p.validate(&schema()).unwrap_err().contains("non-finite"));
    }
    let field = attribute("custom", ScalarType::F64, 1, AttributeSemantic::Custom, "");
    let column = ColumnData {
        values: f64::NAN.to_le_bytes().to_vec(),
        validity: Vec::new(),
    };
    assert!(column.validate(&field, 1).is_err());
}

#[test]
fn stable_source_ids_are_exact_nonnullable_and_unique_within_a_page() {
    let mut p = page();
    p.columns[1].values[8..].fill(0);
    assert!(
        p.validate(&schema())
            .unwrap_err()
            .contains("duplicate source ID")
    );
    let mut s = schema();
    s.attributes[1].nullable = true;
    assert!(s.validate().is_err());
    s.attributes[1].nullable = false;
    s.attributes[1].scalar = ScalarType::F64;
    assert!(s.validate().is_err());
}

#[test]
fn color_intensity_and_class_columns_coexist() {
    let mut s = schema();
    let mut color = attribute("rgba", ScalarType::U8, 4, AttributeSemantic::Color, "");
    color.normalization = Normalization::Unorm;
    s.attributes.extend([
        color,
        attribute(
            "intensity",
            ScalarType::F32,
            1,
            AttributeSemantic::Intensity,
            "raw",
        ),
        attribute("class", ScalarType::U32, 1, AttributeSemantic::Category, ""),
    ]);
    let mut p = page();
    p.columns.extend([
        ColumnData {
            values: vec![255, 0, 0, 255, 0, 255, 0, 128],
            validity: vec![],
        },
        floats(&[150.0, 400.0]),
        ColumnData {
            values: [42u32, u32::MAX]
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect(),
            validity: vec![],
        },
    ]);
    let before = p.clone();
    assert!(p.validate(&s).is_ok());
    assert_eq!(p, before);
}

#[test]
fn normals_radii_and_confidence_preserve_their_semantics() {
    let normal = attribute("normal", ScalarType::F32, 3, AttributeSemantic::Normal, "");
    assert!(floats(&[0.0, 0.0, 1.0]).validate(&normal, 1).is_ok());
    assert!(floats(&[0.0; 3]).validate(&normal, 1).is_err());
    assert!(floats(&[1.0; 3]).validate(&normal, 1).is_err());
    let radius = attribute("radius", ScalarType::F32, 1, AttributeSemantic::Radius, "m");
    assert!(floats(&[0.01]).validate(&radius, 1).is_ok());
    for bad in [0.0, -1.0] {
        assert!(floats(&[bad]).validate(&radius, 1).is_err());
    }
    let confidence = attribute(
        "confidence",
        ScalarType::F32,
        1,
        AttributeSemantic::Confidence,
        "",
    );
    assert!(floats(&[0.0, 1.0]).validate(&confidence, 2).is_ok());
    assert!(floats(&[1.01]).validate(&confidence, 1).is_err());
}

#[test]
fn page_counts_and_column_lengths_are_checked_before_sample_access() {
    let mut p = page();
    p.count = MAX_PAGE_POINTS + 1;
    assert!(p.validate(&schema()).is_err());
    p.count = 2;
    p.columns[0].values.pop();
    assert!(p.validate(&schema()).is_err());
    p = page();
    p.columns.pop();
    assert!(p.validate(&schema()).is_err());
    let empty = CloudPage {
        count: 0,
        columns: vec![floats(&[]), floats(&[])],
    };
    assert!(empty.validate(&schema()).is_ok());
}

#[test]
fn acquisition_offsets_use_integer_nanoseconds() {
    let mut field = attribute(
        "time_offset",
        ScalarType::U64,
        1,
        AttributeSemantic::AcquisitionOffset,
        "ns",
    );
    assert!(field.validate().is_ok());
    field.units = "s".into();
    assert!(field.validate().is_err());
    field.units = "ns".into();
    field.scalar = ScalarType::F64;
    assert!(field.validate().is_err());
}
