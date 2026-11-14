use gearbox_api::wire::{draw_data::*, draw_kind};

fn revision(generation: u64) -> DatasetRevision {
    DatasetRevision {
        incarnation: 9,
        generation,
    }
}
fn target(generation: u64) -> VisibilityTarget {
    VisibilityTarget {
        dataset: revision(generation),
        view: ViewIdentity { id: 2, epoch: 3 },
    }
}
fn status() -> DatasetStatus {
    DatasetStatus {
        revision: 1,
        key: DataKey::new("mapping", "cloud").unwrap(),
        kind: DatasetKind::Cloud,
        retained: Some(revision(3)),
        requested: Some(revision(2)),
        visibility: GenerationVisibility::Pending { target: target(2) },
        transfer: None,
        failure: None,
        resources: DataResourceUsage::default(),
        counters: DataCounters::default(),
        paging: None,
    }
}
fn legacy(id: &str) -> DrawSummary {
    DrawSummary {
        namespace: "mapping".into(),
        id: id.into(),
        object: SummaryOwner::Legacy {
            kind: draw_kind::POINTS,
            generation: 1,
            revision: 2,
            points: 10,
            instances: 0,
        },
    }
}
fn dataset(id: &str) -> DrawSummary {
    DrawSummary {
        namespace: "mapping".into(),
        id: id.into(),
        object: SummaryOwner::Dataset {
            kind: DatasetKind::Cloud,
            revision: revision(1),
            scans: 1,
            pages: 1,
            source_samples: 10,
            valid_samples: 9,
            payload_bytes: 1024,
        },
    }
}
fn page() -> DrawSummaryPage {
    DrawSummaryPage {
        revision: 1,
        listing_revision: 10,
        items: vec![legacy("a"), dataset("b")],
        next_cursor: None,
    }
}

#[test]
fn bounded_mixed_list_preserves_type_counts_order_and_identity() {
    let page = page();
    let json = page.to_json().unwrap();
    assert_eq!(DrawSummaryPage::from_json(&json).unwrap(), page);
    assert!(json.len() < DATA_METADATA_BYTES);
    assert_eq!(page.items[0].key(), ("mapping", "a"));
    assert_eq!(page.items[1].key(), ("mapping", "b"));
}

#[test]
fn listing_rejects_unsorted_duplicate_excessive_and_invalid_items() {
    let mut value = page();
    value.items.reverse();
    assert!(value.to_json().is_err());
    value.items = vec![legacy("a"); 2];
    assert!(value.to_json().is_err());
    value.items = (0..=MAX_LIST_ITEMS)
        .map(|i| legacy(&format!("{i:03}")))
        .collect();
    assert!(value.to_json().is_err());
    value.items = vec![DrawSummary {
        object: SummaryOwner::Legacy {
            kind: 0,
            generation: 1,
            revision: 1,
            points: 0,
            instances: 0,
        },
        ..legacy("a")
    }];
    assert!(value.to_json().is_err());
}

#[test]
fn encoded_byte_budget_is_independent_of_item_budget_and_never_truncates_keys() {
    let mut value = page();
    value.items = vec![legacy(&"x".repeat(200))];
    let json = value.to_json().unwrap();
    assert_eq!(
        DrawSummaryPage::from_json(&json).unwrap().items[0].id.len(),
        200
    );
    value.items = (0..100)
        .map(|i| legacy(&format!("{i:03}{}", "x".repeat(200))))
        .collect();
    assert!(value.validate().is_ok());
    assert!(value.to_json().is_err());
    value.items = vec![legacy(&"\u{0001}".repeat(3000))];
    assert!(value.validate().is_ok());
    assert!(value.to_json().is_err());
}

#[test]
fn list_cursors_are_bounded_and_empty_pages_do_not_loop() {
    let mut value = page();
    for cursor in ["".into(), "bad\n".into(), "x".repeat(MAX_CURSOR_BYTES + 1)] {
        value.next_cursor = Some(cursor);
        assert!(value.to_json().is_err());
    }
    value.next_cursor = Some("cursor".into());
    assert!(value.to_json().is_ok());
    value.items.clear();
    assert!(value.to_json().is_err());
    value.next_cursor = None;
    value.listing_revision = 0;
    assert!(value.to_json().is_ok());
}

#[test]
fn summary_counts_do_not_invent_samples_pages_or_octree_scans() {
    let mut json = serde_json::to_value(dataset("a")).unwrap();
    for (field, invalid) in [
        ("valid_samples", 11u64),
        ("pages", 0),
        ("source_samples", MAX_PAGE_POINTS as u64 + 1),
        ("payload_bytes", 15),
    ] {
        let original = json["object"][field].clone();
        json["object"][field] = invalid.into();
        assert!(
            serde_json::from_value::<DrawSummary>(json.clone())
                .unwrap()
                .validate()
                .is_err()
        );
        json["object"][field] = original;
    }
    json["object"]["kind"] = "octree".into();
    assert!(
        serde_json::from_value::<DrawSummary>(json.clone())
            .unwrap()
            .validate()
            .is_err()
    );
    json["object"]["scans"] = 0.into();
    assert!(
        serde_json::from_value::<DrawSummary>(json)
            .unwrap()
            .validate()
            .is_ok()
    );
}

#[test]
fn visibility_states_roundtrip_without_confusing_retention_with_visibility() {
    let mut value = status();
    for state in [
        GenerationVisibility::Unavailable {
            reason: "no draw-capable view".into(),
        },
        GenerationVisibility::Pending { target: target(2) },
        GenerationVisibility::Visible {
            target: target(2),
            frame: 100,
            disposition: FrontierDisposition::Drawn,
            source_samples: 1000,
            selected_items: 100,
        },
        GenerationVisibility::Visible {
            target: target(2),
            frame: 101,
            disposition: FrontierDisposition::Culled,
            source_samples: 1000,
            selected_items: 100,
        },
        GenerationVisibility::Visible {
            target: target(2),
            frame: 102,
            disposition: FrontierDisposition::Empty,
            source_samples: 10,
            selected_items: 0,
        },
        GenerationVisibility::Superseded {
            target: target(2),
            successor: revision(3),
        },
        GenerationVisibility::ViewGone { target: target(2) },
        GenerationVisibility::HistoryExpired { target: target(2) },
        GenerationVisibility::Failed {
            target: target(2),
            failure: DataFailure {
                phase: DataPhase::Preparing,
                code: data_code::BUDGET,
                reason: "GPU staging budget".into(),
            },
        },
    ] {
        value.visibility = state;
        assert_eq!(
            DatasetStatus::from_json(&value.to_json().unwrap()).unwrap(),
            value
        );
    }
}

#[test]
fn status_does_not_acknowledge_a_later_generation_as_the_requested_one() {
    let mut value = status();
    value.visibility = GenerationVisibility::Visible {
        target: target(3),
        frame: 1,
        disposition: FrontierDisposition::Drawn,
        source_samples: 1,
        selected_items: 1,
    };
    assert!(value.validate().is_err());
    value.requested = Some(revision(3));
    value.validate().unwrap();
    value.retained = None;
    assert!(value.validate().is_err());
    value.retained = Some(DatasetRevision {
        incarnation: 10,
        generation: 3,
    });
    assert!(value.validate().is_err());
}

#[test]
fn supersession_requires_a_newer_retained_successor_in_the_same_incarnation() {
    let mut value = status();
    for successor in [
        revision(1),
        revision(2),
        revision(4),
        DatasetRevision {
            incarnation: 10,
            generation: 3,
        },
    ] {
        value.visibility = GenerationVisibility::Superseded {
            target: target(2),
            successor,
        };
        assert!(value.validate().is_err());
    }
    value.visibility = GenerationVisibility::Superseded {
        target: target(2),
        successor: revision(3),
    };
    value.validate().unwrap();
}

#[test]
fn visible_frontiers_need_valid_view_frame_and_disposition_counts() {
    let value = GenerationVisibility::Visible {
        target: target(2),
        frame: 1,
        disposition: FrontierDisposition::Drawn,
        source_samples: 1,
        selected_items: 1,
    };
    let base = serde_json::to_value(value).unwrap();
    for (field, replacement) in [
        ("frame", 0.into()),
        ("selected_items", 0.into()),
        ("disposition", "empty".into()),
    ] {
        let mut json = base.clone();
        json[field] = replacement;
        assert!(
            serde_json::from_value::<GenerationVisibility>(json)
                .unwrap()
                .validate()
                .is_err()
        );
    }
    let mut json = base;
    json["target"]["view"]["epoch"] = 0.into();
    assert!(
        serde_json::from_value::<GenerationVisibility>(json)
            .unwrap()
            .validate()
            .is_err()
    );
}

#[test]
fn status_failure_reasons_and_progress_are_bounded_and_consistent() {
    let mut value = status();
    value.transfer = Some(TransferProgress {
        transaction: 1,
        phase: DataPhase::Receiving,
        received_bytes: 512,
        expected_bytes: 1024,
        staged_bytes: 256,
        declared_pages: 2,
        complete_pages: 1,
    });
    value.failure = Some(DataFailure {
        phase: DataPhase::Validating,
        code: data_code::INVALID,
        reason: "CRC mismatch".into(),
    });
    value.validate().unwrap();
    value.transfer.as_mut().unwrap().staged_bytes = 513;
    assert!(value.validate().is_err());
    value.transfer = None;
    value.failure.as_mut().unwrap().code = data_code::OK;
    assert!(value.validate().is_err());
    value.failure.as_mut().unwrap().code = data_code::INVALID;
    value.failure.as_mut().unwrap().reason = "x".repeat(513);
    assert!(value.validate().is_err());
}

#[test]
fn resource_totals_are_unique_allocation_totals_not_sums_of_overlapping_categories() {
    let mut usage = DataResourceUsage {
        cpu_bytes: 100,
        resident_page_bytes: 80,
        staged_bytes: 80,
        pinned_retired_bytes: 20,
        ..Default::default()
    };
    usage.validate().unwrap();
    usage.validate_against(&DataLimits::reference()).unwrap();
    usage.pinned_retired_bytes = 81;
    assert!(usage.validate().is_err());
    usage.pinned_retired_bytes = 20;
    usage.cpu_bytes = DataLimits::reference().cpu_bytes + 1;
    assert!(usage.validate_against(&DataLimits::reference()).is_err());
    usage = DataResourceUsage::default();
    usage.writer_leases = DataLimits::reference().writers + 1;
    assert!(usage.validate_against(&DataLimits::reference()).is_err());
}

#[test]
fn status_and_listing_decoders_reject_unknown_fields_and_excessive_bytes() {
    let bytes = vec![b' '; DATA_METADATA_BYTES + 1];
    assert!(DatasetStatus::from_json(&bytes).is_err());
    assert!(DrawSummaryPage::from_json(&bytes).is_err());
    let mut json = serde_json::to_value(status()).unwrap();
    json["unexpected"] = 1.into();
    assert!(DatasetStatus::from_json(&serde_json::to_vec(&json).unwrap()).is_err());
    let mut json = serde_json::to_value(page()).unwrap();
    json["items"][0]["unexpected"] = 1.into();
    assert!(DrawSummaryPage::from_json(&serde_json::to_vec(&json).unwrap()).is_err());
}
