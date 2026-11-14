use std::sync::atomic::{AtomicBool, Ordering};

use super::*;
use crate::client::draw_data::visibility::{self, WaitVisibleError};
use crate::draw_data::RenderCompletion;
use crate::wire::draw_data::{DataKey, DatasetRevision, ViewIdentity};

const WAIT_BYTES: usize = 2 * crate::host::MAX_PAYLOAD_BYTES + 8192;
const POLL_INTERVAL: Duration = Duration::from_millis(20);

impl From<WaitVisibleError> for DataPipelineError {
    fn from(error: WaitVisibleError) -> Self {
        match error {
            WaitVisibleError::Request(error) => error.into(),
            WaitVisibleError::Terminal(state) => Self::Visibility(state),
        }
    }
}

impl DrawDataPipeline {
    /// Waits read-only for an exact receipt, freezing an omitted view on the first reply.
    pub fn wait_visible(
        &self,
        key: &DataKey,
        revision: DatasetRevision,
        mut view: Option<ViewIdentity>,
        deadline: Instant,
        cancelled: Option<&AtomicBool>,
    ) -> Result<RenderCompletion, DataPipelineError> {
        self.check_wait(deadline, cancelled, false)?;
        key.validate().map_err(DataPipelineError::Protocol)?;
        revision.validate().map_err(DataPipelineError::Protocol)?;
        if let Some(view) = view {
            view.validate().map_err(DataPipelineError::Protocol)?;
        }
        let _scratch = self.reserve_sender(WAIT_BYTES)?;
        let mut sent = false;
        loop {
            let request = visibility::request(self.host_epoch(), key, revision, view)?;
            let reply = self.visibility_status(&request, deadline, cancelled, &mut sent)?;
            if let Some(completion) = visibility::observe(&reply, key, revision, &mut view)? {
                return Ok(completion);
            }
            self.check_wait(deadline, cancelled, sent)?;
            wait_poll(deadline);
        }
    }

    fn visibility_status(
        &self,
        request: &DataRequest,
        deadline: Instant,
        cancelled: Option<&AtomicBool>,
        sent: &mut bool,
    ) -> Result<DataReply, DataPipelineError> {
        let mut ticket = loop {
            self.check_wait(deadline, cancelled, *sent)?;
            match self.submit(request, deadline, DataAdmission::Reject) {
                Ok(ticket) => break ticket,
                Err(DataPipelineError::Full) => wait_poll(deadline),
                Err(error) => return Err(error),
            }
        };
        *sent = true;
        loop {
            if let Err(error) = self.check_wait(deadline, cancelled, *sent) {
                ticket.cancel();
                return Err(error);
            }
            match ticket.wait(deadline.min(Instant::now() + POLL_INTERVAL)) {
                Err(DataPipelineError::WaitTimeout) => continue,
                result => return result,
            }
        }
    }

    fn check_wait(
        &self,
        deadline: Instant,
        cancelled: Option<&AtomicBool>,
        sent: bool,
    ) -> Result<(), DataPipelineError> {
        if cancelled.is_some_and(|token| token.load(Ordering::Acquire)) {
            Err(DataPipelineError::Cancelled)
        } else if self.shared.lock().closed {
            Err(DataPipelineError::Closed)
        } else if Instant::now() >= deadline {
            Err(DataPipelineError::Deadline {
                may_have_been_sent: sent,
            })
        } else {
            Ok(())
        }
    }
}

fn wait_poll(deadline: Instant) {
    std::thread::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
}
