use datapod::{DataPod, LeWireHeader};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::time::Instant;

use super::*;
use crate::wire::draw_data::{
    DATA_CHUNK_BYTES, DATA_KEY_BYTES, DATA_METADATA_BYTES, SCHEMA_REVISION, data_op,
};

pub(super) const COMPLETION_BYTES: usize =
    size_of::<Record>() + 2 * size_of::<usize>() + REPLY_BYTES;

pub(super) struct Job {
    pub id: u64,
    pub request: Request,
    pub deadline: Instant,
    pub record: Arc<Record>,
}

pub(super) enum Request {
    Typed(DataRequest),
    Wire { bytes: Vec<u8>, operation: u32 },
}

impl Request {
    fn operation(&self) -> u32 {
        match self {
            Self::Typed(request) => request.operation,
            Self::Wire { operation, .. } => *operation,
        }
    }

    fn call(
        &self,
        client: &mut DrawDataClient,
        deadline: Instant,
    ) -> Result<DataReply, DrawDataError> {
        match self {
            Self::Typed(request) => client.request_with_deadline(request, deadline),
            Self::Wire { bytes: wire, .. } => {
                let request =
                    DataRequest::unpack_bounded(datapod::bind::type_hash::<DataRequest>(), wire)
                        .map_err(DrawDataError::Protocol)?;
                client.request_with_deadline(&request, deadline)
            }
        }
    }
}

pub(super) struct Record {
    pub shared: Weak<Shared>,
    pub outcome: Mutex<Option<Outcome>>,
    pub ready: Condvar,
    pub payload_charge: AtomicUsize,
    pub slot_held: AtomicBool,
}

impl Record {
    pub fn release_slot(&self) {
        if self.slot_held.swap(false, Ordering::AcqRel) {
            if let Some(shared) = self.shared.upgrade() {
                shared.lock().stats.outstanding -= 1;
                shared.changed.notify_all();
            }
        }
    }

    fn release_payload(&self) {
        let payload = self.payload_charge.swap(0, Ordering::AcqRel);
        if let Some(shared) = self.shared.upgrade() {
            shared.lock().stats.charged_bytes -= payload;
            shared.changed.notify_all();
        }
    }

    fn complete(&self, outcome: Outcome) {
        *self.outcome.lock().unwrap_or_else(|e| e.into_inner()) = Some(outcome);
        self.ready.notify_all();
    }
}

impl Drop for Record {
    fn drop(&mut self) {
        drop(
            self.outcome
                .get_mut()
                .unwrap_or_else(|e| e.into_inner())
                .take(),
        );
        if let Some(shared) = self.shared.upgrade() {
            let mut state = shared.lock();
            state.stats.charged_bytes -= COMPLETION_BYTES + *self.payload_charge.get_mut();
            if *self.slot_held.get_mut() {
                state.stats.outstanding -= 1;
            }
            drop(state);
            shared.changed.notify_all();
        }
    }
}

pub(super) struct State {
    pub queue: VecDeque<Job>,
    pub stats: DataPipelineStats,
    pub barrier: bool,
    pub closed: bool,
    pub next: u64,
}

pub(super) struct Shared {
    pub config: DataPipelineConfig,
    pub epoch: u64,
    pub fixed_bytes: usize,
    pub state: Mutex<State>,
    pub changed: Condvar,
}

impl Shared {
    pub fn new(
        config: DataPipelineConfig,
        epoch: u64,
        fixed: usize,
    ) -> Result<Self, DataPipelineError> {
        let queue = VecDeque::with_capacity(config.max_operations);
        let fixed = fixed + (queue.capacity() - config.max_operations) * size_of::<Job>();
        if fixed + COMPLETION_BYTES > config.max_bytes {
            return Err(DataPipelineError::Full);
        }
        Ok(Self {
            config,
            epoch,
            fixed_bytes: fixed,
            state: Mutex::new(State {
                queue,
                stats: DataPipelineStats {
                    charged_bytes: fixed,
                    peak_bytes: fixed,
                    ..Default::default()
                },
                barrier: false,
                closed: false,
                next: 1,
            }),
            changed: Condvar::new(),
        })
    }

    pub fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn reject(&self, error: DataPipelineError) -> DataPipelineError {
        let mut state = self.lock();
        state.stats.rejected = state.stats.rejected.saturating_add(1);
        error
    }

    pub fn submit(
        self: &Arc<Self>,
        request: &DataRequest,
        deadline: Instant,
        admission: DataAdmission,
    ) -> Result<DataRequestTicket, DataPipelineError> {
        let payload = request
            .key
            .capacity()
            .checked_add(request.metadata.capacity())
            .and_then(|n| n.checked_add(request.payload.capacity()));
        if request.host_epoch != self.epoch {
            return Err(self.reject(DataPipelineError::Invalid(
                "request does not match negotiated epoch".into(),
            )));
        }
        if request.revision != SCHEMA_REVISION
            || !(data_op::OPEN_WRITER..=data_op::FIELD_QUERY).contains(&request.operation)
            || request.flags != 0
            || request.reserved != 0
            || request.key.len() > DATA_KEY_BYTES
            || request.metadata.len() > DATA_METADATA_BYTES
            || request.payload.len() > DATA_CHUNK_BYTES
        {
            return Err(self.reject(DataPipelineError::Invalid(
                "invalid request envelope bounds or flags".into(),
            )));
        }
        let payload = payload.ok_or_else(|| self.reject(DataPipelineError::Full))?;
        self.admit(payload, deadline, admission, || {
            Request::Typed(request.clone())
        })
    }

    pub fn submit_wire(
        self: &Arc<Self>,
        type_hash: u64,
        wire: &[u8],
        deadline: Instant,
        admission: DataAdmission,
    ) -> Result<DataRequestTicket, DataPipelineError> {
        self.submit_wire_parts(type_hash, &[wire], deadline, admission)
    }

    pub fn submit_wire_parts(
        self: &Arc<Self>,
        type_hash: u64,
        parts: &[&[u8]],
        deadline: Instant,
        admission: DataAdmission,
    ) -> Result<DataRequestTicket, DataPipelineError> {
        if parts.is_empty() || parts.len() > 4 {
            return Err(self.reject(DataPipelineError::Invalid(
                "data wire requires one to four sections".into(),
            )));
        }
        let wire = parts[0];
        let length = parts
            .iter()
            .try_fold(0usize, |n, part| n.checked_add(part.len()));
        if type_hash != datapod::bind::type_hash::<DataRequest>()
            || wire.len() < <DataRequest as DataPod>::Header::LE_WIRE_SIZE
            || length.is_none_or(|n| n > crate::host::MAX_PAYLOAD_BYTES)
        {
            return Err(self.reject(DataPipelineError::Invalid(
                "invalid data wire type or length".into(),
            )));
        }
        let header = <DataRequest as DataPod>::Header::read_le(
            &wire[..<DataRequest as DataPod>::Header::LE_WIRE_SIZE],
        )
        .map_err(|error| self.reject(DataPipelineError::Invalid(error.to_string())))?;
        if header.revision != SCHEMA_REVISION
            || !(data_op::OPEN_WRITER..=data_op::FIELD_QUERY).contains(&header.operation)
            || header.flags != 0
            || header.reserved != 0
            || header.host_epoch != self.epoch
        {
            return Err(self.reject(DataPipelineError::Invalid(
                "invalid data wire revision, flags, operation or epoch".into(),
            )));
        }
        self.admit(length.unwrap(), deadline, admission, || Request::Wire {
            bytes: parts.concat(),
            operation: header.operation,
        })
    }

    fn admit(
        self: &Arc<Self>,
        payload: usize,
        deadline: Instant,
        admission: DataAdmission,
        copy: impl FnOnce() -> Request,
    ) -> Result<DataRequestTicket, DataPipelineError> {
        let deadline = deadline.min(Instant::now() + self.config.request_timeout);
        let charge = payload
            .checked_add(COMPLETION_BYTES)
            .ok_or_else(|| self.reject(DataPipelineError::Full))?;
        if charge > self.config.max_bytes - self.fixed_bytes {
            return Err(self.reject(DataPipelineError::Full));
        }
        let mut state = self.lock();
        loop {
            let error = if state.closed {
                Some(DataPipelineError::Closed)
            } else if Instant::now() >= deadline {
                Some(DataPipelineError::Deadline {
                    may_have_been_sent: false,
                })
            } else {
                None
            };
            if let Some(error) = error {
                state.stats.rejected = state.stats.rejected.saturating_add(1);
                return Err(error);
            }
            if state.stats.outstanding < self.config.max_operations
                && charge <= self.config.max_bytes - state.stats.charged_bytes
            {
                break;
            }
            if admission == DataAdmission::Reject {
                state.stats.rejected = state.stats.rejected.saturating_add(1);
                return Err(DataPipelineError::Full);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            state = self
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        let id = state.next;
        state.next = id
            .checked_add(1)
            .ok_or_else(|| DataPipelineError::Invalid("request sequence exhausted".into()))?;
        let record = Arc::new(Record {
            shared: Arc::downgrade(self),
            outcome: Mutex::new(None),
            ready: Condvar::new(),
            payload_charge: AtomicUsize::new(payload),
            slot_held: AtomicBool::new(true),
        });
        state.stats.outstanding += 1;
        state.stats.queued += 1;
        state.stats.charged_bytes += charge;
        state.stats.peak_bytes = state.stats.peak_bytes.max(state.stats.charged_bytes);
        state.stats.submitted = state.stats.submitted.saturating_add(1);
        state.queue.push_back(Job {
            id,
            request: copy(),
            deadline,
            record: record.clone(),
        });
        drop(state);
        self.changed.notify_all();
        Ok(DataRequestTicket {
            id,
            record: Some(record),
        })
    }

    pub fn cancel(&self, id: u64) -> bool {
        let removed = {
            let mut state = self.lock();
            let Some(index) = state.queue.iter().position(|job| job.id == id) else {
                return false;
            };
            let job = state.queue.remove(index).unwrap();
            state.stats.queued -= 1;
            state.stats.cancelled = state.stats.cancelled.saturating_add(1);
            job
        };
        drop(removed.request);
        removed.record.release_payload();
        removed.record.complete(Err(DataPipelineError::Cancelled));
        self.changed.notify_all();
        true
    }

    pub fn close(&self) {
        {
            let mut state = self.lock();
            state.closed = true;
        }
        loop {
            let next = self.lock().queue.front().map(|job| job.id);
            let Some(next) = next else {
                break;
            };
            self.cancel(next);
        }
        self.changed.notify_all();
    }

    fn next(&self) -> Option<Job> {
        let mut state = self.lock();
        loop {
            if state.closed {
                return None;
            }
            if let Some(job) = state.queue.front() {
                let barrier = job.request.operation() != data_op::CHUNK;
                if !state.barrier && (!barrier || state.stats.running == 0) {
                    state.barrier = barrier;
                    state.stats.queued -= 1;
                    state.stats.running += 1;
                    return state.queue.pop_front();
                }
            }
            state = self.changed.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }
}

pub(super) fn worker(shared: Arc<Shared>, mut client: DrawDataClient) {
    while let Some(job) = shared.next() {
        let result = if Instant::now() >= job.deadline {
            Err(DataPipelineError::Deadline {
                may_have_been_sent: false,
            })
        } else {
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                job.request.call(&mut client, job.deadline)
            })) {
                Ok(result) => result.map_err(Into::into),
                Err(_) => {
                    shared.close();
                    Err(DataPipelineError::Transport(
                        "request worker panicked; pipeline closed".into(),
                    ))
                }
            }
        };
        drop(job.request);
        job.record.release_payload();
        {
            let mut state = shared.lock();
            state.stats.running -= 1;
            state.barrier = false;
            state.stats.finished = state.stats.finished.saturating_add(1);
        }
        job.record.complete(result);
        shared.changed.notify_all();
    }
}
