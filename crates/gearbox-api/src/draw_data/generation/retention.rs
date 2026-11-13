use std::cmp::Ordering;
use std::sync::Arc;

use super::{Generation, GenerationCounts, RetentionCounts, invalid};
use crate::draw_data::memory::Buffer;
use crate::draw_data::{CpuBudget, MemoryClass, ScanMetadata, StorageError};
use crate::wire::draw_data::{AgeClock, ScanDescriptor, StreamMode};

fn clock(mode: &StreamMode) -> Option<AgeClock> {
    match mode {
        StreamMode::Accumulate { age_clock, .. } => Some(*age_clock),
        _ => None,
    }
}

fn compare(a: &ScanDescriptor, b: &ScanDescriptor, clock: AgeClock) -> Ordering {
    let a = a.frame.capture.as_ref().unwrap();
    let b = b.frame.capture.as_ref().unwrap();
    match clock {
        AgeClock::Simulation => a.simulation_time_s.total_cmp(&b.simulation_time_s),
        AgeClock::Source => a.source_time_ns.unwrap().cmp(&b.source_time_ns.unwrap()),
    }
}

impl Generation {
    pub(in crate::draw_data) fn set_accumulation_clock(
        &mut self,
        previous: Option<&Self>,
    ) -> Result<(), StorageError> {
        let Some(age_clock) = clock(&self.descriptor().stream) else {
            return Ok(());
        };
        let previous = previous.filter(|old| clock(&old.descriptor().stream) == Some(age_clock));
        let mut anchor = previous.and_then(|old| old.clock_anchor.clone());
        for scan in &self.scans.data {
            if let Some(old) = &anchor {
                let before = old.value().frame.capture.as_ref().unwrap();
                let stamp = scan.value().frame.capture.as_ref().unwrap();
                if before.scene_epoch != stamp.scene_epoch
                    || age_clock == AgeClock::Source && before.source_clock != stamp.source_clock
                {
                    return Err(invalid(
                        "accumulation clock or scene epoch changed; delete/recreate the dataset",
                    ));
                }
            }
            if anchor
                .as_ref()
                .is_none_or(|old| compare(scan.value(), old.value(), age_clock).is_gt())
            {
                anchor = Some(scan.clone());
            }
        }
        self.clock_anchor = anchor;
        Ok(())
    }

    pub(in crate::draw_data) fn refresh_oldest_capture(&mut self) {
        self.oldest_simulation_s = if clock(&self.descriptor().stream) == Some(AgeClock::Simulation)
        {
            self.scans
                .data
                .iter()
                .map(|scan| {
                    scan.value()
                        .frame
                        .capture
                        .as_ref()
                        .unwrap()
                        .simulation_time_s
                })
                .min_by(f64::total_cmp)
        } else {
            None
        };
    }

    pub(in crate::draw_data) fn age_due(&self, now_s: f64) -> bool {
        let StreamMode::Accumulate { max_age_s, .. } = self.descriptor().stream else {
            return false;
        };
        self.oldest_simulation_s
            .is_some_and(|oldest| now_s - oldest >= max_age_s)
    }

    pub(in crate::draw_data) fn apply_retention(
        &mut self,
        previous: Option<&Self>,
        now_s: f64,
    ) -> Result<RetentionCounts, StorageError> {
        let StreamMode::Accumulate {
            age_clock,
            max_scans,
            max_age_s,
            max_bytes,
        } = self.descriptor().stream
        else {
            return Ok(RetentionCounts::default());
        };
        self.set_accumulation_clock(previous)?;
        let mut index = 0;
        while index < self.pages.data.len() {
            let key = self.pages.data[index].descriptor().address.scan_key;
            let mut bytes = 0;
            while index < self.pages.data.len()
                && self.pages.data[index].descriptor().address.scan_key == key
            {
                bytes += self.pages.data[index].descriptor().byte_len;
                index += 1;
            }
            if bytes > max_bytes {
                return Err(StorageError::RetentionBudget {
                    bytes,
                    limit: max_bytes,
                });
            }
        }
        self.scans.data.sort_unstable_by(|a, b| {
            compare(a.value(), b.value(), age_clock)
                .then_with(|| a.value().scan_key.cmp(&b.value().scan_key))
        });
        let mut removed = 0;
        let mut bytes = self.counts.payload_bytes;
        let mut outcome = RetentionCounts::default();
        for scan in &self.scans.data {
            let stamp = scan.value().frame.capture.as_ref().unwrap();
            let age = match age_clock {
                AgeClock::Simulation => now_s - stamp.simulation_time_s,
                AgeClock::Source => {
                    let watermark = self
                        .clock_anchor
                        .as_ref()
                        .unwrap()
                        .value()
                        .frame
                        .capture
                        .as_ref()
                        .unwrap()
                        .source_time_ns
                        .unwrap();
                    watermark.saturating_sub(stamp.source_time_ns.unwrap()) as f64 / 1_000_000_000.0
                }
            };
            if age < max_age_s
                && self.scans.data.len() - removed <= max_scans as usize
                && bytes <= max_bytes
            {
                break;
            }
            let key = scan.value().scan_key;
            let start = self
                .pages
                .data
                .partition_point(|page| page.descriptor().address.scan_key < key);
            for page in self.pages.data[start..]
                .iter()
                .take_while(|page| page.descriptor().address.scan_key == key)
            {
                bytes -= page.descriptor().byte_len;
            }
            if previous.is_some_and(|old| {
                old.scans
                    .data
                    .binary_search_by_key(&key, |scan| scan.value().scan_key)
                    .is_ok()
            }) {
                outcome.evicted += 1;
            } else {
                outcome.dropped += 1;
            }
            removed += 1;
        }
        self.scans.data.drain(..removed);
        self.scans
            .data
            .sort_unstable_by_key(|scan| scan.value().scan_key);
        self.pages.data.retain(|page| {
            self.scans
                .data
                .binary_search_by_key(&page.descriptor().address.scan_key, |scan| {
                    scan.value().scan_key
                })
                .is_ok()
        });
        self.counts = GenerationCounts {
            scans: self.scans.data.len() as u32,
            pages: self.pages.data.len() as u32,
            source_samples: self
                .pages
                .data
                .iter()
                .map(|page| u64::from(page.descriptor().items))
                .sum(),
            valid_samples: self
                .pages
                .data
                .iter()
                .map(|page| page.valid_count(&self.descriptor().shape))
                .sum(),
            payload_bytes: bytes,
        };
        self.refresh_oldest_capture();
        Ok(outcome)
    }

    pub(in crate::draw_data) fn expire_scans(
        &self,
        budget: &CpuBudget,
        now_s: f64,
    ) -> Result<(Arc<Self>, RetentionCounts), StorageError> {
        if !self.descriptor.belongs_to(budget) {
            return Err(invalid("expiry budget differs from generation"));
        }
        let bytes = (size_of::<Self>()
            + 2 * size_of::<usize>()
            + self.scans.data.len() * size_of::<Arc<ScanMetadata>>()
            + self.pages.data.len() * size_of::<Arc<crate::draw_data::StoredPage>>())
            as u64;
        let mut reservation = budget.reserve(bytes)?;
        let scans = Buffer::cloned(&mut reservation, &self.scans.data, MemoryClass::Metadata)?;
        let pages = Buffer::cloned(&mut reservation, &self.pages.data, MemoryClass::Metadata)?;
        let charge = reservation.charge(reservation.bytes(), MemoryClass::Metadata)?;
        let mut generation = Self {
            descriptor: self.descriptor.clone(),
            scans,
            pages,
            counts: self.counts,
            scan_high_watermark: self.scan_high_watermark,
            retained_at_s: self.retained_at_s,
            clock_anchor: self.clock_anchor.clone(),
            oldest_simulation_s: self.oldest_simulation_s,
            _charge: charge,
        };
        let outcome = generation.apply_retention(Some(self), now_s)?;
        Ok((Arc::new(generation), outcome))
    }
}
