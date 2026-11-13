use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::draw::{DrawKey, DrawStore};
use crate::wire::draw::ns_within;
use crate::wire::draw_data::{DataKey, DatasetKind, DatasetRevision};
use crate::wire::{DrawEntity, draw_op};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentityError {
    Invalid(String),
    KindConflict,
    GenerationConflict,
    Cancelled,
    Budget,
    Exhausted,
    Entropy(String),
}

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => write!(f, "invalid drawing: {reason}"),
            Self::KindConflict => {
                f.write_str("kind_conflict: delete the existing drawing before changing ownership")
            }
            Self::GenerationConflict => f.write_str("dataset base generation changed"),
            Self::Cancelled => f.write_str("dataset publication permit was cancelled or consumed"),
            Self::Budget => f.write_str("dataset identity or pending permit limit reached"),
            Self::Exhausted => f.write_str("drawing identity counter exhausted"),
            Self::Entropy(reason) => write!(f, "drawing identity randomness unavailable: {reason}"),
        }
    }
}

impl std::error::Error for IdentityError {}

#[derive(Debug)]
pub struct DatasetEntry<T> {
    pub kind: DatasetKind,
    pub revision: DatasetRevision,
    pub payload: Arc<T>,
    pub lifetime: DatasetLifetime,
}

#[derive(Debug)]
struct LifetimeState {
    live: AtomicBool,
    _charge: Option<super::Charge>,
}

/// Shared retirement state for every generation of one dataset incarnation.
#[derive(Clone, Debug)]
pub struct DatasetLifetime(Arc<LifetimeState>);

impl DatasetLifetime {
    fn new(budget: Option<&super::CpuBudget>) -> Result<Self, IdentityError> {
        let bytes = (size_of::<LifetimeState>() + 2 * size_of::<usize>()) as u64;
        let charge = budget
            .map(|budget| {
                budget
                    .reserve(bytes)?
                    .charge(bytes, super::MemoryClass::Metadata)
            })
            .transpose()
            .map_err(|_| IdentityError::Budget)?;
        Ok(Self(Arc::new(LifetimeState {
            live: AtomicBool::new(true),
            _charge: charge,
        })))
    }

    pub fn is_live(&self) -> bool {
        self.0.live.load(Ordering::Acquire)
    }

    fn retire(&self) {
        self.0.live.store(false, Ordering::Release);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WritePermit {
    registry: u64,
    sequence: u64,
}

struct Pending {
    key: DrawKey,
    kind: DatasetKind,
    base: Option<DatasetRevision>,
}

struct Reservation {
    kind: DatasetKind,
    permits: usize,
}

pub struct DrawingStore<T> {
    legacy: DrawStore,
    datasets: BTreeMap<DrawKey, DatasetEntry<T>>,
    pending: HashMap<WritePermit, Pending>,
    reservations: BTreeMap<DrawKey, Reservation>,
    max_datasets: usize,
    max_pending: usize,
    next_permit: u64,
    registry: u64,
    next_incarnation: u64,
    revision: u64,
    barrier_revision: u64,
    _charge: Option<super::Charge>,
    budget: Option<super::CpuBudget>,
    retired: Vec<Arc<T>>,
    defer_retirement: bool,
}

impl<T> DrawingStore<T> {
    pub fn new(max_datasets: usize, max_pending: usize) -> Result<Self, IdentityError> {
        if !(1..=65_536).contains(&max_datasets) || !(1..=65_536).contains(&max_pending) {
            return Err(IdentityError::Invalid(
                "identity limits must be 1..65536".into(),
            ));
        }
        let registry = loop {
            let value = getrandom::u64().map_err(|e| IdentityError::Entropy(e.to_string()))?;
            if value != 0 {
                break value;
            }
        };
        Ok(Self {
            legacy: DrawStore::default(),
            datasets: BTreeMap::new(),
            pending: HashMap::new(),
            reservations: BTreeMap::new(),
            max_datasets,
            max_pending,
            next_permit: 1,
            registry,
            next_incarnation: 1,
            revision: 0,
            barrier_revision: 0,
            _charge: None,
            budget: None,
            retired: Vec::new(),
            defer_retirement: false,
        })
    }

    pub(super) fn new_charged(
        budget: &super::CpuBudget,
        datasets: usize,
        pending: usize,
    ) -> Result<Self, super::TransactionError> {
        use super::{MemoryClass, memory::tree_allowance};
        let bytes = size_of::<Self>() as u64
            + 2 * size_of::<usize>() as u64
            + tree_allowance::<DrawKey, DatasetEntry<T>>(datasets)
            + tree_allowance::<WritePermit, Pending>(pending)
            + tree_allowance::<DrawKey, Reservation>(pending)
            + (datasets as u64 + pending as u64 * 2) * 382
            + (datasets * size_of::<Arc<T>>()) as u64;
        let charge = budget
            .reserve(bytes)?
            .charge(bytes, MemoryClass::Metadata)?;
        let mut store = Self::new(datasets, pending)?;
        store._charge = Some(charge);
        store.budget = Some(budget.clone());
        store
            .retired
            .try_reserve_exact(datasets)
            .map_err(|_| super::StorageError::Allocation)?;
        if store.retired.capacity() != datasets {
            return Err(super::StorageError::Allocation.into());
        }
        store.defer_retirement = true;
        Ok(store)
    }

    pub fn legacy(&self) -> &DrawStore {
        &self.legacy
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn barrier_revision(&self) -> u64 {
        self.barrier_revision
    }
    pub fn dataset_len(&self) -> usize {
        self.datasets.len()
    }
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }
    pub fn get(&self, key: &DrawKey) -> Option<&DatasetEntry<T>> {
        self.datasets.get(key)
    }
    pub fn get_parts(&self, namespace: &str, id: &str) -> Option<&DatasetEntry<T>> {
        super::borrowed_key::get(&self.datasets, namespace, id)
    }
    pub fn datasets(&self) -> impl Iterator<Item = (&DrawKey, &DatasetEntry<T>)> {
        self.datasets.iter()
    }
    pub(super) fn datasets_namespace_after<'a>(
        &'a self,
        namespace: &'a str,
        after: Option<&DrawKey>,
    ) -> impl Iterator<Item = (&'a DrawKey, &'a DatasetEntry<T>)> {
        crate::draw::namespace_range(&self.datasets, namespace, after)
    }
    pub(super) fn datasets_after<'a>(
        &'a self,
        after: Option<&DrawKey>,
    ) -> impl Iterator<Item = (&'a DrawKey, &'a DatasetEntry<T>)> {
        use std::ops::Bound::{Excluded, Unbounded};
        self.datasets
            .range::<DrawKey, _>((after.map_or(Unbounded, Excluded), Unbounded))
    }
    pub fn pop_retired(&mut self) -> Option<Arc<T>> {
        self.retired.pop()
    }

    pub(super) fn replace_storage(
        &mut self,
        key: &DataKey,
        revision: DatasetRevision,
        source: &Arc<T>,
        replacement: Arc<T>,
    ) -> Result<Arc<T>, IdentityError> {
        let version = self
            .revision
            .checked_add(1)
            .ok_or(IdentityError::Exhausted)?;
        let entry = self
            .datasets
            .get_mut(&(key.namespace.clone(), key.id.clone()))
            .ok_or(IdentityError::Cancelled)?;
        if entry.revision != revision || !Arc::ptr_eq(&entry.payload, source) {
            return Err(IdentityError::GenerationConflict);
        }
        let previous = std::mem::replace(&mut entry.payload, replacement);
        self.revision = version;
        Ok(previous)
    }

    pub fn begin_dataset(
        &mut self,
        key: &DataKey,
        kind: DatasetKind,
        base: Option<DatasetRevision>,
    ) -> Result<WritePermit, IdentityError> {
        key.validate().map_err(IdentityError::Invalid)?;
        if let Some(base) = base {
            base.validate().map_err(IdentityError::Invalid)?;
        }
        let key = (key.namespace.clone(), key.id.clone());
        if self.legacy.get(&key.0, &key.1).is_some()
            || self.legacy.has_pending(&key.0, &key.1)
            || self
                .datasets
                .get(&key)
                .is_some_and(|entry| entry.kind != kind)
            || self
                .reservations
                .get(&key)
                .is_some_and(|entry| entry.kind != kind)
        {
            return Err(IdentityError::KindConflict);
        }
        if self.datasets.get(&key).map(|entry| entry.revision) != base {
            return Err(IdentityError::GenerationConflict);
        }
        let slots = self.datasets.len()
            + self.retired.len()
            + self
                .reservations
                .keys()
                .filter(|key| !self.datasets.contains_key(*key))
                .count();
        if self.pending.len() == self.max_pending
            || (!self.datasets.contains_key(&key)
                && !self.reservations.contains_key(&key)
                && slots == self.max_datasets)
        {
            return Err(IdentityError::Budget);
        }
        let next = self
            .next_permit
            .checked_add(1)
            .ok_or(IdentityError::Exhausted)?;
        let permit = WritePermit {
            registry: self.registry,
            sequence: self.next_permit,
        };
        self.next_permit = next;
        self.reservations
            .entry(key.clone())
            .or_insert(Reservation { kind, permits: 0 })
            .permits += 1;
        self.pending.insert(permit, Pending { key, kind, base });
        Ok(permit)
    }

    /// Publishes an immutable payload only while its exact base and permit remain valid.
    pub fn commit_dataset(
        &mut self,
        permit: WritePermit,
        payload: Arc<T>,
    ) -> Result<DatasetRevision, IdentityError> {
        let pending = self.take_permit(permit).ok_or(IdentityError::Cancelled)?;
        let current = self.datasets.get(&pending.key).map(|entry| entry.revision);
        if current != pending.base {
            return Err(IdentityError::GenerationConflict);
        }
        let revision = match current {
            Some(current) => DatasetRevision {
                incarnation: current.incarnation,
                generation: current
                    .generation
                    .checked_add(1)
                    .ok_or(IdentityError::Exhausted)?,
            },
            None => {
                let next = self
                    .next_incarnation
                    .checked_add(1)
                    .ok_or(IdentityError::Exhausted)?;
                let revision = DatasetRevision {
                    incarnation: self.next_incarnation,
                    generation: 1,
                };
                self.next_incarnation = next;
                revision
            }
        };
        let version = self
            .revision
            .checked_add(1)
            .ok_or(IdentityError::Exhausted)?;
        let lifetime = match self.datasets.get(&pending.key) {
            Some(entry) => entry.lifetime.clone(),
            None => DatasetLifetime::new(self.budget.as_ref())?,
        };
        self.datasets.insert(
            pending.key,
            DatasetEntry {
                kind: pending.kind,
                revision,
                payload,
                lifetime,
            },
        );
        self.revision = version;
        Ok(revision)
    }

    pub fn cancel(&mut self, permit: WritePermit) -> bool {
        self.take_permit(permit).is_some()
    }

    pub fn permit_is_live(&self, permit: WritePermit) -> bool {
        self.pending.contains_key(&permit)
    }

    pub fn apply_legacy(
        &mut self,
        message: DrawEntity,
        now_s: f64,
    ) -> Result<usize, IdentityError> {
        message.validate().map_err(IdentityError::Invalid)?;
        let key = (message.ns().trim_matches('/').into(), message.id());
        if message.op == draw_op::DELETE {
            return self.delete(&key);
        }
        if message.op == draw_op::CLEAR {
            return self.clear(&key.0);
        }
        if self.datasets.contains_key(&key) || self.reservations.contains_key(&key) {
            return Err(IdentityError::KindConflict);
        }
        let version = self
            .revision
            .checked_add(1)
            .ok_or(IdentityError::Exhausted)?;
        let result = self
            .legacy
            .apply(message, now_s)
            .map_err(IdentityError::Invalid);
        self.revision = version;
        result
    }

    pub fn delete(&mut self, key: &DrawKey) -> Result<usize, IdentityError> {
        let version = self
            .revision
            .checked_add(1)
            .ok_or(IdentityError::Exhausted)?;
        let key = (key.0.trim_matches('/').to_string(), key.1.clone());
        let legacy = self
            .legacy
            .apply(DrawEntity::delete(&key.0, &key.1), 0.0)
            .map_err(IdentityError::Invalid)?;
        let data = if let Some(entry) = self.datasets.remove(&key) {
            entry.lifetime.retire();
            if self.defer_retirement {
                self.retired.push(entry.payload);
            }
            1
        } else {
            0
        };
        self.pending.retain(|_, pending| pending.key != key);
        self.reservations.remove(&key);
        self.revision = version;
        self.barrier_revision = version;
        Ok(legacy + data)
    }

    pub fn clear(&mut self, namespace: &str) -> Result<usize, IdentityError> {
        let version = self
            .revision
            .checked_add(1)
            .ok_or(IdentityError::Exhausted)?;
        let namespace = namespace.trim_matches('/');
        let before = self.datasets.len();
        self.datasets.retain(|(ns, _), entry| {
            let remove = ns_within(ns, namespace);
            if remove {
                entry.lifetime.retire();
            }
            if remove && self.defer_retirement {
                self.retired.push(entry.payload.clone());
            }
            !remove
        });
        self.pending
            .retain(|_, pending| !ns_within(&pending.key.0, namespace));
        self.reservations
            .retain(|(ns, _), _| !ns_within(ns, namespace));
        let removed = before - self.datasets.len() + self.legacy.clear(namespace);
        self.revision = version;
        self.barrier_revision = version;
        Ok(removed)
    }

    pub fn expire_legacy(&mut self, now_s: f64) -> Result<usize, IdentityError> {
        let version = self
            .revision
            .checked_add(1)
            .ok_or(IdentityError::Exhausted)?;
        let removed = self.legacy.expire(now_s);
        if removed > 0 {
            self.revision = version;
        }
        Ok(removed)
    }

    fn take_permit(&mut self, permit: WritePermit) -> Option<Pending> {
        let pending = self.pending.remove(&permit)?;
        let reservation = self
            .reservations
            .get_mut(&pending.key)
            .expect("pending permit owns a reservation");
        reservation.permits -= 1;
        if reservation.permits == 0 {
            self.reservations.remove(&pending.key);
        }
        Some(pending)
    }
}
