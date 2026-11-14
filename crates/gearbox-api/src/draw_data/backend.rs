use std::ops::Deref;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use super::{
    Charge, CpuBudget, DatasetStorage, DrawingStore, GpuBudget, MemoryClass, PageRequest,
    PagedGeneration, ProviderFactory, RegisteredProvider, SharedDrawingStore, TransactionError,
};
use crate::draw::DrawStore;
use crate::wire::{DrawEntity, draw_data::DataLimits};

pub(super) struct Backend {
    pub(super) registry: SharedDrawingStore,
    budget: CpuBudget,
    gpu: GpuBudget,
    pub(super) reports: super::render_reports::Reports,
    pub(super) residencies: super::render_reports::Residencies,
    pub(super) visibility: super::visibility::Visibility,
    limits: DataLimits,
    service_claimed: AtomicBool,
    factories: Mutex<Vec<Arc<dyn ProviderFactory>>>,
    _charge: Charge,
}

pub const MAX_PROVIDER_FACTORIES: usize = 16;

#[derive(Clone)]
pub struct DrawingBackend(pub(super) Arc<Backend>);

impl DrawingBackend {
    pub fn new(limits: DataLimits) -> Result<Self, TransactionError> {
        limits.validate().map_err(|error| {
            TransactionError::new(crate::wire::draw_data::data_code::INVALID, error)
        })?;
        let budget = CpuBudget::new(limits.cpu_bytes);
        let gpu = GpuBudget::new(&budget, limits.gpu_bytes)?;
        let bytes = (size_of::<Backend>()
            + 2 * size_of::<usize>()
            + MAX_PROVIDER_FACTORIES * size_of::<Arc<dyn ProviderFactory>>())
            as u64;
        let charge = budget
            .reserve(bytes)?
            .charge(bytes, MemoryClass::Metadata)?;
        let registry = Arc::new(Mutex::new(DrawingStore::new_charged(
            &budget,
            limits.datasets as usize,
            limits.transactions as usize,
        )?));
        Ok(Self(Arc::new(Backend {
            registry,
            budget,
            gpu,
            reports: Mutex::new(None),
            residencies: Mutex::new(None),
            visibility: Mutex::new(None),
            limits,
            service_claimed: AtomicBool::new(false),
            factories: Mutex::new(Vec::with_capacity(MAX_PROVIDER_FACTORIES)),
            _charge: charge,
        })))
    }

    /// Registers trusted host code that can reload pages for matching datasets.
    pub fn register_provider_factory(
        &self,
        factory: Arc<dyn ProviderFactory>,
    ) -> Result<(), TransactionError> {
        use crate::wire::draw_data::{MAX_PROVIDER_NAME_BYTES, data_code};
        let name = factory.name();
        if name.is_empty() || name.len() > MAX_PROVIDER_NAME_BYTES {
            return Err(TransactionError::new(
                data_code::INVALID,
                "provider name must be 1..64 bytes",
            ));
        }
        let mut factories = self.0.factories.lock().unwrap_or_else(|p| p.into_inner());
        if factories.iter().any(|known| known.name() == name) {
            return Err(TransactionError::new(
                data_code::CONFLICT,
                "provider name is already registered",
            ));
        }
        if factories.len() == MAX_PROVIDER_FACTORIES {
            return Err(TransactionError::new(
                data_code::BUDGET,
                "provider factory limit reached",
            ));
        }
        factories.push(factory);
        Ok(())
    }

    pub fn provider_names(&self) -> Vec<String> {
        self.0
            .factories
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .map(|factory| factory.name().to_owned())
            .collect()
    }

    /// Builds the named provider for the exact generation and attaches it.
    pub fn attach_named_provider(
        &self,
        key: &crate::wire::draw_data::DataKey,
        revision: crate::wire::draw_data::DatasetRevision,
        name: &str,
        request: PageRequest<'_>,
    ) -> Result<Arc<PagedGeneration>, TransactionError> {
        use crate::wire::draw_data::data_code;
        let factory = self
            .0
            .factories
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|factory| factory.name() == name)
            .cloned()
            .ok_or_else(|| {
                TransactionError::new(data_code::UNSUPPORTED, "page provider is not registered")
            })?;
        let source = {
            let registry = self.0.registry.lock().unwrap_or_else(|p| p.into_inner());
            let entry = registry.get_parts(&key.namespace, &key.id).ok_or_else(|| {
                TransactionError::new(data_code::NOT_FOUND, "dataset does not exist")
            })?;
            entry.payload.clone()
        };
        let provider = factory.create(self.budget(), source.descriptor(), source.scans())?;
        drop(source);
        self.attach_provider(key, revision, provider, request)
    }

    pub fn registry(&self) -> SharedDrawingStore {
        self.0.registry.clone()
    }
    pub fn budget(&self) -> &CpuBudget {
        &self.0.budget
    }
    pub fn gpu(&self) -> &GpuBudget {
        &self.0.gpu
    }
    pub fn limits(&self) -> &DataLimits {
        &self.0.limits
    }
    pub fn lock(&self) -> LegacyDrawGuard<'_> {
        LegacyDrawGuard(self.0.registry.lock().unwrap_or_else(|p| p.into_inner()))
    }

    pub fn attach_provider(
        &self,
        key: &crate::wire::draw_data::DataKey,
        revision: crate::wire::draw_data::DatasetRevision,
        provider: Arc<RegisteredProvider>,
        request: PageRequest<'_>,
    ) -> Result<Arc<PagedGeneration>, TransactionError> {
        use crate::wire::draw_data::data_code;
        key.validate()
            .map_err(|error| TransactionError::new(data_code::INVALID, error))?;
        let (source, lifetime) = {
            let registry = self.0.registry.lock().unwrap_or_else(|p| p.into_inner());
            let entry = registry.get_parts(&key.namespace, &key.id).ok_or_else(|| {
                TransactionError::new(data_code::NOT_FOUND, "dataset does not exist")
            })?;
            if entry.revision != revision {
                return Err(TransactionError::new(
                    data_code::CONFLICT,
                    "provider attachment requires an exact generation",
                ));
            }
            (entry.payload.clone(), entry.lifetime.clone())
        };
        let resident = source.resident().ok_or_else(|| {
            TransactionError::new(data_code::CONFLICT, "generation already has page providers")
        })?;
        let paged = PagedGeneration::verified_live(
            self.budget(),
            resident,
            provider,
            request.with_lifetime(&lifetime),
            Some(lifetime.clone()),
        )?;
        let replacement = DatasetStorage::from_paged(self.budget(), paged.clone())?;
        request.with_lifetime(&lifetime).check()?;
        let result = self
            .0
            .registry
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .replace_storage(key, revision, &source, replacement.clone());
        drop(replacement);
        drop(result?);
        Ok(paged)
    }
    pub(super) fn claim(&self) -> Result<ServiceOwner, TransactionError> {
        self.0
            .service_claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| {
                TransactionError::new(
                    crate::wire::draw_data::data_code::CONFLICT,
                    "backend already has a transaction owner",
                )
            })?;
        Ok(ServiceOwner(self.clone()))
    }
}

pub(super) struct ServiceOwner(DrawingBackend);

impl Drop for ServiceOwner {
    fn drop(&mut self) {
        self.0.0.service_claimed.store(false, Ordering::Release);
    }
}

/// `GEARBOX_DRAW_PROFILE=low-memory` selects the low-memory acceptance profile;
/// `GEARBOX_DRAW_GPU_BYTES` then replaces its GPU budget.
impl Default for DrawingBackend {
    fn default() -> Self {
        let mut limits = match std::env::var("GEARBOX_DRAW_PROFILE").as_deref() {
            Ok("low-memory") => DataLimits::low_memory(),
            _ => DataLimits::reference(),
        };
        if let Some(bytes) = std::env::var("GEARBOX_DRAW_GPU_BYTES")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|bytes| *bytes > 0)
        {
            limits.gpu_bytes = bytes;
        }
        Self::new(limits).expect("default drawing backend")
    }
}

impl std::fmt::Debug for DrawingBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DrawingBackend")
            .field("memory", &self.0.budget.usage())
            .finish_non_exhaustive()
    }
}

pub struct LegacyDrawGuard<'a>(MutexGuard<'a, DrawingStore<DatasetStorage>>);

impl Deref for LegacyDrawGuard<'_> {
    type Target = DrawStore;
    fn deref(&self) -> &Self::Target {
        self.0.legacy()
    }
}

impl LegacyDrawGuard<'_> {
    pub fn apply(&mut self, entity: DrawEntity, now_s: f64) -> Result<usize, String> {
        self.0
            .apply_legacy(entity, now_s)
            .map_err(|error| error.to_string())
    }
    pub fn clear(&mut self, namespace: &str) -> usize {
        self.0.clear(namespace).expect("drawing revision exhausted")
    }
    pub fn expire(&mut self, now_s: f64) -> usize {
        self.0
            .expire_legacy(now_s)
            .expect("drawing revision exhausted")
    }
}
