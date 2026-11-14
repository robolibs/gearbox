use std::fmt::{self, Write};
use std::ops::Deref;
use std::sync::{RwLock, RwLockReadGuard, TryLockError};

use super::owned::List;

pub(super) trait Live {
    fn live(&self) -> bool;
}

pub(super) struct LiveList<T>(RwLock<List<T>>);

impl<T> Default for LiveList<T> {
    fn default() -> Self {
        Self(RwLock::new(List::default()))
    }
}

impl<T> From<List<T>> for LiveList<T> {
    fn from(values: List<T>) -> Self {
        Self(RwLock::new(if values.is_empty() {
            List::default()
        } else {
            values
        }))
    }
}

impl<T> LiveList<T> {
    pub fn read(&self) -> Read<'_, T> {
        Read(Some(self.0.read().unwrap_or_else(|p| p.into_inner())))
    }

    pub fn len(&self) -> usize {
        self.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.read().is_empty()
    }
}

impl<T: Live> LiveList<T> {
    pub fn prune(&self) -> usize {
        let mut values = match self.0.try_write() {
            Ok(values) => values,
            Err(TryLockError::Poisoned(p)) => p.into_inner(),
            Err(TryLockError::WouldBlock) => return 0,
        };
        let before = values.len();
        values.retain(Live::live);
        let removed = before - values.len();
        if values.is_empty() {
            *values = List::default();
        }
        removed
    }
}

pub(super) struct Read<'a, T>(Option<RwLockReadGuard<'a, List<T>>>);

impl<T> Default for Read<'_, T> {
    fn default() -> Self {
        Self(None)
    }
}

impl<T> Read<'_, T> {
    pub fn capacity(&self) -> usize {
        self.0.as_ref().map_or(0, |values| values.capacity())
    }
}

impl<T> Deref for Read<'_, T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        self.0.as_ref().map_or(&[], |values| values.as_slice())
    }
}

pub(super) struct Text {
    bytes: [u8; 512],
    len: usize,
    full: bool,
}

impl Text {
    pub fn new(value: &impl fmt::Display) -> Self {
        let mut text = Self {
            bytes: [0; 512],
            len: 0,
            full: false,
        };
        write!(&mut text, "{value}").expect("bounded text formatter");
        text
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len]).expect("UTF-8 text boundary")
    }
}

impl Write for Text {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if self.full {
            return Ok(());
        }
        let count = value.floor_char_boundary((self.bytes.len() - self.len).min(value.len()));
        self.bytes[self.len..self.len + count].copy_from_slice(&value.as_bytes()[..count]);
        self.len += count;
        self.full = count < value.len();
        Ok(())
    }
}

impl fmt::Display for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
#[path = "retirement_tests.rs"]
mod tests;
