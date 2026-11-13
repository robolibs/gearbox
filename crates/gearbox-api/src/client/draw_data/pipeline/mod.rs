use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use peerbus::EndpointId;
use serde::Serialize;

use super::{DrawDataClient, DrawDataError};
use crate::Client;
use crate::wire::draw_data::{
    DataCapabilities, DataLimits, DataReply, DataRequest, GenerationVisibility,
};

mod state;
mod ticket;
mod visibility;

use state::{Shared, worker};
pub use ticket::{DataCancelOutcome, DataRequestCancellation, DataRequestTicket};

const MAX_ERROR_BYTES: usize = 1024;
const WORKER_BYTES: usize = 2 << 20;
const REPLY_BYTES: usize = 2 * crate::host::MAX_PAYLOAD_BYTES + MAX_ERROR_BYTES;

#[derive(Clone, Copy, Debug)]
pub struct DataPipelineConfig {
    pub max_bytes: usize,
    pub max_operations: usize,
    pub parallel_requests: usize,
    pub request_timeout: Duration,
}

impl Default for DataPipelineConfig {
    fn default() -> Self {
        Self {
            max_bytes: 32 << 20,
            max_operations: 64,
            parallel_requests: 4,
            request_timeout: Duration::from_secs(5),
        }
    }
}

impl DataPipelineConfig {
    fn validate(&self, limits: &DataLimits) -> Result<usize, DataPipelineError> {
        if !(1..=8).contains(&self.parallel_requests)
            || !(1..=1024).contains(&self.max_operations)
            || self.max_operations < self.parallel_requests
            || self.max_bytes as u64 > limits.client_queue_bytes
            || self.request_timeout.is_zero()
            || self.request_timeout > Duration::from_secs(60)
        {
            return Err(DataPipelineError::Invalid("invalid pipeline limits".into()));
        }
        let fixed = WORKER_BYTES * self.parallel_requests
            + size_of::<state::Job>() * self.max_operations
            + size_of::<Shared>();
        if self.max_bytes < fixed + state::COMPLETION_BYTES {
            return Err(DataPipelineError::Invalid(
                "pipeline budget cannot hold its workspace and one result".into(),
            ));
        }
        Ok(fixed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataAdmission {
    Reject,
    Wait,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct DataPipelineStats {
    pub queued: usize,
    pub running: usize,
    pub outstanding: usize,
    pub charged_bytes: usize,
    pub peak_bytes: usize,
    pub submitted: u64,
    pub finished: u64,
    pub cancelled: u64,
    pub rejected: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum DataPipelineError {
    Full,
    Closed,
    Cancelled,
    Consumed,
    WaitTimeout,
    Deadline { may_have_been_sent: bool },
    Invalid(String),
    Protocol(String),
    Unsupported(String),
    Transport(String),
    Rejected { code: u32, message: String },
    Visibility(GenerationVisibility),
}

impl std::fmt::Display for DataPipelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full => write!(f, "data pipeline byte or operation budget exhausted"),
            Self::Closed => write!(f, "data pipeline is closed"),
            Self::Cancelled => write!(f, "data operation cancelled"),
            Self::Consumed => write!(f, "data request result already consumed"),
            Self::WaitTimeout => write!(f, "result wait expired; request is not cancelled"),
            Self::Deadline { may_have_been_sent } => write!(
                f,
                "data request deadline expired; may have been sent: {may_have_been_sent}"
            ),
            Self::Invalid(message) => write!(f, "invalid data pipeline request: {message}"),
            Self::Protocol(message) => write!(f, "data pipeline protocol: {message}"),
            Self::Unsupported(message) => write!(f, "unsupported data pipeline: {message}"),
            Self::Transport(message) => write!(f, "data pipeline transport: {message}"),
            Self::Rejected { code, message } => {
                write!(f, "data request rejected ({code}): {message}")
            }
            Self::Visibility(state) => write!(f, "visibility did not complete: {state:?}"),
        }
    }
}

impl std::error::Error for DataPipelineError {}

fn bounded(mut message: String) -> String {
    let mut end = message.len().min(MAX_ERROR_BYTES);
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    message.truncate(end);
    message.into_boxed_str().into_string()
}

impl From<DrawDataError> for DataPipelineError {
    fn from(error: DrawDataError) -> Self {
        match error {
            DrawDataError::Timeout => Self::Deadline {
                may_have_been_sent: true,
            },
            DrawDataError::Protocol(message) => Self::Protocol(bounded(message)),
            DrawDataError::Unsupported(message) => Self::Unsupported(bounded(message)),
            DrawDataError::Transport(error) => Self::Transport(bounded(error.to_string())),
            DrawDataError::Rejected { code, message } => Self::Rejected {
                code,
                message: bounded(message),
            },
        }
    }
}

/// Fixed workers with bounded request/result ownership and FIFO mutation barriers.
pub struct DrawDataPipeline {
    shared: Arc<Shared>,
    capabilities: DataCapabilities,
    workers: Mutex<Vec<JoinHandle<()>>>,
}

impl Client {
    pub fn draw_data_pipeline(
        &self,
        config: DataPipelineConfig,
        deadline: Instant,
    ) -> Result<DrawDataPipeline, DataPipelineError> {
        self.draw_data_pipeline_at(self.data_host()?, config, deadline)
    }

    pub fn draw_data_pipeline_at(
        &self,
        host: EndpointId,
        config: DataPipelineConfig,
        deadline: Instant,
    ) -> Result<DrawDataPipeline, DataPipelineError> {
        let first = self.draw_data_at_with_deadline(host, deadline)?;
        let fixed = config.validate(&first.capabilities().limits)?;
        let epoch = first.capabilities().host_epoch;
        let capabilities = first.capabilities().clone();
        let mut clients = Vec::with_capacity(config.parallel_requests);
        clients.push(first);
        for _ in 1..config.parallel_requests {
            let client = self.draw_data_at_with_deadline(host, deadline)?;
            if client.capabilities().host_epoch != epoch {
                return Err(DataPipelineError::Invalid(
                    "host restarted during pipeline negotiation".into(),
                ));
            }
            config.validate(&client.capabilities().limits)?;
            clients.push(client);
        }
        let shared = Arc::new(Shared::new(config, epoch, fixed)?);
        let mut pipeline = DrawDataPipeline {
            shared,
            capabilities,
            workers: Mutex::new(Vec::with_capacity(config.parallel_requests)),
        };
        for (index, client) in clients.into_iter().enumerate() {
            if Instant::now() >= deadline {
                return Err(DataPipelineError::Deadline {
                    may_have_been_sent: false,
                });
            }
            let shared = pipeline.shared.clone();
            let worker = std::thread::Builder::new()
                .name(format!("draw-data-{index}"))
                .spawn(move || worker(shared, client));
            match worker {
                Ok(worker) => pipeline.workers.get_mut().unwrap().push(worker),
                Err(error) => return Err(DataPipelineError::Transport(bounded(error.to_string()))),
            }
        }
        if Instant::now() >= deadline {
            return Err(DataPipelineError::Deadline {
                may_have_been_sent: false,
            });
        }
        Ok(pipeline)
    }
}

impl DrawDataPipeline {
    pub fn capabilities(&self) -> &DataCapabilities {
        &self.capabilities
    }

    /// Borrows up to four wire sections, concatenating them only after queue admission.
    pub fn submit_wire_parts(
        &self,
        type_hash: u64,
        parts: &[&[u8]],
        deadline: Instant,
        admission: DataAdmission,
    ) -> Result<DataRequestTicket, DataPipelineError> {
        self.shared
            .submit_wire_parts(type_hash, parts, deadline, admission)
    }

    pub fn config(&self) -> DataPipelineConfig {
        self.shared.config
    }

    pub(super) fn reserve_sender(
        &self,
        bytes: usize,
    ) -> Result<SenderReservation, DataPipelineError> {
        let mut state = self.shared.lock();
        if state.closed {
            return Err(DataPipelineError::Closed);
        }
        if bytes + state::COMPLETION_BYTES + crate::host::MAX_PAYLOAD_BYTES
            > self
                .shared
                .config
                .max_bytes
                .saturating_sub(state.stats.charged_bytes)
        {
            return Err(DataPipelineError::Full);
        }
        state.stats.charged_bytes += bytes;
        state.stats.peak_bytes = state.stats.peak_bytes.max(state.stats.charged_bytes);
        Ok(SenderReservation {
            shared: self.shared.clone(),
            bytes,
        })
    }

    pub fn host_epoch(&self) -> u64 {
        self.shared.epoch
    }

    pub fn stats(&self) -> DataPipelineStats {
        self.shared.lock().stats
    }

    /// Borrows input until admission, then copies it into the charged queue under one deadline.
    pub fn submit(
        &self,
        request: &DataRequest,
        deadline: Instant,
        admission: DataAdmission,
    ) -> Result<DataRequestTicket, DataPipelineError> {
        self.shared.submit(request, deadline, admission)
    }

    /// Copies caller-owned wire bytes after admission; decoding runs on the bounded workers.
    pub fn submit_wire(
        &self,
        type_hash: u64,
        wire: &[u8],
        deadline: Instant,
        admission: DataAdmission,
    ) -> Result<DataRequestTicket, DataPipelineError> {
        self.shared
            .submit_wire(type_hash, wire, deadline, admission)
    }

    /// Stops queued work and joins active bounded requests; delivered mutations may complete.
    pub fn close(&self) {
        self.shutdown();
        let mut workers = self.workers.lock().unwrap_or_else(|e| e.into_inner());
        for worker in workers.drain(..) {
            let _ = worker.join();
        }
    }

    /// Closes admission and cancels queued requests without waiting for active calls.
    pub fn shutdown(&self) {
        self.shared.close();
    }
}

pub(super) struct SenderReservation {
    shared: Arc<Shared>,
    bytes: usize,
}

impl Drop for SenderReservation {
    fn drop(&mut self) {
        self.shared.lock().stats.charged_bytes -= self.bytes;
        self.shared.changed.notify_all();
    }
}

impl Drop for DrawDataPipeline {
    fn drop(&mut self) {
        self.close();
    }
}

type Outcome = Result<DataReply, DataPipelineError>;
