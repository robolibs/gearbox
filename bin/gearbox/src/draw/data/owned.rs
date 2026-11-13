use std::ops::{Deref, DerefMut};

use gearbox_api::draw_data::{Charge, CpuBudget, MemoryClass, StorageError};

#[path = "owned_map.rs"]
mod map;
pub(super) use map::Map;

#[cfg(test)]
#[path = "owned_tests.rs"]
mod tests;

pub(super) struct List<T> {
    values: Vec<T>,
    limit: usize,
    charge: Option<Charge>,
}

impl<T> Default for List<T> {
    fn default() -> Self {
        Self {
            values: Vec::new(),
            limit: 0,
            charge: None,
        }
    }
}

impl<T> List<T> {
    pub fn new(budget: &CpuBudget, count: usize, class: MemoryClass) -> Result<Self, StorageError> {
        let bytes = count
            .checked_mul(size_of::<T>())
            .ok_or(StorageError::Allocation)?;
        let charge = if bytes == 0 {
            None
        } else {
            let bytes = bytes
                .checked_add(size_of::<Self>())
                .ok_or(StorageError::Allocation)? as u64;
            Some(budget.reserve(bytes)?.charge(bytes, class)?)
        };
        let mut values = Vec::new();
        values
            .try_reserve_exact(count)
            .map_err(|_| StorageError::Allocation)?;
        if size_of::<T>() != 0 && values.capacity() != count {
            return Err(StorageError::Allocation);
        }
        Ok(Self {
            values,
            limit: count,
            charge,
        })
    }

    pub fn push(&mut self, value: T) -> Result<(), StorageError> {
        if self.values.len() == self.limit {
            return Err(StorageError::Allocation);
        }
        self.values.push(value);
        Ok(())
    }

    pub fn resize_with(
        &mut self,
        count: usize,
        value: impl FnMut() -> T,
    ) -> Result<(), StorageError> {
        if count > self.limit {
            return Err(StorageError::Allocation);
        }
        self.values.resize_with(count, value);
        Ok(())
    }

    pub fn remaining(&self) -> usize {
        self.limit - self.values.len()
    }
    pub fn capacity(&self) -> usize {
        self.limit
    }
    pub fn clear(&mut self) {
        self.values.clear();
    }

    pub fn pop(&mut self) -> Option<T> {
        self.values.pop()
    }
    pub fn retain(&mut self, keep: impl FnMut(&T) -> bool) {
        self.values.retain(keep);
    }
    pub fn as_slice(&self) -> &[T] {
        &self.values
    }
}

impl<T> Deref for List<T> {
    type Target = [T];
    fn deref(&self) -> &Self::Target {
        &self.values
    }
}

impl<T> DerefMut for List<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.values
    }
}

impl<'a, T> IntoIterator for &'a List<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.values.iter()
    }
}

impl<'a, T> IntoIterator for &'a mut List<T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.values.iter_mut()
    }
}

pub(super) struct IntoIter<T> {
    values: std::vec::IntoIter<T>,
    _charge: Option<Charge>,
}

impl<T> Iterator for IntoIter<T> {
    type Item = T;
    fn next(&mut self) -> Option<Self::Item> {
        self.values.next()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.values.size_hint()
    }
}

impl<T> IntoIterator for List<T> {
    type Item = T;
    type IntoIter = IntoIter<T>;
    fn into_iter(self) -> Self::IntoIter {
        IntoIter {
            values: self.values.into_iter(),
            _charge: self.charge,
        }
    }
}
