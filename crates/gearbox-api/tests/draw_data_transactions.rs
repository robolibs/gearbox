use std::sync::Arc;

use gearbox_api::draw_data::*;
use gearbox_api::wire::DrawEntity;
use gearbox_api::wire::draw_data::*;

#[allow(dead_code)]
#[path = "support/draw_data_contracts.rs"]
mod contracts;

#[path = "draw_data_transactions/accumulation.rs"]
mod accumulation;

const EPOCH: u64 = 17;

fn limits() -> DataLimits {
    DataLimits {
        datasets: 8,
        pages: 32,
        scans: 16,
        writers: 4,
        transactions: 8,
        writer_transactions: 4,
        completion_history: 8,
        cpu_bytes: 16 << 20,
        incomplete_bytes: 4 << 20,
        writer_incomplete_bytes: 2 << 20,
        writer_staged_bytes: 8 << 20,
        page_bytes: 1 << 20,
        client_queue_bytes: 1 << 20,
        ..DataLimits::reference()
    }
}

fn schema() -> CloudSchema {
    let mut schema = contracts::schema();
    schema.attributes.truncate(2);
    schema
}

fn descriptor() -> DataDescriptor {
    DataDescriptor {
        shape: DataShape::Cloud { schema: schema() },
        stream: StreamMode::RetainedMap,
        ..contracts::cloud()
    }
}

fn scan(key: u64, count: u64) -> ScanDescriptor {
    ScanDescriptor {
        source_count: count,
        organization: None,
        ..contracts::scan(key)
    }
}

fn page(scan_key: u64, page_id: u64, ids: &[u64]) -> (PageDescriptor, Vec<u8>) {
    let input = CloudPage {
        count: ids.len() as u32,
        columns: vec![
            ColumnData {
                values: vec![0; ids.len() * 24],
                validity: vec![],
            },
            ColumnData {
                values: ids.iter().flat_map(|id| id.to_le_bytes()).collect(),
                validity: vec![],
            },
        ],
    };
    let bytes = input.encode(&schema()).unwrap();
    (
        PageDescriptor {
            address: PageAddress { scan_key, page_id },
            encoding: PageEncoding::CloudColumnsV1,
            items: ids.len() as u32,
            byte_len: bytes.len() as u64,
            crc32: page_crc32(&bytes),
            origin_m: [0.0; 3],
        },
        bytes,
    )
}

fn req(op: u32) -> DataRequest {
    DataRequest {
        host_epoch: EPOCH,
        ..DataRequest::new(op)
    }
}

fn call(store: &mut StreamingStore, request: &DataRequest, now: u64) -> DataReply {
    let reply = store.handle(request, now);
    reply.pack_checked().unwrap();
    reply
}

fn ok(store: &mut StreamingStore, request: &DataRequest, now: u64) -> DataReply {
    let reply = call(store, request, now);
    assert_eq!(
        reply.code,
        data_code::OK,
        "{}",
        String::from_utf8_lossy(&reply.message)
    );
    reply
}

fn open(store: &mut StreamingStore, now: u64) -> u64 {
    ok(store, &req(data_op::OPEN_WRITER), now).lease
}

fn begin(
    lease: u64,
    id: u64,
    descriptor: DataDescriptor,
    mode: CommitMode,
    base: Option<DatasetRevision>,
    scans: u32,
    pages: u32,
    removed_scans: u32,
    removed_pages: u32,
    bytes: u64,
) -> DataRequest {
    let request = DataRequest {
        lease,
        transaction: id,
        incarnation: base.map_or(0, |base| base.incarnation),
        base_generation: base.map_or(0, |base| base.generation),
        total_bytes: bytes,
        ..req(data_op::BEGIN)
    }
    .with_key(&descriptor.key)
    .unwrap();
    request
        .with_metadata(&BeginMetadata {
            descriptor,
            mode,
            scans,
            pages,
            removed_scans,
            removed_pages,
        })
        .unwrap()
}

fn edit(begin: &DataRequest, operation: u32) -> DataRequest {
    DataRequest {
        lease: begin.lease,
        transaction: begin.transaction,
        incarnation: begin.incarnation,
        base_generation: begin.base_generation,
        key: begin.key.clone(),
        ..req(operation)
    }
}

fn declare_scan(store: &mut StreamingStore, begin: &DataRequest, scan: &ScanDescriptor, now: u64) {
    let request = DataRequest {
        scan_key: scan.scan_key,
        ..edit(begin, data_op::DECLARE_SCAN)
    }
    .with_metadata(scan)
    .unwrap();
    ok(store, &request, now);
}

fn declare_page(begin: &DataRequest, page: &PageDescriptor) -> DataRequest {
    DataRequest {
        scan_key: page.address.scan_key,
        page_id: page.address.page_id,
        ..edit(begin, data_op::DECLARE_PAGE)
    }
    .with_metadata(page)
    .unwrap()
}

fn chunk(begin: &DataRequest, page: &PageDescriptor, offset: usize, bytes: &[u8]) -> DataRequest {
    DataRequest {
        scan_key: page.address.scan_key,
        page_id: page.address.page_id,
        offset: offset as u64,
        payload: bytes.to_vec(),
        ..edit(begin, data_op::CHUNK)
    }
}

fn upload(
    store: &mut StreamingStore,
    begin: &DataRequest,
    page: &PageDescriptor,
    bytes: &[u8],
    now: u64,
) {
    ok(store, &declare_page(begin, page), now);
    let size = store.limits().chunk_bytes as usize;
    for (index, part) in bytes.chunks(size).enumerate() {
        ok(store, &chunk(begin, page, index * size, part), now);
    }
}

fn commit(store: &mut StreamingStore, begin: &DataRequest, now: u64) -> DatasetRevision {
    let reply = ok(store, &edit(begin, data_op::COMMIT), now);
    assert_eq!(reply.stage, data_stage::RETAINED);
    assert_eq!(reply.visible_generation, 0);
    DatasetRevision {
        incarnation: reply.incarnation,
        generation: reply.stored_generation,
    }
}

fn retained(store: &StreamingStore, key: &DataKey) -> Arc<Generation> {
    store
        .registry()
        .lock()
        .unwrap()
        .get(&(key.namespace.clone(), key.id.clone()))
        .unwrap()
        .payload
        .resident()
        .expect("transaction commits publish resident storage")
        .clone()
}

fn seed(store: &mut StreamingStore, lease: u64) -> DatasetRevision {
    let (page, bytes) = page(0, 1, &[1, 2]);
    let begin = begin(
        lease,
        1,
        descriptor(),
        CommitMode::Replace,
        None,
        1,
        1,
        0,
        0,
        bytes.len() as u64,
    );
    ok(store, &begin, 0);
    declare_scan(store, &begin, &scan(0, 2), 0);
    upload(store, &begin, &page, &bytes, 0);
    commit(store, &begin, 0)
}

fn clear(store: &mut StreamingStore, lease: u64, now: u64) {
    let request = DataRequest {
        lease,
        ..req(data_op::CLEAR)
    }
    .with_metadata(&NamespaceFilter {
        namespace: "".into(),
    })
    .unwrap();
    ok(store, &request, now);
}

#[test]
fn replace_is_atomic_and_retries_keep_the_exact_retained_result() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let baseline = store.budget().usage().committed();
    let lease = open(&mut store, 0);
    let base = seed(&mut store, lease);
    let old = retained(&store, &descriptor().key);
    let (page, bytes) = page(0, 1, &[3, 4, 5]);
    let begin = begin(
        lease,
        2,
        descriptor(),
        CommitMode::Replace,
        Some(base),
        1,
        1,
        0,
        0,
        bytes.len() as u64,
    );
    ok(&mut store, &begin, 1);
    let usage = store.usage();
    ok(&mut store, &begin, 1);
    assert_eq!(store.usage(), usage);
    declare_scan(&mut store, &begin, &scan(0, 3), 1);
    ok(&mut store, &declare_page(&begin, &page), 1);
    ok(&mut store, &chunk(&begin, &page, 32, &bytes[32..]), 1);
    assert_eq!(store.usage().incomplete_bytes, bytes.len() as u64);
    assert!(Arc::ptr_eq(&old, &retained(&store, &descriptor().key)));
    ok(&mut store, &chunk(&begin, &page, 0, &bytes[..32]), 1);
    assert_eq!(store.usage().incomplete_bytes, 0);
    assert_eq!(store.usage().staged_bytes, bytes.len() as u64);
    let usage = store.usage();
    ok(&mut store, &chunk(&begin, &page, 32, &bytes[32..]), 1);
    assert_eq!(store.usage(), usage);
    let reply = ok(&mut store, &edit(&begin, data_op::COMMIT), 1);
    assert_eq!(reply.stored_generation, 2);
    let duplicate = ok(&mut store, &edit(&begin, data_op::COMMIT), 2);
    assert_eq!(
        reply.pack_checked().unwrap().wire(),
        duplicate.pack_checked().unwrap().wire()
    );
    assert_eq!(old.counts().source_samples, 2);
    assert_eq!(
        retained(&store, &descriptor().key).counts().source_samples,
        3
    );
    clear(&mut store, lease, 2);
    assert_eq!(store.usage().transactions, 0);
    assert!(store.budget().usage().committed() > baseline);
    drop(old);
    assert_eq!(store.budget().usage().committed(), baseline);
}

#[test]
fn delta_shares_unchanged_pages_and_compare_exchange_rejects_stale_commit() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    let base = seed(&mut store, lease);
    let old = retained(&store, &descriptor().key);
    let (page, bytes) = page(0, 2, &[3]);
    let a = begin(
        lease,
        2,
        descriptor(),
        CommitMode::Delta,
        Some(base),
        1,
        1,
        0,
        0,
        bytes.len() as u64,
    );
    let b = begin(
        lease,
        3,
        descriptor(),
        CommitMode::Delta,
        Some(base),
        1,
        1,
        0,
        0,
        bytes.len() as u64,
    );
    for begin in [&a, &b] {
        ok(&mut store, begin, 0);
        declare_scan(&mut store, begin, &scan(0, 3), 0);
        upload(&mut store, begin, &page, &bytes, 0);
    }
    let revision = commit(&mut store, &a, 0);
    let current = retained(&store, &descriptor().key);
    assert!(Arc::ptr_eq(&old.pages()[0], &current.pages()[0]));
    assert_eq!(current.counts().source_samples, 3);
    assert_eq!(
        call(&mut store, &edit(&b, data_op::COMMIT), 0).code,
        data_code::CONFLICT
    );
    assert!(Arc::ptr_eq(&current, &retained(&store, &descriptor().key)));
    let remove = begin(
        lease,
        4,
        descriptor(),
        CommitMode::Delta,
        Some(revision),
        1,
        0,
        0,
        1,
        0,
    );
    ok(&mut store, &remove, 0);
    declare_scan(&mut store, &remove, &scan(0, 1), 0);
    let request = DataRequest {
        page_id: 1,
        ..edit(&remove, data_op::REMOVE_PAGE)
    };
    ok(&mut store, &request, 0);
    ok(&mut store, &request, 0);
    commit(&mut store, &remove, 0);
    assert!(Arc::ptr_eq(
        &current.pages()[1],
        &retained(&store, &descriptor().key).pages()[0]
    ));
}

#[test]
fn any_upload_failure_is_terminal_and_preserves_the_old_generation() {
    for failure_kind in 0..6 {
        let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
        let lease = open(&mut store, 0);
        let base = seed(&mut store, lease);
        let old = retained(&store, &descriptor().key);
        let (mut page, bytes) = page(0, 1, &[3]);
        if failure_kind == 0 {
            page.crc32 ^= 1;
        }
        let begin = begin(
            lease,
            2,
            descriptor(),
            CommitMode::Replace,
            Some(base),
            1,
            1,
            0,
            0,
            bytes.len() as u64,
        );
        ok(&mut store, &begin, 0);
        declare_scan(&mut store, &begin, &scan(0, 1), 0);
        ok(&mut store, &declare_page(&begin, &page), 0);
        let bad = match failure_kind {
            0 => chunk(&begin, &page, 0, &bytes),
            1 => {
                ok(&mut store, &chunk(&begin, &page, 0, &bytes[..32]), 0);
                chunk(&begin, &page, 16, &bytes[16..])
            }
            2 => {
                ok(&mut store, &chunk(&begin, &page, 0, &bytes), 0);
                let mut changed = bytes.clone();
                changed[0] ^= 1;
                chunk(&begin, &page, 0, &changed)
            }
            3 => {
                ok(&mut store, &chunk(&begin, &page, 0, &bytes[..32]), 0);
                edit(&begin, data_op::COMMIT)
            }
            4 => chunk(&begin, &page, bytes.len(), &bytes[..1]),
            _ => {
                let mut request = chunk(&begin, &page, 0, &bytes);
                request.flags = 1;
                request
            }
        };
        assert_ne!(call(&mut store, &bad, 0).code, data_code::OK);
        assert_eq!(store.usage().transactions, 0);
        assert_eq!(store.usage().memory.page_bytes, old.counts().payload_bytes);
        assert!(Arc::ptr_eq(&old, &retained(&store, &descriptor().key)));
        assert_ne!(
            call(&mut store, &edit(&begin, data_op::COMMIT), 0).code,
            data_code::OK
        );
        clear(&mut store, lease, 0);
        drop(old);
        assert_eq!(store.usage().memory.page_bytes, 0);
        assert_eq!(store.usage().memory.reserved, 0);
    }
}

#[test]
fn incomplete_quota_is_per_page_and_staging_still_consumes_writer_capacity() {
    let (a, bytes_a) = page(0, 1, &[1]);
    let (b, bytes_b) = page(0, 2, &[2]);
    let total = (bytes_a.len() + bytes_b.len()) as u64;
    let limits = DataLimits {
        incomplete_bytes: a.byte_len,
        writer_incomplete_bytes: a.byte_len,
        page_bytes: a.byte_len,
        chunk_bytes: 32,
        writer_staged_bytes: total,
        ..limits()
    };
    let mut store = StreamingStore::new(EPOCH, limits).unwrap();
    let lease = open(&mut store, 0);
    let begin = begin(
        lease,
        1,
        descriptor(),
        CommitMode::Replace,
        None,
        1,
        2,
        0,
        0,
        total,
    );
    ok(&mut store, &begin, 0);
    declare_scan(&mut store, &begin, &scan(0, 2), 0);
    upload(&mut store, &begin, &a, &bytes_a, 0);
    assert_eq!(store.usage().staged_bytes, a.byte_len);
    assert_eq!(store.usage().incomplete_bytes, 0);
    upload(&mut store, &begin, &b, &bytes_b, 0);
    assert_eq!(store.usage().staged_bytes, total);
    commit(&mut store, &begin, 0);
    assert_eq!(store.usage().staged_bytes, 0);
    assert_eq!(store.usage().memory.page_bytes, total);
}

#[test]
fn second_incomplete_page_exceeds_global_or_writer_quota_without_hiding_old_data() {
    for global in [true, false] {
        let (page, _) = page(0, 1, &[1]);
        let mut limits = limits();
        limits.writer_incomplete_bytes = page.byte_len;
        limits.page_bytes = page.byte_len;
        limits.chunk_bytes = 32;
        limits.incomplete_bytes = page.byte_len * if global { 1 } else { 2 };
        let mut store = StreamingStore::new(EPOCH, limits).unwrap();
        let first = open(&mut store, 0);
        let second = if global { open(&mut store, 0) } else { first };
        let a = begin(
            first,
            1,
            descriptor(),
            CommitMode::Replace,
            None,
            1,
            1,
            0,
            0,
            page.byte_len,
        );
        let mut other = descriptor();
        other.key.id = "other".into();
        let b = begin(
            second,
            2,
            other,
            CommitMode::Replace,
            None,
            1,
            1,
            0,
            0,
            page.byte_len,
        );
        ok(&mut store, &a, 0);
        ok(&mut store, &b, 0);
        ok(&mut store, &declare_page(&a, &page), 0);
        assert_eq!(
            call(&mut store, &declare_page(&b, &page), 0).code,
            data_code::BUDGET
        );
        assert_eq!(store.usage().transactions, 1);
        assert_eq!(store.usage().incomplete_bytes, page.byte_len);
        clear(&mut store, first, 0);
        assert_eq!(store.usage().memory.reserved, 0);
    }
}

#[test]
fn fake_clock_expires_idle_transactions_separately_from_live_writers() {
    let mut store = StreamingStore::new(
        EPOCH,
        DataLimits {
            lease_timeout_ms: 10,
            ..limits()
        },
    )
    .unwrap();
    let lease = open(&mut store, 0);
    let a = begin(
        lease,
        1,
        descriptor(),
        CommitMode::Replace,
        None,
        0,
        0,
        0,
        0,
        0,
    );
    let mut other = descriptor();
    other.key.id = "other".into();
    let b = begin(lease, 2, other, CommitMode::Replace, None, 0, 0, 0, 0, 0);
    ok(&mut store, &a, 0);
    ok(&mut store, &b, 9);
    store.maintain(10).unwrap();
    assert_eq!(store.usage().writers, 1);
    assert_eq!(store.usage().transactions, 1);
    assert_eq!(
        call(&mut store, &edit(&a, data_op::COMMIT), 10).code,
        data_code::EXPIRED
    );
    store.maintain(20).unwrap();
    assert_eq!(store.usage().writers, 0);
    assert_eq!(store.usage().transactions, 0);
    assert_eq!(store.usage().completion_history, 0);
    assert_eq!(store.usage().memory.reserved, 0);
    assert_eq!(store.counters().expired_transactions, 2);
    assert!(store.maintain(19).is_err());
}

#[test]
fn close_cancel_and_clear_release_all_owned_transaction_allocations() {
    for operation in [data_op::CLOSE_WRITER, data_op::CANCEL, data_op::CLEAR] {
        let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
        let baseline = store.budget().usage().committed();
        let lease = open(&mut store, 0);
        let (page, bytes) = page(0, 1, &[1]);
        let begin = begin(
            lease,
            1,
            descriptor(),
            CommitMode::Replace,
            None,
            1,
            1,
            0,
            0,
            bytes.len() as u64,
        );
        ok(&mut store, &begin, 0);
        upload(&mut store, &begin, &page, &bytes, 0);
        match operation {
            data_op::CANCEL => {
                ok(&mut store, &edit(&begin, operation), 0);
                clear(&mut store, lease, 0);
            }
            data_op::CLEAR => clear(&mut store, lease, 0),
            _ => {
                ok(
                    &mut store,
                    &DataRequest {
                        lease,
                        ..req(operation)
                    },
                    0,
                );
            }
        }
        assert_eq!(store.budget().usage().committed(), baseline);
        assert_eq!(store.registry().lock().unwrap().pending_len(), 0);
    }
}

#[test]
fn bounded_completion_history_never_reexecutes_evicted_transaction_ids() {
    let mut store = StreamingStore::new(
        EPOCH,
        DataLimits {
            completion_history: 2,
            ..limits()
        },
    )
    .unwrap();
    let lease = open(&mut store, 0);
    let a = begin(
        lease,
        1,
        descriptor(),
        CommitMode::Replace,
        None,
        0,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &a, 0);
    let rev = commit(&mut store, &a, 0);
    let b = begin(
        lease,
        2,
        descriptor(),
        CommitMode::Replace,
        Some(rev),
        0,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &b, 0);
    let rev = commit(&mut store, &b, 0);
    let c = begin(
        lease,
        3,
        descriptor(),
        CommitMode::Replace,
        Some(rev),
        0,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &c, 0);
    commit(&mut store, &c, 0);
    assert_eq!(store.usage().completion_history, 2);
    assert_eq!(call(&mut store, &a, 0).code, data_code::HISTORY_EXPIRED);
    assert_eq!(
        call(&mut store, &edit(&a, data_op::COMMIT), 0).code,
        data_code::HISTORY_EXPIRED
    );
    assert_eq!(
        store
            .registry()
            .lock()
            .unwrap()
            .get(&("mapping".into(), "cloud".into()))
            .unwrap()
            .revision
            .generation,
        3
    );
}

#[test]
fn legacy_delete_revokes_pending_publication_and_old_successful_history() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    let base = seed(&mut store, lease);
    let begin = begin(
        lease,
        2,
        descriptor(),
        CommitMode::Delta,
        Some(base),
        0,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &begin, 0);
    store
        .registry()
        .lock()
        .unwrap()
        .apply_legacy(DrawEntity::delete("mapping", "cloud"), 0.0)
        .unwrap();
    assert_eq!(
        call(&mut store, &edit(&begin, data_op::COMMIT), 1).code,
        data_code::HISTORY_EXPIRED
    );
    assert_eq!(store.registry().lock().unwrap().dataset_len(), 0);
    assert_eq!(store.usage().memory.page_bytes, 0);
    clear(&mut store, lease, 1);
    assert_eq!(store.usage().completion_history, 0);
}

#[test]
fn latest_supersession_requires_a_valid_new_base_and_never_replaces_current_early() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    let mut descriptor = descriptor();
    descriptor.stream = StreamMode::Latest;
    let a = begin(
        lease,
        1,
        descriptor.clone(),
        CommitMode::Replace,
        None,
        0,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &a, 0);
    let base = commit(&mut store, &a, 0);
    let old = retained(&store, &descriptor.key);
    let b = begin(
        lease,
        2,
        descriptor.clone(),
        CommitMode::Replace,
        Some(base),
        0,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &b, 0);
    let invalid = begin(
        lease,
        3,
        descriptor.clone(),
        CommitMode::Replace,
        None,
        0,
        0,
        0,
        0,
        0,
    );
    assert_eq!(call(&mut store, &invalid, 0).code, data_code::CONFLICT);
    assert_eq!(store.usage().transactions, 1);
    let c = begin(
        lease,
        4,
        descriptor.clone(),
        CommitMode::Replace,
        Some(base),
        0,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &c, 0);
    assert_eq!(store.usage().transactions, 1);
    assert_eq!(store.counters().superseded_transactions, 1);
    assert_eq!(
        call(&mut store, &edit(&b, data_op::COMMIT), 0).code,
        data_code::SUPERSEDED
    );
    assert!(Arc::ptr_eq(&old, &retained(&store, &descriptor.key)));
    assert_eq!(commit(&mut store, &c, 0).generation, 2);
}

#[test]
fn acquired_scan_watermark_survives_empty_replacement_and_resets_only_on_delete() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    let mut descriptor = descriptor();
    descriptor.stream = StreamMode::Latest;
    let a = begin(
        lease,
        1,
        descriptor.clone(),
        CommitMode::Replace,
        None,
        1,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &a, 0);
    declare_scan(&mut store, &a, &scan(7, 0), 0);
    let base = commit(&mut store, &a, 0);
    let b = begin(
        lease,
        2,
        descriptor.clone(),
        CommitMode::Replace,
        Some(base),
        0,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &b, 0);
    let base = commit(&mut store, &b, 0);
    assert_eq!(retained(&store, &descriptor.key).scan_high_watermark(), 7);
    let c = begin(
        lease,
        3,
        descriptor.clone(),
        CommitMode::Replace,
        Some(base),
        1,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &c, 0);
    let bad = DataRequest {
        scan_key: 7,
        ..edit(&c, data_op::DECLARE_SCAN)
    }
    .with_metadata(&scan(7, 0))
    .unwrap();
    assert_eq!(call(&mut store, &bad, 0).code, data_code::CONFLICT);
    clear(&mut store, lease, 0);
    let d = begin(
        lease,
        4,
        descriptor,
        CommitMode::Replace,
        None,
        1,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &d, 0);
    declare_scan(&mut store, &d, &scan(1, 0), 0);
    assert_ne!(commit(&mut store, &d, 0).incarnation, base.incarnation);
}

#[test]
fn stale_epoch_wrong_lease_or_wrong_key_cannot_modify_a_pending_transaction() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    let a = begin(
        lease,
        1,
        descriptor(),
        CommitMode::Replace,
        None,
        0,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &a, 0);
    let mut wrong = edit(&a, data_op::COMMIT);
    wrong.host_epoch += 1;
    assert_eq!(call(&mut store, &wrong, 0).code, data_code::STALE_EPOCH);
    let mut wrong = edit(&a, data_op::COMMIT);
    wrong.lease ^= 12345;
    assert_eq!(call(&mut store, &wrong, 0).code, data_code::EXPIRED);
    let wrong = edit(&a, data_op::COMMIT)
        .with_key(&DataKey::new("else", "other").unwrap())
        .unwrap();
    assert_eq!(call(&mut store, &wrong, 0).code, data_code::CONFLICT);
    assert_eq!(store.usage().transactions, 1);
    commit(&mut store, &a, 0);
}

#[test]
fn registry_pins_keep_metadata_and_payload_charged_after_service_drop() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    seed(&mut store, lease);
    let registry = store.registry();
    let budget = store.budget().clone();
    drop(store);
    assert!(budget.usage().page_bytes > 0);
    assert!(budget.usage().metadata_bytes > 0);
    assert_eq!(registry.lock().unwrap().pending_len(), 0);
    drop(registry);
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn exact_begin_bytes_and_page_scan_declarations_are_idempotent_not_replaceable() {
    for changed in 0..3 {
        let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
        let lease = open(&mut store, 0);
        let (page, bytes) = page(0, 1, &[1]);
        let begin = begin(
            lease,
            1,
            descriptor(),
            CommitMode::Replace,
            None,
            1,
            1,
            0,
            0,
            bytes.len() as u64,
        );
        ok(&mut store, &begin, 0);
        declare_scan(&mut store, &begin, &scan(0, 1), 0);
        declare_scan(&mut store, &begin, &scan(0, 1), 0);
        ok(&mut store, &declare_page(&begin, &page), 0);
        let usage = store.usage();
        ok(&mut store, &declare_page(&begin, &page), 0);
        assert_eq!(store.usage(), usage);
        let conflict = match changed {
            0 => {
                let mut retry = begin.clone();
                retry.metadata.push(b' ');
                retry
            }
            1 => {
                let mut different = page;
                different.crc32 ^= 1;
                declare_page(&begin, &different)
            }
            _ => DataRequest {
                scan_key: 0,
                ..edit(&begin, data_op::DECLARE_SCAN)
            }
            .with_metadata(&scan(0, 2))
            .unwrap(),
        };
        assert_eq!(call(&mut store, &conflict, 0).code, data_code::CONFLICT);
        assert_eq!(store.usage().transactions, 0);
        assert_eq!(store.usage().memory.page_bytes, 0);
        assert_eq!(store.usage().memory.reserved, 0);
        assert_eq!(
            call(&mut store, &edit(&begin, data_op::COMMIT), 0).code,
            data_code::CONFLICT
        );
    }
}

#[test]
fn clear_removes_failed_and_cancelled_history_even_through_legacy_registry() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let baseline = store.usage().memory.committed();
    let lease = open(&mut store, 0);
    let a = begin(
        lease,
        1,
        descriptor(),
        CommitMode::Replace,
        None,
        1,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &a, 0);
    assert_eq!(
        call(&mut store, &edit(&a, data_op::COMMIT), 0).code,
        data_code::INVALID
    );
    let b = begin(
        lease,
        2,
        descriptor(),
        CommitMode::Replace,
        None,
        0,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &b, 0);
    ok(&mut store, &edit(&b, data_op::CANCEL), 0);
    let c = begin(
        lease,
        3,
        descriptor(),
        CommitMode::Replace,
        None,
        0,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &c, 0);
    store.registry().lock().unwrap().clear("").unwrap();
    store.maintain(1).unwrap();
    assert_eq!(store.usage().transactions, 0);
    assert_eq!(store.usage().completion_history, 0);
    assert_eq!(store.usage().memory.committed(), baseline);
    for begin in [&a, &b, &c] {
        assert_eq!(call(&mut store, begin, 1).code, data_code::HISTORY_EXPIRED);
    }
}

#[test]
fn cancellation_retries_do_not_masquerade_as_successful_commits() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    let begin = begin(
        lease,
        1,
        descriptor(),
        CommitMode::Replace,
        None,
        0,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &begin, 0);
    let cancelled = ok(&mut store, &edit(&begin, data_op::CANCEL), 0);
    let duplicate = ok(&mut store, &edit(&begin, data_op::CANCEL), 0);
    assert_eq!(
        cancelled.pack_checked().unwrap().wire(),
        duplicate.pack_checked().unwrap().wire()
    );
    assert_eq!(
        call(&mut store, &edit(&begin, data_op::COMMIT), 0).code,
        data_code::EXPIRED
    );
    assert_eq!(call(&mut store, &begin, 0).code, data_code::EXPIRED);
    assert_eq!(store.registry().lock().unwrap().dataset_len(), 0);
}

#[test]
fn aggregate_writer_reservations_count_staged_and_not_yet_declared_payloads() {
    let (page, bytes) = page(0, 1, &[1]);
    let mut store = StreamingStore::new(
        EPOCH,
        DataLimits {
            writer_staged_bytes: page.byte_len,
            ..limits()
        },
    )
    .unwrap();
    let lease = open(&mut store, 0);
    let a = begin(
        lease,
        1,
        descriptor(),
        CommitMode::Replace,
        None,
        1,
        1,
        0,
        0,
        page.byte_len,
    );
    ok(&mut store, &a, 0);
    let mut descriptor = descriptor();
    descriptor.key.id = "another".into();
    let b = begin(
        lease,
        2,
        descriptor.clone(),
        CommitMode::Replace,
        None,
        1,
        1,
        0,
        0,
        page.byte_len,
    );
    assert_eq!(call(&mut store, &b, 0).code, data_code::BUDGET);
    upload(&mut store, &a, &page, &bytes, 0);
    assert_eq!(store.usage().staged_bytes, page.byte_len);
    let c = begin(
        lease,
        3,
        descriptor,
        CommitMode::Replace,
        None,
        1,
        1,
        0,
        0,
        page.byte_len,
    );
    assert_eq!(call(&mut store, &c, 0).code, data_code::BUDGET);
    assert_eq!(store.usage().transactions, 1);
    assert_eq!(store.usage().declared_bytes, page.byte_len);
}

#[test]
fn negotiated_page_sample_and_chunk_limits_fail_before_acceptance() {
    for sample_limit in [true, false] {
        let mut store = StreamingStore::new(
            EPOCH,
            DataLimits {
                page_points: 1,
                chunk_bytes: 16,
                ..limits()
            },
        )
        .unwrap();
        let lease = open(&mut store, 0);
        let (page, bytes) = page(0, 1, if sample_limit { &[1, 2] } else { &[1] });
        let begin = begin(
            lease,
            1,
            descriptor(),
            CommitMode::Replace,
            None,
            1,
            1,
            0,
            0,
            page.byte_len,
        );
        ok(&mut store, &begin, 0);
        let request = if sample_limit {
            declare_page(&begin, &page)
        } else {
            ok(&mut store, &declare_page(&begin, &page), 0);
            chunk(&begin, &page, 0, &bytes[..17])
        };
        assert_eq!(call(&mut store, &request, 0).code, data_code::BUDGET);
        assert_eq!(store.usage().transactions, 0);
        assert_eq!(store.usage().memory.page_bytes, 0);
    }
}

#[test]
fn writer_transaction_and_identity_slots_are_bounded_across_keys() {
    let mut store = StreamingStore::new(
        EPOCH,
        DataLimits {
            writers: 2,
            transactions: 2,
            writer_transactions: 1,
            datasets: 1,
            ..limits()
        },
    )
    .unwrap();
    let first = open(&mut store, 0);
    let second = open(&mut store, 0);
    assert_ne!(first, second);
    assert_eq!(
        call(&mut store, &req(data_op::OPEN_WRITER), 0).code,
        data_code::BUDGET
    );
    let a = begin(
        first,
        1,
        descriptor(),
        CommitMode::Replace,
        None,
        0,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &a, 0);
    let b = begin(
        first,
        2,
        descriptor(),
        CommitMode::Replace,
        None,
        0,
        0,
        0,
        0,
        0,
    );
    assert_eq!(call(&mut store, &b, 0).code, data_code::BUDGET);
    let mut other = descriptor();
    other.key.id = "other".into();
    let c = begin(second, 1, other, CommitMode::Replace, None, 0, 0, 0, 0, 0);
    assert_eq!(call(&mut store, &c, 0).code, data_code::BUDGET);
    let d = begin(
        second,
        2,
        descriptor(),
        CommitMode::Replace,
        None,
        0,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &d, 0);
    assert_eq!(store.usage().transactions, 2);
    commit(&mut store, &a, 0);
    assert_eq!(
        call(&mut store, &edit(&d, data_op::COMMIT), 0).code,
        data_code::CONFLICT
    );
    assert_eq!(store.registry().lock().unwrap().dataset_len(), 1);
}

#[test]
fn full_cpu_reservation_rejects_admission_without_a_leaked_permit() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    let budget = store.budget().clone();
    let headroom = budget
        .reserve(budget.usage().limit - budget.usage().committed() - 128 * 1024)
        .unwrap();
    let begin = begin(
        lease,
        1,
        descriptor(),
        CommitMode::Replace,
        None,
        1,
        1,
        0,
        0,
        1 << 20,
    );
    assert_eq!(call(&mut store, &begin, 0).code, data_code::BUDGET);
    assert_eq!(store.registry().lock().unwrap().pending_len(), 0);
    assert_eq!(store.usage().transactions, 0);
    assert!(budget.usage().peak <= budget.usage().limit);
    drop(headroom);
    clear(&mut store, lease, 0);
    drop(store);
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn removal_conflicts_and_changed_scan_metadata_leave_accumulated_data_intact() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    let mut descriptor = descriptor();
    descriptor.stream = contracts::cloud().stream;
    let a = begin(
        lease,
        1,
        descriptor.clone(),
        CommitMode::Replace,
        None,
        2,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &a, 0);
    declare_scan(&mut store, &a, &scan(1, 0), 0);
    declare_scan(&mut store, &a, &scan(2, 0), 0);
    let base = commit(&mut store, &a, 0);
    let old = retained(&store, &descriptor.key);
    let b = begin(
        lease,
        2,
        descriptor.clone(),
        CommitMode::Delta,
        Some(base),
        1,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &b, 0);
    let mut changed = scan(1, 0);
    changed.source = Some("different".into());
    let request = DataRequest {
        scan_key: 1,
        ..edit(&b, data_op::DECLARE_SCAN)
    }
    .with_metadata(&changed)
    .unwrap();
    assert_eq!(call(&mut store, &request, 0).code, data_code::CONFLICT);
    assert!(Arc::ptr_eq(&old, &retained(&store, &descriptor.key)));
    let c = begin(
        lease,
        3,
        descriptor.clone(),
        CommitMode::Delta,
        Some(base),
        0,
        0,
        1,
        0,
        0,
    );
    ok(&mut store, &c, 0);
    let remove = DataRequest {
        scan_key: 1,
        ..edit(&c, data_op::REMOVE_SCAN)
    };
    ok(&mut store, &remove, 0);
    ok(&mut store, &remove, 0);
    let base = commit(&mut store, &c, 0);
    assert_eq!(retained(&store, &descriptor.key).scans().len(), 1);
    let d = begin(
        lease,
        4,
        descriptor.clone(),
        CommitMode::Delta,
        Some(base),
        1,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &d, 0);
    let request = DataRequest {
        scan_key: 1,
        ..edit(&d, data_op::DECLARE_SCAN)
    }
    .with_metadata(&scan(1, 0))
    .unwrap();
    assert_eq!(call(&mut store, &request, 0).code, data_code::CONFLICT);
    assert_eq!(retained(&store, &descriptor.key).scan_high_watermark(), 2);
}

#[test]
fn octree_parent_replacement_is_atomic_and_cross_page_overlap_cannot_publish() {
    let descriptor = contracts::octree();
    let DataShape::Octree { config, .. } = &descriptor.shape else {
        unreachable!()
    };
    let occupancy = |id, depth| {
        let bytes = encode_occupancy_page(
            config,
            &[OccupancyLeaf {
                key: VoxelKey {
                    tile: [-3, 1, 0],
                    depth,
                    morton: 0,
                },
                value: OccupancyValue {
                    probability: Some(0.8),
                    confidence: None,
                    class_id: None,
                },
            }],
        )
        .unwrap();
        (
            PageDescriptor {
                address: PageAddress {
                    scan_key: 0,
                    page_id: id,
                },
                encoding: PageEncoding::OccupancyLeavesV1,
                items: 1,
                byte_len: bytes.len() as u64,
                crc32: page_crc32(&bytes),
                origin_m: [0.0; 3],
            },
            bytes,
        )
    };
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    let (parent, bytes) = occupancy(1, 0);
    let a = begin(
        lease,
        1,
        descriptor.clone(),
        CommitMode::Replace,
        None,
        0,
        1,
        0,
        0,
        bytes.len() as u64,
    );
    ok(&mut store, &a, 0);
    upload(&mut store, &a, &parent, &bytes, 0);
    let base = commit(&mut store, &a, 0);
    let old = retained(&store, &descriptor.key);
    let (child, bytes) = occupancy(2, 1);
    let b = begin(
        lease,
        2,
        descriptor.clone(),
        CommitMode::Delta,
        Some(base),
        0,
        1,
        0,
        0,
        bytes.len() as u64,
    );
    ok(&mut store, &b, 0);
    upload(&mut store, &b, &child, &bytes, 0);
    assert_eq!(
        call(&mut store, &edit(&b, data_op::COMMIT), 0).code,
        data_code::INVALID
    );
    assert!(Arc::ptr_eq(&old, &retained(&store, &descriptor.key)));
    let c = begin(
        lease,
        3,
        descriptor.clone(),
        CommitMode::Delta,
        Some(base),
        0,
        1,
        0,
        1,
        bytes.len() as u64,
    );
    ok(&mut store, &c, 0);
    upload(&mut store, &c, &child, &bytes, 0);
    ok(
        &mut store,
        &DataRequest {
            page_id: 1,
            ..edit(&c, data_op::REMOVE_PAGE)
        },
        0,
    );
    commit(&mut store, &c, 0);
    assert_eq!(
        retained(&store, &descriptor.key).pages()[0]
            .voxel_keys()
            .unwrap()
            .next()
            .unwrap()
            .depth,
        1
    );
}

#[test]
fn conditional_delete_respects_exact_base_and_kind_conflicts_are_bidirectional() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    let base = seed(&mut store, lease);
    let legacy = DrawEntity::points("mapping", "cloud", &[[0.0; 3]]);
    assert_eq!(
        store
            .registry()
            .lock()
            .unwrap()
            .apply_legacy(legacy.clone(), 0.0),
        Err(IdentityError::KindConflict)
    );
    let delete = DataRequest {
        lease,
        incarnation: base.incarnation,
        base_generation: base.generation + 1,
        ..req(data_op::DELETE)
    }
    .with_key(&descriptor().key)
    .unwrap();
    assert_eq!(call(&mut store, &delete, 0).code, data_code::CONFLICT);
    assert_eq!(store.registry().lock().unwrap().dataset_len(), 1);
    let mut other = contracts::octree();
    other.key = descriptor().key;
    let request = begin(
        lease,
        2,
        other,
        CommitMode::Replace,
        Some(base),
        0,
        0,
        0,
        0,
        0,
    );
    assert_eq!(call(&mut store, &request, 0).code, data_code::CONFLICT);
    let delete = DataRequest {
        base_generation: base.generation,
        ..delete
    };
    ok(&mut store, &delete, 0);
    assert_eq!(store.registry().lock().unwrap().dataset_len(), 0);
    assert_eq!(store.usage().memory.page_bytes, 0);
    store
        .registry()
        .lock()
        .unwrap()
        .apply_legacy(legacy, 0.0)
        .unwrap();
    let request = begin(
        lease,
        3,
        descriptor(),
        CommitMode::Replace,
        None,
        0,
        0,
        0,
        0,
        0,
    );
    assert_eq!(call(&mut store, &request, 0).code, data_code::CONFLICT);
}

#[test]
fn bounded_json_expansion_has_a_separate_always_available_request_allowance() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let baseline = store.usage().memory;
    assert_eq!(baseline.scratch_bytes, 2 << 20);
    let lease = open(&mut store, 0);
    let mut request = begin(
        lease,
        1,
        descriptor(),
        CommitMode::Replace,
        None,
        0,
        0,
        0,
        0,
        0,
    );
    request.metadata = format!("[{}0]", "0,".repeat((DATA_METADATA_BYTES - 3) / 2)).into_bytes();
    assert!(request.metadata.len() <= DATA_METADATA_BYTES);
    assert!(request.validate().is_ok());
    assert_eq!(call(&mut store, &request, 0).code, data_code::INVALID);
    assert_eq!(store.usage().memory, baseline);
    assert_eq!(store.usage().transactions, 0);
}

#[test]
fn one_million_points_publish_and_replace_under_a_single_cpu_budget() {
    let mut store = StreamingStore::new(
        EPOCH,
        DataLimits {
            cpu_bytes: 160 << 20,
            page_bytes: 4 << 20,
            writer_incomplete_bytes: 4 << 20,
            incomplete_bytes: 8 << 20,
            writer_staged_bytes: 64 << 20,
            ..limits()
        },
    )
    .unwrap();
    let lease = open(&mut store, 0);
    let count = 65_536u32;
    let pages = 16;
    let total = u64::from(count) * u64::from(pages);
    let sample_ids: Vec<u64> = (0..u64::from(count)).collect();
    let page_bytes = page(0, 0, &sample_ids).0.byte_len;
    let a = begin(
        lease,
        1,
        descriptor(),
        CommitMode::Replace,
        None,
        1,
        pages,
        0,
        0,
        page_bytes * u64::from(pages),
    );
    ok(&mut store, &a, 0);
    declare_scan(&mut store, &a, &scan(0, total), 0);
    for index in 0..pages {
        let ids: Vec<u64> = (0..u64::from(count))
            .map(|id| id + u64::from(index) * u64::from(count))
            .collect();
        let (page, bytes) = page(0, u64::from(index), &ids);
        upload(&mut store, &a, &page, &bytes, 0);
    }
    let base = commit(&mut store, &a, 0);
    let old = retained(&store, &descriptor().key);
    assert_eq!(old.counts().source_samples, total);
    let b = begin(
        lease,
        2,
        descriptor(),
        CommitMode::Delta,
        Some(base),
        0,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &b, 0);
    commit(&mut store, &b, 0);
    for (before, after) in old
        .pages()
        .iter()
        .zip(retained(&store, &descriptor().key).pages())
    {
        assert!(Arc::ptr_eq(before, after));
    }
    assert!(store.usage().memory.peak <= store.limits().cpu_bytes);
    let budget = store.budget().clone();
    clear(&mut store, lease, 0);
    drop(old);
    drop(store);
    assert_eq!(budget.usage().committed(), 0);
}

fn list_request(namespace: &str, limit: u32, cursor: Option<String>) -> DataRequest {
    req(data_op::LIST)
        .with_metadata(&DataListQuery {
            namespace: namespace.into(),
            limit,
            cursor,
        })
        .unwrap()
}

fn status_request(key: &DataKey) -> DataRequest {
    req(data_op::STATUS)
        .with_key(key)
        .unwrap()
        .with_metadata(&DataStatusQuery::default())
        .unwrap()
}

#[test]
fn shared_backend_has_one_service_owner_and_preserves_legacy_ownership() {
    let backend = DrawingBackend::new(limits()).unwrap();
    let mut store = StreamingStore::with_backend(EPOCH, backend.clone()).unwrap();
    assert!(StreamingStore::with_backend(EPOCH + 1, backend.clone()).is_err());
    let lease = open(&mut store, 0);
    seed(&mut store, lease);
    assert!(
        backend
            .lock()
            .apply(DrawEntity::points("mapping", "cloud", &[[0.0; 3]]), 0.0)
            .is_err()
    );
    let shared = backend.clone();
    shared.lock().clear("mapping");
    store.maintain(1).unwrap();
    assert_eq!(backend.registry().lock().unwrap().dataset_len(), 0);
    drop(store);
    assert!(StreamingStore::with_backend(EPOCH + 1, backend).is_ok());
}

#[test]
fn mixed_listing_bounds_encoded_bytes_and_replays_cursors_without_allocating_new_ones() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    seed(&mut store, lease);
    for i in 0..30 {
        store
            .registry()
            .lock()
            .unwrap()
            .apply_legacy(
                DrawEntity::points(
                    "mapping",
                    &format!("{i:03}-{}", "x".repeat(500)),
                    &[[0.0; 3]],
                ),
                0.0,
            )
            .unwrap();
    }
    let query = list_request("mapping", 128, None);
    let first = ok(&mut store, &query, 0);
    let page = DrawSummaryPage::from_json(&first.metadata).unwrap();
    assert!(page.items.len() < 31 && page.next_cursor.is_some());
    assert!(first.metadata.len() <= DATA_METADATA_BYTES);
    for _ in 0..10 {
        assert_eq!(ok(&mut store, &query, 0).metadata, first.metadata);
    }
    assert_eq!(store.cursor_len(), 1);
    let mut items = page.items;
    let mut next = page.next_cursor;
    while let Some(cursor) = next {
        let query = list_request("mapping", 128, Some(cursor));
        let reply = ok(&mut store, &query, 0);
        assert_eq!(ok(&mut store, &query, 0).metadata, reply.metadata);
        let page = DrawSummaryPage::from_json(&reply.metadata).unwrap();
        next = page.next_cursor;
        items.extend(page.items);
    }
    assert_eq!(items.len(), 31);
    assert!(items.windows(2).all(|pair| pair[0].key() < pair[1].key()));
    assert!(matches!(
        items.last().unwrap().object,
        SummaryOwner::Dataset {
            source_samples: 2,
            ..
        }
    ));
}

#[test]
fn cursor_expiry_query_binding_capacity_eviction_and_mutation_are_explicit() {
    let mut store = StreamingStore::new(
        EPOCH,
        DataLimits {
            cursors: 2,
            cursor_timeout_ms: 10,
            ..limits()
        },
    )
    .unwrap();
    for i in 0..5 {
        store
            .registry()
            .lock()
            .unwrap()
            .apply_legacy(DrawEntity::points("a", &i.to_string(), &[[0.0; 3]]), 0.0)
            .unwrap();
    }
    let first =
        DrawSummaryPage::from_json(&ok(&mut store, &list_request("a", 1, None), 0).metadata)
            .unwrap()
            .next_cursor
            .unwrap();
    for query in [
        list_request("b", 1, Some(first.clone())),
        list_request("a", 2, Some(first.clone())),
    ] {
        assert_eq!(call(&mut store, &query, 0).code, data_code::STALE_CURSOR);
    }
    let mut cursor = Some(first.clone());
    while let Some(token) = cursor {
        let page = DrawSummaryPage::from_json(
            &ok(&mut store, &list_request("a", 1, Some(token)), 0).metadata,
        )
        .unwrap();
        cursor = page.next_cursor;
        assert!(store.cursor_len() <= 2);
    }
    assert_eq!(
        call(&mut store, &list_request("a", 1, Some(first)), 0).code,
        data_code::STALE_CURSOR
    );
    let cursor =
        DrawSummaryPage::from_json(&ok(&mut store, &list_request("a", 1, None), 0).metadata)
            .unwrap()
            .next_cursor;
    assert_eq!(
        call(&mut store, &list_request("a", 1, cursor), 10).code,
        data_code::STALE_CURSOR
    );
    let cursor =
        DrawSummaryPage::from_json(&ok(&mut store, &list_request("a", 1, None), 10).metadata)
            .unwrap()
            .next_cursor;
    store
        .registry()
        .lock()
        .unwrap()
        .apply_legacy(DrawEntity::points("b", "new", &[[0.0; 3]]), 0.0)
        .unwrap();
    assert_eq!(
        call(&mut store, &list_request("a", 1, cursor), 10).code,
        data_code::STALE_CURSOR
    );
}

#[test]
fn oversized_legacy_summary_is_rejected_without_truncating_its_identity() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let id = "x".repeat(13_000);
    store
        .registry()
        .lock()
        .unwrap()
        .apply_legacy(DrawEntity::points("long", &id, &[[0.0; 3]]), 0.0)
        .unwrap();
    assert_eq!(
        call(&mut store, &list_request("", 128, None), 0).code,
        data_code::BUDGET
    );
    assert!(
        store
            .registry()
            .lock()
            .unwrap()
            .legacy()
            .get("long", &id)
            .is_some()
    );
    assert_eq!(store.cursor_len(), 0);
}

#[test]
fn status_separates_retention_visibility_failures_and_pinned_retired_payloads() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    let base = seed(&mut store, lease);
    let old = retained(&store, &descriptor().key);
    let bad = begin(
        lease,
        2,
        descriptor(),
        CommitMode::Replace,
        Some(base),
        1,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &bad, 0);
    let query = status_request(&descriptor().key);
    let status = DatasetStatus::from_json(&ok(&mut store, &query, 0).metadata).unwrap();
    assert_eq!(status.retained, Some(base));
    assert_eq!(status.transfer.unwrap().transaction, 2);
    assert!(matches!(
        status.visibility,
        GenerationVisibility::Unavailable { .. }
    ));
    assert_eq!(
        call(&mut store, &edit(&bad, data_op::COMMIT), 0).code,
        data_code::INVALID
    );
    let status = DatasetStatus::from_json(&ok(&mut store, &query, 0).metadata).unwrap();
    assert_eq!(status.failure.unwrap().phase, DataPhase::Validating);
    assert_eq!(status.retained, Some(base));
    let next = begin(
        lease,
        3,
        descriptor(),
        CommitMode::Replace,
        Some(base),
        0,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &next, 0);
    commit(&mut store, &next, 0);
    let status = DatasetStatus::from_json(&ok(&mut store, &query, 0).metadata).unwrap();
    assert!(status.failure.is_none());
    assert_eq!(
        status.resources.pinned_retired_bytes,
        old.counts().payload_bytes
    );
    assert_eq!(status.counters.received_bytes, old.counts().payload_bytes);
    drop(old);
    let status = DatasetStatus::from_json(&ok(&mut store, &query, 0).metadata).unwrap();
    assert_eq!(status.resources.pinned_retired_bytes, 0);
    assert_eq!(status.resources.resident_page_bytes, 0);
}

#[test]
fn presentation_shares_geometry_and_rejects_bad_selectors_and_stale_bases() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    let base = seed(&mut store, lease);
    let old = retained(&store, &descriptor().key);
    let mut presentation = descriptor().presentation;
    presentation.color = ColorMode::Constant {
        rgba: [240, 80, 30, 210],
    };
    presentation.blur = 0.9;
    let request = DataRequest {
        lease,
        incarnation: base.incarnation,
        base_generation: base.generation,
        ..req(data_op::PATCH_PRESENTATION)
    }
    .with_key(&descriptor().key)
    .unwrap()
    .with_metadata(&presentation)
    .unwrap();
    let reply = ok(&mut store, &request, 0);
    assert_eq!(reply.stored_generation, 2);
    let new = retained(&store, &descriptor().key);
    assert!(Arc::ptr_eq(&new.pages()[0], &old.pages()[0]));
    assert!(Arc::ptr_eq(&new.scans()[0], &old.scans()[0]));
    assert_eq!(new.descriptor().presentation, presentation);
    assert_eq!(store.usage().memory.page_bytes, old.counts().payload_bytes);
    assert_eq!(call(&mut store, &request, 0).code, data_code::CONFLICT);
    presentation.color = ColorMode::Attribute {
        name: "missing".into(),
        space: ColorSpace::Srgb,
    };
    let invalid = DataRequest {
        base_generation: 2,
        ..request
    }
    .with_metadata(&presentation)
    .unwrap();
    assert_eq!(call(&mut store, &invalid, 0).code, data_code::INVALID);
    assert!(Arc::ptr_eq(&new, &retained(&store, &descriptor().key)));
    assert_eq!(store.registry().lock().unwrap().pending_len(), 0);
}

#[test]
fn simulation_lifetime_pauses_and_presentation_does_not_reset_its_start() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    store.advance_simulation(10.0).unwrap();
    let mut descriptor = descriptor();
    descriptor.lifetime_s = Some(2.0);
    let first = begin(
        lease,
        1,
        descriptor.clone(),
        CommitMode::Replace,
        None,
        0,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &first, 0);
    let base = commit(&mut store, &first, 0);
    store.maintain(500).unwrap();
    store.advance_simulation(10.0).unwrap();
    assert_eq!(retained(&store, &descriptor.key).retained_at_s(), 10.0);
    store.advance_simulation(11.0).unwrap();
    let request = DataRequest {
        lease,
        incarnation: base.incarnation,
        base_generation: base.generation,
        ..req(data_op::PATCH_PRESENTATION)
    }
    .with_key(&descriptor.key)
    .unwrap()
    .with_metadata(&descriptor.presentation)
    .unwrap();
    ok(&mut store, &request, 500);
    assert_eq!(retained(&store, &descriptor.key).retained_at_s(), 10.0);
    assert_eq!(store.advance_simulation(11.999).unwrap(), 0);
    assert_eq!(store.advance_simulation(12.0).unwrap(), 1);
    store.maintain(501).unwrap();
    assert_eq!(store.registry().lock().unwrap().dataset_len(), 0);
    assert_eq!(store.usage().memory.reserved, 0);
    assert!(store.advance_simulation(f64::NAN).is_err());
}

#[test]
fn latest_replacement_cannot_reuse_the_current_scans_identity() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    let lease = open(&mut store, 0);
    let mut descriptor = descriptor();
    descriptor.stream = StreamMode::Latest;
    let a = begin(
        lease,
        1,
        descriptor.clone(),
        CommitMode::Replace,
        None,
        1,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &a, 0);
    declare_scan(&mut store, &a, &scan(1, 0), 0);
    let base = commit(&mut store, &a, 0);
    let b = begin(
        lease,
        2,
        descriptor,
        CommitMode::Replace,
        Some(base),
        1,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &b, 0);
    let duplicate = DataRequest {
        scan_key: 1,
        ..edit(&b, data_op::DECLARE_SCAN)
    }
    .with_metadata(&scan(1, 0))
    .unwrap();
    assert_eq!(call(&mut store, &duplicate, 0).code, data_code::CONFLICT);
    assert_eq!(
        store
            .registry()
            .lock()
            .unwrap()
            .get(&("mapping".into(), "cloud".into()))
            .unwrap()
            .revision,
        base
    );
}

#[test]
fn deletion_defers_payload_destruction_until_service_maintenance() {
    let backend = DrawingBackend::new(limits()).unwrap();
    let mut store = StreamingStore::with_backend(EPOCH, backend.clone()).unwrap();
    let lease = open(&mut store, 0);
    seed(&mut store, lease);
    let generation = Arc::downgrade(&retained(&store, &descriptor().key));
    backend.lock().clear("mapping");
    assert!(generation.upgrade().is_some());
    store.maintain(1).unwrap();
    assert!(generation.upgrade().is_none());
    assert_eq!(store.usage().memory.page_bytes, 0);
}

#[test]
fn simulation_rewind_preserves_permanent_maps_and_revokes_all_pending_publications() {
    let mut store = StreamingStore::new(EPOCH, limits()).unwrap();
    store.advance_simulation(20.0).unwrap();
    let lease = open(&mut store, 0);
    let base = seed(&mut store, lease);
    let map = retained(&store, &descriptor().key);
    let mut temporary = descriptor();
    temporary.key.id = "temporary".into();
    temporary.lifetime_s = Some(5.0);
    let first = begin(
        lease,
        2,
        temporary.clone(),
        CommitMode::Replace,
        None,
        0,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &first, 0);
    commit(&mut store, &first, 0);
    let mut acquired = descriptor();
    acquired.key.id = "acquired".into();
    acquired.stream = StreamMode::Latest;
    let latest = begin(
        lease,
        3,
        acquired.clone(),
        CommitMode::Replace,
        None,
        1,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &latest, 0);
    declare_scan(&mut store, &latest, &scan(1, 0), 0);
    commit(&mut store, &latest, 0);
    let pending = begin(
        lease,
        4,
        descriptor(),
        CommitMode::Delta,
        Some(base),
        0,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &pending, 0);
    for invalid in [f64::NAN, f64::INFINITY, -1.0] {
        assert!(store.advance_simulation(invalid).is_err());
    }
    assert_eq!(store.usage().transactions, 1);
    assert_eq!(store.advance_simulation(19.0).unwrap(), 2);
    store.maintain(1).unwrap();
    assert_eq!(store.registry().lock().unwrap().dataset_len(), 1);
    assert!(Arc::ptr_eq(&map, &retained(&store, &descriptor().key)));
    assert_eq!(store.usage().transactions, 0);
    assert_eq!(store.usage().memory.reserved, 0);
    assert_eq!(
        call(&mut store, &edit(&pending, data_op::COMMIT), 1).code,
        data_code::HISTORY_EXPIRED
    );
    assert_eq!(
        call(&mut store, &status_request(&temporary.key), 1).code,
        data_code::NOT_FOUND
    );
    assert_eq!(
        call(&mut store, &status_request(&acquired.key), 1).code,
        data_code::NOT_FOUND
    );
}

#[test]
fn deferred_retirement_respects_dataset_slot_capacity_until_drained() {
    let backend = DrawingBackend::new(DataLimits {
        datasets: 1,
        ..limits()
    })
    .unwrap();
    let mut store = StreamingStore::with_backend(EPOCH, backend.clone()).unwrap();
    let lease = open(&mut store, 0);
    seed(&mut store, lease);
    let registry = backend.registry();
    let weak = Arc::downgrade(&retained(&store, &descriptor().key));
    {
        let mut registry = registry.lock().unwrap();
        registry.clear("").unwrap();
        assert_eq!(
            registry
                .begin_dataset(
                    &DataKey::new("other", "new").unwrap(),
                    DatasetKind::Cloud,
                    None
                )
                .unwrap_err(),
            IdentityError::Budget
        );
        assert_eq!(registry.clear("").unwrap(), 0);
    }
    store.maintain(1).unwrap();
    assert!(weak.upgrade().is_none());
    let next = begin(
        lease,
        2,
        descriptor(),
        CommitMode::Replace,
        None,
        0,
        0,
        0,
        0,
        0,
    );
    ok(&mut store, &next, 1);
    commit(&mut store, &next, 1);
}
