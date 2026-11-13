use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::Duration;

use super::*;
use crate::wire::{DrawEntity, draw_data::*};

thread_local! {
    static HOOK: RefCell<Option<Box<dyn Fn(&str)>>> = RefCell::new(None);
}

pub(super) fn checkpoint(stage: &str) {
    HOOK.with(|hook| {
        if let Some(callback) = hook.borrow().as_ref() {
            callback(stage);
        }
    });
}

fn wire(operation: u32, lease: u64) -> DataRequest {
    DataRequest {
        host_epoch: 77,
        lease,
        ..DataRequest::new(operation)
    }
}

fn ok(store: &mut StreamingStore, request: &DataRequest) -> DataReply {
    let reply = store.handle(request, 0);
    assert_eq!(
        reply.code,
        data_code::OK,
        "{}",
        String::from_utf8_lossy(&reply.message)
    );
    reply
}

fn descriptor(kind: DatasetKind) -> DataDescriptor {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../tests/fixtures/draw_data/wire-v1.json")).unwrap();
    let packet = fixture["packets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|packet| {
            packet["name"]
                == if kind == DatasetKind::Cloud {
                    "begin"
                } else {
                    "begin_octree"
                }
        })
        .unwrap();
    let mut begin: BeginMetadata =
        serde_json::from_str(packet["json"]["metadata_json"].as_str().unwrap()).unwrap();
    begin.descriptor.key = DataKey::new("concurrency", "cloud").unwrap();
    begin.descriptor.stream = StreamMode::RetainedMap;
    if let DataShape::Cloud { schema } = &mut begin.descriptor.shape {
        schema.attributes.truncate(2);
    }
    begin.descriptor
}

fn upload(
    store: &mut StreamingStore,
    lease: u64,
    transaction: u64,
    base: Option<DatasetRevision>,
    kind: DatasetKind,
) -> DataReply {
    let descriptor = descriptor(kind);
    let count = 8192;
    let (bytes, encoding) = match &descriptor.shape {
        DataShape::Cloud { schema } => (
            CloudPage {
                count,
                columns: vec![
                    ColumnData {
                        values: vec![0; count as usize * 24],
                        validity: vec![],
                    },
                    ColumnData {
                        values: (0..u64::from(count)).flat_map(u64::to_le_bytes).collect(),
                        validity: vec![],
                    },
                ],
            }
            .encode(schema)
            .unwrap(),
            PageEncoding::CloudColumnsV1,
        ),
        DataShape::Octree { config, .. } => (
            encode_occupancy_page(
                config,
                &(0..count)
                    .map(|index| OccupancyLeaf {
                        key: VoxelKey {
                            tile: [-1, 0, 0],
                            depth: config.max_depth,
                            morton: u64::from(index),
                        },
                        value: OccupancyValue {
                            probability: Some(0.8),
                            confidence: None,
                            class_id: None,
                        },
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap(),
            PageEncoding::OccupancyLeavesV1,
        ),
        _ => unreachable!("concurrency tests upload clouds and octrees"),
    };
    let begin = DataRequest {
        transaction,
        total_bytes: bytes.len() as u64,
        incarnation: base.map_or(0, |r| r.incarnation),
        base_generation: base.map_or(0, |r| r.generation),
        ..wire(data_op::BEGIN, lease)
    }
    .with_key(&descriptor.key)
    .unwrap()
    .with_metadata(&BeginMetadata {
        descriptor,
        mode: CommitMode::Replace,
        scans: u32::from(kind == DatasetKind::Cloud),
        pages: 1,
        removed_scans: 0,
        removed_pages: 0,
    })
    .unwrap();
    ok(store, &begin);
    let edit = |operation| DataRequest {
        operation,
        metadata: vec![],
        total_bytes: 0,
        ..begin.clone()
    };
    let scan = ScanDescriptor {
        revision: SCHEMA_REVISION,
        scan_key: 0,
        source: None,
        frame: DataFrame::default(),
        source_count: u64::from(count),
        organization: None,
    };
    if kind == DatasetKind::Cloud {
        ok(
            store,
            &edit(data_op::DECLARE_SCAN).with_metadata(&scan).unwrap(),
        );
    }
    let page = PageDescriptor {
        address: PageAddress {
            scan_key: 0,
            page_id: 0,
        },
        encoding,
        items: count,
        byte_len: bytes.len() as u64,
        crc32: page_crc32(&bytes),
        origin_m: [0.0; 3],
    };
    ok(
        store,
        &edit(data_op::DECLARE_PAGE).with_metadata(&page).unwrap(),
    );
    for (index, payload) in bytes.chunks(DATA_CHUNK_BYTES).enumerate() {
        let reply = store.handle(
            &DataRequest {
                offset: (index * DATA_CHUNK_BYTES) as u64,
                payload: payload.to_vec(),
                ..edit(data_op::CHUNK)
            },
            0,
        );
        if reply.code != data_code::OK {
            return reply;
        }
    }
    store.handle(&edit(data_op::COMMIT), 0)
}

#[test]
fn namespace_ranges_match_component_filter_for_every_cursor() {
    let mut map = BTreeMap::new();
    for namespace in [
        "",
        "a",
        "a!",
        "a.",
        "a/child",
        "a/child/deep",
        "a/other",
        "a0",
        "ab",
        "z",
        "é",
        "é/x",
    ] {
        for id in ["0", "a", "é"] {
            map.insert((namespace.to_owned(), id.to_owned()), ());
        }
    }
    let mut cursors: Vec<_> = map.keys().cloned().map(Some).collect();
    cursors.extend([
        None,
        Some(("a/absent".into(), "x".into())),
        Some(("zz".into(), "".into())),
    ]);
    for namespace in ["", "a", "a/child", "absent", "é"] {
        for after in &cursors {
            let expected: Vec<_> = map
                .keys()
                .filter(|key| {
                    crate::wire::draw::ns_within(&key.0, namespace)
                        && after.as_ref().is_none_or(|after| *key > after)
                })
                .collect();
            let actual: Vec<_> = crate::draw::namespace_range(&map, namespace, after.as_ref())
                .map(|(key, _)| key)
                .collect();
            assert_eq!(actual, expected, "namespace={namespace:?} after={after:?}");
        }
    }
}

#[test]
fn page_and_generation_validation_leave_registry_available_and_reset_revokes_publication() {
    for kind in [DatasetKind::Cloud, DatasetKind::Octree] {
        for stage in ["page_validation", "generation_validation"] {
            for reset in [false, true] {
                let backend = DrawingBackend::default();
                let registry = backend.registry();
                let mut store = StreamingStore::with_backend(77, backend.clone()).unwrap();
                let baseline = store.budget().usage().committed();
                let lease = ok(&mut store, &wire(data_op::OPEN_WRITER, 0)).lease;
                let first = upload(&mut store, lease, 1, None, kind);
                assert_eq!(first.code, data_code::OK);
                let old = DatasetRevision {
                    incarnation: first.incarnation,
                    generation: first.stored_generation,
                };
                let (entered, ready) = mpsc::sync_channel(1);
                let (resume, released) = mpsc::sync_channel(1);
                let worker = std::thread::spawn(move || {
                    let once = AtomicBool::new(false);
                    HOOK.with(|hook| {
                        *hook.borrow_mut() = Some(Box::new(move |name| {
                            if name == stage && !once.swap(true, Ordering::SeqCst) {
                                entered.send(()).unwrap();
                                released.recv_timeout(Duration::from_secs(5)).unwrap();
                            }
                        }))
                    });
                    let reply = upload(&mut store, lease, 2, Some(old), kind);
                    HOOK.with(|hook| hook.borrow_mut().take());
                    (store, reply)
                });
                let reached = ready.recv_timeout(Duration::from_secs(5));
                let readable = match registry.try_lock() {
                    Ok(mut guard) => {
                        assert_eq!(
                            guard
                                .get(&("concurrency".into(), "cloud".into()))
                                .unwrap()
                                .revision,
                            old
                        );
                        guard
                            .apply_legacy(
                                DrawEntity::points("concurrency", "legacy", &[[1.0; 3]]),
                                0.0,
                            )
                            .unwrap();
                        if reset {
                            guard.clear("concurrency").unwrap();
                        }
                        true
                    }
                    Err(_) => false,
                };
                let _ = resume.send(());
                let (mut store, reply) = worker.join().unwrap();
                assert!(
                    reached.is_ok() && readable,
                    "{stage} blocked registry access"
                );
                if reset {
                    assert!(matches!(
                        reply.code,
                        data_code::EXPIRED | data_code::HISTORY_EXPIRED
                    ));
                    assert_eq!(registry.lock().unwrap().dataset_len(), 0);
                } else {
                    assert_eq!(reply.code, data_code::OK);
                    assert_eq!(reply.stored_generation, old.generation + 1);
                }
                backend.lock().clear("concurrency");
                ok(&mut store, &wire(data_op::CLOSE_WRITER, lease));
                store.maintain(1).unwrap();
                assert_eq!(store.usage().transactions, 0);
                assert_eq!(registry.lock().unwrap().pending_len(), 0);
                assert_eq!(store.budget().usage().committed(), baseline);
            }
        }
    }
}

#[test]
fn list_encoding_is_off_lock_and_concurrent_mutation_returns_stale_not_mixed_data() {
    for mutate in [false, true] {
        let backend = DrawingBackend::default();
        for i in 0..128 {
            backend
                .lock()
                .apply(
                    DrawEntity::points("list", &format!("{i:04}"), &[[0.0; 3]]),
                    0.0,
                )
                .unwrap();
        }
        let registry = backend.registry();
        let observed = Arc::new(AtomicBool::new(false));
        let cloned = observed.clone();
        HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |stage| {
                if stage == "list_summary" && !cloned.swap(true, Ordering::SeqCst) {
                    let mut guard = registry
                        .try_lock()
                        .expect("list encoding holds the shared registry");
                    if mutate {
                        guard
                            .apply_legacy(DrawEntity::points("list", "new", &[[0.0; 3]]), 0.0)
                            .unwrap();
                    }
                }
            }))
        });
        let mut store = StreamingStore::with_backend(77, backend).unwrap();
        let request = wire(data_op::LIST, 0)
            .with_metadata(&DataListQuery {
                namespace: "list".into(),
                limit: 128,
                cursor: None,
            })
            .unwrap();
        let reply = store.handle(&request, 0);
        HOOK.with(|hook| hook.borrow_mut().take());
        assert!(observed.load(Ordering::SeqCst));
        assert_eq!(
            reply.code,
            if mutate {
                data_code::STALE_CURSOR
            } else {
                data_code::OK
            }
        );
        if mutate {
            assert_eq!(store.cursor_len(), 0);
        }
    }
}

#[test]
fn rejected_publication_releases_candidate_after_unlocking_registry() {
    let backend = DrawingBackend::default();
    let registry = backend.registry();
    let baseline = backend.budget().usage().committed();
    let descriptor =
        DatasetMetadata::new(backend.budget(), &descriptor(DatasetKind::Cloud)).unwrap();
    let permit = registry
        .lock()
        .unwrap()
        .begin_dataset(&descriptor.value().key, DatasetKind::Cloud, None)
        .unwrap();
    registry.lock().unwrap().cancel(permit);
    let candidate = DatasetStorage::from_resident(
        backend.budget(),
        Generation::build(backend.budget(), descriptor, &[], &[]).unwrap(),
    )
    .unwrap();
    let observed = Arc::new(AtomicBool::new(false));
    let cloned = observed.clone();
    let probe = registry.clone();
    HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |stage| {
            if stage == "generation_drop" {
                let _guard = probe
                    .try_lock()
                    .expect("rejected candidate dropped under registry lock");
                cloned.store(true, Ordering::SeqCst);
            }
        }))
    });
    let result = super::transaction::Publication::new(registry, permit).publish(candidate);
    HOOK.with(|hook| hook.borrow_mut().take());
    assert_eq!(result.unwrap_err().code, data_code::EXPIRED);
    assert!(observed.load(Ordering::SeqCst));
    assert_eq!(backend.budget().usage().committed(), baseline);
}
