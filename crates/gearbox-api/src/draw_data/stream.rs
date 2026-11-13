use std::collections::BTreeMap;
use std::sync::Arc;

use super::memory::tree_allowance;
use super::transaction::{BeginRecord, Transaction, request_base};
use super::{
    Charge, CpuBudget, DrawingBackend, MemoryClass, MemoryUsage, SharedDrawingStore,
    TransactionError,
};
use crate::wire::draw::ns_within;
use crate::wire::draw_data::*;

type TransactionKey = (u64, u64);

const REQUEST_WORKSPACE_BYTES: u64 = 2 << 20;

mod field;
mod query;
mod retention;

struct Writer {
    last_activity: u64,
    highest_transaction: u64,
}

struct Completed {
    record: Arc<BeginRecord>,
    result: Result<Option<DatasetRevision>, TransactionError>,
    sequence: u64,
    phase: DataPhase,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamUsage {
    pub writers: usize,
    pub transactions: usize,
    pub completion_history: usize,
    pub incomplete_bytes: u64,
    pub staged_bytes: u64,
    pub declared_bytes: u64,
    pub memory: MemoryUsage,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamCounters {
    pub received_bytes: u64,
    pub rejected_bytes: u64,
    pub rejected_transactions: u64,
    pub expired_transactions: u64,
    pub cancelled_transactions: u64,
    pub superseded_transactions: u64,
    pub retained_transactions: u64,
    pub dropped_scans: u64,
    pub superseded_scans: u64,
    pub evicted_scans: u64,
}

/// Single service-thread owner; immutable publications use a separate short registry lock.
pub struct StreamingStore {
    epoch: u64,
    limits: DataLimits,
    budget: CpuBudget,
    gpu: super::GpuBudget,
    backend: DrawingBackend,
    registry: SharedDrawingStore,
    writers: BTreeMap<u64, Writer>,
    transactions: BTreeMap<TransactionKey, Transaction>,
    completed: BTreeMap<TransactionKey, Completed>,
    last_now: u64,
    sequence: u64,
    counters: StreamCounters,
    barrier_revision: u64,
    queries: query::CursorBook,
    simulation_time_s: f64,
    maintenance_failures: BTreeMap<crate::draw::DrawKey, retention::MaintenanceFailure>,
    _tables: Charge,
    _request_workspace: Charge,
    _owner: super::backend::ServiceOwner,
}

impl StreamingStore {
    pub fn new(epoch: u64, limits: DataLimits) -> Result<Self, TransactionError> {
        Self::with_backend(epoch, DrawingBackend::new(limits)?)
    }

    pub fn with_backend(epoch: u64, backend: DrawingBackend) -> Result<Self, TransactionError> {
        if epoch == 0 {
            return Err(failure(data_code::INVALID, "host epoch must be nonzero"));
        }
        let budget = backend.budget().clone();
        let gpu = backend.gpu().clone();
        let limits = backend.limits().clone();
        let registry = backend.registry();
        let owner = backend.claim()?;
        let bytes = size_of::<Self>() as u64
            + tree_allowance::<u64, Writer>(limits.writers as usize)
            + tree_allowance::<TransactionKey, Transaction>(limits.transactions as usize)
            + tree_allowance::<TransactionKey, Completed>(limits.completion_history as usize)
            + u64::from(limits.completion_history) * 512;
        let bytes = bytes
            + query::CursorBook::allowance(limits.cursors)
            + tree_allowance::<crate::draw::DrawKey, retention::MaintenanceFailure>(
                limits.datasets as usize,
            )
            + u64::from(limits.datasets) * (382 + 512);
        let mut reservation = budget.reserve(bytes + REQUEST_WORKSPACE_BYTES)?;
        let tables = reservation.charge(bytes, MemoryClass::Metadata)?;
        let request_workspace =
            reservation.charge(REQUEST_WORKSPACE_BYTES, MemoryClass::Scratch)?;
        Ok(Self {
            epoch,
            limits,
            budget,
            gpu,
            backend,
            registry,
            writers: BTreeMap::new(),
            transactions: BTreeMap::new(),
            completed: BTreeMap::new(),
            last_now: 0,
            sequence: 0,
            counters: StreamCounters::default(),
            barrier_revision: 0,
            queries: query::CursorBook::default(),
            simulation_time_s: 0.0,
            maintenance_failures: BTreeMap::new(),
            _tables: tables,
            _request_workspace: request_workspace,
            _owner: owner,
        })
    }

    pub fn registry(&self) -> SharedDrawingStore {
        self.registry.clone()
    }
    pub fn budget(&self) -> &CpuBudget {
        &self.budget
    }
    pub fn limits(&self) -> &DataLimits {
        &self.limits
    }
    pub fn counters(&self) -> StreamCounters {
        self.counters
    }

    pub fn capabilities(&self) -> DataCapabilities {
        let mut capabilities = DataCapabilities::wire_only(self.epoch);
        capabilities.limits = self.limits.clone();
        capabilities.retained = vec![DatasetKind::Cloud, DatasetKind::Octree, DatasetKind::Surfels, DatasetKind::MeshBlocks, DatasetKind::Volume, DatasetKind::Elevation];
        capabilities.features.atomic_generations = true;
        capabilities.features.delta_updates = true;
        capabilities.features.presentation_updates = true;
        capabilities.features.unified_listing = true;
        capabilities.features.accumulation = true;
        capabilities.rendered = self.backend.renderer_kinds();
        capabilities.features.per_view_visibility = !capabilities.rendered.is_empty();
        capabilities.page_providers = self.backend.provider_names();
        capabilities
    }

    pub fn advance_simulation(&mut self, now_s: f64) -> Result<usize, TransactionError> {
        if !now_s.is_finite() || now_s < 0.0 {
            return Err(failure(
                data_code::INVALID,
                "simulation time must be finite and nonnegative",
            ));
        }
        let rewind = now_s < self.simulation_time_s;
        self.simulation_time_s = now_s;
        if rewind {
            while let Some(key) = self.transactions.keys().next().copied() {
                self.finish(
                    key,
                    Err(failure(data_code::EXPIRED, "simulation clock rewound")),
                );
            }
        }
        let mut removed = 0;
        loop {
            let expired = {
                let registry = self.registry.lock().unwrap_or_else(|p| p.into_inner());
                registry
                    .datasets()
                    .find(|(_, entry)| {
                        let descriptor = entry.payload.descriptor();
                        rewind
                            && (!matches!(descriptor.stream, StreamMode::RetainedMap)
                                || descriptor.lifetime_s.is_some())
                            || descriptor.lifetime_s.is_some_and(|lifetime| {
                                now_s - entry.payload.retained_at_s() >= lifetime
                            })
                    })
                    .map(|(key, entry)| (key.clone(), entry.revision, entry.payload.clone()))
            };
            let Some((key, revision, pinned)) = expired else {
                break;
            };
            {
                let mut registry = self.registry.lock().unwrap_or_else(|p| p.into_inner());
                if registry
                    .get(&key)
                    .is_some_and(|entry| entry.revision == revision)
                {
                    removed += registry.delete(&key)?;
                }
            }
            drop(pinned);
        }
        self.expire_accumulation(now_s)?;
        Ok(removed)
    }

    pub fn usage(&self) -> StreamUsage {
        StreamUsage {
            writers: self.writers.len(),
            transactions: self.transactions.len(),
            completion_history: self.completed.len(),
            incomplete_bytes: self
                .transactions
                .values()
                .map(|tx| tx.incomplete_bytes)
                .sum(),
            staged_bytes: self.transactions.values().map(|tx| tx.staged_bytes).sum(),
            declared_bytes: self
                .transactions
                .values()
                .map(|tx| tx.record.total_bytes)
                .sum(),
            memory: self.budget.usage(),
        }
    }

    pub fn maintain(&mut self, now_ms: u64) -> Result<(), TransactionError> {
        self.backend.prune_renderer_reports();
        if now_ms < self.last_now {
            return Err(failure(
                data_code::INVALID,
                "transport clock moved backwards",
            ));
        }
        self.last_now = now_ms;
        self.reap_retired();
        let (barrier, revision) = {
            let registry = self.registry.lock().unwrap_or_else(|p| p.into_inner());
            (registry.barrier_revision(), registry.revision())
        };
        self.queries
            .maintain(now_ms, revision, self.limits.cursor_timeout_ms);
        if self.barrier_revision != barrier {
            self.completed.clear();
            self.transactions.retain(|_, tx| tx.live());
            self.barrier_revision = barrier;
        }
        while let Some(lease) = self.writers.iter().find_map(|(key, writer)| {
            (now_ms - writer.last_activity >= self.limits.lease_timeout_ms).then_some(*key)
        }) {
            let count = self
                .transactions
                .keys()
                .filter(|(owner, _)| *owner == lease)
                .count() as u64;
            self.counters.expired_transactions =
                self.counters.expired_transactions.saturating_add(count);
            self.close(lease);
        }
        while let Some(key) = self.transactions.iter().find_map(|(key, tx)| {
            (now_ms - tx.last_activity >= self.limits.lease_timeout_ms || !tx.live())
                .then_some(*key)
        }) {
            self.counters.expired_transactions =
                self.counters.expired_transactions.saturating_add(1);
            self.finish(
                key,
                Err(failure(
                    data_code::EXPIRED,
                    "transaction expired or publication revoked",
                )),
            );
        }
        let registry = self.registry.lock().unwrap_or_else(|p| p.into_inner());
        self.maintenance_failures.retain(|key, failure| {
            registry
                .get(key)
                .is_some_and(|entry| entry.revision == failure.revision)
        });
        self.completed.retain(|_, done| match &done.result {
            Ok(Some(revision)) => registry
                .get(&(
                    done.record.key().namespace.clone(),
                    done.record.key().id.clone(),
                ))
                .is_some_and(|entry| entry.revision.incarnation == revision.incarnation),
            _ => true,
        });
        Ok(())
    }

    pub fn handle(&mut self, request: &DataRequest, now_ms: u64) -> DataReply {
        if request.operation != data_op::CAPABILITIES && request.host_epoch != self.epoch {
            return self.error(failure(
                data_code::STALE_EPOCH,
                "host restarted; negotiate a new writer",
            ));
        }
        if let Err(error) = self.maintain(now_ms) {
            return self.error(error);
        }
        let operation = match request.operation() {
            Ok(operation) => operation,
            Err(message) => {
                let error = failure(data_code::INVALID, message);
                self.counters.rejected_bytes = self
                    .counters
                    .rejected_bytes
                    .saturating_add(request.payload.len() as u64);
                let key = (request.lease, request.transaction);
                if self
                    .transactions
                    .get(&key)
                    .is_some_and(|tx| tx.record.address_matches(request))
                {
                    self.finish(key, Err(error.clone()));
                }
                return self.error(error);
            }
        };
        let result = self.dispatch(request, operation, now_ms);
        match result {
            Ok(reply) => reply,
            Err(error) => {
                self.counters.rejected_bytes = self
                    .counters
                    .rejected_bytes
                    .saturating_add(request.payload.len() as u64);
                self.error(error)
            }
        }
    }

    fn dispatch(
        &mut self,
        request: &DataRequest,
        operation: DataOperation,
        now: u64,
    ) -> Result<DataReply, TransactionError> {
        if matches!(operation, DataOperation::Capabilities) {
            let capabilities = self.capabilities();
            return Ok(DataReply {
                metadata: capabilities
                    .to_json()
                    .map_err(|message| failure(data_code::INTERNAL, message))?,
                ..DataReply::new(self.epoch)
            });
        }
        if matches!(operation, DataOperation::OpenWriter) {
            if self.writers.len() >= self.limits.writers as usize {
                return Err(failure(data_code::BUDGET, "writer limit reached"));
            }
            let lease = loop {
                let token = getrandom::u64()
                    .map_err(|error| failure(data_code::INTERNAL, error.to_string()))?;
                if token != 0 && !self.writers.contains_key(&token) {
                    break token;
                }
            };
            self.writers.insert(
                lease,
                Writer {
                    last_activity: now,
                    highest_transaction: 0,
                },
            );
            return Ok(DataReply {
                lease,
                ..DataReply::new(self.epoch)
            });
        }
        if request.lease != 0 {
            self.writers
                .get_mut(&request.lease)
                .ok_or_else(|| failure(data_code::EXPIRED, "writer lease is missing or expired"))?
                .last_activity = now;
        }
        match operation {
            DataOperation::CloseWriter => {
                self.close(request.lease);
                Ok(DataReply::new(self.epoch))
            }
            DataOperation::Begin(begin) => self.begin(request, &begin, now),
            DataOperation::DeclareScan(_)
            | DataOperation::DeclarePage(_)
            | DataOperation::Chunk
            | DataOperation::RemovePage
            | DataOperation::RemoveScan
            | DataOperation::Commit
            | DataOperation::Cancel => self.edit(request, operation, now),
            DataOperation::Delete => self.delete(request),
            DataOperation::List(query) => self.list(&query, now),
            DataOperation::Status(query) => self.status(request, &query),
            DataOperation::PatchPresentation(presentation) => {
                self.presentation(request, &presentation)
            }
            DataOperation::Clear(filter) => {
                self.registry
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clear(&filter.namespace)?;
                self.purge(|key| ns_within(&key.namespace, &filter.namespace));
                self.reap_retired();
                Ok(DataReply::new(self.epoch))
            }
            DataOperation::AttachProvider(attachment) => self.attach(request, &attachment),
            DataOperation::FieldQuery(query) => self.field(request, &query),
            _ => Err(failure(
                data_code::UNSUPPORTED,
                "dataset query/presentation service is not integrated",
            )),
        }
    }

    fn begin(
        &mut self,
        request: &DataRequest,
        begin: &BeginMetadata,
        now: u64,
    ) -> Result<DataReply, TransactionError> {
        let key = (request.lease, request.transaction);
        if let Some(tx) = self.transactions.get(&key) {
            if tx.record.matches(request) && tx.record.address_matches(request) {
                self.transactions.get_mut(&key).unwrap().last_activity = now;
                return Ok(self.received(key));
            }
            let error = failure(
                data_code::CONFLICT,
                "transaction ID has a conflicting BEGIN",
            );
            self.finish(key, Err(error.clone()));
            return Err(error);
        }
        if let Some(done) = self.completed.get(&key) {
            if !done.record.matches(request) || !done.record.address_matches(request) {
                return Err(failure(
                    data_code::CONFLICT,
                    "completed transaction has a different BEGIN",
                ));
            }
            if matches!(done.result, Ok(None)) {
                return Err(failure(data_code::EXPIRED, "transaction was cancelled"));
            }
            return self.completed_reply(key, done);
        }
        let writer = self.writers.get_mut(&request.lease).unwrap();
        if request.transaction <= writer.highest_transaction {
            return Err(failure(
                data_code::HISTORY_EXPIRED,
                "transaction result is no longer retained",
            ));
        }
        writer.highest_transaction = request.transaction;
        let record = BeginRecord::new(&self.budget, request, begin)?;
        let result = self.admit(record.clone(), request.lease, now);
        match result {
            Ok(transaction) => {
                self.transactions.insert(key, transaction);
                Ok(self.received(key))
            }
            Err(error) => {
                self.remember(key, record, Err(error.clone()), DataPhase::Receiving);
                Err(error)
            }
        }
    }

    fn admit(
        &mut self,
        record: Arc<BeginRecord>,
        lease: u64,
        now: u64,
    ) -> Result<Transaction, TransactionError> {
        if record.mode == CommitMode::Delta
            && matches!(record.descriptor.value().stream, StreamMode::Latest)
        {
            return Err(failure(
                data_code::INVALID,
                "latest-frame submissions must replace a complete frame",
            ));
        }
        let supersedes = |tx: &Transaction| {
            matches!(record.descriptor.value().stream, StreamMode::Latest)
                && tx.record.key() == record.key()
                && matches!(tx.record.descriptor.value().stream, StreamMode::Latest)
        };
        let total_count = self
            .transactions
            .values()
            .filter(|tx| !supersedes(tx))
            .count();
        let writer_count = self
            .transactions
            .iter()
            .filter(|((owner, _), tx)| *owner == lease && !supersedes(tx))
            .count();
        let writer_bytes: u64 = self
            .transactions
            .iter()
            .filter(|((owner, _), tx)| *owner == lease && !supersedes(tx))
            .map(|(_, tx)| tx.record.total_bytes)
            .sum();
        if total_count >= self.limits.transactions as usize
            || writer_count >= self.limits.writer_transactions as usize
            || record.total_bytes > self.limits.writer_staged_bytes.saturating_sub(writer_bytes)
        {
            return Err(failure(
                data_code::BUDGET,
                "pending transaction or writer payload limit reached",
            ));
        }
        let transaction = Transaction::new(
            self.registry.clone(),
            &self.budget,
            &self.limits,
            record.clone(),
            now,
        )?;
        while let Some(key) = self
            .transactions
            .iter()
            .find_map(|(key, tx)| supersedes(tx).then_some(*key))
        {
            self.counters.superseded_transactions =
                self.counters.superseded_transactions.saturating_add(1);
            self.finish(
                key,
                Err(failure(
                    data_code::SUPERSEDED,
                    "newer latest-frame submission accepted",
                )),
            );
        }
        Ok(transaction)
    }

    fn edit(
        &mut self,
        request: &DataRequest,
        operation: DataOperation,
        now: u64,
    ) -> Result<DataReply, TransactionError> {
        let key = (request.lease, request.transaction);
        if let Some(done) = self.completed.get(&key) {
            if !done.record.address_matches(request) {
                return Err(failure(
                    data_code::CONFLICT,
                    "completed transaction address differs",
                ));
            }
            if request.operation != data_op::CANCEL && matches!(done.result, Ok(None)) {
                return Err(failure(data_code::EXPIRED, "transaction was cancelled"));
            }
            return if matches!(operation, DataOperation::Commit | DataOperation::Cancel) {
                self.completed_reply(key, done)
            } else {
                Err(failure(
                    data_code::EXPIRED,
                    "transaction is already complete",
                ))
            };
        }
        let Some(tx) = self.transactions.get(&key) else {
            let code = if request.transaction <= self.writers[&request.lease].highest_transaction {
                data_code::HISTORY_EXPIRED
            } else {
                data_code::NOT_FOUND
            };
            return Err(failure(code, "transaction is not pending"));
        };
        if !tx.record.address_matches(request) {
            return Err(failure(
                data_code::CONFLICT,
                "transaction key or base differs",
            ));
        }
        let result = self.apply_edit(key, request, operation, now);
        match result {
            Ok(Some(revision)) => {
                self.counters.retained_transactions =
                    self.counters.retained_transactions.saturating_add(1);
                self.finish(key, Ok(Some(revision)));
                self.completed_reply(key, &self.completed[&key])
            }
            Ok(None) if request.operation == data_op::CANCEL => {
                self.counters.cancelled_transactions =
                    self.counters.cancelled_transactions.saturating_add(1);
                self.finish(key, Ok(None));
                self.completed_reply(key, &self.completed[&key])
            }
            Ok(None) => Ok(self.received(key)),
            Err(error) => {
                self.finish(key, Err(error.clone()));
                Err(error)
            }
        }
    }

    fn apply_edit(
        &mut self,
        key: TransactionKey,
        request: &DataRequest,
        operation: DataOperation,
        now: u64,
    ) -> Result<Option<DatasetRevision>, TransactionError> {
        if let DataOperation::DeclarePage(page) = &operation {
            if self.transactions[&key].page_declaration(page)? {
                let total: u64 = self
                    .transactions
                    .values()
                    .map(|tx| tx.incomplete_bytes)
                    .sum();
                let writer: u64 = self
                    .transactions
                    .iter()
                    .filter(|((owner, _), _)| *owner == key.0)
                    .map(|(_, tx)| tx.incomplete_bytes)
                    .sum();
                if page.byte_len > self.limits.page_bytes
                    || page.items > self.limits.page_points
                    || page.byte_len > self.limits.incomplete_bytes.saturating_sub(total)
                    || page.byte_len > self.limits.writer_incomplete_bytes.saturating_sub(writer)
                {
                    return Err(failure(
                        data_code::BUDGET,
                        "page or incomplete payload quota exceeded",
                    ));
                }
            }
        }
        if request.payload.len() > self.limits.chunk_bytes as usize {
            return Err(failure(data_code::BUDGET, "chunk exceeds negotiated limit"));
        }
        let tx = self.transactions.get_mut(&key).unwrap();
        tx.last_activity = now;
        let address = PageAddress {
            scan_key: request.scan_key,
            page_id: request.page_id,
        };
        match operation {
            DataOperation::DeclareScan(scan) => tx.declare_scan(&scan)?,
            DataOperation::DeclarePage(page) => tx.declare_page(page)?,
            DataOperation::Chunk => {
                let before = tx.received_bytes;
                let result = tx.chunk(address, request.offset, &request.payload);
                self.counters.received_bytes = self
                    .counters
                    .received_bytes
                    .saturating_add(tx.received_bytes - before);
                result?;
            }
            DataOperation::RemovePage => tx.remove_page(address)?,
            DataOperation::RemoveScan => tx.remove_scan(request.scan_key)?,
            DataOperation::Commit => {
                let (revision, retention) = tx.commit(&self.limits, self.simulation_time_s)?;
                self.counters.evicted_scans = self
                    .counters
                    .evicted_scans
                    .saturating_add(retention.evicted);
                self.counters.dropped_scans = self
                    .counters
                    .dropped_scans
                    .saturating_add(retention.dropped);
                return Ok(Some(revision));
            }
            DataOperation::Cancel => {}
            _ => unreachable!(),
        }
        Ok(None)
    }

    fn delete(&mut self, request: &DataRequest) -> Result<DataReply, TransactionError> {
        let key = request
            .data_key()
            .map_err(|error| failure(data_code::INVALID, error))?;
        {
            let mut registry = self.registry.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(base) = request_base(request) {
                if registry
                    .get(&(key.namespace.clone(), key.id.clone()))
                    .is_none_or(|entry| entry.revision != base)
                {
                    return Err(failure(
                        data_code::CONFLICT,
                        "delete base generation changed",
                    ));
                }
            }
            registry.delete(&(key.namespace.clone(), key.id.clone()))?;
        }
        self.purge(|candidate| candidate == &key);
        self.reap_retired();
        Ok(DataReply::new(self.epoch))
    }

    fn reap_retired(&self) {
        loop {
            let payload = self
                .registry
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .pop_retired();
            let Some(payload) = payload else {
                break;
            };
            drop(payload);
        }
    }

    fn purge(&mut self, matches: impl Fn(&DataKey) -> bool) {
        self.transactions.retain(|_, tx| !matches(tx.record.key()));
        self.completed.retain(|_, done| !matches(done.record.key()));
    }

    fn close(&mut self, lease: u64) {
        let dropped: u64 = self
            .transactions
            .iter()
            .filter(|((owner, _), tx)| {
                *owner == lease
                    && !matches!(tx.record.descriptor.value().stream, StreamMode::RetainedMap)
            })
            .map(|(_, tx)| u64::from(tx.record.scans))
            .sum();
        self.counters.dropped_scans = self.counters.dropped_scans.saturating_add(dropped);
        self.transactions.retain(|(owner, _), _| *owner != lease);
        self.completed.retain(|(owner, _), _| *owner != lease);
        self.writers.remove(&lease);
    }

    fn finish(
        &mut self,
        key: TransactionKey,
        result: Result<Option<DatasetRevision>, TransactionError>,
    ) {
        if let Some(tx) = self.transactions.remove(&key) {
            let phase = tx.phase;
            if !matches!(tx.record.descriptor.value().stream, StreamMode::RetainedMap) {
                if result
                    .as_ref()
                    .is_err_and(|error| error.code == data_code::SUPERSEDED)
                {
                    self.counters.superseded_scans = self
                        .counters
                        .superseded_scans
                        .saturating_add(u64::from(tx.record.scans));
                } else if !matches!(result, Ok(Some(_))) {
                    self.counters.dropped_scans = self
                        .counters
                        .dropped_scans
                        .saturating_add(u64::from(tx.record.scans));
                }
            }
            let record = tx.record.clone();
            drop(tx);
            self.remember(key, record, result, phase);
        }
    }

    fn remember(
        &mut self,
        key: TransactionKey,
        record: Arc<BeginRecord>,
        result: Result<Option<DatasetRevision>, TransactionError>,
        phase: DataPhase,
    ) {
        if result.is_err() {
            self.counters.rejected_transactions =
                self.counters.rejected_transactions.saturating_add(1);
        }
        if self.completed.len() >= self.limits.completion_history as usize {
            let oldest = *self
                .completed
                .iter()
                .min_by_key(|(_, value)| value.sequence)
                .unwrap()
                .0;
            self.completed.remove(&oldest);
        }
        let result = result.map_err(|error| {
            let reply = self.error(error);
            failure(reply.code, String::from_utf8(reply.message).unwrap())
        });
        self.sequence = self.sequence.saturating_add(1);
        self.completed.insert(
            key,
            Completed {
                record,
                result,
                sequence: self.sequence,
                phase,
            },
        );
    }

    fn completed_reply(
        &self,
        key: TransactionKey,
        done: &Completed,
    ) -> Result<DataReply, TransactionError> {
        let revision = done.result.as_ref().map_err(Clone::clone)?;
        Ok(DataReply {
            lease: key.0,
            transaction: key.1,
            stage: if revision.is_some() {
                data_stage::RETAINED
            } else {
                data_stage::NONE
            },
            incarnation: revision.map_or(0, |revision| revision.incarnation),
            stored_generation: revision.map_or(0, |revision| revision.generation),
            ..DataReply::new(self.epoch)
        })
    }

    fn received(&self, key: TransactionKey) -> DataReply {
        DataReply {
            lease: key.0,
            transaction: key.1,
            stage: data_stage::RECEIVED,
            ..DataReply::new(self.epoch)
        }
    }

    fn error(&self, error: TransactionError) -> DataReply {
        DataReply::error(self.epoch, error.code, &error.message)
    }
}

fn failure(code: u32, message: impl Into<String>) -> TransactionError {
    TransactionError::new(code, message)
}
