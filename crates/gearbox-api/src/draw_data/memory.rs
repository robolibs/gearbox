use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StorageError {
    Budget { requested: u64, available: u64 },
    RetentionBudget { bytes: u64, limit: u64 },
    Allocation,
    Invalid(String),
    Incomplete,
    Conflict,
    Unavailable(String),
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Budget {
                requested,
                available,
            } => {
                write!(
                    f,
                    "CPU budget: requested {requested} bytes, {available} available"
                )
            }
            Self::Allocation => f.write_str("page allocation failed"),
            Self::RetentionBudget { bytes, limit } => write!(
                f,
                "scan payload {bytes} exceeds accumulation budget {limit}"
            ),
            Self::Invalid(message) => write!(f, "invalid spatial data: {message}"),
            Self::Incomplete => f.write_str("page upload is incomplete"),
            Self::Conflict => {
                f.write_str("chunk overlaps or conflicts with previously accepted bytes")
            }
            Self::Unavailable(reason) => write!(f, "page payload unavailable: {reason}"),
        }
    }
}

impl std::error::Error for StorageError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryClass {
    Page,
    UploadIndex,
    Scratch,
    Metadata,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemoryUsage {
    pub limit: u64,
    pub reserved: u64,
    pub allocated: u64,
    pub peak: u64,
    pub page_bytes: u64,
    pub upload_index_bytes: u64,
    pub scratch_bytes: u64,
    pub metadata_bytes: u64,
}

impl MemoryUsage {
    pub fn committed(&self) -> u64 {
        self.reserved + self.allocated
    }

    fn class_mut(&mut self, class: MemoryClass) -> &mut u64 {
        match class {
            MemoryClass::Page => &mut self.page_bytes,
            MemoryClass::UploadIndex => &mut self.upload_index_bytes,
            MemoryClass::Scratch => &mut self.scratch_bytes,
            MemoryClass::Metadata => &mut self.metadata_bytes,
        }
    }
}

/// Shared owned-allocation accounting, excluding allocator internals and process RSS.
#[derive(Clone, Debug)]
pub struct CpuBudget(Arc<Mutex<MemoryUsage>>);

impl CpuBudget {
    pub fn new(limit: u64) -> Self {
        Self(Arc::new(Mutex::new(MemoryUsage {
            limit,
            ..Default::default()
        })))
    }

    fn ledger(&self) -> MutexGuard<'_, MemoryUsage> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn usage(&self) -> MemoryUsage {
        *self.ledger()
    }

    pub fn reserve(&self, bytes: u64) -> Result<Reservation, StorageError> {
        let mut usage = self.ledger();
        let available = usage.limit - usage.committed();
        if bytes > available {
            return Err(StorageError::Budget {
                requested: bytes,
                available,
            });
        }
        usage.reserved += bytes;
        usage.peak = usage.peak.max(usage.committed());
        Ok(Reservation {
            budget: self.clone(),
            bytes,
        })
    }
}

#[derive(Debug)]
pub struct Reservation {
    budget: CpuBudget,
    bytes: u64,
}

impl Reservation {
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub(super) fn belongs_to(&self, budget: &CpuBudget) -> bool {
        Arc::ptr_eq(&self.budget.0, &budget.0)
    }

    pub fn merge(&mut self, mut other: Self) -> Result<(), StorageError> {
        if !Arc::ptr_eq(&self.budget.0, &other.budget.0) {
            return Err(StorageError::Invalid(
                "cannot merge reservations from different budgets".into(),
            ));
        }
        self.bytes = self
            .bytes
            .checked_add(other.bytes)
            .ok_or(StorageError::Allocation)?;
        other.bytes = 0;
        Ok(())
    }

    pub fn split(&mut self, bytes: u64) -> Result<Self, StorageError> {
        if bytes > self.bytes {
            return Err(StorageError::Budget {
                requested: bytes,
                available: self.bytes,
            });
        }
        self.bytes -= bytes;
        Ok(Self {
            budget: self.budget.clone(),
            bytes,
        })
    }

    pub fn charge(&mut self, bytes: u64, class: MemoryClass) -> Result<Charge, StorageError> {
        if bytes > self.bytes {
            return Err(StorageError::Budget {
                requested: bytes,
                available: self.bytes,
            });
        }
        let mut usage = self.budget.ledger();
        usage.reserved -= bytes;
        usage.allocated += bytes;
        *usage.class_mut(class) += bytes;
        self.bytes -= bytes;
        Ok(Charge {
            budget: self.budget.clone(),
            bytes,
            class,
        })
    }
}

pub(super) fn tree_allowance<K, V>(items: usize) -> u64 {
    (items as u64 + 1) * 16 * (size_of::<K>() + size_of::<V>() + 64) as u64
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.ledger().reserved -= self.bytes;
    }
}

#[derive(Debug)]
pub struct Charge {
    budget: CpuBudget,
    bytes: u64,
    class: MemoryClass,
}

impl Charge {
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub(super) fn belongs_to(&self, budget: &CpuBudget) -> bool {
        Arc::ptr_eq(&self.budget.0, &budget.0)
    }
}

impl Drop for Charge {
    fn drop(&mut self) {
        let mut usage = self.budget.ledger();
        usage.allocated -= self.bytes;
        *usage.class_mut(self.class) -= self.bytes;
    }
}

#[derive(Debug)]
pub(super) struct Buffer<T> {
    pub data: Vec<T>,
    _charge: Charge,
}

impl<T> Buffer<T> {
    pub fn with_capacity(
        reservation: &mut Reservation,
        count: usize,
        class: MemoryClass,
    ) -> Result<Self, StorageError> {
        let bytes = count
            .checked_mul(size_of::<T>())
            .ok_or(StorageError::Allocation)?;
        let charge = reservation.charge(bytes as u64, class)?;
        let mut data = Vec::new();
        data.try_reserve_exact(count)
            .map_err(|_| StorageError::Allocation)?;
        if data.capacity() != count {
            return Err(StorageError::Allocation);
        }
        Ok(Self {
            data,
            _charge: charge,
        })
    }
}

impl<T: Copy> Buffer<T> {
    pub fn filled(
        reservation: &mut Reservation,
        count: usize,
        value: T,
        class: MemoryClass,
    ) -> Result<Self, StorageError> {
        let bytes = count
            .checked_mul(size_of::<T>())
            .ok_or(StorageError::Allocation)?;
        let charge = reservation.charge(bytes as u64, class)?;
        let mut data = Vec::new();
        data.try_reserve_exact(count)
            .map_err(|_| StorageError::Allocation)?;
        if data.capacity() != count {
            return Err(StorageError::Allocation);
        }
        data.resize(count, value);
        Ok(Self {
            data,
            _charge: charge,
        })
    }
}

impl<T: Clone> Buffer<T> {
    pub fn cloned(
        reservation: &mut Reservation,
        source: &[T],
        class: MemoryClass,
    ) -> Result<Self, StorageError> {
        let bytes = source
            .len()
            .checked_mul(size_of::<T>())
            .ok_or(StorageError::Allocation)?;
        let charge = reservation.charge(bytes as u64, class)?;
        let mut data = Vec::new();
        data.try_reserve_exact(source.len())
            .map_err(|_| StorageError::Allocation)?;
        if data.capacity() != source.len() {
            return Err(StorageError::Allocation);
        }
        data.extend_from_slice(source);
        Ok(Self {
            data,
            _charge: charge,
        })
    }
}

impl<T> Buffer<T> {
    pub fn from_iter(
        reservation: &mut Reservation,
        count: usize,
        source: impl IntoIterator<Item = T>,
        class: MemoryClass,
    ) -> Result<Self, StorageError> {
        let bytes = count
            .checked_mul(size_of::<T>())
            .ok_or(StorageError::Allocation)?;
        let charge = reservation.charge(bytes as u64, class)?;
        let mut data = Vec::new();
        data.try_reserve_exact(count)
            .map_err(|_| StorageError::Allocation)?;
        if data.capacity() != count {
            return Err(StorageError::Allocation);
        }
        for value in source {
            if data.len() == count {
                return Err(StorageError::Invalid(
                    "iterator exceeds reserved capacity".into(),
                ));
            }
            data.push(value);
        }
        if data.len() != count {
            return Err(StorageError::Invalid(
                "iterator does not fill reserved capacity".into(),
            ));
        }
        Ok(Self {
            data,
            _charge: charge,
        })
    }
}
