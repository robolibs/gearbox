use std::fmt;
use std::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;

use crate::wire::draw_data::{DATA_CHUNK_BYTES, PageDescriptor};

use super::memory::Buffer;
use super::{
    Charge, CpuBudget, DatasetLifetime, MemoryClass, PageMetadata, StorageError, StoredPage,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderAvailability {
    Available,
    Pending,
    Missing,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProviderError {
    Storage(StorageError),
    Unavailable(ProviderAvailability),
    Busy,
    Cancelled,
    Deadline,
    Changed,
    Incomplete,
}

impl From<StorageError> for ProviderError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(error) => error.fmt(f),
            Self::Unavailable(state) => write!(f, "page provider is {state:?}"),
            Self::Busy => f.write_str("page reload is already in flight"),
            Self::Cancelled => f.write_str("page reload cancelled"),
            Self::Deadline => f.write_str("page reload deadline expired"),
            Self::Changed => f.write_str("page provider returned changed content"),
            Self::Incomplete => f.write_str("page provider returned incomplete content"),
        }
    }
}

impl std::error::Error for ProviderError {}

impl From<ProviderError> for StorageError {
    fn from(error: ProviderError) -> Self {
        match error {
            ProviderError::Storage(error) => error,
            ProviderError::Busy
            | ProviderError::Deadline
            | ProviderError::Unavailable(ProviderAvailability::Pending) => {
                Self::Unavailable(error.to_string())
            }
            _ => Self::Invalid(error.to_string()),
        }
    }
}

#[derive(Clone, Copy)]
pub struct PageRequest<'a> {
    deadline: Instant,
    cancelled: &'a AtomicBool,
    lifetime: Option<&'a DatasetLifetime>,
}

impl<'a> PageRequest<'a> {
    pub fn new(deadline: Instant, cancelled: &'a AtomicBool) -> Self {
        Self {
            deadline,
            cancelled,
            lifetime: None,
        }
    }

    pub fn check(&self) -> Result<(), ProviderError> {
        if self.cancelled.load(Ordering::Acquire)
            || self.lifetime.is_some_and(|lifetime| !lifetime.is_live())
        {
            Err(ProviderError::Cancelled)
        } else if Instant::now() >= self.deadline {
            Err(ProviderError::Deadline)
        } else {
            Ok(())
        }
    }

    pub fn with_lifetime(mut self, lifetime: &'a DatasetLifetime) -> Self {
        self.lifetime = Some(lifetime);
        self
    }
}

pub trait PageProvider: Send + Sync + 'static {
    /// Uncharged owned heap capacity, excluding the provider value itself.
    fn owned_bytes(&self) -> u64;
    fn availability(&self, page: &PageDescriptor) -> ProviderAvailability;
    fn load(&self, sink: &mut PageSink<'_>) -> Result<(), ProviderError>;
}

/// Trusted host code that builds a provider for one dataset generation.
pub trait ProviderFactory: Send + Sync + 'static {
    fn name(&self) -> &str;
    fn create(
        &self,
        budget: &CpuBudget,
        descriptor: &crate::wire::draw_data::DataDescriptor,
        scans: &[Arc<super::ScanMetadata>],
    ) -> Result<Arc<RegisteredProvider>, StorageError>;
}

pub struct RegisteredProvider {
    provider: Box<dyn PageProvider>,
    charge: Charge,
}

impl RegisteredProvider {
    pub fn new<P: PageProvider>(
        budget: &CpuBudget,
        provider: P,
    ) -> Result<Arc<Self>, StorageError> {
        let bytes = (size_of::<Self>() + size_of::<P>() + 2 * size_of::<usize>()) as u64;
        let bytes = bytes
            .checked_add(provider.owned_bytes())
            .ok_or(StorageError::Allocation)?;
        let charge = budget
            .reserve(bytes)?
            .charge(bytes, MemoryClass::Metadata)?;
        Ok(Arc::new(Self {
            provider: Box::new(provider),
            charge,
        }))
    }

    fn load(
        &self,
        budget: &CpuBudget,
        metadata: Arc<PageMetadata>,
        request: PageRequest<'_>,
    ) -> Result<Arc<StoredPage>, ProviderError> {
        request.check()?;
        if !self.charge.belongs_to(budget) || !metadata.belongs_to(budget) {
            return Err(StorageError::Invalid(
                "provider resources must share one CPU budget".into(),
            )
            .into());
        }
        let available = self.provider.availability(metadata.descriptor());
        if available != ProviderAvailability::Available {
            return Err(ProviderError::Unavailable(available));
        }
        let mut sink = PageSink::new(budget, metadata, request)?;
        self.provider.load(&mut sink)?;
        sink.finish()
    }
}

pub struct PageSink<'a> {
    budget: &'a CpuBudget,
    metadata: Arc<PageMetadata>,
    request: PageRequest<'a>,
    bytes: Buffer<u8>,
    written: usize,
    charge: Charge,
}

impl<'a> PageSink<'a> {
    fn new(
        budget: &'a CpuBudget,
        metadata: Arc<PageMetadata>,
        request: PageRequest<'a>,
    ) -> Result<Self, ProviderError> {
        request.check()?;
        let page_bytes = metadata.descriptor().byte_len;
        let owner_bytes = (size_of::<StoredPage>() + 2 * size_of::<usize>()) as u64;
        let mut reservation = budget.reserve(
            page_bytes
                .checked_add(owner_bytes)
                .ok_or(StorageError::Allocation)?,
        )?;
        let count = usize::try_from(page_bytes).map_err(|_| StorageError::Allocation)?;
        let bytes = Buffer::filled(&mut reservation, count, 0, MemoryClass::Page)?;
        let charge = reservation.charge(owner_bytes, MemoryClass::Metadata)?;
        Ok(Self {
            budget,
            metadata,
            request,
            bytes,
            written: 0,
            charge,
        })
    }

    pub fn budget(&self) -> &CpuBudget {
        self.budget
    }

    pub fn descriptor(&self) -> &PageDescriptor {
        self.metadata.descriptor()
    }

    pub fn check(&self) -> Result<(), ProviderError> {
        self.request.check()
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<(), ProviderError> {
        self.check()?;
        let end = self
            .written
            .checked_add(chunk.len())
            .ok_or(StorageError::Allocation)?;
        if chunk.is_empty() || chunk.len() > DATA_CHUNK_BYTES || end > self.bytes.data.len() {
            return Err(StorageError::Invalid(
                "provider chunk exceeds its reserved page bounds".into(),
            )
            .into());
        }
        self.bytes.data[self.written..end].copy_from_slice(chunk);
        self.written = end;
        self.check()
    }

    fn finish(self) -> Result<Arc<StoredPage>, ProviderError> {
        self.check()?;
        if self.written != self.bytes.data.len() {
            return Err(ProviderError::Incomplete);
        }
        if blake3::hash(&self.bytes.data).as_bytes() != self.metadata.digest() {
            return Err(ProviderError::Changed);
        }
        self.check()?;
        Ok(Arc::new(self.metadata.restore(self.bytes, self.charge)))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProviderStatistics {
    pub hits: u64,
    pub misses: u64,
    pub loads: u64,
    pub failures: u64,
    pub evictions: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageState {
    Resident,
    Loading,
    Provider(ProviderAvailability),
    Invalidated,
}

struct State {
    resident: Option<Arc<StoredPage>>,
    loading: bool,
    discard: bool,
    invalidated: bool,
    statistics: ProviderStatistics,
}

pub struct ReloadablePage {
    budget: CpuBudget,
    metadata: Arc<PageMetadata>,
    provider: Option<Arc<RegisteredProvider>>,
    lifetime: Option<DatasetLifetime>,
    state: Mutex<State>,
    _charge: Charge,
}

impl ReloadablePage {
    pub(super) fn bind_lifetime(&mut self, lifetime: Option<DatasetLifetime>) {
        self.lifetime = lifetime;
    }

    pub fn retained(budget: &CpuBudget, page: Arc<StoredPage>) -> Result<Arc<Self>, ProviderError> {
        let metadata = page.metadata(budget)?;
        Self::new(budget, page, metadata, None)
    }

    pub fn verified(
        budget: &CpuBudget,
        page: Arc<StoredPage>,
        provider: Arc<RegisteredProvider>,
        request: PageRequest<'_>,
    ) -> Result<Arc<Self>, ProviderError> {
        Self::verified_live(budget, page, provider, request, None)
    }

    pub(super) fn verified_live(
        budget: &CpuBudget,
        page: Arc<StoredPage>,
        provider: Arc<RegisteredProvider>,
        request: PageRequest<'_>,
        lifetime: Option<DatasetLifetime>,
    ) -> Result<Arc<Self>, ProviderError> {
        let request = lifetime
            .as_ref()
            .map_or(request, |lifetime| request.with_lifetime(lifetime));
        request.check()?;
        let metadata = page.metadata(budget)?;
        drop(provider.load(budget, metadata.clone(), request)?);
        request.check()?;
        let mut result = Self::new(budget, page, metadata, Some(provider))?;
        Arc::get_mut(&mut result).unwrap().lifetime = lifetime;
        Ok(result)
    }

    fn new(
        budget: &CpuBudget,
        page: Arc<StoredPage>,
        metadata: Arc<PageMetadata>,
        provider: Option<Arc<RegisteredProvider>>,
    ) -> Result<Arc<Self>, ProviderError> {
        let bytes = (size_of::<Self>() + 2 * size_of::<usize>()) as u64;
        let charge = budget
            .reserve(bytes)?
            .charge(bytes, MemoryClass::Metadata)?;
        Ok(Arc::new(Self {
            budget: budget.clone(),
            metadata,
            provider,
            lifetime: None,
            state: Mutex::new(State {
                resident: Some(page),
                loading: false,
                discard: false,
                invalidated: false,
                statistics: ProviderStatistics::default(),
            }),
            _charge: charge,
        }))
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn metadata(&self) -> &Arc<PageMetadata> {
        &self.metadata
    }

    pub fn status(&self) -> PageState {
        if self
            .lifetime
            .as_ref()
            .is_some_and(|lifetime| !lifetime.is_live())
        {
            return PageState::Invalidated;
        }
        {
            let state = self.state();
            if state.invalidated {
                return PageState::Invalidated;
            }
            if state.resident.is_some() {
                return PageState::Resident;
            }
            if state.loading {
                return PageState::Loading;
            }
        }
        PageState::Provider(
            self.provider
                .as_ref()
                .map_or(ProviderAvailability::Missing, |p| {
                    p.provider.availability(self.metadata.descriptor())
                }),
        )
    }

    pub fn statistics(&self) -> ProviderStatistics {
        self.state().statistics
    }

    pub fn holds(&self, payload: &Arc<StoredPage>) -> bool {
        self.state()
            .resident
            .as_ref()
            .is_some_and(|page| Arc::ptr_eq(page, payload))
    }

    pub fn acquire(&self, request: PageRequest<'_>) -> Result<Arc<StoredPage>, ProviderError> {
        let request = self
            .lifetime
            .as_ref()
            .map_or(request, |lifetime| request.with_lifetime(lifetime));
        request.check()?;
        {
            let mut state = self.state();
            if state.invalidated {
                return Err(ProviderError::Cancelled);
            }
            if let Some(page) = state.resident.clone() {
                state.statistics.hits = state.statistics.hits.saturating_add(1);
                return Ok(page);
            }
            if state.loading {
                return Err(ProviderError::Busy);
            }
            state.loading = true;
            state.discard = false;
            state.statistics.misses = state.statistics.misses.saturating_add(1);
        }
        let mut loading = Loading {
            page: self,
            succeeded: false,
        };
        let provider = self
            .provider
            .as_ref()
            .ok_or(ProviderError::Unavailable(ProviderAvailability::Missing))?;
        let page = provider.load(&self.budget, self.metadata.clone(), request)?;
        request.check()?;
        {
            let mut state = self.state();
            if state.discard || state.invalidated {
                return Err(ProviderError::Cancelled);
            }
            state.resident = Some(page.clone());
            state.statistics.loads = state.statistics.loads.saturating_add(1);
        }
        loading.succeeded = true;
        Ok(page)
    }

    pub fn evict(&self) -> bool {
        if self.provider.is_none() {
            return false;
        }
        let mut state = self.state();
        state.discard = state.loading;
        let previous = state.resident.take();
        if previous.is_some() {
            state.statistics.evictions = state.statistics.evictions.saturating_add(1);
        }
        drop(state);
        previous.is_some()
    }

    pub fn invalidate(&self) {
        let mut state = self.state();
        state.invalidated = true;
        state.discard = true;
        let previous = state.resident.take();
        drop(state);
        drop(previous);
    }
}

struct Loading<'a> {
    page: &'a ReloadablePage,
    succeeded: bool,
}

impl Drop for Loading<'_> {
    fn drop(&mut self) {
        let mut state = self.page.state();
        state.loading = false;
        if !self.succeeded {
            state.statistics.failures = state.statistics.failures.saturating_add(1);
        }
    }
}
