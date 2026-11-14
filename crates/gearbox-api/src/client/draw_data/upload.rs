use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::pipeline::SenderReservation;
use super::{DataAdmission, DataPipelineError, DataRequestTicket, DrawDataPipeline};
use crate::wire::draw_data::*;

const SENDER_BYTES: usize = 2 << 20;
const CLEANUP_TIMEOUT: Duration = Duration::from_millis(250);

pub struct DataUploadPage<'a> {
    pub descriptor: &'a PageDescriptor,
    pub bytes: &'a [u8],
}

/// Borrowed source pages and atomic replacement/delta metadata.
pub struct DataUpload<'a> {
    pub descriptor: &'a DataDescriptor,
    pub mode: CommitMode,
    pub base: Option<DatasetRevision>,
    pub scans: &'a [ScanDescriptor],
    pub pages: &'a [DataUploadPage<'a>],
    pub removed_pages: &'a [PageAddress],
    pub removed_scans: &'a [u64],
}

impl<'a> DataUpload<'a> {
    pub fn replace(
        descriptor: &'a DataDescriptor,
        scans: &'a [ScanDescriptor],
        pages: &'a [DataUploadPage<'a>],
    ) -> Self {
        Self {
            descriptor,
            mode: CommitMode::Replace,
            base: None,
            scans,
            pages,
            removed_pages: &[],
            removed_scans: &[],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum DataUploadCleanup {
    NotStarted,
    Cancelled,
    Retained(DatasetRevision),
    Unconfirmed(DataPipelineError),
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct DataUploadError {
    pub cause: DataPipelineError,
    pub host_epoch: u64,
    pub lease: u64,
    pub transaction: u64,
    pub cleanup: DataUploadCleanup,
}

impl std::fmt::Display for DataUploadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "upload {}/{}: {}; cleanup: {:?}",
            self.lease, self.transaction, self.cause, self.cleanup
        )
    }
}

impl std::error::Error for DataUploadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

/// A writer lease with serial atomic submissions and a bounded CHUNK window.
pub struct DrawDataWriter<'a> {
    pipeline: PipelineOwner<'a>,
    lease: u64,
    next: u64,
    window: usize,
    closed: bool,
    _reservation: SenderReservation,
}

enum PipelineOwner<'a> {
    Borrowed(&'a DrawDataPipeline),
    Shared(Arc<DrawDataPipeline>),
}

impl std::ops::Deref for PipelineOwner<'_> {
    type Target = DrawDataPipeline;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Borrowed(pipeline) => pipeline,
            Self::Shared(pipeline) => pipeline,
        }
    }
}

impl DrawDataPipeline {
    pub fn writer(
        &self,
        chunk_window: usize,
        deadline: Instant,
    ) -> Result<DrawDataWriter<'_>, DataPipelineError> {
        DrawDataWriter::open(PipelineOwner::Borrowed(self), chunk_window, deadline)
    }

    pub fn writer_owned(
        self: &Arc<Self>,
        chunk_window: usize,
        deadline: Instant,
    ) -> Result<DrawDataWriter<'static>, DataPipelineError> {
        DrawDataWriter::open(PipelineOwner::Shared(self.clone()), chunk_window, deadline)
    }
}

impl<'a> DrawDataWriter<'a> {
    fn open(
        pipeline: PipelineOwner<'a>,
        chunk_window: usize,
        deadline: Instant,
    ) -> Result<Self, DataPipelineError> {
        if chunk_window == 0 || chunk_window > pipeline.config().max_operations.min(64) {
            return Err(DataPipelineError::Invalid(
                "writer chunk window outside its operation budget".into(),
            ));
        }
        let reservation = pipeline.reserve_sender(SENDER_BYTES)?;
        let reply = call(
            &pipeline,
            &DataRequest {
                host_epoch: pipeline.host_epoch(),
                ..DataRequest::new(data_op::OPEN_WRITER)
            },
            deadline,
        )?;
        if reply.lease == 0 {
            return Err(DataPipelineError::Protocol(
                "writer acquisition returned a zero lease".into(),
            ));
        }
        Ok(DrawDataWriter {
            pipeline,
            lease: reply.lease,
            next: 1,
            window: chunk_window,
            closed: false,
            _reservation: reservation,
        })
    }
}

impl DrawDataWriter<'_> {
    pub fn lease(&self) -> u64 {
        self.lease
    }

    pub fn host_epoch(&self) -> u64 {
        self.pipeline.host_epoch()
    }

    /// All upload operations share one deadline; failure cleanup has a separate 250 ms budget.
    pub fn upload(
        &mut self,
        source: &DataUpload<'_>,
        deadline: Instant,
        cancelled: Option<&AtomicBool>,
    ) -> Result<DataReply, DataUploadError> {
        let transaction = self.next;
        let error = |cause| DataUploadError {
            cause,
            host_epoch: self.host_epoch(),
            lease: self.lease,
            transaction,
            cleanup: DataUploadCleanup::NotStarted,
        };
        if self.closed {
            return Err(error(DataPipelineError::Closed));
        }
        check(deadline, cancelled).map_err(error)?;
        let begin = self.begin(source, transaction).map_err(error)?;
        self.next = self.next.checked_add(1).ok_or_else(|| {
            error(DataPipelineError::Invalid(
                "writer transaction sequence exhausted".into(),
            ))
        })?;
        let mut pending = Pending(
            VecDeque::with_capacity(self.window),
            self.lease,
            transaction,
        );
        let result = self.send(source, &begin, &mut pending, deadline, cancelled);
        match result {
            Ok(reply) => Ok(reply),
            Err(cause) => {
                drop(pending);
                let cleanup = match call(
                    &self.pipeline,
                    &edit(&begin, data_op::CANCEL),
                    Instant::now() + CLEANUP_TIMEOUT,
                ) {
                    Ok(reply)
                        if reply.stage == data_stage::RETAINED
                            || reply.stage == data_stage::VISIBLE =>
                    {
                        DataUploadCleanup::Retained(DatasetRevision {
                            incarnation: reply.incarnation,
                            generation: reply.stored_generation,
                        })
                    }
                    Ok(reply)
                        if reply.stage == data_stage::NONE
                            && reply.incarnation == 0
                            && reply.stored_generation == 0 =>
                    {
                        DataUploadCleanup::Cancelled
                    }
                    Ok(_) => DataUploadCleanup::Unconfirmed(DataPipelineError::Protocol(
                        "cancel did not acknowledge a terminal outcome".into(),
                    )),
                    Err(error) => DataUploadCleanup::Unconfirmed(error),
                };
                Err(DataUploadError {
                    cause,
                    host_epoch: self.host_epoch(),
                    lease: self.lease,
                    transaction,
                    cleanup,
                })
            }
        }
    }

    fn begin(
        &self,
        source: &DataUpload<'_>,
        transaction: u64,
    ) -> Result<DataRequest, DataPipelineError> {
        source
            .descriptor
            .validate()
            .map_err(DataPipelineError::Invalid)?;
        let count = |n| {
            u32::try_from(n).map_err(|_| {
                DataPipelineError::Invalid("upload declaration count overflows u32".into())
            })
        };
        let total = source.pages.iter().try_fold(0u64, |sum, page| {
            if page.descriptor.byte_len != page.bytes.len() as u64 {
                return Err(DataPipelineError::Invalid(
                    "page payload length differs from declaration".into(),
                ));
            }
            page.descriptor
                .validate()
                .map_err(DataPipelineError::Invalid)?;
            sum.checked_add(page.descriptor.byte_len)
                .ok_or_else(|| DataPipelineError::Invalid("upload byte count overflow".into()))
        })?;
        let metadata = BeginMetadata {
            descriptor: source.descriptor.clone(),
            mode: source.mode,
            scans: count(source.scans.len())?,
            pages: count(source.pages.len())?,
            removed_pages: count(source.removed_pages.len())?,
            removed_scans: count(source.removed_scans.len())?,
        };
        metadata.validate().map_err(DataPipelineError::Invalid)?;
        let request = DataRequest {
            host_epoch: self.host_epoch(),
            lease: self.lease,
            transaction,
            incarnation: source.base.map_or(0, |base| base.incarnation),
            base_generation: source.base.map_or(0, |base| base.generation),
            total_bytes: total,
            ..DataRequest::new(data_op::BEGIN)
        }
        .with_key(&source.descriptor.key)
        .map_err(DataPipelineError::Invalid)?
        .with_metadata(&metadata)
        .map_err(DataPipelineError::Invalid)?;
        request.operation().map_err(DataPipelineError::Invalid)?;
        Ok(request)
    }

    fn send(
        &self,
        source: &DataUpload<'_>,
        begin: &DataRequest,
        pending: &mut Pending,
        deadline: Instant,
        cancelled: Option<&AtomicBool>,
    ) -> Result<DataReply, DataPipelineError> {
        check(deadline, cancelled)?;
        call(&self.pipeline, begin, deadline)?;
        for scan in source.scans {
            check(deadline, cancelled)?;
            call(
                &self.pipeline,
                &DataRequest {
                    scan_key: scan.scan_key,
                    ..edit(begin, data_op::DECLARE_SCAN)
                }
                .with_metadata(scan)
                .map_err(DataPipelineError::Invalid)?,
                deadline,
            )?;
        }
        for page in source.pages {
            check(deadline, cancelled)?;
            let request = DataRequest {
                scan_key: page.descriptor.address.scan_key,
                page_id: page.descriptor.address.page_id,
                ..edit(begin, data_op::DECLARE_PAGE)
            }
            .with_metadata(page.descriptor)
            .map_err(DataPipelineError::Invalid)?;
            call(&self.pipeline, &request, deadline)?;
            for (index, bytes) in page.bytes.chunks(DATA_CHUNK_BYTES).enumerate() {
                check(deadline, cancelled)?;
                if pending.0.len() == self.window {
                    pending.take(deadline)?;
                }
                let request = DataRequest {
                    scan_key: page.descriptor.address.scan_key,
                    page_id: page.descriptor.address.page_id,
                    offset: (index * DATA_CHUNK_BYTES) as u64,
                    payload: bytes.to_vec(),
                    ..edit(begin, data_op::CHUNK)
                };
                pending.0.push_back(self.pipeline.submit(
                    &request,
                    deadline,
                    DataAdmission::Wait,
                )?);
            }
            while !pending.0.is_empty() {
                check(deadline, cancelled)?;
                pending.take(deadline)?;
            }
        }
        for page in source.removed_pages {
            check(deadline, cancelled)?;
            call(
                &self.pipeline,
                &DataRequest {
                    scan_key: page.scan_key,
                    page_id: page.page_id,
                    ..edit(begin, data_op::REMOVE_PAGE)
                },
                deadline,
            )?;
        }
        for scan_key in source.removed_scans {
            check(deadline, cancelled)?;
            call(
                &self.pipeline,
                &DataRequest {
                    scan_key: *scan_key,
                    ..edit(begin, data_op::REMOVE_SCAN)
                },
                deadline,
            )?;
        }
        check(deadline, cancelled)?;
        let reply = call(&self.pipeline, &edit(begin, data_op::COMMIT), deadline)?;
        if !matches!(reply.stage, data_stage::RETAINED | data_stage::VISIBLE)
            || reply.incarnation == 0
            || reply.stored_generation == 0
        {
            return Err(DataPipelineError::Protocol(
                "commit did not acknowledge a retained generation".into(),
            ));
        }
        Ok(reply)
    }

    /// Binds an exact retained generation to a host page provider named in capabilities.
    pub fn attach_provider(
        &mut self,
        key: &DataKey,
        revision: DatasetRevision,
        provider: &str,
        deadline: Instant,
    ) -> Result<DataReply, DataPipelineError> {
        if self.closed {
            return Err(DataPipelineError::Closed);
        }
        let request = DataRequest {
            host_epoch: self.host_epoch(),
            lease: self.lease,
            incarnation: revision.incarnation,
            base_generation: revision.generation,
            ..DataRequest::new(data_op::ATTACH_PROVIDER)
        }
        .with_key(key)
        .and_then(|request| {
            request.with_metadata(&ProviderAttachment {
                provider: provider.into(),
            })
        })
        .map_err(DataPipelineError::Invalid)?;
        call(&self.pipeline, &request, deadline)
    }

    /// Closes the lease without deleting retained datasets; a failed close can be retried.
    pub fn close(&mut self, deadline: Instant) -> Result<(), DataPipelineError> {
        if !self.closed {
            call(
                &self.pipeline,
                &DataRequest {
                    host_epoch: self.host_epoch(),
                    lease: self.lease,
                    ..DataRequest::new(data_op::CLOSE_WRITER)
                },
                deadline,
            )?;
            self.closed = true;
        }
        Ok(())
    }
}

impl Drop for DrawDataWriter<'_> {
    fn drop(&mut self) {
        let _ = self.close(Instant::now() + CLEANUP_TIMEOUT);
    }
}

struct Pending(VecDeque<DataRequestTicket>, u64, u64);

impl Pending {
    fn take(&mut self, deadline: Instant) -> Result<(), DataPipelineError> {
        let reply = self.0.front_mut().unwrap().wait(deadline)?;
        self.0.pop_front();
        let reply = checked(reply)?;
        identity(&reply, self.1, self.2)
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        for ticket in &self.0 {
            ticket.cancel();
        }
    }
}

fn call(
    pipeline: &DrawDataPipeline,
    request: &DataRequest,
    deadline: Instant,
) -> Result<DataReply, DataPipelineError> {
    let mut ticket = pipeline.submit(request, deadline, DataAdmission::Wait)?;
    match ticket.wait(deadline) {
        Ok(reply) => {
            let reply = checked(reply)?;
            if request.transaction != 0 {
                identity(&reply, request.lease, request.transaction)?;
            }
            Ok(reply)
        }
        Err(error) => {
            ticket.cancel();
            Err(error)
        }
    }
}

fn identity(reply: &DataReply, lease: u64, transaction: u64) -> Result<(), DataPipelineError> {
    if reply.lease != lease || reply.transaction != transaction {
        Err(DataPipelineError::Protocol(
            "reply does not identify the submitted transaction".into(),
        ))
    } else {
        Ok(())
    }
}

fn checked(reply: DataReply) -> Result<DataReply, DataPipelineError> {
    if reply.code == data_code::OK {
        Ok(reply)
    } else {
        Err(DataPipelineError::Rejected {
            code: reply.code,
            message: String::from_utf8_lossy(&reply.message).into_owned(),
        })
    }
}

fn check(deadline: Instant, cancelled: Option<&AtomicBool>) -> Result<(), DataPipelineError> {
    if cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        Err(DataPipelineError::Cancelled)
    } else if Instant::now() >= deadline {
        Err(DataPipelineError::Deadline {
            may_have_been_sent: false,
        })
    } else {
        Ok(())
    }
}

fn edit(begin: &DataRequest, operation: u32) -> DataRequest {
    DataRequest {
        host_epoch: begin.host_epoch,
        lease: begin.lease,
        transaction: begin.transaction,
        incarnation: begin.incarnation,
        base_generation: begin.base_generation,
        key: begin.key.clone(),
        ..DataRequest::new(operation)
    }
}
