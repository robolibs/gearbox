use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hash};
use std::ops::Index;

use gearbox_api::draw_data::{CpuBudget, MemoryClass, StorageError};

use super::List;

enum Slot<K, V> {
    Empty,
    Removed,
    Value(K, V, bool),
}

pub(in crate::draw::data) struct Map<K, V> {
    slots: List<Slot<K, V>>,
    len: usize,
    hash: RandomState,
    class: MemoryClass,
}

impl<K, V> Default for Map<K, V> {
    fn default() -> Self {
        Self::with_class(MemoryClass::Metadata)
    }
}

impl<K, V> Map<K, V> {
    pub fn with_class(class: MemoryClass) -> Self {
        Self {
            slots: List::default(),
            len: 0,
            hash: RandomState::new(),
            class,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }
    pub fn clear(&mut self) {
        self.slots = List::default();
        self.len = 0;
    }
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.slots.iter().filter_map(|slot| match slot {
            Slot::Value(_, value, _) => Some(value),
            _ => None,
        })
    }
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.slots.iter_mut().filter_map(|slot| match slot {
            Slot::Value(_, value, _) => Some(value),
            _ => None,
        })
    }
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.slots.iter().filter_map(|slot| match slot {
            Slot::Value(key, value, _) => Some((key, value)),
            _ => None,
        })
    }
    pub fn retain(&mut self, mut keep: impl FnMut(&K, &mut V) -> bool) {
        for slot in self.slots.iter_mut() {
            if let Slot::Value(key, value, _) = slot
                && !keep(key, value)
            {
                *slot = Slot::Removed;
                self.len -= 1;
            }
        }
        if self.len == 0 {
            self.clear();
        }
    }
}

impl<K: Eq + Hash, V> Map<K, V> {
    fn find(&self, key: &K) -> Option<usize> {
        if self.slots.is_empty() {
            return None;
        }
        let start = self.hash.hash_one(key) as usize;
        for step in 0..self.slots.len() {
            let i = start.wrapping_add(step) & (self.slots.len() - 1);
            match &self.slots[i] {
                Slot::Empty => return None,
                Slot::Value(found, _, _) if found == key => return Some(i),
                _ => {}
            }
        }
        None
    }

    pub fn contains_key(&self, key: &K) -> bool {
        self.find(key).is_some()
    }
    pub fn get(&self, key: &K) -> Option<&V> {
        self.find(key).map(|i| match &self.slots[i] {
            Slot::Value(_, value, _) => value,
            _ => unreachable!(),
        })
    }
    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        self.find(key).map(|i| match &mut self.slots[i] {
            Slot::Value(_, value, _) => value,
            _ => unreachable!(),
        })
    }

    pub fn retain_keys(
        &mut self,
        keys: impl IntoIterator<Item = K>,
        removed: impl FnMut(&K, &mut V),
    ) {
        self.retain_keys_by(
            |mark| {
                for key in keys {
                    mark(&key);
                }
            },
            removed,
        );
    }

    pub fn retain_keys_by(
        &mut self,
        enumerate: impl FnOnce(&mut dyn FnMut(&K)),
        mut removed: impl FnMut(&K, &mut V),
    ) {
        for slot in self.slots.iter_mut() {
            if let Slot::Value(_, _, marked) = slot {
                *marked = false;
            }
        }
        enumerate(&mut |key| {
            if let Some(i) = self.find(key)
                && let Slot::Value(_, _, marked) = &mut self.slots[i]
            {
                *marked = true;
            }
        });
        for slot in self.slots.iter_mut() {
            if let Slot::Value(key, value, false) = slot {
                removed(key, value);
                *slot = Slot::Removed;
                self.len -= 1;
            }
        }
        if self.len == 0 {
            self.clear();
        }
    }

    pub fn reserve(&mut self, budget: &CpuBudget, additional: usize) -> Result<(), StorageError> {
        let needed = self
            .len
            .checked_add(additional)
            .ok_or(StorageError::Allocation)?;
        if needed <= self.slots.len() / 2 {
            return Ok(());
        }
        let count = needed
            .checked_mul(2)
            .and_then(|n| n.max(4).checked_next_power_of_two())
            .ok_or(StorageError::Allocation)?;
        self.resize(budget, count)
    }

    pub fn compact(&mut self, budget: &CpuBudget) -> Result<(), StorageError> {
        if self.len == 0 {
            self.clear();
            return Ok(());
        }
        let count = self
            .len
            .checked_mul(2)
            .and_then(|n| n.max(4).checked_next_power_of_two())
            .ok_or(StorageError::Allocation)?;
        if count < self.slots.len() {
            self.resize(budget, count)?;
        }
        Ok(())
    }

    fn resize(&mut self, budget: &CpuBudget, count: usize) -> Result<(), StorageError> {
        let mut slots = List::new(budget, count, self.class)?;
        slots.resize_with(count, || Slot::Empty)?;
        let previous = std::mem::replace(&mut self.slots, slots);
        self.len = 0;
        for slot in previous {
            if let Slot::Value(key, value, _) = slot {
                self.place(key, value);
            }
        }
        Ok(())
    }

    fn place(&mut self, key: K, value: V) {
        let start = self.hash.hash_one(&key) as usize;
        for step in 0..self.slots.len() {
            let i = start.wrapping_add(step) & (self.slots.len() - 1);
            if !matches!(self.slots[i], Slot::Value(..)) {
                self.slots[i] = Slot::Value(key, value, false);
                self.len += 1;
                return;
            }
        }
        unreachable!("cache insertion requires reserved slots")
    }

    pub fn insert(
        &mut self,
        budget: &CpuBudget,
        key: K,
        value: V,
    ) -> Result<Option<V>, StorageError> {
        if let Some(i) = self.find(&key) {
            let Slot::Value(_, old, _) =
                std::mem::replace(&mut self.slots[i], Slot::Value(key, value, false))
            else {
                unreachable!()
            };
            return Ok(Some(old));
        }
        self.reserve(budget, 1)?;
        self.place(key, value);
        Ok(None)
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let value = self.take_retaining_capacity(key)?;
        if self.len == 0 {
            self.clear();
        }
        Some(value)
    }

    pub fn take_retaining_capacity(&mut self, key: &K) -> Option<V> {
        let i = self.find(key)?;
        let Slot::Value(_, value, _) = std::mem::replace(&mut self.slots[i], Slot::Removed) else {
            unreachable!()
        };
        self.len -= 1;
        Some(value)
    }
}

impl<K: Eq + Hash, V> Index<&K> for Map<K, V> {
    type Output = V;
    fn index(&self, key: &K) -> &Self::Output {
        self.get(key).expect("present cache key")
    }
}
