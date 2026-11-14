use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use super::memory::{Buffer, tree_allowance};
use super::{
    Charge, ChunkOutcome, CpuBudget, DatasetLifetime, DatasetMetadata, DatasetStorage,
    DrawingStore, Generation, IdentityError, MemoryClass, PageFootprint, PageUpload, Reservation,
    ScanMetadata, StagedPage, StorageError, StoredPage, WritePermit,
};
use crate::wire::draw_data::*;

pub type SharedDrawingStore = Arc<Mutex<DrawingStore<DatasetStorage>>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransactionError {
    pub code: u32,
    pub message: String,
}

impl TransactionError {
    pub(super) fn new(code: u32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for TransactionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TransactionError {}

impl From<StorageError> for TransactionError {
    fn from(error: StorageError) -> Self {
        let code = match error {
            StorageError::Budget { .. }
            | StorageError::RetentionBudget { .. }
            | StorageError::Allocation => data_code::BUDGET,
            StorageError::Conflict | StorageError::Unavailable(_) => data_code::CONFLICT,
            _ => data_code::INVALID,
        };
        Self::new(code, error.to_string())
    }
}

impl From<IdentityError> for TransactionError {
    fn from(error: IdentityError) -> Self {
        let code = match error {
            IdentityError::KindConflict | IdentityError::GenerationConflict => data_code::CONFLICT,
            IdentityError::Cancelled => data_code::EXPIRED,
            IdentityError::Budget => data_code::BUDGET,
            IdentityError::Invalid(_) => data_code::INVALID,
            _ => data_code::INTERNAL,
        };
        Self::new(code, error.to_string())
    }
}

impl From<super::ProviderError> for TransactionError {
    fn from(error: super::ProviderError) -> Self {
        use super::ProviderError;
        let code = match &error {
            ProviderError::Storage(error) => return error.clone().into(),
            ProviderError::Cancelled | ProviderError::Deadline => data_code::EXPIRED,
            ProviderError::Busy => data_code::CONFLICT,
            ProviderError::Unavailable(_) => data_code::NOT_FOUND,
            ProviderError::Changed | ProviderError::Incomplete => data_code::INVALID,
        };
        Self::new(code, error.to_string())
    }
}

pub(super) struct BeginRecord {
    pub descriptor: Arc<DatasetMetadata>,
    pub mode: CommitMode,
    pub scans: u32,
    pub pages: u32,
    pub removed_scans: u32,
    pub removed_pages: u32,
    pub total_bytes: u64,
    pub base: Option<DatasetRevision>,
    raw_metadata: Buffer<u8>,
    _charge: Charge,
}

impl BeginRecord {
    pub fn new(
        budget: &CpuBudget,
        request: &DataRequest,
        begin: &BeginMetadata,
    ) -> Result<Arc<Self>, TransactionError> {
        let descriptor = DatasetMetadata::new(budget, &begin.descriptor)?;
        let bytes = (size_of::<Self>() + 2 * size_of::<usize>() + request.metadata.len()) as u64;
        let mut reservation = budget.reserve(bytes)?;
        let raw_metadata =
            Buffer::cloned(&mut reservation, &request.metadata, MemoryClass::Metadata)?;
        let charge = reservation.charge(reservation.bytes(), MemoryClass::Metadata)?;
        Ok(Arc::new(Self {
            descriptor,
            mode: begin.mode,
            scans: begin.scans,
            pages: begin.pages,
            removed_scans: begin.removed_scans,
            removed_pages: begin.removed_pages,
            total_bytes: request.total_bytes,
            base: request_base(request),
            raw_metadata,
            _charge: charge,
        }))
    }

    pub fn key(&self) -> &DataKey {
        &self.descriptor.value().key
    }
    pub fn matches(&self, request: &DataRequest) -> bool {
        request.total_bytes == self.total_bytes
            && request_base(request) == self.base
            && request.metadata == self.raw_metadata.data
    }
    pub fn address_matches(&self, request: &DataRequest) -> bool {
        request_base(request) == self.base && request.data_key().as_ref() == Ok(self.key())
    }
}

pub(super) fn request_base(request: &DataRequest) -> Option<DatasetRevision> {
    (request.incarnation != 0).then_some(DatasetRevision {
        incarnation: request.incarnation,
        generation: request.base_generation,
    })
}

pub(super) struct Publication {
    registry: SharedDrawingStore,
    permit: Option<WritePermit>,
}

impl Publication {
    pub(super) fn new(registry: SharedDrawingStore, permit: WritePermit) -> Self {
        Self {
            registry,
            permit: Some(permit),
        }
    }
    fn live(&self) -> bool {
        self.permit.is_some_and(|permit| {
            self.registry
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .permit_is_live(permit)
        })
    }
    pub(super) fn publish(
        &mut self,
        generation: Arc<DatasetStorage>,
    ) -> Result<DatasetRevision, TransactionError> {
        let permit = self
            .permit
            .take()
            .ok_or_else(|| TransactionError::new(data_code::EXPIRED, "publication revoked"))?;
        let result = self
            .registry
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .commit_dataset(permit, generation.clone());
        drop(generation);
        Ok(result?)
    }
}

impl Drop for Publication {
    fn drop(&mut self) {
        if let Some(permit) = self.permit.take() {
            self.registry
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .cancel(permit);
        }
    }
}

enum PageState {
    Upload(PageUpload),
    Staged(StagedPage),
}

impl PageState {
    fn descriptor(&self) -> &PageDescriptor {
        match self {
            Self::Upload(page) => page.descriptor(),
            Self::Staged(page) => page.page().descriptor(),
        }
    }
}

pub(super) struct Transaction {
    pub record: Arc<BeginRecord>,
    pub last_activity: u64,
    pub phase: DataPhase,
    publication: Publication,
    base: Option<Arc<DatasetStorage>>,
    lifetime: Option<DatasetLifetime>,
    pages: BTreeMap<PageAddress, PageState>,
    scans: BTreeMap<u64, Arc<ScanMetadata>>,
    removed_pages: BTreeSet<PageAddress>,
    removed_scans: BTreeSet<u64>,
    declared_bytes: u64,
    pub received_bytes: u64,
    pub incomplete_bytes: u64,
    pub staged_bytes: u64,
    budget: CpuBudget,
    payload: Reservation,
    construction: Reservation,
    merge_allowance: u64,
    _tables: Charge,
}

impl Transaction {
    pub fn new(
        registry: SharedDrawingStore,
        budget: &CpuBudget,
        limits: &DataLimits,
        record: Arc<BeginRecord>,
        now: u64,
    ) -> Result<Self, TransactionError> {
        if record.scans > limits.scans
            || record.removed_scans > limits.scans
            || record.pages > limits.pages
            || record.removed_pages > limits.pages
            || record.total_bytes > limits.writer_staged_bytes
        {
            return Err(TransactionError::new(
                data_code::BUDGET,
                "transaction declarations exceed host limits",
            ));
        }
        let descriptor = record.descriptor.value();
        let kind = descriptor.shape.kind();
        let (permit, base, lifetime) = {
            let mut store = registry.lock().unwrap_or_else(|p| p.into_inner());
            let permit = store.begin_dataset(record.key(), kind, record.base)?;
            let key = (record.key().namespace.clone(), record.key().id.clone());
            (
                permit,
                store.get(&key).map(|entry| entry.payload.clone()),
                store.get(&key).map(|entry| entry.lifetime.clone()),
            )
        };
        let publication = Publication {
            registry,
            permit: Some(permit),
        };
        if record.mode == CommitMode::Delta
            && base.as_ref().is_none_or(|base| {
                base.descriptor().shape != descriptor.shape
                    || base.descriptor().stream != descriptor.stream
            })
        {
            return Err(invalid(
                "delta requires an existing generation with the same geometry schema and placement",
            ));
        }
        let inherited = if record.mode == CommitMode::Delta {
            base.as_deref()
        } else {
            None
        };
        let base_counts = inherited.map_or(Default::default(), DatasetStorage::counts);
        let multiple = if matches!(descriptor.stream, StreamMode::Accumulate { .. }) {
            2
        } else {
            1
        };
        let pages = (base_counts.pages + record.pages).min(multiple * limits.pages) as usize;
        let scans = (base_counts.scans + record.scans).min(multiple * limits.scans) as usize;
        let stride = match &descriptor.shape {
            DataShape::Cloud { schema } | DataShape::Surfels { schema, .. } => schema
                .attributes
                .iter()
                .map(|field| field.scalar.size() * usize::from(field.components))
                .sum::<usize>(),
            DataShape::Octree { .. } => 48,
            DataShape::MeshBlocks { .. } => 12,
            DataShape::Volume { .. } | DataShape::Elevation { .. } => 4,
        };
        let new_samples = (record.total_bytes / stride as u64)
            .min(u64::from(record.pages) * u64::from(MAX_PAGE_POINTS));
        let generation_bytes = Generation::reservation_bytes(
            scans,
            pages,
            base_counts.source_samples + new_samples,
            &descriptor.shape,
        )?;
        let merge_allowance = tree_allowance::<PageAddress, Arc<StoredPage>>(pages)
            + tree_allowance::<u64, Arc<ScanMetadata>>(scans);
        let vectors =
            (pages * size_of::<Arc<StoredPage>>() + scans * size_of::<Arc<ScanMetadata>>()) as u64;
        let construction = budget.reserve(generation_bytes + merge_allowance + vectors)?;
        let payload = budget.reserve(record.total_bytes)?;
        let table_bytes = size_of::<Self>() as u64
            + tree_allowance::<PageAddress, PageState>(record.pages as usize)
            + tree_allowance::<u64, Arc<ScanMetadata>>(record.scans as usize)
            + tree_allowance::<PageAddress, ()>(record.removed_pages as usize)
            + tree_allowance::<u64, ()>(record.removed_scans as usize);
        let tables = budget
            .reserve(table_bytes)?
            .charge(table_bytes, MemoryClass::Metadata)?;
        Ok(Self {
            record,
            last_activity: now,
            phase: DataPhase::Receiving,
            publication,
            base,
            lifetime,
            pages: BTreeMap::new(),
            scans: BTreeMap::new(),
            removed_pages: BTreeSet::new(),
            removed_scans: BTreeSet::new(),
            declared_bytes: 0,
            received_bytes: 0,
            incomplete_bytes: 0,
            staged_bytes: 0,
            budget: budget.clone(),
            payload,
            construction,
            merge_allowance,
            _tables: tables,
        })
    }

    pub fn live(&self) -> bool {
        self.publication.live()
    }

    pub fn progress(&self) -> TransferProgress {
        TransferProgress {
            transaction: 1,
            phase: self.phase,
            received_bytes: self.received_bytes,
            expected_bytes: self.record.total_bytes,
            staged_bytes: self.staged_bytes,
            declared_pages: self.record.pages,
            complete_pages: self
                .pages
                .values()
                .filter(|page| matches!(page, PageState::Staged(_)))
                .count() as u32,
        }
    }

    pub fn declare_scan(&mut self, scan: &ScanDescriptor) -> Result<(), TransactionError> {
        if let Some(previous) = self.scans.get(&scan.scan_key) {
            return if previous.value() == scan {
                Ok(())
            } else {
                Err(conflict("conflicting scan declaration"))
            };
        }
        if self.scans.len() >= self.record.scans as usize
            || self.removed_scans.contains(&scan.scan_key)
        {
            return Err(invalid(
                "scan declaration exceeds count or conflicts with removal",
            ));
        }
        self.record
            .descriptor
            .value()
            .validate_scan(scan)
            .map_err(|error| invalid(&error))?;
        let previous = self.base.as_ref().and_then(|base| {
            base.scans()
                .iter()
                .find(|old| old.value().scan_key == scan.scan_key)
        });
        if scan.scan_key != 0 {
            if matches!(self.record.descriptor.value().stream, StreamMode::Latest)
                && self
                    .base
                    .as_ref()
                    .is_some_and(|base| scan.scan_key <= base.scan_high_watermark())
            {
                return Err(conflict("latest-frame replacement requires a new scan key"));
            }
            if let Some(previous) = previous {
                if previous.value() != scan {
                    return Err(conflict("acquired scan metadata is immutable"));
                }
            } else if self
                .base
                .as_ref()
                .is_some_and(|base| scan.scan_key <= base.scan_high_watermark())
            {
                return Err(conflict(
                    "acquired scan key does not exceed the incarnation high-water mark",
                ));
            }
        }
        let record = ScanMetadata::new(&self.budget, scan)?;
        self.scans.insert(scan.scan_key, record);
        Ok(())
    }

    pub fn page_declaration(&self, descriptor: &PageDescriptor) -> Result<bool, TransactionError> {
        if let Some(previous) = self.pages.get(&descriptor.address) {
            return if previous.descriptor() == descriptor {
                Ok(false)
            } else {
                Err(conflict("conflicting page declaration"))
            };
        }
        if self.pages.len() >= self.record.pages as usize
            || self.removed_pages.contains(&descriptor.address)
            || self.removed_scans.contains(&descriptor.address.scan_key)
        {
            return Err(invalid(
                "page declaration exceeds count or conflicts with removal",
            ));
        }
        if descriptor.byte_len > self.record.total_bytes - self.declared_bytes {
            return Err(invalid("page declarations exceed transaction byte count"));
        }
        let expected = self.record.descriptor.value().shape.encoding();
        if descriptor.encoding != expected {
            return Err(invalid("page kind differs from transaction kind"));
        }
        Ok(true)
    }

    pub fn declare_page(&mut self, descriptor: PageDescriptor) -> Result<(), TransactionError> {
        if !self.page_declaration(&descriptor)? {
            return Ok(());
        }
        let footprint = PageFootprint::for_descriptor(&descriptor)?;
        let overhead = self
            .budget
            .reserve(footprint.total() - descriptor.byte_len)?;
        let mut reservation = self.payload.split(descriptor.byte_len)?;
        reservation.merge(overhead)?;
        let page = PageUpload::from_reservation(descriptor.clone(), reservation)?;
        self.declared_bytes += descriptor.byte_len;
        self.incomplete_bytes += descriptor.byte_len;
        self.pages
            .insert(descriptor.address, PageState::Upload(page));
        Ok(())
    }

    pub fn chunk(
        &mut self,
        address: PageAddress,
        offset: u64,
        bytes: &[u8],
    ) -> Result<(), TransactionError> {
        let page = self
            .pages
            .get_mut(&address)
            .ok_or_else(|| invalid("chunk references an undeclared page"))?;
        match page {
            PageState::Staged(page) => {
                page.retry(offset, bytes)?;
                return Ok(());
            }
            PageState::Upload(page) => {
                if page.push(offset, bytes)? == ChunkOutcome::Accepted {
                    self.received_bytes += bytes.len() as u64;
                }
                if !page.is_complete() {
                    return Ok(());
                }
            }
        }
        let Some(PageState::Upload(page)) = self.pages.remove(&address) else {
            unreachable!()
        };
        let size = page.descriptor().byte_len;
        self.phase = DataPhase::Validating;
        #[cfg(test)]
        super::concurrency_tests::checkpoint("page_validation");
        let staged = match &self.record.descriptor.value().shape {
            DataShape::Cloud { schema } | DataShape::Surfels { schema, .. } => page.finish_cloud(schema)?,
            DataShape::Octree { config, .. } => page.finish_occupancy(config)?,
            DataShape::MeshBlocks { .. } => page.finish_mesh()?,
            DataShape::Volume { field, .. } => page.finish_volume(field)?,
            DataShape::Elevation { grid, .. } => page.finish_elevation(grid)?,
        };
        self.incomplete_bytes -= size;
        self.staged_bytes += size;
        self.pages.insert(address, PageState::Staged(staged));
        self.phase = DataPhase::Receiving;
        Ok(())
    }

    pub fn remove_page(&mut self, address: PageAddress) -> Result<(), TransactionError> {
        if self.removed_pages.contains(&address) {
            return Ok(());
        }
        if self.record.mode != CommitMode::Delta
            || self.removed_pages.len() >= self.record.removed_pages as usize
            || self.pages.contains_key(&address)
            || self.removed_scans.contains(&address.scan_key)
            || self
                .base
                .as_ref()
                .and_then(|base| base.page(address))
                .is_none()
        {
            return Err(invalid(
                "page removal is undeclared, missing or conflicts with an upsert",
            ));
        }
        self.removed_pages.insert(address);
        Ok(())
    }

    pub fn remove_scan(&mut self, key: u64) -> Result<(), TransactionError> {
        if self.removed_scans.contains(&key) {
            return Ok(());
        }
        if self.record.mode != CommitMode::Delta
            || self.removed_scans.len() >= self.record.removed_scans as usize
            || self.scans.contains_key(&key)
            || self.pages.keys().any(|address| address.scan_key == key)
            || self
                .removed_pages
                .iter()
                .any(|address| address.scan_key == key)
            || self
                .base
                .as_ref()
                .is_none_or(|base| !base.scans().iter().any(|scan| scan.value().scan_key == key))
        {
            return Err(invalid(
                "scan removal is undeclared, missing or conflicts with another edit",
            ));
        }
        self.removed_scans.insert(key);
        Ok(())
    }

    pub fn commit(
        &mut self,
        limits: &DataLimits,
        now_s: f64,
    ) -> Result<(DatasetRevision, super::generation::RetentionCounts), TransactionError> {
        self.phase = DataPhase::Validating;
        if !self.live() {
            return Err(TransactionError::new(
                data_code::EXPIRED,
                "dataset publication was revoked",
            ));
        }
        if self.pages.len() != self.record.pages as usize
            || self.scans.len() != self.record.scans as usize
            || self.removed_pages.len() != self.record.removed_pages as usize
            || self.removed_scans.len() != self.record.removed_scans as usize
            || self.declared_bytes != self.record.total_bytes
            || self.staged_bytes != self.record.total_bytes
        {
            return Err(invalid(
                "transaction has missing declarations or incomplete pages",
            ));
        }
        let _merge = self
            .construction
            .charge(self.merge_allowance, MemoryClass::Scratch)?;
        let mut pages = BTreeMap::new();
        let mut scans = BTreeMap::new();
        if self.record.mode == CommitMode::Delta {
            let base = self
                .base
                .as_ref()
                .ok_or_else(|| invalid("delta base is missing"))?;
            let cancelled = std::sync::atomic::AtomicBool::new(false);
            let request = super::PageRequest::new(
                std::time::Instant::now() + std::time::Duration::from_secs(30),
                &cancelled,
            );
            let request = self
                .lifetime
                .as_ref()
                .map_or(request, |lifetime| request.with_lifetime(lifetime));
            for index in 0..base.counts().pages as usize {
                let source = base.page_at(index).expect("generation page count");
                let address = source.descriptor().address;
                if self.removed_pages.contains(&address)
                    || self.removed_scans.contains(&address.scan_key)
                    || self.pages.contains_key(&address)
                {
                    continue;
                }
                if !self.live() {
                    return Err(TransactionError::new(
                        data_code::EXPIRED,
                        "dataset publication was revoked",
                    ));
                }
                pages.insert(address, source.acquire(request)?);
            }
            scans.extend(
                base.scans()
                    .iter()
                    .map(|scan| (scan.value().scan_key, scan.clone())),
            );
        }
        for key in &self.removed_scans {
            scans.remove(key);
            pages.retain(|address, _| address.scan_key != *key);
        }
        for address in &self.removed_pages {
            pages.remove(address);
        }
        let accumulation = matches!(
            self.record.descriptor.value().stream,
            StreamMode::Accumulate { .. }
        );
        let multiple = if accumulation { 2 } else { 1 };
        for (key, scan) in &self.scans {
            if scans.len() == multiple * limits.scans as usize && !scans.contains_key(key) {
                return Err(invalid("merged scan table exceeds host limit"));
            }
            scans.insert(*key, scan.clone());
        }
        for (address, page) in &self.pages {
            if pages.len() == multiple * limits.pages as usize && !pages.contains_key(address) {
                return Err(invalid("merged page table exceeds host limit"));
            }
            let PageState::Staged(page) = page else {
                return Err(invalid("page is not staged"));
            };
            pages.insert(*address, page.shared_page());
        }
        let pages = Buffer::from_iter(
            &mut self.construction,
            pages.len(),
            pages.values().cloned(),
            MemoryClass::Scratch,
        )?;
        let scans = Buffer::from_iter(
            &mut self.construction,
            scans.len(),
            scans.values().cloned(),
            MemoryClass::Scratch,
        )?;
        let high_watermark = self
            .base
            .as_ref()
            .map_or(0, |base| base.scan_high_watermark())
            .max(self.scans.keys().next_back().copied().unwrap_or(0));
        let samples = pages
            .data
            .iter()
            .map(|page| u64::from(page.descriptor().items))
            .sum();
        let bytes = Generation::reservation_bytes(
            scans.data.len(),
            pages.data.len(),
            samples,
            &self.record.descriptor.value().shape,
        )?;
        let delta = self.record.mode == CommitMode::Delta;
        let fresh_count = if delta { self.pages.len() } else { 0 };
        let mut fresh_reservation = self
            .budget
            .reserve((fresh_count * size_of::<PageAddress>()) as u64)?;
        let fresh = Buffer::from_iter(
            &mut fresh_reservation,
            fresh_count,
            self.pages.keys().copied().take(fresh_count),
            MemoryClass::Scratch,
        )?;
        let mut generation = Generation::from_reservation(
            &self.budget,
            self.record.descriptor.clone(),
            &scans.data,
            &pages.data,
            high_watermark,
            accumulation,
            delta.then_some(fresh.data.as_slice()),
            self.construction.split(bytes)?,
        )?;
        drop(fresh);
        let mutable = Arc::get_mut(&mut generation).unwrap();
        let retention = mutable.apply_retention(
            self.base
                .as_ref()
                .and_then(|base| base.resident())
                .map(Arc::as_ref),
            now_s,
        )?;
        if mutable.counts().scans > limits.scans || mutable.counts().pages > limits.pages {
            return Err(TransactionError::new(
                data_code::BUDGET,
                "retained generation exceeds host scan/page limits",
            ));
        }
        mutable.stamp_retention(now_s);
        let storage = if self.record.mode == CommitMode::Delta {
            if let Some(paged) = self.base.as_ref().and_then(|base| base.paged()) {
                let cancelled = std::sync::atomic::AtomicBool::new(false);
                let request = super::PageRequest::new(
                    std::time::Instant::now() + std::time::Duration::from_secs(30),
                    &cancelled,
                );
                DatasetStorage::from_paged(
                    &self.budget,
                    paged.inherit(&self.budget, &generation, request)?,
                )?
            } else {
                DatasetStorage::from_resident(&self.budget, generation)?
            }
        } else {
            DatasetStorage::from_resident(&self.budget, generation)?
        };
        self.phase = DataPhase::Retaining;
        Ok((self.publication.publish(storage)?, retention))
    }
}

fn invalid(message: &str) -> TransactionError {
    TransactionError::new(data_code::INVALID, message)
}
fn conflict(message: &str) -> TransactionError {
    TransactionError::new(data_code::CONFLICT, message)
}
