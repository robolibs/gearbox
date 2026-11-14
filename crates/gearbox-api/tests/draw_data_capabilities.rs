use gearbox_api::wire::draw_data::*;

#[test]
fn default_profiles_match_the_pinned_resource_budgets() {
    let profiles: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/draw_data/profiles.json"
    ))
    .unwrap();
    for (name, limits) in [
        ("reference", DataLimits::reference()),
        ("low-memory", DataLimits::low_memory()),
    ] {
        limits.validate().unwrap();
        let json = serde_json::to_value(limits).unwrap();
        for field in [
            "cpu_bytes",
            "gpu_bytes",
            "incomplete_bytes",
            "writer_incomplete_bytes",
            "writer_staged_bytes",
            "client_queue_bytes",
            "visible_points",
            "page_points",
            "upload_bytes_per_frame",
            "lease_timeout_ms",
        ] {
            assert_eq!(json[field], profiles[name][field], "{name}/{field}");
        }
    }
}

#[test]
fn byte_budgets_and_counts_are_positive_nested_and_hard_bounded() {
    let base = serde_json::to_value(DataLimits::reference()).unwrap();
    for field in base.as_object().unwrap().keys() {
        let mut value = base.clone();
        value[field] = 0.into();
        let limits: DataLimits = serde_json::from_value(value).unwrap();
        assert!(limits.validate().is_err(), "zero {field}");
    }
    let reference = DataLimits::reference();
    for (field, value) in [
        ("writer_incomplete_bytes", 129u64 << 20),
        ("incomplete_bytes", reference.cpu_bytes + 1),
        ("page_bytes", MAX_ENCODED_PAGE_BYTES as u64 + 1),
        ("page_points", 65_537),
        ("upload_bytes_per_frame", reference.gpu_bytes + 1),
        ("writer_transactions", 65),
        ("chunk_bytes", DATA_CHUNK_BYTES as u64 + 1),
        ("scans", u64::from(MAX_SCANS) + 1),
        ("list_items", u64::from(MAX_LIST_ITEMS) + 1),
    ] {
        let mut json = base.clone();
        json[field] = value.into();
        assert!(
            serde_json::from_value::<DataLimits>(json)
                .unwrap()
                .validate()
                .is_err(),
            "{field}"
        );
    }
}

#[test]
fn wire_only_does_not_advertise_storage_rendering_or_visibility() {
    let caps = DataCapabilities::wire_only(1);
    assert_eq!(
        caps.decodable,
        vec![DatasetKind::Cloud, DatasetKind::Octree, DatasetKind::Surfels, DatasetKind::MeshBlocks, DatasetKind::Volume, DatasetKind::Elevation]
    );
    assert!(caps.retained.is_empty() && caps.rendered.is_empty());
    assert_eq!(caps.features, DataFeatures::default());
    assert_eq!(
        DataCapabilities::from_json(&caps.to_json().unwrap()).unwrap(),
        caps
    );
    assert!(caps.to_json().unwrap().len() < DATA_METADATA_BYTES);
}

#[test]
fn feature_flags_cannot_claim_missing_backends() {
    let base = serde_json::to_value(DataCapabilities::wire_only(1)).unwrap();
    for field in base["features"].as_object().unwrap().keys() {
        let mut json = base.clone();
        json["features"][field] = true.into();
        assert!(
            DataCapabilities::from_json(&serde_json::to_vec(&json).unwrap()).is_err(),
            "{field}"
        );
    }
    let mut caps = DataCapabilities::wire_only(1);
    caps.retained.push(DatasetKind::Cloud);
    assert!(caps.validate().is_err());
    caps.features.atomic_generations = true;
    caps.features.accumulation = true;
    caps.validate().unwrap();
    caps.rendered.push(DatasetKind::Octree);
    assert!(caps.validate().is_err());
    caps.rendered = vec![DatasetKind::Cloud];
    caps.features.per_view_visibility = true;
    caps.validate().unwrap();
    caps.decodable.clear();
    assert!(caps.validate().is_err());
}

#[test]
fn schemas_allow_future_negotiation_but_reject_duplicates_and_unbounded_lists() {
    let mut caps = DataCapabilities::wire_only(1);
    for schemas in [vec![1, 2], vec![2]] {
        caps.schemas = schemas;
        assert!(caps.validate().is_ok());
    }
    for schemas in [vec![], vec![0], vec![1, 1], (1..=17).collect()] {
        caps.schemas = schemas;
        assert!(caps.validate().is_err());
    }
    caps = DataCapabilities::wire_only(0);
    assert!(caps.validate().is_err());
    caps = DataCapabilities::wire_only(1);
    caps.decodable.push(DatasetKind::Cloud);
    assert!(caps.validate().is_err());
}

#[test]
fn bounded_capabilities_reject_unknown_fields_and_wrong_revision() {
    assert!(DataCapabilities::from_json(&vec![b' '; DATA_METADATA_BYTES + 1]).is_err());
    let mut json = serde_json::to_value(DataCapabilities::wire_only(1)).unwrap();
    json["revision"] = 2.into();
    assert!(DataCapabilities::from_json(&serde_json::to_vec(&json).unwrap()).is_err());
    json["revision"] = 1.into();
    json["surprise"] = true.into();
    assert!(DataCapabilities::from_json(&serde_json::to_vec(&json).unwrap()).is_err());
}

#[test]
fn error_replies_bound_utf8_without_splitting_characters_or_allowing_controls() {
    let r = DataReply::error(1, data_code::INVALID, &format!("\n{}\t", "🙂".repeat(200)));
    r.validate().unwrap();
    assert_eq!(r.message.len(), 512);
    assert_eq!(
        std::str::from_utf8(&r.message).unwrap().chars().count(),
        128
    );
    assert_eq!(r.stage, data_stage::NONE);
}
