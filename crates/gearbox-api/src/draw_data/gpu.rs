use std::sync::{Arc, Mutex, MutexGuard};

use super::{Charge, CpuBudget, MemoryClass, StorageError};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuUsage {
    pub bytes: u64,
    pub peak: u64,
    pub uploaded: u64,
    pub records: u64,
    pub uniforms: u64,
    pub staging: u64,
    pub auxiliary: u64,
    pub other: u64,
    pub retiring: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuClass {
    Records,
    Uniforms,
    Staging,
    Auxiliary,
    Other,
}

impl GpuUsage {
    fn category(&mut self, class: GpuClass) -> &mut u64 {
        match class {
            GpuClass::Records => &mut self.records,
            GpuClass::Uniforms => &mut self.uniforms,
            GpuClass::Staging => &mut self.staging,
            GpuClass::Auxiliary => &mut self.auxiliary,
            GpuClass::Other => &mut self.other,
        }
    }
}

struct Ledger {
    limit: u64,
    usage: Mutex<GpuUsage>,
    _metadata: Charge,
}

#[derive(Clone)]
pub struct GpuBudget(Arc<Ledger>);

impl GpuBudget {
    pub(super) fn new(cpu: &CpuBudget, limit: u64) -> Result<Self, StorageError> {
        let bytes = (size_of::<Ledger>() + 2 * size_of::<usize>()) as u64;
        let metadata = cpu.reserve(bytes)?.charge(bytes, MemoryClass::Metadata)?;
        Ok(Self(Arc::new(Ledger {
            limit,
            usage: Mutex::new(GpuUsage::default()),
            _metadata: metadata,
        })))
    }

    fn ledger(&self) -> MutexGuard<'_, GpuUsage> {
        self.0
            .usage
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn usage(&self) -> GpuUsage {
        *self.ledger()
    }

    pub fn limit(&self) -> u64 {
        self.0.limit
    }

    pub fn reserve(&self, bytes: u64) -> Option<GpuCharge> {
        self.reserve_class(bytes, GpuClass::Other)
    }

    pub fn reserve_class(&self, bytes: u64, class: GpuClass) -> Option<GpuCharge> {
        let mut usage = self.ledger();
        let total = usage
            .bytes
            .checked_add(bytes)
            .filter(|total| *total <= self.0.limit)?;
        usage.bytes = total;
        usage.peak = usage.peak.max(total);
        *usage.category(class) += bytes;
        Some(GpuCharge {
            budget: self.clone(),
            bytes,
            class,
            retiring: false,
        })
    }

    pub fn record_upload(&self, bytes: u64) {
        let mut usage = self.ledger();
        usage.uploaded = usage.uploaded.saturating_add(bytes);
    }
}

pub struct GpuCharge {
    budget: GpuBudget,
    bytes: u64,
    class: GpuClass,
    retiring: bool,
}

impl GpuCharge {
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub fn retire(&mut self) {
        if !self.retiring {
            self.budget.ledger().retiring += self.bytes;
            self.retiring = true;
        }
    }
}

impl Drop for GpuCharge {
    fn drop(&mut self) {
        let mut usage = self.budget.ledger();
        usage.bytes -= self.bytes;
        *usage.category(self.class) -= self.bytes;
        if self.retiring {
            usage.retiring -= self.bytes;
        }
    }
}
