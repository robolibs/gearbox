use gearbox_api::wire::{colormap, draw_data::*};

#[allow(dead_code)]
#[path = "support/draw_data_contracts.rs"]
mod contracts;

fn request(op: u32) -> DataRequest {
    let mut r = DataRequest::new(op);
    if op != data_op::CAPABILITIES {
        r.host_epoch = 1;
    }
    if (data_op::CLOSE_WRITER..=data_op::CLEAR).contains(&op) {
        r.lease = 2;
    }
    if (data_op::BEGIN..=data_op::CANCEL).contains(&op) {
        r.transaction = 3;
    }
    if (data_op::BEGIN..=data_op::DELETE).contains(&op) || op == data_op::STATUS {
        r = r.with_key(&contracts::cloud().key).unwrap();
    }
    r
}

fn begin() -> (DataRequest, BeginMetadata) {
    let m = BeginMetadata {
        descriptor: contracts::cloud(),
        mode: CommitMode::Replace,
        scans: 1,
        pages: 1,
        removed_scans: 0,
        removed_pages: 0,
    };
    let mut r = request(data_op::BEGIN).with_metadata(&m).unwrap();
    r.total_bytes = 16;
    (r, m)
}

#[test]
fn every_operation_has_typed_metadata_and_valid_routing() {
    use data_op::*;
    for op in [
        CAPABILITIES,
        OPEN_WRITER,
        CLOSE_WRITER,
        REMOVE_PAGE,
        REMOVE_SCAN,
        COMMIT,
        CANCEL,
        DELETE,
    ] {
        assert!(request(op).operation().is_ok(), "operation {op}");
    }
    let mut chunk = request(CHUNK);
    chunk.payload = vec![1];
    assert_eq!(chunk.operation().unwrap(), DataOperation::Chunk);
    assert!(matches!(
        begin().0.operation().unwrap(),
        DataOperation::Begin(_)
    ));
    let mut scan = request(DECLARE_SCAN)
        .with_metadata(&contracts::scan(1))
        .unwrap();
    scan.scan_key = 1;
    assert!(matches!(
        scan.operation().unwrap(),
        DataOperation::DeclareScan(_)
    ));
    let page = PageDescriptor {
        address: PageAddress {
            scan_key: 0,
            page_id: 0,
        },
        encoding: PageEncoding::CloudColumnsV1,
        items: 0,
        byte_len: 16,
        crc32: 0,
        origin_m: [0.0; 3],
    };
    assert!(matches!(
        request(DECLARE_PAGE)
            .with_metadata(&page)
            .unwrap()
            .operation()
            .unwrap(),
        DataOperation::DeclarePage(_)
    ));
    assert!(
        request(PATCH_PRESENTATION)
            .with_metadata(&Presentation::default())
            .unwrap()
            .operation()
            .is_ok()
    );
    assert!(
        request(CLEAR)
            .with_metadata(&NamespaceFilter {
                namespace: "mapping".into()
            })
            .unwrap()
            .operation()
            .is_ok()
    );
    assert!(
        request(LIST)
            .with_metadata(&DataListQuery::default())
            .unwrap()
            .operation()
            .is_ok()
    );
    assert!(
        request(STATUS)
            .with_metadata(&DataStatusQuery::default())
            .unwrap()
            .operation()
            .is_ok()
    );
}

#[test]
fn begin_checks_key_expected_generation_and_delta_base() {
    let (mut r, mut m) = begin();
    r.key = serde_json::to_vec(&DataKey::new("mapping", "other").unwrap()).unwrap();
    assert!(r.operation().is_err());
    r = r.with_key(&m.descriptor.key).unwrap();
    r.incarnation = 7;
    assert!(r.operation().is_err());
    r.base_generation = 9;
    assert!(r.operation().is_ok());
    m.mode = CommitMode::Delta;
    r = r.with_metadata(&m).unwrap();
    assert!(r.operation().is_ok());
    r.incarnation = 0;
    r.base_generation = 0;
    assert!(r.operation().is_err());
}

#[test]
fn begin_bytes_are_bounded_by_declared_pages_including_empty_generation() {
    let (mut r, mut m) = begin();
    for bytes in [0, 15, MAX_ENCODED_PAGE_BYTES as u64 + 1, u64::MAX] {
        r.total_bytes = bytes;
        assert!(r.operation().is_err());
    }
    r.total_bytes = MAX_ENCODED_PAGE_BYTES as u64;
    assert!(r.operation().is_ok());
    m.pages = 0;
    r = r.with_metadata(&m).unwrap();
    assert!(r.operation().is_err());
    r.total_bytes = 0;
    assert!(r.operation().is_ok());
}

#[test]
fn octree_transactions_cannot_declare_or_remove_cloud_scans() {
    let (mut r, mut m) = begin();
    m.descriptor = contracts::octree();
    r = r
        .with_key(&m.descriptor.key)
        .unwrap()
        .with_metadata(&m)
        .unwrap();
    assert!(r.operation().is_err());
    m.scans = 0;
    r = r.with_metadata(&m).unwrap();
    assert!(r.operation().is_ok());
    m.mode = CommitMode::Delta;
    m.removed_scans = 1;
    r.incarnation = 2;
    r.base_generation = 1;
    assert!(r.with_metadata(&m).unwrap().operation().is_err());
}

#[test]
fn declaration_addresses_must_match_the_request_header() {
    let mut r = request(data_op::DECLARE_SCAN)
        .with_metadata(&contracts::scan(4))
        .unwrap();
    assert!(r.operation().is_err());
    r.scan_key = 4;
    assert!(r.operation().is_ok());
    let page = PageDescriptor {
        address: PageAddress {
            scan_key: 4,
            page_id: 7,
        },
        encoding: PageEncoding::CloudColumnsV1,
        items: 0,
        byte_len: 16,
        crc32: 0,
        origin_m: [0.0; 3],
    };
    r = request(data_op::DECLARE_PAGE).with_metadata(&page).unwrap();
    r.scan_key = 4;
    assert!(r.operation().is_err());
    r.page_id = 7;
    assert!(r.operation().is_ok());
    r.scan_key = 5;
    assert!(r.operation().is_err());
}

#[test]
fn list_and_clear_reject_noncanonical_or_unbounded_filters() {
    for ns in ["/mapping", "mapping/", " a", "a\n", &"é".repeat(128)] {
        assert!(
            request(data_op::CLEAR)
                .with_metadata(&NamespaceFilter {
                    namespace: ns.into()
                })
                .unwrap()
                .operation()
                .is_err()
        );
    }
    let mut q = DataListQuery::default();
    for limit in [0, MAX_LIST_ITEMS + 1, u32::MAX] {
        q.limit = limit;
        assert!(
            request(data_op::LIST)
                .with_metadata(&q)
                .unwrap()
                .operation()
                .is_err()
        );
    }
    q.limit = MAX_LIST_ITEMS;
    for cursor in ["".into(), "x\n".into(), "x".repeat(MAX_CURSOR_BYTES + 1)] {
        q.cursor = Some(cursor);
        assert!(
            request(data_op::LIST)
                .with_metadata(&q)
                .unwrap()
                .operation()
                .is_err()
        );
    }
    q.cursor = Some("a".repeat(MAX_CURSOR_BYTES));
    assert!(
        request(data_op::LIST)
            .with_metadata(&q)
            .unwrap()
            .operation()
            .is_ok()
    );
}

#[test]
fn status_requires_nonzero_requested_generation_and_complete_view_identity() {
    for q in [
        DataStatusQuery {
            generation: Some(0),
            view: None,
        },
        DataStatusQuery {
            generation: Some(1),
            view: Some(ViewIdentity { id: 0, epoch: 1 }),
        },
        DataStatusQuery {
            generation: None,
            view: Some(ViewIdentity { id: 1, epoch: 0 }),
        },
    ] {
        assert!(
            request(data_op::STATUS)
                .with_metadata(&q)
                .unwrap()
                .operation()
                .is_err()
        );
    }
    let q = DataStatusQuery {
        generation: Some(u64::MAX),
        view: Some(ViewIdentity { id: 1, epoch: 1 }),
    };
    assert!(
        request(data_op::STATUS)
            .with_metadata(&q)
            .unwrap()
            .operation()
            .is_ok()
    );
}

#[test]
fn presentation_patches_validate_intrinsics_without_inventing_a_dataset() {
    let mut p = Presentation::default();
    p.blur = 1.1;
    assert!(p.validate_intrinsic().is_err());
    p.blur = 0.7;
    p.opacity = -0.1;
    assert!(p.validate_intrinsic().is_err());
    p.opacity = 0.5;
    p.color = ColorMode::Scalar {
        name: "not_yet_known".into(),
        colormap: colormap::VIRIDIS,
        range: [0.0, 1.0],
        missing: [0; 4],
    };
    assert!(
        request(data_op::PATCH_PRESENTATION)
            .with_metadata(&p)
            .unwrap()
            .operation()
            .is_ok()
    );
    assert!(p.validate(Some(&contracts::schema()), None, true).is_err());
    p.color = ColorMode::Category {
        name: "class".into(),
        entries: vec![
            PaletteEntry {
                value: 1,
                rgba: [0; 4]
            };
            2
        ],
        missing: [0; 4],
    };
    assert!(
        request(data_op::PATCH_PRESENTATION)
            .with_metadata(&p)
            .unwrap()
            .operation()
            .is_err()
    );
    let mut display = OctreeDisplay::default();
    display.depth_limit = Some(MAX_OCTREE_DEPTH + 1);
    assert!(display.validate_intrinsic().is_err());
    display.depth_limit = None;
    display.show_unknown = true;
    assert!(display.validate_intrinsic().is_err());
}

#[test]
fn metadata_rejects_unknown_fields_duplicate_fields_and_invalid_json() {
    for json in [
        r#"{"generation":null,"view":null,"extra":0}"#,
        r#"{"generation":1,"generation":2,"view":null}"#,
        "{",
        "[]",
        "null",
    ] {
        let mut r = request(data_op::STATUS);
        r.metadata = json.as_bytes().to_vec();
        assert!(r.operation().is_err());
    }
}
