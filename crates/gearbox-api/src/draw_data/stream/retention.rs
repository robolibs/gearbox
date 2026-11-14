use super::{StreamingStore, failure};
use crate::draw_data::{TransactionError, transaction::Publication};
use crate::wire::draw_data::{DataFailure, DataPhase, DatasetRevision, data_code};

pub(super) struct MaintenanceFailure {
    pub revision: DatasetRevision,
    pub failure: DataFailure,
    pub sequence: u64,
    memory: u64,
    permits: usize,
    attempted_s: f64,
}

impl StreamingStore {
    pub(super) fn expire_accumulation(&mut self, now_s: f64) -> Result<(), TransactionError> {
        {
            let registry = self.registry.lock().unwrap_or_else(|p| p.into_inner());
            self.maintenance_failures.retain(|key, failure| {
                registry
                    .get(key)
                    .is_some_and(|entry| entry.revision == failure.revision)
            });
        }
        let mut after = None;
        let mut first_error = None;
        for _ in 0..self.limits.datasets {
            let memory = self.budget.usage().committed();
            let candidate = {
                let registry = self.registry.lock().unwrap_or_else(|p| p.into_inner());
                registry
                    .datasets_after(after.as_ref())
                    .find(|(key, entry)| {
                        entry.payload.age_due(now_s)
                            && self.maintenance_failures.get(*key).is_none_or(|old| {
                                old.revision != entry.revision
                                    || old.memory != memory
                                    || old.permits != registry.pending_len()
                                    || now_s - old.attempted_s >= 1.0
                            })
                    })
                    .map(|(key, entry)| {
                        (
                            key.clone(),
                            entry.kind,
                            entry.revision,
                            entry.payload.clone(),
                        )
                    })
            };
            let Some((key, kind, revision, base)) = candidate else {
                break;
            };
            after = Some(key.clone());
            let result = (|| {
                let permit = self
                    .registry
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .begin_dataset(&base.descriptor().key, kind, Some(revision))?;
                let mut publication = Publication::new(self.registry.clone(), permit);
                let (generation, outcome) = base.expire_scans(&self.budget, now_s)?;
                publication.publish(generation)?;
                Ok::<_, TransactionError>(outcome)
            })();
            match result {
                Ok(outcome) => {
                    self.maintenance_failures.remove(&key);
                    self.counters.evicted_scans =
                        self.counters.evicted_scans.saturating_add(outcome.evicted);
                    self.counters.dropped_scans =
                        self.counters.dropped_scans.saturating_add(outcome.dropped);
                }
                Err(error) if matches!(error.code, data_code::CONFLICT | data_code::EXPIRED) => {}
                Err(error) => {
                    let reply = self.error(error);
                    let reason = String::from_utf8(reply.message).unwrap();
                    let registry = self.registry.lock().unwrap_or_else(|p| p.into_inner());
                    self.maintenance_failures.retain(|key, failure| {
                        registry
                            .get(key)
                            .is_some_and(|entry| entry.revision == failure.revision)
                    });
                    if registry
                        .get(&key)
                        .is_none_or(|entry| entry.revision != revision)
                    {
                        continue;
                    }
                    let changed = self.maintenance_failures.get(&key).is_none_or(|old| {
                        old.revision != revision
                            || old.failure.code != reply.code
                            || old.failure.reason != reason
                    });
                    if changed && first_error.is_none() {
                        first_error = Some(failure(reply.code, reason.clone()));
                    }
                    if changed {
                        self.sequence = self.sequence.saturating_add(1);
                    }
                    let sequence = if changed {
                        self.sequence
                    } else {
                        self.maintenance_failures[&key].sequence
                    };
                    self.maintenance_failures.insert(
                        key,
                        MaintenanceFailure {
                            revision,
                            sequence,
                            failure: DataFailure {
                                phase: DataPhase::Retaining,
                                code: reply.code,
                                reason,
                            },
                            memory: self.budget.usage().committed(),
                            permits: registry.pending_len(),
                            attempted_s: now_s,
                        },
                    );
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}
