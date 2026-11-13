use std::sync::{
    Arc, Barrier,
    atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use gearbox_api::draw_data::*;
use gearbox_api::wire::draw_data::*;

#[path = "support/draw_provider_fixture.rs"]
mod fixture;
use fixture::Grid;

static CANCELLED: AtomicBool = AtomicBool::new(false);

fn request() -> PageRequest<'static> {
    PageRequest::new(Instant::now() + Duration::from_secs(10), &CANCELLED)
}

fn grid(count: u32) -> Grid {
    Grid {
        count,
        start: 65_536,
        page: 7,
    }
}

#[test]
fn verified_metadata_survives_actual_payload_eviction_and_exact_reload() {
    for count in [0, 1, 65_536] {
        let budget = CpuBudget::new(8 << 20);
        let grid = grid(count);
        let original = grid.stored(&budget);
        let digest = *blake3::hash(original.bytes()).as_bytes();
        let provider = RegisteredProvider::new(&budget, grid).unwrap();
        let page = ReloadablePage::verified(&budget, original, provider, request()).unwrap();
        let lease = page.acquire(request()).unwrap();
        assert!(page.evict());
        assert_eq!(budget.usage().page_bytes, grid.len() as u64);
        assert_eq!(page.metadata().descriptor(), &grid.descriptor());
        assert_eq!(page.metadata().digest(), &digest);
        drop(lease);
        assert_eq!(budget.usage().page_bytes, 0);
        assert_eq!(
            page.status(),
            PageState::Provider(ProviderAvailability::Available)
        );
        let loaded = page.acquire(request()).unwrap();
        assert_eq!(*blake3::hash(loaded.bytes()).as_bytes(), digest);
        assert_eq!(loaded.source_ids().unwrap().len(), count as usize);
        for (index, id) in loaded.source_ids().unwrap().enumerate() {
            assert_eq!(id, u64::MAX - grid.start - index as u64);
        }
        assert_eq!(
            page.statistics(),
            ProviderStatistics {
                hits: 1,
                misses: 1,
                loads: 1,
                evictions: 1,
                failures: 0
            }
        );
        assert!(page.evict());
        drop(loaded);
        assert_eq!(budget.usage().page_bytes, 0);
        assert!(budget.usage().metadata_bytes > 0);
        drop(page);
        assert_eq!(budget.usage().committed(), 0);
    }
}

#[test]
fn retained_only_pages_cannot_be_evicted_and_leases_pin_their_bytes() {
    let budget = CpuBudget::new(1 << 20);
    let page = ReloadablePage::retained(&budget, grid(256).stored(&budget)).unwrap();
    let lease = page.acquire(request()).unwrap();
    assert!(!page.evict());
    page.invalidate();
    assert_eq!(page.status(), PageState::Invalidated);
    assert!(matches!(
        page.acquire(request()),
        Err(ProviderError::Cancelled)
    ));
    drop(page);
    assert!(budget.usage().page_bytes > 0);
    drop(lease);
    assert_eq!(budget.usage().committed(), 0);
}

struct Switches {
    mode: AtomicU8,
    calls: AtomicU64,
    entered: Barrier,
    released: Barrier,
}

struct Controlled {
    grid: Grid,
    switches: Arc<Switches>,
}

impl PageProvider for Controlled {
    fn owned_bytes(&self) -> u64 {
        (size_of::<Switches>() + 2 * size_of::<usize>()) as u64
    }

    fn availability(&self, _: &PageDescriptor) -> ProviderAvailability {
        match self.switches.mode.load(Ordering::Acquire) {
            1 => ProviderAvailability::Missing,
            2 => ProviderAvailability::Pending,
            _ => ProviderAvailability::Available,
        }
    }

    fn load(&self, sink: &mut PageSink<'_>) -> Result<(), ProviderError> {
        self.switches.calls.fetch_add(1, Ordering::Relaxed);
        let mode = self.switches.mode.load(Ordering::Acquire);
        if mode == 6 {
            panic!("fixture provider panic");
        }
        if mode == 8 {
            self.switches.entered.wait();
            self.switches.released.wait();
        }
        if mode == 5 {
            return sink.push(&[0; DATA_CHUNK_BYTES + 1]);
        }
        let len = self.grid.len() - usize::from(mode == 4);
        let mut bytes = [0; DATA_CHUNK_BYTES];
        for offset in (0..len).step_by(bytes.len()) {
            let count = bytes.len().min(len - offset);
            self.grid.fill(offset, &mut bytes[..count]);
            if mode == 3 && offset == 0 {
                bytes[0] ^= 1;
            }
            sink.push(&bytes[..count])?;
        }
        Ok(())
    }
}

fn controlled(budget: &CpuBudget, grid: Grid) -> (Arc<RegisteredProvider>, Arc<Switches>) {
    let switches = Arc::new(Switches {
        mode: AtomicU8::new(0),
        calls: AtomicU64::new(0),
        entered: Barrier::new(2),
        released: Barrier::new(2),
    });
    (
        RegisteredProvider::new(
            budget,
            Controlled {
                grid,
                switches: switches.clone(),
            },
        )
        .unwrap(),
        switches,
    )
}

#[test]
fn provider_attachment_is_verified_before_retained_data_can_be_evicted() {
    let budget = CpuBudget::new(1 << 20);
    let original = grid(1024).stored(&budget);
    let (provider, switches) = controlled(&budget, grid(1024));
    let before = budget.usage().committed();
    switches.mode.store(3, Ordering::Release);
    assert!(matches!(
        ReloadablePage::verified(&budget, original.clone(), provider.clone(), request()),
        Err(ProviderError::Changed)
    ));
    assert_eq!(budget.usage().committed(), before);
    assert_eq!(
        original.source_ids().unwrap().next(),
        Some(u64::MAX - 65_536)
    );
    drop((original, provider));
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn unavailable_incomplete_changed_and_oversized_provider_results_never_publish() {
    let budget = CpuBudget::new(1 << 20);
    let (provider, switches) = controlled(&budget, grid(1024));
    let page =
        ReloadablePage::verified(&budget, grid(1024).stored(&budget), provider, request()).unwrap();
    assert!(page.evict());
    let baseline = budget.usage().committed();
    for mode in 1..=5 {
        switches.mode.store(mode, Ordering::Release);
        let result = page.acquire(request());
        match mode {
            1 => assert!(matches!(
                result,
                Err(ProviderError::Unavailable(ProviderAvailability::Missing))
            )),
            2 => assert!(matches!(
                result,
                Err(ProviderError::Unavailable(ProviderAvailability::Pending))
            )),
            3 => assert!(matches!(result, Err(ProviderError::Changed))),
            4 => assert!(matches!(result, Err(ProviderError::Incomplete))),
            _ => assert!(matches!(
                result,
                Err(ProviderError::Storage(StorageError::Invalid(_)))
            )),
        }
        assert_eq!(budget.usage().page_bytes, 0);
        assert_eq!(budget.usage().committed(), baseline);
    }
    switches.mode.store(0, Ordering::Release);
    drop(page.acquire(request()).unwrap());
    assert_eq!(page.statistics().failures, 5);
    drop(page);
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn cpu_pressure_deadlines_cancellation_and_provider_panics_release_reload_ownership() {
    let budget = CpuBudget::new(1 << 20);
    let (provider, switches) = controlled(&budget, grid(1024));
    let page =
        ReloadablePage::verified(&budget, grid(1024).stored(&budget), provider, request()).unwrap();
    assert!(page.evict());
    let baseline = budget.usage().committed();
    let hold = budget.reserve(budget.usage().limit - baseline).unwrap();
    assert!(matches!(
        page.acquire(request()),
        Err(ProviderError::Storage(StorageError::Budget { .. }))
    ));
    drop(hold);
    assert_eq!(budget.usage().committed(), baseline);
    assert!(matches!(
        page.acquire(PageRequest::new(Instant::now(), &CANCELLED)),
        Err(ProviderError::Deadline)
    ));
    let cancel = AtomicBool::new(true);
    assert!(matches!(
        page.acquire(PageRequest::new(
            Instant::now() + Duration::from_secs(1),
            &cancel
        )),
        Err(ProviderError::Cancelled)
    ));
    switches.mode.store(6, Ordering::Release);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| page.acquire(request()))).is_err()
    );
    assert_eq!(budget.usage().committed(), baseline);
    assert_ne!(page.status(), PageState::Loading);
    switches.mode.store(0, Ordering::Release);
    drop(page.acquire(request()).unwrap());
    drop(page);
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn single_flight_reload_can_be_cancelled_without_holding_the_cache_lock() {
    for invalidate in [false, true] {
        let budget = CpuBudget::new(1 << 20);
        let (provider, switches) = controlled(&budget, grid(1024));
        let page =
            ReloadablePage::verified(&budget, grid(1024).stored(&budget), provider, request())
                .unwrap();
        page.evict();
        let baseline = budget.usage().committed();
        switches.mode.store(8, Ordering::Release);
        let loading = page.clone();
        let thread = std::thread::spawn(move || loading.acquire(request()));
        switches.entered.wait();
        assert_eq!(page.status(), PageState::Loading);
        assert!(matches!(page.acquire(request()), Err(ProviderError::Busy)));
        if invalidate {
            page.invalidate();
        } else {
            assert!(!page.evict());
        }
        switches.released.wait();
        assert!(matches!(
            thread.join().unwrap(),
            Err(ProviderError::Cancelled)
        ));
        assert_eq!(budget.usage().committed(), baseline);
        switches.mode.store(0, Ordering::Release);
        if invalidate {
            assert!(matches!(
                page.acquire(request()),
                Err(ProviderError::Cancelled)
            ));
        } else {
            drop(page.acquire(request()).unwrap());
        }
        drop(page);
        assert_eq!(budget.usage().committed(), 0);
    }
}

#[test]
fn provider_and_metadata_cannot_cross_cpu_budget_owners() {
    let first = CpuBudget::new(1 << 20);
    let other = CpuBudget::new(1 << 20);
    let original = grid(256).stored(&first);
    assert!(original.metadata(&other).is_err());
    let provider = RegisteredProvider::new(&other, grid(256)).unwrap();
    assert!(matches!(
        ReloadablePage::verified(&first, original.clone(), provider.clone(), request()),
        Err(ProviderError::Storage(StorageError::Invalid(_)))
    ));
    drop((original, provider));
    assert_eq!(first.usage().committed(), 0);
    assert_eq!(other.usage().committed(), 0);
}

#[test]
fn cancellation_during_provider_execution_releases_the_reserved_payload() {
    let budget = CpuBudget::new(1 << 20);
    let (provider, switches) = controlled(&budget, grid(1024));
    let page =
        ReloadablePage::verified(&budget, grid(1024).stored(&budget), provider, request()).unwrap();
    page.evict();
    let baseline = budget.usage().committed();
    switches.mode.store(8, Ordering::Release);
    let cancelled = Arc::new(AtomicBool::new(false));
    let (loading, cancellation) = (page.clone(), cancelled.clone());
    let thread = std::thread::spawn(move || {
        loading.acquire(PageRequest::new(
            Instant::now() + Duration::from_secs(10),
            &cancellation,
        ))
    });
    switches.entered.wait();
    cancelled.store(true, Ordering::Release);
    switches.released.wait();
    assert!(matches!(
        thread.join().unwrap(),
        Err(ProviderError::Cancelled)
    ));
    assert_eq!(budget.usage().committed(), baseline);
    drop(page);
    assert_eq!(budget.usage().committed(), 0);
}

struct TiledGrid {
    missing: Option<u64>,
}

impl PageProvider for TiledGrid {
    fn owned_bytes(&self) -> u64 {
        0
    }
    fn availability(&self, page: &PageDescriptor) -> ProviderAvailability {
        if self.missing == Some(page.address.page_id) {
            ProviderAvailability::Missing
        } else {
            ProviderAvailability::Available
        }
    }
    fn load(&self, sink: &mut PageSink<'_>) -> Result<(), ProviderError> {
        let descriptor = sink.descriptor();
        Grid {
            count: descriptor.items,
            start: descriptor.address.page_id * 4096,
            page: descriptor.address.page_id,
        }
        .load(sink)
    }
}

fn generation(budget: &CpuBudget, stream: StreamMode) -> Arc<Generation> {
    let scan_key = u64::from(!matches!(stream, StreamMode::RetainedMap));
    let descriptor = DatasetMetadata::new(
        budget,
        &DataDescriptor {
            revision: SCHEMA_REVISION,
            key: DataKey::new("provider-test", "tiles").unwrap(),
            shape: DataShape::Cloud {
                schema: Grid::schema(),
            },
            stream,
            lifetime_s: Some(30.0),
            presentation: Presentation::default(),
        },
    )
    .unwrap();
    let scans = [ScanMetadata::new(
        budget,
        &ScanDescriptor {
            revision: SCHEMA_REVISION,
            scan_key,
            source: Some("synthetic/provider-tiles".into()),
            frame: DataFrame {
                capture: (scan_key != 0).then_some(CaptureStamp {
                    simulation_time_s: 1.0,
                    source_time_ns: None,
                    source_clock: None,
                    sample: 1,
                    scene_epoch: 1,
                }),
                ..Default::default()
            },
            source_count: 8192 + 123,
            organization: None,
        },
    )
    .unwrap()];
    let pages: Vec<_> = [4096, 4096, 123]
        .into_iter()
        .enumerate()
        .map(|(page, count)| {
            Grid {
                count,
                start: page as u64 * 4096,
                page: page as u64,
            }
            .stored_scan(budget, scan_key)
        })
        .collect();
    Generation::build(budget, descriptor, &scans, &pages).unwrap()
}

#[test]
fn paged_generation_retains_identity_and_counts_without_pinning_original_payloads() {
    let budget = CpuBudget::new(4 << 20);
    let source = generation(&budget, StreamMode::RetainedMap);
    let counts = source.counts();
    let descriptor = source.descriptor().to_json().unwrap();
    let provider = RegisteredProvider::new(&budget, TiledGrid { missing: None }).unwrap();
    let paged = PagedGeneration::verified(&budget, &source, provider, request()).unwrap();
    assert_eq!(paged.counts(), counts);
    assert_eq!(paged.descriptor().to_json().unwrap(), descriptor);
    assert_eq!(paged.scans().len(), 1);
    assert_eq!(paged.scan_high_watermark(), source.scan_high_watermark());
    assert_eq!(paged.retained_at_s(), source.retained_at_s());
    drop(source);
    for page in paged.pages() {
        assert!(page.evict());
    }
    assert_eq!(budget.usage().page_bytes, 0);
    assert_eq!(paged.counts().payload_bytes, counts.payload_bytes);
    for index in 0..3 {
        let address = PageAddress {
            scan_key: 0,
            page_id: index,
        };
        let page = paged.page(address).unwrap();
        let loaded = page.acquire(request()).unwrap();
        assert_eq!(loaded.descriptor().address, address);
        assert_eq!(
            loaded.source_ids().unwrap().next(),
            Some(u64::MAX - index * 4096)
        );
        drop(loaded);
        page.evict();
        assert_eq!(budget.usage().page_bytes, 0);
    }
    assert!(
        paged
            .page(PageAddress {
                scan_key: 0,
                page_id: 3
            })
            .is_none()
    );
    paged.invalidate();
    assert!(
        paged
            .pages()
            .iter()
            .all(|page| page.status() == PageState::Invalidated)
    );
    drop(paged);
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn failed_generation_conversion_leaves_the_original_unchanged_and_releases_partial_work() {
    let budget = CpuBudget::new(4 << 20);
    let source = generation(&budget, StreamMode::RetainedMap);
    let counts = source.counts();
    let provider = RegisteredProvider::new(&budget, TiledGrid { missing: Some(2) }).unwrap();
    let baseline = budget.usage();
    assert!(matches!(
        PagedGeneration::verified(&budget, &source, provider.clone(), request()),
        Err(ProviderError::Unavailable(ProviderAvailability::Missing))
    ));
    assert_eq!(budget.usage().committed(), baseline.committed());
    assert_eq!(budget.usage().page_bytes, baseline.page_bytes);
    assert_eq!(source.counts(), counts);
    assert_eq!(
        source.pages()[2].source_ids().unwrap().next(),
        Some(u64::MAX - 8192)
    );
    drop((source, provider));
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn generation_conversion_rejects_other_stream_policies_and_budget_owners() {
    let budget = CpuBudget::new(4 << 20);
    let source = generation(&budget, StreamMode::Latest);
    let provider = RegisteredProvider::new(&budget, TiledGrid { missing: None }).unwrap();
    let baseline = budget.usage().committed();
    assert!(matches!(
        PagedGeneration::verified(&budget, &source, provider.clone(), request()),
        Err(ProviderError::Storage(StorageError::Invalid(_)))
    ));
    assert_eq!(budget.usage().committed(), baseline);
    drop(source);
    let source = generation(&budget, StreamMode::RetainedMap);
    let other = CpuBudget::new(4 << 20);
    assert!(matches!(
        PagedGeneration::verified(&other, &source, provider.clone(), request()),
        Err(ProviderError::Storage(StorageError::Invalid(_)))
    ));
    assert_eq!(other.usage().committed(), 0);
    drop((source, provider));
    assert_eq!(budget.usage().committed(), 0);
}

const EPOCH: u64 = 23;

fn published(backend: &DrawingBackend) -> (DataKey, DatasetRevision) {
    let source = generation(backend.budget(), StreamMode::RetainedMap);
    let key = source.descriptor().key.clone();
    let storage = DatasetStorage::from_resident(backend.budget(), source).unwrap();
    let registry = backend.registry();
    let mut registry = registry.lock().unwrap();
    let permit = registry
        .begin_dataset(&key, DatasetKind::Cloud, None)
        .unwrap();
    (key, registry.commit_dataset(permit, storage).unwrap())
}

fn stored(backend: &DrawingBackend, key: &DataKey) -> (DatasetRevision, Arc<DatasetStorage>) {
    let registry = backend.registry();
    let registry = registry.lock().unwrap();
    let entry = registry.get_parts(&key.namespace, &key.id).unwrap();
    (entry.revision, entry.payload.clone())
}

fn tiles(backend: &DrawingBackend) -> Arc<RegisteredProvider> {
    RegisteredProvider::new(backend.budget(), TiledGrid { missing: None }).unwrap()
}

fn drain_retired(backend: &DrawingBackend) {
    let registry = backend.registry();
    let mut registry = registry.lock().unwrap();
    while registry.pop_retired().is_some() {}
}

#[test]
fn registry_attachment_swaps_storage_and_releases_original_payloads() {
    let backend = DrawingBackend::default();
    let baseline = backend.budget().usage().committed();
    let (key, revision) = published(&backend);
    let sequence = backend.registry().lock().unwrap().revision();
    let paged = backend
        .attach_provider(&key, revision, tiles(&backend), request())
        .unwrap();
    let (current, storage) = stored(&backend, &key);
    assert_eq!(current, revision);
    assert!(backend.registry().lock().unwrap().revision() > sequence);
    assert!(Arc::ptr_eq(storage.paged().unwrap(), &paged));
    assert_eq!(storage.counts(), paged.counts());
    for page in paged.pages() {
        assert!(page.evict());
    }
    assert_eq!(backend.budget().usage().page_bytes, 0);
    for index in 0..storage.page_len() {
        let loaded = storage.page_at(index).unwrap().acquire(request()).unwrap();
        assert_eq!(
            loaded.source_ids().unwrap().next(),
            Some(u64::MAX - index as u64 * 4096)
        );
    }
    assert!(backend.budget().usage().page_bytes > 0);

    let repeated = backend.attach_provider(&key, revision, tiles(&backend), request());
    assert_eq!(repeated.err().unwrap().code, data_code::CONFLICT);
    let stale = DatasetRevision {
        generation: revision.generation + 1,
        ..revision
    };
    let stale = backend.attach_provider(&key, stale, tiles(&backend), request());
    assert_eq!(stale.err().unwrap().code, data_code::CONFLICT);
    let absent = DataKey::new("provider-test", "absent").unwrap();
    let absent = backend.attach_provider(&absent, revision, tiles(&backend), request());
    assert_eq!(absent.err().unwrap().code, data_code::NOT_FOUND);

    drop(storage);
    backend
        .registry()
        .lock()
        .unwrap()
        .delete(&(key.namespace.clone(), key.id.clone()))
        .unwrap();
    assert!(
        paged
            .pages()
            .iter()
            .all(|page| page.status() == PageState::Invalidated)
    );
    assert!(matches!(
        paged.pages()[0].acquire(request()),
        Err(ProviderError::Cancelled)
    ));
    drop(paged);
    drain_retired(&backend);
    assert_eq!(backend.budget().usage().committed(), baseline);
}

#[test]
fn failed_registry_attachment_keeps_resident_storage() {
    let backend = DrawingBackend::default();
    let (key, revision) = published(&backend);
    let (_, before) = stored(&backend, &key);
    let provider =
        RegisteredProvider::new(backend.budget(), TiledGrid { missing: Some(1) }).unwrap();
    let result = backend.attach_provider(&key, revision, provider, request());
    assert_eq!(result.err().unwrap().code, data_code::NOT_FOUND);
    let (current, after) = stored(&backend, &key);
    assert_eq!(current, revision);
    assert!(Arc::ptr_eq(&before, &after));
    assert!(after.resident().is_some());
}

#[test]
fn presentation_update_of_paged_storage_keeps_handles_without_loading() {
    let backend = DrawingBackend::default();
    let (key, revision) = published(&backend);
    let paged = backend
        .attach_provider(&key, revision, tiles(&backend), request())
        .unwrap();
    for page in paged.pages() {
        page.evict();
    }
    let (_, storage) = stored(&backend, &key);
    let presentation = Presentation {
        glow: storage.descriptor().presentation.glow + 0.25,
        ..storage.descriptor().presentation.clone()
    };
    let restyled = storage.restyle(backend.budget(), &presentation).unwrap();
    assert_eq!(restyled.descriptor().presentation, presentation);
    let restyled = restyled.paged().unwrap();
    for (old, new) in paged.pages().iter().zip(restyled.pages()) {
        assert!(Arc::ptr_eq(old, new));
        assert_eq!(new.statistics().loads, 0);
    }
    assert_eq!(backend.budget().usage().page_bytes, 0);
}

fn data_request(op: u32) -> DataRequest {
    DataRequest {
        host_epoch: EPOCH,
        ..DataRequest::new(op)
    }
}

fn accepted(store: &mut StreamingStore, request: &DataRequest) -> DataReply {
    let reply = store.handle(request, 0);
    assert_eq!(
        reply.code,
        data_code::OK,
        "{}",
        String::from_utf8_lossy(&reply.message)
    );
    reply
}

fn edit(begin: &DataRequest, operation: u32, page: PageAddress) -> DataRequest {
    DataRequest {
        lease: begin.lease,
        transaction: begin.transaction,
        incarnation: begin.incarnation,
        base_generation: begin.base_generation,
        key: begin.key.clone(),
        scan_key: page.scan_key,
        page_id: page.page_id,
        ..data_request(operation)
    }
}

#[test]
fn delta_commit_over_paged_storage_inherits_unchanged_handles() {
    let backend = DrawingBackend::default();
    let mut store = StreamingStore::with_backend(EPOCH, backend.clone()).unwrap();
    let (key, revision) = published(&backend);
    let paged = backend
        .attach_provider(&key, revision, tiles(&backend), request())
        .unwrap();
    for page in paged.pages() {
        page.evict();
    }
    let replacement = Grid {
        count: 123,
        start: 1 << 20,
        page: 2,
    };
    let mut bytes = vec![0; replacement.len()];
    replacement.fill(0, &mut bytes);
    let descriptor = replacement.descriptor();
    let lease = accepted(&mut store, &data_request(data_op::OPEN_WRITER)).lease;
    let begin = DataRequest {
        lease,
        transaction: 1,
        incarnation: revision.incarnation,
        base_generation: revision.generation,
        total_bytes: bytes.len() as u64,
        ..data_request(data_op::BEGIN)
    }
    .with_key(&key)
    .unwrap()
    .with_metadata(&BeginMetadata {
        descriptor: stored(&backend, &key).1.descriptor().clone(),
        mode: CommitMode::Delta,
        scans: 0,
        pages: 1,
        removed_scans: 0,
        removed_pages: 0,
    })
    .unwrap();
    accepted(&mut store, &begin);
    let declare = edit(&begin, data_op::DECLARE_PAGE, descriptor.address)
        .with_metadata(&descriptor)
        .unwrap();
    accepted(&mut store, &declare);
    for (index, part) in bytes.chunks(DATA_CHUNK_BYTES).enumerate() {
        let chunk = DataRequest {
            offset: (index * DATA_CHUNK_BYTES) as u64,
            payload: part.to_vec(),
            ..edit(&begin, data_op::CHUNK, descriptor.address)
        };
        accepted(&mut store, &chunk);
    }
    let reply = accepted(
        &mut store,
        &edit(
            &begin,
            data_op::COMMIT,
            PageAddress {
                scan_key: 0,
                page_id: 0,
            },
        ),
    );
    assert_eq!(reply.stored_generation, revision.generation + 1);

    let (current, storage) = stored(&backend, &key);
    assert_eq!(current.generation, revision.generation + 1);
    let inherited = storage
        .paged()
        .expect("delta keeps provider-backed storage");
    assert!(Arc::ptr_eq(&inherited.pages()[0], &paged.pages()[0]));
    assert!(Arc::ptr_eq(&inherited.pages()[1], &paged.pages()[1]));
    let replaced = &inherited.pages()[2];
    assert!(!Arc::ptr_eq(replaced, &paged.pages()[2]));
    assert_eq!(replaced.metadata().descriptor(), &descriptor);
    assert!(!replaced.evict());
    assert!(inherited.pages()[0].evict());
    assert!(inherited.pages()[1].evict());
    assert_eq!(
        backend.budget().usage().page_bytes,
        replacement.len() as u64
    );
    let loaded = replaced.acquire(request()).unwrap();
    assert_eq!(
        loaded.source_ids().unwrap().next(),
        Some(u64::MAX - (1 << 20))
    );
}

fn survey_dataset(backend: &DrawingBackend, points: u32) -> (DataKey, DatasetRevision) {
    use gearbox_api::draw_data::survey::*;
    let budget = backend.budget();
    let key = DataKey::new("provider-test", "survey").unwrap();
    let descriptor = DataDescriptor {
        revision: SCHEMA_REVISION,
        key: key.clone(),
        shape: DataShape::Cloud {
            schema: survey_schema(),
        },
        stream: StreamMode::RetainedMap,
        lifetime_s: None,
        presentation: Presentation::default(),
    };
    let pages: Vec<_> = (0..survey_pages(points))
        .map(|page| {
            let (descriptor, bytes) = survey_page(points, page).unwrap();
            let mut upload = PageUpload::new(budget, descriptor).unwrap();
            for (index, chunk) in bytes.chunks(DATA_CHUNK_BYTES).enumerate() {
                upload
                    .push((index * DATA_CHUNK_BYTES) as u64, chunk)
                    .unwrap();
            }
            upload.finish_cloud(&survey_schema()).unwrap().into_page()
        })
        .collect();
    let scan = ScanMetadata::new(budget, &survey_scan(points)).unwrap();
    let generation = Generation::build(
        budget,
        DatasetMetadata::new(budget, &descriptor).unwrap(),
        &[scan],
        &pages,
    )
    .unwrap();
    let storage = DatasetStorage::from_resident(budget, generation).unwrap();
    let registry = backend.registry();
    let mut registry = registry.lock().unwrap();
    let permit = registry
        .begin_dataset(&key, DatasetKind::Cloud, None)
        .unwrap();
    (key, registry.commit_dataset(permit, storage).unwrap())
}

fn attachment(lease: u64, key: &DataKey, revision: DatasetRevision, provider: &str) -> DataRequest {
    DataRequest {
        lease,
        incarnation: revision.incarnation,
        base_generation: revision.generation,
        ..data_request(data_op::ATTACH_PROVIDER)
    }
    .with_key(key)
    .unwrap()
    .with_metadata(&ProviderAttachment {
        provider: provider.into(),
    })
    .unwrap()
}

#[test]
fn named_survey_provider_attaches_over_the_protocol_and_reloads_exact_pages() {
    use gearbox_api::draw_data::survey::*;
    let backend = DrawingBackend::default();
    register_host_providers(&backend).unwrap();
    register_host_providers(&backend).unwrap();
    let mut store = StreamingStore::with_backend(EPOCH, backend.clone()).unwrap();
    let capabilities = store.capabilities();
    capabilities.validate().unwrap();
    assert_eq!(
        capabilities.page_providers,
        vec![SURVEY_PROVIDER.to_owned()]
    );
    let points = 3 * SURVEY_PAGE + 77;
    let (key, revision) = survey_dataset(&backend, points);
    let lease = accepted(&mut store, &data_request(data_op::OPEN_WRITER)).lease;

    let unknown = store.handle(&attachment(lease, &key, revision, "fixture/absent"), 0);
    assert_eq!(
        unknown.code,
        data_code::UNSUPPORTED,
        "{}",
        String::from_utf8_lossy(&unknown.message)
    );
    let unleased = store.handle(&attachment(0, &key, revision, SURVEY_PROVIDER), 0);
    assert_eq!(unleased.code, data_code::INVALID);
    let stale = DatasetRevision {
        generation: revision.generation + 1,
        ..revision
    };
    let stale = store.handle(&attachment(lease, &key, stale, SURVEY_PROVIDER), 0);
    assert_eq!(stale.code, data_code::CONFLICT);
    assert!(stored(&backend, &key).1.resident().is_some());

    let scratch = backend.budget().usage().scratch_bytes;
    let reply = accepted(
        &mut store,
        &attachment(lease, &key, revision, SURVEY_PROVIDER),
    );
    assert_eq!(reply.stored_generation, revision.generation);
    let (current, storage) = stored(&backend, &key);
    assert_eq!(current, revision);
    let paged = storage
        .paged()
        .expect("survey attachment publishes paged storage");
    for page in paged.pages() {
        assert!(page.evict());
    }
    assert_eq!(backend.budget().usage().page_bytes, 0);
    for page in [0, 3] {
        let (descriptor, bytes) = survey_page(points, page).unwrap();
        let address = descriptor.address;
        let loaded = storage.page(address).unwrap().acquire(request()).unwrap();
        assert_eq!(loaded.descriptor(), &descriptor);
        assert_eq!(loaded.bytes(), &bytes[..]);
    }
    assert_eq!(backend.budget().usage().scratch_bytes, scratch);
}

#[test]
fn survey_provider_rejects_datasets_it_did_not_generate() {
    use gearbox_api::draw_data::survey::*;
    let backend = DrawingBackend::default();
    register_host_providers(&backend).unwrap();
    let (key, revision) = published(&backend);
    let result = backend.attach_named_provider(&key, revision, SURVEY_PROVIDER, request());
    assert_eq!(result.err().unwrap().code, data_code::INVALID);
    assert!(stored(&backend, &key).1.resident().is_some());
}
