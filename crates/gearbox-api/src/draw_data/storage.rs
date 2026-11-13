use std::sync::Arc;

use crate::wire::draw_data::{DataDescriptor, PageAddress, PageDescriptor, Presentation};

use super::{
    Charge, CpuBudget, DatasetMetadata, Generation, GenerationCounts, MemoryClass, PageRequest,
    PagedGeneration, ProviderError, ReloadablePage, ScanMetadata, StorageError, StoredPage,
};

enum Content {
    Resident(Arc<Generation>),
    Paged(Arc<PagedGeneration>),
}

pub struct DatasetStorage {
    content: Content,
    _charge: Charge,
}

#[derive(Clone, Copy)]
pub enum PageSource<'a> {
    Resident(&'a Arc<StoredPage>),
    Paged(&'a Arc<ReloadablePage>),
}

impl PageSource<'_> {
    pub fn descriptor(&self) -> &PageDescriptor {
        match self {
            Self::Resident(page) => page.descriptor(),
            Self::Paged(page) => page.metadata().descriptor(),
        }
    }

    pub fn acquire(&self, request: PageRequest<'_>) -> Result<Arc<StoredPage>, ProviderError> {
        request.check()?;
        match self {
            Self::Resident(page) => Ok((*page).clone()),
            Self::Paged(page) => page.acquire(request),
        }
    }

    /// True when both sources are the same page handle.
    pub fn same(&self, other: &PageSource<'_>) -> bool {
        match (self, other) {
            (Self::Resident(a), PageSource::Resident(b)) => Arc::ptr_eq(a, b),
            (Self::Paged(a), PageSource::Paged(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    /// True when this source currently yields exactly `payload`.
    pub fn holds(&self, payload: &Arc<StoredPage>) -> bool {
        match self {
            Self::Resident(page) => Arc::ptr_eq(page, payload),
            Self::Paged(page) => page.holds(payload),
        }
    }
}

impl DatasetStorage {
    fn new(budget: &CpuBudget, content: Content) -> Result<Arc<Self>, StorageError> {
        let metadata = match &content {
            Content::Resident(generation) => generation.metadata(),
            Content::Paged(generation) => generation.metadata(),
        };
        if !metadata.belongs_to(budget) {
            return Err(StorageError::Invalid(
                "storage budget differs from generation".into(),
            ));
        }
        let bytes = (size_of::<Self>() + 2 * size_of::<usize>()) as u64;
        let charge = budget
            .reserve(bytes)?
            .charge(bytes, MemoryClass::Metadata)?;
        Ok(Arc::new(Self {
            content,
            _charge: charge,
        }))
    }

    pub fn from_resident(
        budget: &CpuBudget,
        generation: Arc<Generation>,
    ) -> Result<Arc<Self>, StorageError> {
        Self::new(budget, Content::Resident(generation))
    }

    pub fn from_paged(
        budget: &CpuBudget,
        generation: Arc<PagedGeneration>,
    ) -> Result<Arc<Self>, StorageError> {
        Self::new(budget, Content::Paged(generation))
    }

    pub fn resident(&self) -> Option<&Arc<Generation>> {
        match &self.content {
            Content::Resident(generation) => Some(generation),
            _ => None,
        }
    }

    pub fn paged(&self) -> Option<&Arc<PagedGeneration>> {
        match &self.content {
            Content::Paged(generation) => Some(generation),
            _ => None,
        }
    }

    pub fn metadata(&self) -> &Arc<DatasetMetadata> {
        match &self.content {
            Content::Resident(generation) => generation.metadata(),
            Content::Paged(generation) => generation.metadata(),
        }
    }

    pub fn descriptor(&self) -> &DataDescriptor {
        self.metadata().value()
    }

    pub fn scans(&self) -> &[Arc<ScanMetadata>] {
        match &self.content {
            Content::Resident(generation) => generation.scans(),
            Content::Paged(generation) => generation.scans(),
        }
    }

    pub fn counts(&self) -> GenerationCounts {
        match &self.content {
            Content::Resident(generation) => generation.counts(),
            Content::Paged(generation) => generation.counts(),
        }
    }

    pub fn scan_high_watermark(&self) -> u64 {
        match &self.content {
            Content::Resident(generation) => generation.scan_high_watermark(),
            Content::Paged(generation) => generation.scan_high_watermark(),
        }
    }

    pub fn retained_at_s(&self) -> f64 {
        match &self.content {
            Content::Resident(generation) => generation.retained_at_s(),
            Content::Paged(generation) => generation.retained_at_s(),
        }
    }

    pub fn accumulation_clock_anchor(&self) -> Option<&ScanMetadata> {
        self.resident()
            .and_then(|generation| generation.accumulation_clock_anchor())
    }

    pub fn page_len(&self) -> usize {
        match &self.content {
            Content::Resident(generation) => generation.pages().len(),
            Content::Paged(generation) => generation.pages().len(),
        }
    }

    pub fn page_at(&self, index: usize) -> Option<PageSource<'_>> {
        match &self.content {
            Content::Resident(generation) => {
                generation.pages().get(index).map(PageSource::Resident)
            }
            Content::Paged(generation) => generation.pages().get(index).map(PageSource::Paged),
        }
    }

    pub fn page(&self, address: PageAddress) -> Option<PageSource<'_>> {
        match &self.content {
            Content::Resident(generation) => generation.page(address).map(PageSource::Resident),
            Content::Paged(generation) => generation.page(address).map(PageSource::Paged),
        }
    }

    pub fn restyle(
        &self,
        budget: &CpuBudget,
        presentation: &Presentation,
    ) -> Result<Arc<Self>, StorageError> {
        match &self.content {
            Content::Resident(generation) => {
                Self::from_resident(budget, generation.restyle(budget, presentation)?)
            }
            Content::Paged(generation) => {
                Self::from_paged(budget, generation.restyle(budget, presentation)?)
            }
        }
    }

    pub(super) fn age_due(&self, now_s: f64) -> bool {
        self.resident()
            .is_some_and(|generation| generation.age_due(now_s))
    }

    pub(super) fn expire_scans(
        &self,
        budget: &CpuBudget,
        now_s: f64,
    ) -> Result<(Arc<Self>, super::generation::RetentionCounts), StorageError> {
        let generation = self.resident().ok_or_else(|| {
            StorageError::Invalid("paged retained maps do not expire scans".into())
        })?;
        let (generation, counts) = generation.expire_scans(budget, now_s)?;
        Ok((Self::from_resident(budget, generation)?, counts))
    }
}
