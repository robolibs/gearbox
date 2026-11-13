use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use bevy::prelude::Resource;
use gearbox_api::draw_data::{Charge, CpuBudget, StorageError};

const MAX_IN_FLIGHT: usize = 2;

struct State {
    pending: AtomicUsize,
    _charge: Charge,
}

#[derive(Resource, Default)]
pub(super) struct Window(Option<Arc<State>>);

pub(super) struct Permit(Arc<State>);

impl Window {
    pub fn clear(&mut self) {
        if self
            .0
            .as_ref()
            .is_some_and(|state| state.pending.load(Ordering::Acquire) == 0)
        {
            self.0 = None;
        }
    }

    pub fn acquire(&mut self, budget: &CpuBudget) -> Result<Option<Permit>, StorageError> {
        if self.0.is_none() {
            self.0 = Some(Arc::new(State {
                pending: AtomicUsize::new(0),
                _charge: super::worker::charge(
                    budget,
                    size_of::<State>() + 2 * size_of::<usize>(),
                )?,
            }));
        }
        let state = self.0.as_ref().unwrap();
        Ok(state
            .pending
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < MAX_IN_FLIGHT).then_some(count + 1)
            })
            .ok()
            .map(|_| Permit(state.clone())))
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.0.pending.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_gpu_completion_reopens_the_bounded_upload_window() {
        let budget = CpuBudget::new(4096);
        let mut window = Window::default();
        let first = window.acquire(&budget).unwrap().unwrap();
        let second = window.acquire(&budget).unwrap().unwrap();
        let charged = budget.usage().committed();
        let pressure = budget.reserve(4096 - charged).unwrap();
        assert!(window.acquire(&budget).unwrap().is_none());
        drop(first);
        let third = window.acquire(&budget).unwrap().unwrap();
        assert!(window.acquire(&budget).unwrap().is_none());
        drop((second, third, pressure));
        window.clear();
        assert_eq!(budget.usage().committed(), 0);
    }

    #[test]
    fn clear_keeps_pending_work_charged_until_completion() {
        let budget = CpuBudget::new(4096);
        let mut window = Window::default();
        let permit = window.acquire(&budget).unwrap().unwrap();
        let other = window.acquire(&budget).unwrap().unwrap();
        let charged = budget.usage().committed();
        window.clear();
        assert_eq!(budget.usage().committed(), charged);
        assert!(window.acquire(&budget).unwrap().is_none());
        drop((permit, other));
        window.clear();
        assert_eq!(budget.usage().committed(), 0);
        assert!(Window::default().acquire(&CpuBudget::new(0)).is_err());
    }
}
