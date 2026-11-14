use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gearbox_api::draw_data::{
    Charge, CpuBudget, DatasetLifetime, DatasetMetadata, DatasetStorage, DrawingBackend,
    MemoryClass, PageRequest, PageSource, PageState, ScanMetadata, StorageError, StoredPage,
};

const PAGE_LOAD_TIMEOUT: Duration = Duration::from_secs(10);

/// Every page of a resident generation, acquired in order under its lifetime.
pub(super) fn acquire_pages(storage: &DatasetStorage, lifetime: &DatasetLifetime) -> Result<Vec<Arc<StoredPage>>, StorageError> {
    let cancelled = AtomicBool::new(false);
    (0..storage.page_len())
        .map(|index| {
            let source = storage.page_at(index).ok_or_else(|| super::cloud::invalid("page index outside generation"))?;
            let request = PageRequest::new(Instant::now() + PAGE_LOAD_TIMEOUT, &cancelled).with_lifetime(lifetime);
            source.acquire(request).map_err(StorageError::from)
        })
        .collect()
}
/// Threads that prepare one dataset's pages together.
const PREPARE_THREADS: usize = 8;
use gearbox_api::wire::draw_data::{
    DataKey, DataPhase, DataShape, DatasetKind, DatasetRevision, PageAddress, data_code,
};

use super::{
    appearance::{Appearance, AppearanceKey},
    cloud::{Geometry, invalid},
    owned::List,
    retirement::{Live, LiveList, Read, Text},
};

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;

#[cfg(test)]
pub(super) fn test_dataset(budget: &CpuBudget, empty: bool) -> Arc<Dataset> {
    let mut entry = tests::retained(budget);
    if empty {
        let generation = gearbox_api::draw_data::Generation::build(
            budget,
            DatasetMetadata::new(budget, entry.generation.descriptor()).unwrap(),
            &[],
            &[],
        )
        .unwrap();
        entry.generation = DatasetStorage::from_resident(budget, generation).unwrap();
    }
    prepare_dataset(budget, &entry, None, 1, &AtomicU64::new(1)).unwrap()
}

#[cfg(test)]
pub(super) fn test_dataset_points(budget: &CpuBudget, points: &[[f64; 3]]) -> Arc<Dataset> {
    prepare_dataset(
        budget,
        &tests::retained_points(budget, points),
        None,
        1,
        &AtomicU64::new(1),
    )
    .unwrap()
}

#[cfg(test)]
pub(super) fn test_dataset_paged(
    budget: &CpuBudget,
    points: &[[f64; 3]],
    page_size: usize,
) -> Arc<Dataset> {
    prepare_dataset(
        budget,
        &tests::retained_paged(budget, points, page_size),
        None,
        1,
        &AtomicU64::new(1),
    )
    .unwrap()
}

#[cfg(test)]
pub(super) fn test_scene(budget: &CpuBudget, datasets: Vec<Arc<Dataset>>) -> Arc<Scene> {
    let mut owned = List::new(budget, datasets.len(), MemoryClass::Metadata).unwrap();
    for data in datasets {
        owned.push(data).unwrap();
    }
    Arc::new(Scene {
        sequence: 1,
        _charge: charge(budget, size_of::<Scene>() + 16).unwrap(),
        datasets: owned.into(),
        failures: LiveList::default(),
        retryable: false,
    })
}

#[cfg(test)]
pub(super) fn test_snapshot(budget: &CpuBudget, datasets: &[Arc<Dataset>]) -> Arc<Snapshot> {
    let mut entries = List::new(budget, datasets.len(), MemoryClass::Metadata).unwrap();
    for data in datasets {
        entries
            .push(Retained {
                revision: data.revision,
                generation: data.generation.clone(),
                lifetime: data.lifetime.clone(),
            })
            .unwrap();
    }
    Arc::new(Snapshot {
        entries: entries.into(),
        _charge: charge(budget, size_of::<Snapshot>() + 16).unwrap(),
    })
}

/// Pages any view wants at full detail; provider-backed pages outside it are hollow.
pub(super) struct Demand {
    pages: List<super::lod::PageKey>,
    _charge: Charge,
}

impl Demand {
    pub fn new(
        budget: &CpuBudget,
        keys: impl Iterator<Item = super::lod::PageKey> + Clone,
    ) -> Result<Self, StorageError> {
        let mut pages = List::new(budget, keys.clone().count(), MemoryClass::Metadata)?;
        for key in keys {
            pages.push(key)?;
        }
        pages.sort_unstable();
        let mut previous = None;
        pages.retain(|key| {
            let keep = previous != Some(*key);
            previous = Some(*key);
            keep
        });
        Ok(Self {
            pages,
            _charge: charge(budget, size_of::<Self>() + 16)?,
        })
    }

    pub fn contains(&self, incarnation: u64, address: PageAddress) -> bool {
        self.pages
            .binary_search(&super::lod::PageKey {
                incarnation,
                scan: address.scan_key,
                page: address.page_id,
            })
            .is_ok()
    }
}

/// Provider residency of one prepared dataset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Paging {
    pub pages: u32,
    pub resident: u32,
    pub hollow: u32,
    pub unavailable: u32,
    pub misses: u64,
    pub max_load_us: u64,
}

impl From<Paging> for gearbox_api::wire::draw_data::PageResidency {
    fn from(paging: Paging) -> Self {
        Self {
            pages: paging.pages,
            resident: paging.resident,
            evicted: paging.hollow,
            unavailable: paging.unavailable,
            misses: paging.misses,
            max_load_us: paging.max_load_us,
        }
    }
}

pub(super) fn charge(budget: &CpuBudget, bytes: usize) -> Result<Charge, StorageError> {
    budget
        .reserve(bytes as u64)?
        .charge(bytes as u64, MemoryClass::Metadata)
}

pub(super) struct Retained {
    pub revision: DatasetRevision,
    pub generation: Arc<DatasetStorage>,
    pub lifetime: DatasetLifetime,
}

impl Live for Retained {
    fn live(&self) -> bool {
        self.lifetime.is_live()
    }
}

pub(super) struct Snapshot {
    pub entries: LiveList<Retained>,
    _charge: Charge,
}

impl Snapshot {
    pub fn capture(
        backend: &DrawingBackend,
        previous: &mut u64,
    ) -> Result<Option<Arc<Self>>, StorageError> {
        let registry = backend.registry();
        let registry = match registry.try_lock() {
            Ok(registry) => registry,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Ok(None),
        };
        if registry.revision() == *previous {
            return Ok(None);
        }
        let count = registry
            .datasets()
            .filter(|(_, entry)| entry.kind == DatasetKind::Cloud)
            .count();
        let reservation = charge(backend.budget(), size_of::<Self>() + 16)?;
        let mut entries = List::new(backend.budget(), count, MemoryClass::Metadata)?;
        for (_, entry) in registry
            .datasets()
            .filter(|(_, entry)| entry.kind == DatasetKind::Cloud)
        {
            entries.push(Retained {
                revision: entry.revision,
                generation: entry.payload.clone(),
                lifetime: entry.lifetime.clone(),
            })?;
        }
        *previous = registry.revision();
        drop(registry);
        Ok(Some(Arc::new(Self {
            entries: entries.into(),
            _charge: reservation,
        })))
    }

    pub fn same(&self, other: &Self) -> bool {
        if std::ptr::eq(self, other) {
            return true;
        }
        let entries = self.entries.read();
        let other = other.entries.read();
        entries.len() == other.len()
            && entries.capacity() == other.capacity()
            && entries
                .iter()
                .zip(other.iter())
                .all(|(a, b)| a.revision == b.revision && Arc::ptr_eq(&a.generation, &b.generation))
    }
    pub fn contains(&self, key: &DataKey, incarnation: u64) -> bool {
        self.entries.read().iter().any(|entry| {
            entry.lifetime.is_live()
                && entry.generation.descriptor().key == *key
                && entry.revision.incarnation == incarnation
        })
    }
}

pub(super) struct Page {
    pub scan: Arc<ScanMetadata>,
    pub appearance: Arc<Appearance>,
    pub clip: super::clip::Clip,
    _charge: Charge,
}

pub(super) struct Dataset {
    pub revision: DatasetRevision,
    pub generation: Arc<DatasetStorage>,
    pub lifetime: DatasetLifetime,
    pub pages: Vec<Arc<Page>>,
    pub paging: Paging,
    pub source_clocks: super::clock::SourceClocks,
    pub alpha: super::alpha::AlphaStyle,
    _charge: Charge,
}

impl Dataset {
    pub fn needs_frost(&self, samples: u32) -> bool {
        self.generation.descriptor().presentation.blur > 0.0
            && self.pages.iter().any(|page| {
                self.alpha.required(page.appearance.alpha, samples) & super::alpha::TRANSMITTING
                    != 0
            })
    }

    pub fn key(&self) -> &DataKey {
        &self.generation.descriptor().key
    }

    /// True when every page's residency already matches `demand`.
    pub fn settled(&self, demand: Option<&Demand>) -> bool {
        if self.generation.paged().is_none() {
            return true;
        }
        self.paging.unavailable == 0
            && self.pages.iter().all(|page| {
                let address = page.appearance.geometry.descriptor().address;
                page.appearance.geometry.resident()
                    == demand
                        .is_none_or(|demand| demand.contains(self.revision.incarnation, address))
            })
    }

    pub fn render_pages(&self) -> usize {
        self.pages
            .iter()
            .filter(|page| page.appearance.geometry.len() != 0)
            .count()
    }
}

pub(super) struct Scene {
    pub sequence: u64,
    pub datasets: LiveList<Arc<Dataset>>,
    pub failures: LiveList<Failure>,
    retryable: bool,
    _charge: Charge,
}

impl Live for Arc<Dataset> {
    fn live(&self) -> bool {
        self.lifetime.is_live()
    }
}

pub(super) struct Failure {
    metadata: Arc<DatasetMetadata>,
    lifetime: DatasetLifetime,
    pub message: Text,
}

impl Failure {
    pub fn key(&self) -> &DataKey {
        &self.metadata.value().key
    }
}

impl Live for Failure {
    fn live(&self) -> bool {
        self.lifetime.is_live()
    }
}

impl Scene {
    pub fn prune(&self) {
        self.datasets.prune();
        self.failures.prune();
    }
}

struct Mailbox {
    pending: Option<(u64, Arc<Snapshot>)>,
    finished: Option<Arc<Scene>>,
    prune: bool,
    stop: bool,
    demand: Option<Arc<Demand>>,
    repeat: bool,
}

pub(super) struct Worker {
    shared: Arc<(Mutex<Mailbox>, Condvar)>,
    sequence: Arc<AtomicU64>,
    thread: Option<JoinHandle<()>>,
    _charge: Charge,
}

impl Worker {
    pub fn new(backend: DrawingBackend) -> Result<Self, String> {
        let metadata = charge(
            backend.budget(),
            size_of::<Self>() + size_of::<Mailbox>() + 256,
        )
        .map_err(|error| error.to_string())?;
        let shared = Arc::new((
            Mutex::new(Mailbox {
                pending: None,
                finished: None,
                prune: false,
                stop: false,
                demand: None,
                repeat: false,
            }),
            Condvar::new(),
        ));
        let sequence = Arc::new(AtomicU64::new(0));
        let state = shared.clone();
        let serial = sequence.clone();
        let thread = std::thread::Builder::new()
            .name("gearbox-cloud-prepare".into())
            .spawn(move || {
                let mut previous: Option<Arc<Scene>> = None;
                let mut retry: Option<(u64, Arc<Snapshot>, Instant)> = None;
                let mut current: Option<Arc<Snapshot>> = None;
                loop {
                    let (number, snapshot, demand) = {
                        let mut mailbox = state.0.lock().unwrap_or_else(|p| p.into_inner());
                        loop {
                            if mailbox.stop {
                                return;
                            }
                            if mailbox.prune {
                                mailbox.prune = false;
                                let pending = mailbox
                                    .pending
                                    .as_ref()
                                    .map(|(_, snapshot)| snapshot.clone());
                                let finished = mailbox.finished.clone();
                                drop(mailbox);
                                if let Some(snapshot) = pending {
                                    snapshot.entries.prune();
                                }
                                if let Some(scene) = finished {
                                    scene.prune();
                                }
                                if let Some(scene) = &previous {
                                    scene.prune();
                                }
                                if previous.as_ref().is_some_and(|scene| {
                                    scene.datasets.is_empty() && scene.failures.is_empty()
                                }) {
                                    previous = None;
                                }
                                if let Some((_, snapshot, _)) = &retry {
                                    snapshot.entries.prune();
                                }
                                if let Some(snapshot) = &current {
                                    snapshot.entries.prune();
                                }
                                if retry
                                    .as_ref()
                                    .is_some_and(|(_, snapshot, _)| snapshot.entries.is_empty())
                                {
                                    retry = None;
                                }
                                mailbox = state.0.lock().unwrap_or_else(|p| p.into_inner());
                            }
                            // The newest snapshot with the newest demand answers every request so far.
                            if let Some((_, snapshot)) = mailbox.pending.take() {
                                retry = None;
                                mailbox.repeat = false;
                                break (serial.load(Ordering::Acquire), snapshot, mailbox.demand.clone());
                            }
                            if mailbox.repeat
                                && let Some(snapshot) = &current
                            {
                                mailbox.repeat = false;
                                retry = None;
                                break (
                                    serial.load(Ordering::Acquire),
                                    snapshot.clone(),
                                    mailbox.demand.clone(),
                                );
                            }
                            if let Some((_, _, at)) = &retry {
                                let now = Instant::now();
                                if now >= *at {
                                    let (_, snapshot, _) = retry.take().unwrap();
                                    break (
                                        serial.load(Ordering::Acquire),
                                        snapshot,
                                        mailbox.demand.clone(),
                                    );
                                }
                                mailbox = state
                                    .1
                                    .wait_timeout(mailbox, *at - now)
                                    .unwrap_or_else(|p| p.into_inner())
                                    .0;
                            } else {
                                mailbox = state.1.wait(mailbox).unwrap_or_else(|p| p.into_inner());
                            }
                        }
                    };
                    current = Some(snapshot.clone());
                    let result = prepare_scene(
                        &backend,
                        number,
                        &snapshot,
                        previous.as_deref(),
                        &serial,
                        demand.as_deref(),
                    );
                    if serial.load(Ordering::Acquire) != number {
                        continue;
                    }
                    match result {
                        Ok(scene) => {
                            if scene.retryable {
                                retry = Some((
                                    number,
                                    snapshot,
                                    Instant::now() + Duration::from_millis(250),
                                ));
                            }
                            let scene = Arc::new(scene);
                            let mut mailbox = state.0.lock().unwrap_or_else(|p| p.into_inner());
                            mailbox.finished = Some(scene.clone());
                            previous = Some(scene);
                        }
                        Err(error) => {
                            for entry in snapshot.entries.read().iter() {
                                report_failure(&backend, entry, &error);
                            }
                            tracing::warn!("cloud preparation: {error}");
                            if retryable(&error) {
                                retry = Some((
                                    number,
                                    snapshot,
                                    Instant::now() + Duration::from_millis(250),
                                ));
                            }
                        }
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            shared,
            sequence,
            thread: Some(thread),
            _charge: metadata,
        })
    }

    pub fn submit(&self, snapshot: Arc<Snapshot>) -> u64 {
        let number = self.sequence.fetch_add(1, Ordering::AcqRel) + 1;
        let mut mailbox = self.shared.0.lock().unwrap_or_else(|p| p.into_inner());
        mailbox.pending = Some((number, snapshot));
        mailbox.finished = None;
        self.shared.1.notify_one();
        number
    }

    /// Re-prepares the current snapshot for a new demanded page set.
    pub fn demand(&self, demand: Arc<Demand>) -> u64 {
        let number = self.sequence.fetch_add(1, Ordering::AcqRel) + 1;
        let old = {
            let mut mailbox = self.shared.0.lock().unwrap_or_else(|p| p.into_inner());
            mailbox.repeat = true;
            mailbox.finished = None;
            self.shared.1.notify_one();
            mailbox.demand.replace(demand)
        };
        drop(old);
        number
    }

    pub fn take(&self) -> Option<Arc<Scene>> {
        self.shared
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .finished
            .take()
    }

    pub fn prune(&self) {
        let mut mailbox = self.shared.0.lock().unwrap_or_else(|p| p.into_inner());
        if !mailbox.prune {
            mailbox.prune = true;
            self.shared.1.notify_one();
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.sequence.fetch_add(1, Ordering::AcqRel);
        {
            let mut mailbox = self.shared.0.lock().unwrap_or_else(|p| p.into_inner());
            mailbox.stop = true;
            mailbox.pending = None;
            self.shared.1.notify_one();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn prepare_scene(
    backend: &DrawingBackend,
    sequence: u64,
    snapshot: &Snapshot,
    previous: Option<&Scene>,
    serial: &AtomicU64,
    demand: Option<&Demand>,
) -> Result<Scene, StorageError> {
    let budget = backend.budget();
    let metadata = charge(budget, size_of::<Scene>() + 16)?;
    let entries = snapshot.entries.read();
    let mut datasets = List::new(budget, entries.len(), MemoryClass::Metadata)?;
    let mut failures = List::new(budget, entries.len(), MemoryClass::Metadata)?;
    let mut retry = false;
    let old_datasets = previous.map_or_else(Read::default, |scene| scene.datasets.read());
    for entry in entries.iter() {
        if !entry.lifetime.is_live() {
            continue;
        }
        if serial.load(Ordering::Acquire) != sequence {
            return Err(invalid("preparation superseded"));
        }
        let old = old_datasets.iter().find(|data| {
            data.key() == &entry.generation.descriptor().key
                && data.revision.incarnation == entry.revision.incarnation
        });
        let result = if let Some(old) = old.filter(|old| {
            old.revision == entry.revision
                && Arc::ptr_eq(&old.generation, &entry.generation)
                && old.settled(demand)
        }) {
            Ok(old.clone())
        } else {
            prepare_resident(
                budget,
                entry,
                old.map(Arc::as_ref),
                sequence,
                serial,
                demand,
            )
        };
        if let Ok(data) = &result
            && data.generation.paged().is_some()
        {
            retry |= data.paging.unavailable > 0;
            backend.report_paging(data.key(), data.revision, data.paging.into());
        }
        if !entry.lifetime.is_live() {
            continue;
        }
        match result {
            Ok(data) => {
                backend.clear_render_failure(data.key(), data.revision, DataPhase::Preparing);
                datasets.push(data)?;
            }
            Err(error) => {
                if serial.load(Ordering::Acquire) != sequence {
                    return Err(error);
                }
                report_failure(backend, entry, &error);
                retry |= retryable(&error);
                failures.push(Failure {
                    metadata: entry.generation.metadata().clone(),
                    lifetime: entry.lifetime.clone(),
                    message: Text::new(&error),
                })?;
                if let Some(old) = old {
                    datasets.push(old.clone())?;
                }
            }
        }
    }
    Ok(Scene {
        sequence,
        datasets: datasets.into(),
        failures: failures.into(),
        retryable: retry,
        _charge: metadata,
    })
}

fn retryable(error: &StorageError) -> bool {
    matches!(
        error,
        StorageError::Budget { .. } | StorageError::Allocation | StorageError::Unavailable(_)
    )
}

fn report_failure(backend: &DrawingBackend, entry: &Retained, error: &StorageError) {
    let reason = Text::new(error);
    let code = match error {
        StorageError::Budget { .. }
        | StorageError::RetentionBudget { .. }
        | StorageError::Allocation => data_code::BUDGET,
        StorageError::Unavailable(_) => data_code::CONFLICT,
        _ => data_code::INVALID,
    };
    let result =
        gearbox_api::draw_data::RenderDiagnostic::new(DataPhase::Preparing, code, reason.as_str())
            .and_then(|diagnostic| {
                backend.report_renderer_diagnostic(
                    &entry.generation.descriptor().key,
                    entry.revision,
                    diagnostic,
                )
            });
    if let Err(error) = result {
        tracing::warn!("cloud preparation status: {error}");
    }
}

fn prepare_dataset(
    budget: &CpuBudget,
    entry: &Retained,
    previous: Option<&Dataset>,
    sequence: u64,
    serial: &AtomicU64,
) -> Result<Arc<Dataset>, StorageError> {
    prepare_resident(budget, entry, previous, sequence, serial, None)
}

fn hollow_page(budget: &CpuBudget, page: &Page) -> Result<Arc<Page>, StorageError> {
    let geometry = page.appearance.geometry.hollow(budget)?;
    Ok(Arc::new(Page {
        scan: page.scan.clone(),
        appearance: page.appearance.hollow(budget, geometry)?,
        clip: page.clip,
        _charge: charge(budget, size_of::<Page>() + 16)?,
    }))
}

fn reload_page(
    budget: &CpuBudget,
    schema: &gearbox_api::wire::draw_data::CloudSchema,
    payload: Arc<StoredPage>,
    page: &Page,
    presentation: &gearbox_api::wire::draw_data::Presentation,
    check: &impl Fn() -> Result<(), StorageError>,
) -> Result<Arc<Page>, StorageError> {
    let geometry = Geometry::reload(budget, schema, payload, &page.appearance.geometry, check)?;
    let appearance = Appearance::prepare(
        budget,
        schema,
        geometry,
        presentation,
        &page.scan.value().frame,
    )?;
    Ok(Arc::new(Page {
        scan: page.scan.clone(),
        appearance,
        clip: page.clip,
        _charge: charge(budget, size_of::<Page>() + 16)?,
    }))
}

/// Counts residency and releases the provider's cached payload of a hollow page.
fn settle(source: &PageSource<'_>, page: &Page, paging: &mut Paging) {
    if page.appearance.geometry.resident() {
        paging.resident += 1;
    } else {
        paging.hollow += 1;
        if let PageSource::Paged(handle) = source {
            handle.evict();
        }
    }
}

fn prepare_resident(
    budget: &CpuBudget,
    entry: &Retained,
    previous: Option<&Dataset>,
    sequence: u64,
    serial: &AtomicU64,
    demand: Option<&Demand>,
) -> Result<Arc<Dataset>, StorageError> {
    check_preparation(entry, sequence, serial)?;
    let generation = &entry.generation;
    let DataShape::Cloud { schema } = &generation.descriptor().shape else {
        return Err(invalid("not a cloud"));
    };
    let page_count = generation.page_len();
    let metadata = charge(
        budget,
        size_of::<Dataset>() + 16 + page_count * size_of::<Arc<Page>>(),
    )?;
    let mut pages = Vec::new();
    let source_clocks = super::clock::SourceClocks::prepare(budget, generation.clone())?;
    pages
        .try_reserve_exact(page_count)
        .map_err(|_| StorageError::Allocation)?;
    if pages.capacity() != page_count {
        return Err(StorageError::Allocation);
    }
    let paged = generation.paged().is_some();
    let presentation = &generation.descriptor().presentation;
    let check = || check_preparation(entry, sequence, serial);
    let mut paging = Paging::default();
    let lease = |source: PageSource<'_>, paging: &mut Paging| {
        let cancelled = AtomicBool::new(false);
        let request = PageRequest::new(Instant::now() + PAGE_LOAD_TIMEOUT, &cancelled)
            .with_lifetime(&entry.lifetime);
        let miss =
            matches!(source, PageSource::Paged(page) if page.status() != PageState::Resident);
        let started = Instant::now();
        let payload = source.acquire(request).map_err(StorageError::from);
        if miss {
            paging.misses += 1;
            let elapsed = started.elapsed().as_micros() as u64;
            paging.max_load_us = paging.max_load_us.max(elapsed);
        }
        payload
    };
    let prepare_page = |index: usize| -> Result<(Arc<Page>, Paging), StorageError> {
        let mut paging = Paging::default();
            check()?;
            let source = generation
                .page_at(index)
                .ok_or_else(|| invalid("page index outside generation"))?;
            let address = source.descriptor().address;
            let scan = generation
                .scans()
                .binary_search_by_key(&address.scan_key, |scan| scan.value().scan_key)
                .ok()
                .map(|index| &generation.scans()[index])
                .ok_or_else(|| invalid("scan metadata missing"))?;
            let old = previous
                .filter(|old| old.generation.descriptor().shape == generation.descriptor().shape)
                .and_then(|old| {
                    let page = old
                        .pages
                        .binary_search_by_key(&address, |page| {
                            page.appearance.geometry.descriptor().address
                        })
                        .ok()
                        .map(|index| &old.pages[index])?;
                    let reused = page
                        .appearance
                        .geometry
                        .source()
                        .is_some_and(|payload| source.holds(payload))
                        || old
                            .generation
                            .page(address)
                            .is_some_and(|prior| prior.same(&source));
                    reused.then_some(page)
                });
            let resident = !paged
                || demand.is_none_or(|demand| demand.contains(entry.revision.incarnation, address));
            let key = AppearanceKey::new(presentation, &scan.value().frame)?;
            if let Some(old) = old.filter(|old| {
                old.appearance.key == key
                    && Arc::ptr_eq(&old.scan, scan)
                    && old.clip.bounds == presentation.clip
            }) {
                let page = match (old.appearance.geometry.resident(), resident) {
                    (true, false) => hollow_page(budget, old)?,
                    (false, true) => match lease(source, &mut paging).and_then(|payload| {
                        reload_page(budget, schema, payload, old, presentation, &check)
                    }) {
                        Ok(page) => page,
                        Err(_) => {
                            check()?;
                            paging.unavailable += 1;
                            old.clone()
                        }
                    },
                    _ => old.clone(),
                };
                settle(&source, &page, &mut paging);
                return Ok((page, paging));
            }
            let geometry = match old {
                Some(old) if old.appearance.geometry.resident() => old.appearance.geometry.clone(),
                Some(old) => Geometry::reload(
                    budget,
                    schema,
                    lease(source, &mut paging)?,
                    &old.appearance.geometry,
                    &check,
                )?,
                None => {
                    Geometry::prepare_cancellable(budget, schema, lease(source, &mut paging)?, &check)?
                }
            };
            let appearance = if let Some(old) = old.filter(|old| {
                old.appearance.key == key && Arc::ptr_eq(&old.appearance.geometry, &geometry)
            }) {
                old.appearance.clone()
            } else {
                Appearance::prepare(budget, schema, geometry, presentation, &scan.value().frame)?
            };
            let clip = super::clip::Clip::prepare(
                &appearance.geometry,
                &scan.value().frame,
                presentation.clip,
            )?;
            let page = Arc::new(Page {
                scan: scan.clone(),
                appearance,
                clip,
                _charge: charge(budget, size_of::<Page>() + 16)?,
            });
            let page = if resident {
                page
            } else {
                hollow_page(budget, &page)?
            };
            settle(&source, &page, &mut paging);
            Ok((page, paging))
    };
    let workers = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(PREPARE_THREADS)
        .min(page_count);
    let prepared: Vec<Result<(Arc<Page>, Paging), StorageError>> = if workers <= 1 {
        (0..page_count).map(prepare_page).collect()
    } else {
        let next = std::sync::atomic::AtomicUsize::new(0);
        let slots: Vec<std::sync::Mutex<Option<Result<(Arc<Page>, Paging), StorageError>>>> =
            (0..page_count).map(|_| std::sync::Mutex::new(None)).collect();
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= page_count {
                        break;
                    }
                    let result = prepare_page(index);
                    *slots[index].lock().unwrap_or_else(|p| p.into_inner()) = Some(result);
                });
            }
        });
        slots
            .into_iter()
            .map(|slot| {
                slot.into_inner()
                    .unwrap_or_else(|p| p.into_inner())
                    .unwrap_or_else(|| Err(invalid("page preparation did not run")))
            })
            .collect()
    };
    for result in prepared {
        let (page, delta) = result?;
        paging.resident += delta.resident;
        paging.hollow += delta.hollow;
        paging.unavailable += delta.unavailable;
        paging.misses += delta.misses;
        paging.max_load_us = paging.max_load_us.max(delta.max_load_us);
        pages.push(page);
    }
    check()?;
    paging.pages = page_count as u32;
    Ok(Arc::new(Dataset {
        revision: entry.revision,
        generation: generation.clone(),
        lifetime: entry.lifetime.clone(),
        pages,
        paging,
        source_clocks,
        alpha: super::alpha::AlphaStyle::new(&super::style::Style::new(
            &generation.descriptor().presentation,
        )),
        _charge: metadata,
    }))
}

fn check_preparation(
    entry: &Retained,
    sequence: u64,
    serial: &AtomicU64,
) -> Result<(), StorageError> {
    if !entry.lifetime.is_live() {
        Err(invalid("dataset retired"))
    } else if serial.load(Ordering::Acquire) != sequence {
        Err(invalid("preparation superseded"))
    } else {
        Ok(())
    }
}
