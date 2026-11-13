use std::sync::Mutex;

use super::{
    Charge, CpuBudget, DatasetStorage, DrawingBackend, DrawingStore, MemoryClass, StorageError,
};
use crate::wire::draw_data::{DataFailure, DataKey, DataPhase, DatasetRevision, PageResidency};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Text<const N: usize> {
    bytes: [u8; N],
    len: u16,
}

impl<const N: usize> Text<N> {
    fn new(value: &str) -> Self {
        let mut text = Self {
            bytes: [0; N],
            len: value.len().try_into().expect("bounded text length"),
        };
        text.bytes[..value.len()].copy_from_slice(value.as_bytes());
        text
    }

    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.len)]).expect("copied UTF-8 text")
    }
}

/// A renderer failure with inline, validated UTF-8 text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderDiagnostic {
    phase: DataPhase,
    code: u32,
    reason: Text<512>,
}

impl RenderDiagnostic {
    pub fn new(phase: DataPhase, code: u32, reason: &str) -> Result<Self, StorageError> {
        DataFailure::validate_parts(code, reason).map_err(StorageError::Invalid)?;
        if !matches!(phase, DataPhase::Preparing | DataPhase::Rendering) {
            return Err(StorageError::Invalid(
                "renderer report has a non-render phase".into(),
            ));
        }
        Ok(Self {
            phase,
            code,
            reason: Text::new(reason),
        })
    }

    pub fn phase(&self) -> DataPhase {
        self.phase
    }

    pub fn code(&self) -> u32 {
        self.code
    }

    pub fn reason(&self) -> &str {
        self.reason.as_str()
    }

    fn to_failure(self) -> DataFailure {
        DataFailure {
            phase: self.phase,
            code: self.code,
            reason: self.reason().into(),
        }
    }
}

struct Entry<T> {
    namespace: Text<255>,
    id: Text<127>,
    revision: DatasetRevision,
    value: T,
}

pub(super) struct Book<T> {
    entries: Vec<Entry<T>>,
    limit: usize,
    registry_revision: u64,
    _charge: Charge,
}

impl<T> Book<T> {
    fn new(budget: &CpuBudget, limit: usize) -> Result<Self, StorageError> {
        let bytes = limit
            .checked_mul(size_of::<Entry<T>>())
            .and_then(|bytes| bytes.checked_add(size_of::<Self>()))
            .ok_or(StorageError::Allocation)? as u64;
        let charge = budget
            .reserve(bytes)?
            .charge(bytes, MemoryClass::Metadata)?;
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(limit)
            .map_err(|_| StorageError::Allocation)?;
        if entries.capacity() != limit {
            return Err(StorageError::Allocation);
        }
        Ok(Self {
            entries,
            limit,
            registry_revision: u64::MAX,
            _charge: charge,
        })
    }

    fn index(&self, key: &DataKey) -> Result<usize, usize> {
        self.entries.binary_search_by(|entry| {
            (entry.namespace.as_str(), entry.id.as_str())
                .cmp(&(key.namespace.as_str(), key.id.as_str()))
        })
    }

    fn prune(&mut self, registry: &DrawingStore<DatasetStorage>) {
        if self.registry_revision != registry.revision() {
            self.entries.retain(|report| {
                registry
                    .get_parts(report.namespace.as_str(), report.id.as_str())
                    .is_some_and(|entry| entry.revision == report.revision)
            });
            self.registry_revision = registry.revision();
        }
    }
}

pub(super) type Reports = Mutex<Option<Book<RenderDiagnostic>>>;
pub(super) type Residencies = Mutex<Option<Book<PageResidency>>>;

impl DrawingBackend {
    pub fn enable_renderer_reports(&self) -> Result<(), StorageError> {
        let mut reports = self.0.reports.lock().unwrap_or_else(|p| p.into_inner());
        if reports.is_none() {
            *reports = Some(Book::new(self.budget(), self.limits().datasets as usize)?);
        }
        let mut residencies = self.0.residencies.lock().unwrap_or_else(|p| p.into_inner());
        if residencies.is_none() {
            *residencies = Some(Book::new(self.budget(), self.limits().datasets as usize)?);
        }
        Ok(())
    }

    /// Records provider residency of the exact generation; stale or disabled reports are dropped.
    pub fn report_paging(
        &self,
        key: &DataKey,
        revision: DatasetRevision,
        residency: PageResidency,
    ) {
        if key.validate().is_err() || revision.validate().is_err() || residency.validate().is_err()
        {
            return;
        }
        let registry = self.0.registry.lock().unwrap_or_else(|p| p.into_inner());
        if registry
            .get_parts(&key.namespace, &key.id)
            .is_none_or(|entry| entry.revision != revision)
        {
            return;
        }
        let mut residencies = self.0.residencies.lock().unwrap_or_else(|p| p.into_inner());
        let Some(book) = residencies.as_mut() else {
            return;
        };
        book.prune(&registry);
        let entry = Entry {
            namespace: Text::new(&key.namespace),
            id: Text::new(&key.id),
            revision,
            value: residency,
        };
        match book.index(key) {
            Ok(index) => book.entries[index] = entry,
            Err(index) if book.entries.len() < book.limit => book.entries.insert(index, entry),
            Err(_) => {}
        }
    }

    pub fn page_residency(
        &self,
        key: &DataKey,
        revision: DatasetRevision,
    ) -> Option<PageResidency> {
        let residencies = self.0.residencies.lock().unwrap_or_else(|p| p.into_inner());
        let book = residencies.as_ref()?;
        let entry = &book.entries[book.index(key).ok()?];
        (entry.revision == revision).then_some(entry.value)
    }

    pub fn report_render_failure(
        &self,
        key: &DataKey,
        revision: DatasetRevision,
        failure: &DataFailure,
    ) -> Result<bool, StorageError> {
        key.validate().map_err(StorageError::Invalid)?;
        revision.validate().map_err(StorageError::Invalid)?;
        self.store_renderer_diagnostic(
            key,
            revision,
            RenderDiagnostic::new(failure.phase, failure.code, &failure.reason)?,
        )
    }

    pub fn report_renderer_diagnostic(
        &self,
        key: &DataKey,
        revision: DatasetRevision,
        failure: RenderDiagnostic,
    ) -> Result<bool, StorageError> {
        key.validate().map_err(StorageError::Invalid)?;
        revision.validate().map_err(StorageError::Invalid)?;
        self.store_renderer_diagnostic(key, revision, failure)
    }

    fn store_renderer_diagnostic(
        &self,
        key: &DataKey,
        revision: DatasetRevision,
        failure: RenderDiagnostic,
    ) -> Result<bool, StorageError> {
        let registry = self.0.registry.lock().unwrap_or_else(|p| p.into_inner());
        if registry
            .get_parts(&key.namespace, &key.id)
            .is_none_or(|entry| entry.revision != revision)
        {
            return Ok(false);
        }
        let mut reports = self.0.reports.lock().unwrap_or_else(|p| p.into_inner());
        let Some(book) = reports.as_mut() else {
            return Ok(false);
        };
        book.prune(&registry);
        let entry = Entry {
            namespace: Text::new(&key.namespace),
            id: Text::new(&key.id),
            revision,
            value: failure,
        };
        match book.index(key) {
            Ok(index) => book.entries[index] = entry,
            Err(index) if book.entries.len() < book.limit => book.entries.insert(index, entry),
            Err(_) => return Err(StorageError::Allocation),
        }
        Ok(true)
    }

    pub fn clear_render_failure(&self, key: &DataKey, revision: DatasetRevision, phase: DataPhase) {
        let mut reports = self.0.reports.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(book) = reports.as_mut()
            && let Ok(index) = book.index(key)
            && book.entries[index].revision == revision
            && book.entries[index].value.phase == phase
        {
            book.entries.remove(index);
        }
    }

    pub fn render_failure(&self, key: &DataKey, revision: DatasetRevision) -> Option<DataFailure> {
        self.renderer_diagnostic(key, revision)
            .map(RenderDiagnostic::to_failure)
    }

    pub fn renderer_diagnostic(
        &self,
        key: &DataKey,
        revision: DatasetRevision,
    ) -> Option<RenderDiagnostic> {
        let reports = self.0.reports.lock().unwrap_or_else(|p| p.into_inner());
        let book = reports.as_ref()?;
        let entry = &book.entries[book.index(key).ok()?];
        (entry.revision == revision).then_some(entry.value)
    }

    pub(super) fn prune_renderer_reports(&self) {
        let registry = self.0.registry.lock().unwrap_or_else(|p| p.into_inner());
        let mut reports = self.0.reports.lock().unwrap_or_else(|p| p.into_inner());
        let Some(book) = reports.as_mut() else {
            return;
        };
        book.prune(&registry);
    }
}
