use std::collections::BTreeMap;

use super::{StreamingStore, failure};

const ATTACH_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);
use crate::draw::DrawKey;
use crate::draw_data::{
    TransactionError,
    memory::tree_allowance,
    transaction::{Publication, request_base},
};
use crate::wire::draw_data::*;

struct Cursor {
    namespace: String,
    limit: u32,
    revision: u64,
    after: DrawKey,
    created: u64,
    sequence: u64,
}

#[derive(Default)]
pub(super) struct CursorBook {
    cursors: BTreeMap<u128, Cursor>,
    sequence: u64,
}

impl CursorBook {
    pub fn allowance(count: u32) -> u64 {
        tree_allowance::<u128, Cursor>(count as usize)
            + u64::from(count) * (crate::host::MAX_PAYLOAD_BYTES as u64 + 255)
    }

    pub fn maintain(&mut self, now: u64, revision: u64, timeout: u64) {
        self.cursors
            .retain(|_, cursor| cursor.revision == revision && now - cursor.created < timeout);
    }

    fn after(
        &self,
        query: &DataListQuery,
        revision: u64,
    ) -> Result<Option<&DrawKey>, TransactionError> {
        let Some(token) = &query.cursor else {
            return Ok(None);
        };
        let number = (token.len() == 32)
            .then(|| u128::from_str_radix(token, 16).ok())
            .flatten();
        let cursor = number
            .and_then(|number| self.cursors.get(&number))
            .ok_or_else(stale)?;
        if cursor.namespace != query.namespace
            || cursor.limit != query.limit
            || cursor.revision != revision
        {
            return Err(stale());
        }
        Ok(Some(&cursor.after))
    }

    fn install(
        &mut self,
        query: &DataListQuery,
        revision: u64,
        after: DrawKey,
        now: u64,
        limit: u32,
    ) -> Result<String, TransactionError> {
        if let Some((token, _)) = self.cursors.iter().find(|(_, cursor)| {
            cursor.namespace == query.namespace
                && cursor.limit == query.limit
                && cursor.revision == revision
                && cursor.after == after
        }) {
            return Ok(format!("{token:032x}"));
        }
        if self.cursors.len() >= limit as usize {
            let oldest = *self
                .cursors
                .iter()
                .min_by_key(|(_, cursor)| (cursor.created, cursor.sequence))
                .unwrap()
                .0;
            self.cursors.remove(&oldest);
        }
        let token = loop {
            let high = getrandom::u64().map_err(|e| failure(data_code::INTERNAL, e.to_string()))?;
            let low = getrandom::u64().map_err(|e| failure(data_code::INTERNAL, e.to_string()))?;
            let token = (u128::from(high) << 64) | u128::from(low);
            if token != 0 && !self.cursors.contains_key(&token) {
                break token;
            }
        };
        self.sequence = self.sequence.saturating_add(1);
        self.cursors.insert(
            token,
            Cursor {
                namespace: query.namespace.clone(),
                limit: query.limit,
                revision,
                after,
                created: now,
                sequence: self.sequence,
            },
        );
        Ok(format!("{token:032x}"))
    }
}

fn stale() -> TransactionError {
    failure(
        data_code::STALE_CURSOR,
        "listing changed, cursor expired or cursor does not match its query",
    )
}

impl StreamingStore {
    pub fn cursor_len(&self) -> usize {
        self.queries.cursors.len()
    }

    pub(super) fn list(
        &mut self,
        query: &DataListQuery,
        now: u64,
    ) -> Result<DataReply, TransactionError> {
        if query.limit > self.limits.list_items {
            return Err(failure(
                data_code::BUDGET,
                "list item count exceeds negotiated limit",
            ));
        }
        let mut items = Vec::new();
        let mut bytes = 256usize;
        let mut more = false;
        let (revision, mut after) = {
            let registry = self.registry.lock().unwrap_or_else(|p| p.into_inner());
            let revision = registry.revision();
            (revision, self.queries.after(query, revision)?.cloned())
        };
        loop {
            let candidate = {
                let registry = self.registry.lock().unwrap_or_else(|p| p.into_inner());
                if registry.revision() != revision {
                    return Err(stale());
                }
                let legacy = registry
                    .legacy()
                    .iter_namespace_after(&query.namespace, after.as_ref())
                    .next();
                let data = registry
                    .datasets_namespace_after(&query.namespace, after.as_ref())
                    .next();
                let take_legacy = match (legacy, data) {
                    (Some((a, _)), Some((b, _))) => a < b,
                    (Some(_), None) => true,
                    (None, Some(_)) => false,
                    (None, None) => break,
                };
                let key = if take_legacy {
                    legacy.unwrap().0
                } else {
                    data.unwrap().0
                };
                if items.len() >= query.limit as usize {
                    more = true;
                    break;
                }
                if key.0.len().saturating_add(key.1.len()) > DATA_METADATA_BYTES {
                    if items.is_empty() {
                        return Err(summary_budget());
                    }
                    more = true;
                    break;
                }
                let object = if take_legacy {
                    let item = legacy.unwrap().1;
                    SummaryOwner::Legacy {
                        kind: item.entity.kind,
                        generation: item.generation,
                        revision: item.revision,
                        points: item.entity.point_count() as u64,
                        instances: item.entity.instance_count() as u64,
                    }
                } else {
                    let item = data.unwrap().1;
                    let count = item.payload.counts();
                    SummaryOwner::Dataset {
                        kind: item.kind,
                        revision: item.revision,
                        scans: count.scans,
                        pages: count.pages,
                        source_samples: count.source_samples,
                        valid_samples: count.valid_samples,
                        payload_bytes: count.payload_bytes,
                    }
                };
                DrawSummary {
                    namespace: key.0.clone(),
                    id: key.1.clone(),
                    object,
                }
            };
            #[cfg(test)]
            crate::draw_data::concurrency_tests::checkpoint("list_summary");
            candidate
                .validate()
                .map_err(|error| failure(data_code::INVALID, error))?;
            let size = serde_json::to_vec(&candidate)
                .map_err(|error| failure(data_code::INTERNAL, error.to_string()))?
                .len()
                + 1;
            if bytes + size > DATA_METADATA_BYTES {
                if items.is_empty() {
                    return Err(summary_budget());
                }
                more = true;
                break;
            }
            bytes += size;
            after = Some((candidate.namespace.clone(), candidate.id.clone()));
            items.push(candidate);
        }
        if self
            .registry
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .revision()
            != revision
        {
            return Err(stale());
        }
        let next_cursor = if more {
            let last = items.last().unwrap();
            Some(self.queries.install(
                query,
                revision,
                (last.namespace.clone(), last.id.clone()),
                now,
                self.limits.cursors,
            )?)
        } else {
            None
        };
        let page = DrawSummaryPage {
            revision: SCHEMA_REVISION,
            listing_revision: revision,
            items,
            next_cursor,
        };
        Ok(DataReply {
            metadata: page
                .to_json()
                .map_err(|error| failure(data_code::INTERNAL, error))?,
            ..DataReply::new(self.epoch)
        })
    }

    pub(super) fn status(
        &self,
        request: &DataRequest,
        query: &DataStatusQuery,
    ) -> Result<DataReply, TransactionError> {
        let key = request
            .data_key()
            .map_err(|error| failure(data_code::INVALID, error))?;
        let lookup = (key.namespace.clone(), key.id.clone());
        let (retained, kind, live_bytes, datasets, visibility) = {
            let registry = self.registry.lock().unwrap_or_else(|p| p.into_inner());
            let entry = registry.get(&lookup);
            if entry.is_none() && registry.legacy().get(&lookup.0, &lookup.1).is_some() {
                return Err(failure(
                    data_code::CONFLICT,
                    "drawing is owned by the legacy API",
                ));
            }
            (
                entry.map(|entry| entry.revision),
                entry.map(|entry| entry.kind),
                registry
                    .datasets()
                    .map(|(_, entry)| entry.payload.counts().payload_bytes)
                    .sum::<u64>(),
                registry.dataset_len(),
                entry.map_or_else(
                    || GenerationVisibility::Unavailable {
                        reason: "dataset is not retained".into(),
                    },
                    |entry| {
                        self.backend.visibility_for(
                            &registry,
                            &key,
                            DatasetRevision {
                                generation: query
                                    .generation
                                    .or_else(|| request_base(request).map(|base| base.generation))
                                    .unwrap_or(entry.revision.generation),
                                ..entry.revision
                            },
                            query.view,
                        )
                    },
                ),
            )
        };
        let pending = self
            .transactions
            .iter()
            .filter(|(_, tx)| tx.record.key() == &key)
            .max_by_key(|(id, tx)| (tx.last_activity, **id));
        let completed = self
            .completed
            .values()
            .filter(|done| done.record.key() == &key)
            .max_by_key(|done| done.sequence);
        let record = pending
            .map(|(_, tx)| tx.record.as_ref())
            .or_else(|| completed.map(|done| done.record.as_ref()));
        let kind = kind
            .or_else(|| {
                record.map(|record| record.descriptor.value().shape.kind())
            })
            .ok_or_else(|| {
                failure(
                    data_code::NOT_FOUND,
                    "dataset is not retained, pending or in recent history",
                )
            })?;
        if let Some(base) = request_base(request) {
            if retained.is_none_or(|retained| retained.incarnation != base.incarnation) {
                return Err(failure(
                    data_code::CONFLICT,
                    "status incarnation does not match retained data",
                ));
            }
            if query
                .generation
                .is_some_and(|generation| generation != base.generation)
            {
                return Err(failure(
                    data_code::INVALID,
                    "conflicting status generation selectors",
                ));
            }
        }
        let generation = query
            .generation
            .or_else(|| request_base(request).map(|base| base.generation));
        if generation.is_some_and(|generation| {
            retained.is_none_or(|retained| generation > retained.generation)
        }) {
            return Err(failure(
                data_code::NOT_FOUND,
                "requested generation has not been retained",
            ));
        }
        let requested = retained.map(|retained| DatasetRevision {
            generation: generation.unwrap_or(retained.generation),
            ..retained
        });
        let transfer = pending.map(|(id, tx)| TransferProgress {
            transaction: id.1,
            ..tx.progress()
        });
        let failure = requested
            .and_then(|revision| self.backend.render_failure(&key, revision))
            .or_else(|| {
                self.maintenance_failures
                    .get(&lookup)
                    .filter(|failure| retained == Some(failure.revision))
                    .filter(|failure| completed.is_none_or(|done| failure.sequence > done.sequence))
                    .map(|failure| failure.failure.clone())
                    .or_else(|| {
                        completed.and_then(|done| {
                            done.result.as_ref().err().map(|error| DataFailure {
                                phase: done.phase,
                                code: error.code,
                                reason: error.message.clone(),
                            })
                        })
                    })
            });
        let usage = self.usage();
        let resources = DataResourceUsage {
            cpu_bytes: usage.memory.committed(),
            gpu_bytes: self.gpu.usage().bytes,
            incomplete_bytes: usage.incomplete_bytes,
            staged_bytes: usage.staged_bytes,
            resident_page_bytes: usage.memory.page_bytes,
            pinned_retired_bytes: usage
                .memory
                .page_bytes
                .saturating_sub(live_bytes + usage.incomplete_bytes + usage.staged_bytes),
            metadata_bytes: usage.memory.metadata_bytes,
            pending_transactions: usage.transactions as u32,
            writer_leases: usage.writers as u32,
            retained_datasets: datasets as u32,
        };
        resources
            .validate_against(&self.limits)
            .map_err(|error| super::failure(data_code::INTERNAL, error))?;
        let paging = retained.and_then(|revision| self.backend.page_residency(&key, revision));
        let status = DatasetStatus {
            revision: SCHEMA_REVISION,
            key,
            kind,
            retained,
            requested,
            visibility,
            transfer,
            failure,
            resources,
            counters: DataCounters {
                received_bytes: self.counters.received_bytes,
                rejected_bytes: self.counters.rejected_bytes,
                rejected_transactions: self.counters.rejected_transactions,
                dropped_scans: self.counters.dropped_scans,
                superseded_scans: self.counters.superseded_scans,
                evicted_scans: self.counters.evicted_scans,
            },
            paging,
        };
        Ok(DataReply {
            metadata: status
                .to_json()
                .map_err(|error| super::failure(data_code::INTERNAL, error))?,
            stage: if matches!(status.visibility, GenerationVisibility::Visible { .. }) {
                data_stage::VISIBLE
            } else if retained.is_some() {
                data_stage::RETAINED
            } else {
                data_stage::NONE
            },
            incarnation: retained.map_or(0, |revision| revision.incarnation),
            stored_generation: retained.map_or(0, |revision| revision.generation),
            visible_generation: if matches!(status.visibility, GenerationVisibility::Visible { .. })
            {
                requested.map_or(0, |revision| revision.generation)
            } else {
                0
            },
            view_id: status
                .visibility
                .target()
                .map(|target| target.view)
                .or(query.view)
                .map_or(0, |view| view.id),
            view_epoch: status
                .visibility
                .target()
                .map(|target| target.view)
                .or(query.view)
                .map_or(0, |view| view.epoch),
            ..DataReply::new(self.epoch)
        })
    }

    pub(super) fn attach(
        &mut self,
        request: &DataRequest,
        attachment: &ProviderAttachment,
    ) -> Result<DataReply, TransactionError> {
        let key = request
            .data_key()
            .map_err(|error| failure(data_code::INVALID, error))?;
        let revision = request_base(request).ok_or_else(|| {
            failure(
                data_code::CONFLICT,
                "provider attachment requires an exact base",
            )
        })?;
        let cancelled = std::sync::atomic::AtomicBool::new(false);
        let deadline = std::time::Instant::now() + ATTACH_DEADLINE;
        self.backend.attach_named_provider(
            &key,
            revision,
            &attachment.provider,
            crate::draw_data::PageRequest::new(deadline, &cancelled),
        )?;
        Ok(DataReply {
            lease: request.lease,
            stage: data_stage::RETAINED,
            incarnation: revision.incarnation,
            stored_generation: revision.generation,
            ..DataReply::new(self.epoch)
        })
    }

    pub(super) fn presentation(
        &mut self,
        request: &DataRequest,
        presentation: &Presentation,
    ) -> Result<DataReply, TransactionError> {
        let key = request
            .data_key()
            .map_err(|error| failure(data_code::INVALID, error))?;
        let expected = request_base(request).ok_or_else(|| {
            failure(
                data_code::CONFLICT,
                "presentation update requires an exact base",
            )
        })?;
        let (permit, base) = {
            let mut registry = self.registry.lock().unwrap_or_else(|p| p.into_inner());
            let entry = registry
                .get(&(key.namespace.clone(), key.id.clone()))
                .ok_or_else(|| failure(data_code::NOT_FOUND, "dataset does not exist"))?;
            let (kind, base) = (entry.kind, entry.payload.clone());
            let permit = registry.begin_dataset(&key, kind, Some(expected))?;
            (permit, base)
        };
        let mut publication = Publication::new(self.registry.clone(), permit);
        let generation = base.restyle(&self.budget, presentation)?;
        let revision = publication.publish(generation)?;
        Ok(DataReply {
            lease: request.lease,
            stage: data_stage::RETAINED,
            incarnation: revision.incarnation,
            stored_generation: revision.generation,
            ..DataReply::new(self.epoch)
        })
    }
}

fn summary_budget() -> TransactionError {
    failure(
        data_code::BUDGET,
        "one legacy summary exceeds the data metadata envelope; use the legacy list route",
    )
}
