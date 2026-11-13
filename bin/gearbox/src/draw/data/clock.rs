use std::sync::Arc;

use gearbox_api::draw_data::{Charge, CpuBudget, DatasetStorage, StorageError};
use gearbox_api::wire::draw_data::{AgeClock, CaptureStamp, StreamMode};

pub(super) struct SourceClocks {
    generation: Arc<DatasetStorage>,
    indices: Vec<usize>,
    _charge: Charge,
}

fn stamp(generation: &DatasetStorage, index: usize) -> &CaptureStamp {
    let scan = generation
        .scans()
        .get(index)
        .map(Arc::as_ref)
        .or_else(|| generation.accumulation_clock_anchor())
        .expect("source clock scan");
    scan.value().frame.capture.as_ref().expect("source capture")
}

fn domain(stamp: &CaptureStamp) -> (u64, &str) {
    (
        stamp.scene_epoch,
        stamp.source_clock.as_deref().expect("named source clock"),
    )
}

impl SourceClocks {
    pub fn prepare(
        budget: &CpuBudget,
        generation: Arc<DatasetStorage>,
    ) -> Result<Self, StorageError> {
        let anchor = matches!(
            generation.descriptor().stream,
            StreamMode::Accumulate {
                age_clock: AgeClock::Source,
                ..
            }
        ) && generation.accumulation_clock_anchor().is_some();
        let capacity = generation.scans().len() + usize::from(anchor);
        let charge = super::worker::charge(budget, capacity * size_of::<usize>())?;
        let mut indices = Vec::new();
        indices
            .try_reserve_exact(capacity)
            .map_err(|_| StorageError::Allocation)?;
        if indices.capacity() != capacity {
            return Err(StorageError::Allocation);
        }
        for (index, scan) in generation.scans().iter().enumerate() {
            if scan
                .value()
                .frame
                .capture
                .as_ref()
                .is_some_and(|s| s.source_time_ns.is_some())
            {
                indices.push(index);
            }
        }
        if anchor {
            indices.push(generation.scans().len());
        }
        indices.sort_unstable_by(|&a, &b| {
            let a = stamp(&generation, a);
            let b = stamp(&generation, b);
            domain(a)
                .cmp(&domain(b))
                .then_with(|| b.source_time_ns.cmp(&a.source_time_ns))
        });
        indices.dedup_by(|a, b| domain(stamp(&generation, *a)) == domain(stamp(&generation, *b)));
        Ok(Self {
            generation,
            indices,
            _charge: charge,
        })
    }

    pub fn time_ns(&self, capture: Option<&CaptureStamp>) -> Option<u64> {
        let capture = capture?;
        let key = (capture.scene_epoch, capture.source_clock.as_deref()?);
        let index = self
            .indices
            .binary_search_by_key(&key, |index| domain(stamp(&self.generation, *index)))
            .ok()?;
        stamp(&self.generation, self.indices[index]).source_time_ns
    }
}

#[cfg(test)]
#[path = "clock_tests.rs"]
mod tests;
