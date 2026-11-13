use std::sync::{Arc, Weak};
use std::time::Instant;

use super::state::Record;
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataCancelOutcome {
    Cancelled,
    InFlight,
    Complete,
}

pub struct DataRequestTicket {
    pub(super) id: u64,
    pub(super) record: Option<Arc<Record>>,
}

#[derive(Clone)]
pub struct DataRequestCancellation {
    id: u64,
    record: Weak<Record>,
}

impl DataRequestCancellation {
    pub fn cancel(&self) -> DataCancelOutcome {
        self.record
            .upgrade()
            .map_or(DataCancelOutcome::Complete, |record| {
                cancel(self.id, &record)
            })
    }
}

impl DataRequestTicket {
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn cancellation(&self) -> DataRequestCancellation {
        DataRequestCancellation {
            id: self.id,
            record: self.record.as_ref().map_or_else(Weak::new, Arc::downgrade),
        }
    }

    /// Removes a queued request; an in-flight result may still be a committed mutation.
    pub fn cancel(&self) -> DataCancelOutcome {
        let Some(record) = &self.record else {
            return DataCancelOutcome::Complete;
        };
        cancel(self.id, record)
    }

    pub fn try_take(&mut self) -> Option<Result<DataReply, DataPipelineError>> {
        let Some(record) = &self.record else {
            return Some(Err(DataPipelineError::Consumed));
        };
        let result = record
            .outcome
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if result.is_some() {
            record.release_slot();
            self.record = None;
        }
        result
    }

    /// Waiting does not extend the request deadline or cancel it when the wait expires.
    pub fn wait(&mut self, deadline: Instant) -> Result<DataReply, DataPipelineError> {
        let Some(record) = &self.record else {
            return Err(DataPipelineError::Consumed);
        };
        let mut state = record.outcome.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(result) = state.take() {
                drop(state);
                record.release_slot();
                self.record = None;
                return result;
            }
            if Instant::now() >= deadline {
                return Err(DataPipelineError::WaitTimeout);
            }
            state = record
                .ready
                .wait_timeout(state, deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}

fn cancel(id: u64, record: &Arc<Record>) -> DataCancelOutcome {
    if record
        .shared
        .upgrade()
        .is_some_and(|shared| shared.cancel(id))
    {
        return DataCancelOutcome::Cancelled;
    }
    if record
        .outcome
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some()
    {
        DataCancelOutcome::Complete
    } else {
        DataCancelOutcome::InFlight
    }
}
