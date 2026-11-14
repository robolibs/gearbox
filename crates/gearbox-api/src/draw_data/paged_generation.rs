use std::sync::Arc;

use crate::wire::draw_data::{DataDescriptor, PageAddress, Presentation, StreamMode};

use super::memory::Buffer;
use super::{
    Charge, CpuBudget, DatasetLifetime, DatasetMetadata, Generation, GenerationCounts, MemoryClass,
    PageRequest, ProviderError, RegisteredProvider, ReloadablePage, ScanMetadata, StorageError,
    StoredPage,
};

pub struct PagedGeneration {
    descriptor: Arc<DatasetMetadata>,
    scans: Buffer<Arc<ScanMetadata>>,
    pages: Buffer<Arc<ReloadablePage>>,
    counts: GenerationCounts,
    scan_high_watermark: u64,
    retained_at_s: f64,
    lifetime: Option<DatasetLifetime>,
    _charge: Charge,
}

impl PagedGeneration {
    pub fn verified(
        budget: &CpuBudget,
        source: &Generation,
        provider: Arc<RegisteredProvider>,
        request: PageRequest<'_>,
    ) -> Result<Arc<Self>, ProviderError> {
        Self::verified_live(budget, source, provider, request, None)
    }

    pub(super) fn verified_live(
        budget: &CpuBudget,
        source: &Generation,
        provider: Arc<RegisteredProvider>,
        request: PageRequest<'_>,
        lifetime: Option<DatasetLifetime>,
    ) -> Result<Arc<Self>, ProviderError> {
        Self::from_source(budget, source, request, lifetime.clone(), |page| {
            ReloadablePage::verified_live(
                budget,
                page.clone(),
                provider.clone(),
                request,
                lifetime.clone(),
            )
        })
    }

    pub(super) fn inherit(
        &self,
        budget: &CpuBudget,
        source: &Generation,
        request: PageRequest<'_>,
    ) -> Result<Arc<Self>, ProviderError> {
        Self::from_source(budget, source, request, self.lifetime.clone(), |page| {
            if let Some(old) = self.page(page.descriptor().address) {
                if old.metadata().descriptor() == page.descriptor()
                    && blake3::hash(page.bytes()).as_bytes() == old.metadata().digest()
                {
                    return Ok(old.clone());
                }
            }
            let mut retained = ReloadablePage::retained(budget, page.clone())?;
            Arc::get_mut(&mut retained)
                .unwrap()
                .bind_lifetime(self.lifetime.clone());
            Ok(retained)
        })
    }

    fn from_source(
        budget: &CpuBudget,
        source: &Generation,
        request: PageRequest<'_>,
        lifetime: Option<DatasetLifetime>,
        mut page_handle: impl FnMut(&Arc<StoredPage>) -> Result<Arc<ReloadablePage>, ProviderError>,
    ) -> Result<Arc<Self>, ProviderError> {
        request.check()?;
        if !matches!(source.descriptor().stream, StreamMode::RetainedMap) {
            return Err(StorageError::Invalid(
                "provider-backed generations require retained-map semantics".into(),
            )
            .into());
        }
        if !source.metadata().belongs_to(budget)
            || source.scans().iter().any(|scan| !scan.belongs_to(budget))
        {
            return Err(StorageError::Invalid(
                "paged generation metadata must share its CPU budget".into(),
            )
            .into());
        }
        let owner_bytes = (size_of::<Self>() + 2 * size_of::<usize>()) as u64;
        let list_bytes = source
            .scans()
            .len()
            .checked_mul(size_of::<Arc<ScanMetadata>>())
            .and_then(|scans| {
                source
                    .pages()
                    .len()
                    .checked_mul(size_of::<Arc<ReloadablePage>>())
                    .and_then(|pages| scans.checked_add(pages))
            })
            .ok_or(StorageError::Allocation)? as u64;
        let mut reservation = budget.reserve(
            owner_bytes
                .checked_add(list_bytes)
                .ok_or(StorageError::Allocation)?,
        )?;
        let charge = reservation.charge(owner_bytes, MemoryClass::Metadata)?;
        let scans = Buffer::cloned(&mut reservation, source.scans(), MemoryClass::Metadata)?;
        let mut pages = Buffer::with_capacity(
            &mut reservation,
            source.pages().len(),
            MemoryClass::Metadata,
        )?;
        for page in source.pages() {
            request.check()?;
            pages.data.push(page_handle(page)?);
        }
        request.check()?;
        Ok(Arc::new(Self {
            descriptor: source.metadata().clone(),
            scans,
            pages,
            counts: source.counts(),
            scan_high_watermark: source.scan_high_watermark(),
            retained_at_s: source.retained_at_s(),
            lifetime,
            _charge: charge,
        }))
    }

    pub fn descriptor(&self) -> &DataDescriptor {
        self.descriptor.value()
    }

    pub fn metadata(&self) -> &Arc<DatasetMetadata> {
        &self.descriptor
    }

    pub fn restyle(
        &self,
        budget: &CpuBudget,
        presentation: &Presentation,
    ) -> Result<Arc<Self>, StorageError> {
        if !self.descriptor.belongs_to(budget) {
            return Err(StorageError::Invalid(
                "presentation budget differs from generation".into(),
            ));
        }
        let bytes = (size_of::<Self>()
            + 2 * size_of::<usize>()
            + self.scans.data.len() * size_of::<Arc<ScanMetadata>>()
            + self.pages.data.len() * size_of::<Arc<ReloadablePage>>()) as u64;
        let mut reservation = budget.reserve(bytes + super::metadata::METADATA_SCRATCH)?;
        let _scratch =
            reservation.charge(super::metadata::METADATA_SCRATCH, MemoryClass::Scratch)?;
        let mut descriptor = self.descriptor().clone();
        descriptor.presentation = presentation.clone();
        descriptor.validate().map_err(StorageError::Invalid)?;
        for scan in self.scans() {
            descriptor
                .validate_scan(scan.value())
                .map_err(StorageError::Invalid)?;
        }
        let descriptor = DatasetMetadata::new(budget, &descriptor)?;
        let scans = Buffer::cloned(&mut reservation, self.scans(), MemoryClass::Metadata)?;
        let pages = Buffer::cloned(&mut reservation, self.pages(), MemoryClass::Metadata)?;
        let charge = reservation.charge(reservation.bytes(), MemoryClass::Metadata)?;
        Ok(Arc::new(Self {
            descriptor,
            scans,
            pages,
            counts: self.counts,
            scan_high_watermark: self.scan_high_watermark,
            retained_at_s: self.retained_at_s,
            lifetime: self.lifetime.clone(),
            _charge: charge,
        }))
    }

    pub fn scans(&self) -> &[Arc<ScanMetadata>] {
        &self.scans.data
    }

    pub fn pages(&self) -> &[Arc<ReloadablePage>] {
        &self.pages.data
    }

    pub fn page(&self, address: PageAddress) -> Option<&Arc<ReloadablePage>> {
        self.pages
            .data
            .binary_search_by_key(&address, |page| page.metadata().descriptor().address)
            .ok()
            .map(|index| &self.pages.data[index])
    }

    pub fn counts(&self) -> GenerationCounts {
        self.counts
    }

    pub fn scan_high_watermark(&self) -> u64 {
        self.scan_high_watermark
    }

    pub fn retained_at_s(&self) -> f64 {
        self.retained_at_s
    }

    pub fn invalidate(&self) {
        for page in &self.pages.data {
            page.invalidate();
        }
    }
}
