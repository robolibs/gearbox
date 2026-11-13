use std::time::{Duration, Instant};

use super::{DrawDataClient, DrawDataError, check_deadline, deadline_after};
use crate::draw_data::RenderCompletion;
use crate::wire::draw_data::*;

#[derive(Debug)]
pub enum WaitVisibleError {
    Request(DrawDataError),
    Terminal(GenerationVisibility),
}

impl std::fmt::Display for WaitVisibleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Request(error) => error.fmt(f),
            Self::Terminal(state) => write!(f, "visibility did not complete: {state:?}"),
        }
    }
}
impl std::error::Error for WaitVisibleError {}
impl From<DrawDataError> for WaitVisibleError {
    fn from(error: DrawDataError) -> Self {
        Self::Request(error)
    }
}

impl DrawDataClient {
    /// Waits for an exact generation and freezes an omitted primary view on the first status reply.
    pub fn wait_visible(
        &mut self,
        key: &DataKey,
        revision: DatasetRevision,
        view: Option<ViewIdentity>,
        timeout: Duration,
    ) -> Result<RenderCompletion, WaitVisibleError> {
        self.wait_visible_with_deadline(key, revision, view, deadline_after(timeout)?)
    }

    /// A deadline or terminal visibility state leaves the retained generation unchanged.
    pub fn wait_visible_with_deadline(
        &mut self,
        key: &DataKey,
        revision: DatasetRevision,
        mut view: Option<ViewIdentity>,
        deadline: Instant,
    ) -> Result<RenderCompletion, WaitVisibleError> {
        key.validate().map_err(DrawDataError::Protocol)?;
        revision.validate().map_err(DrawDataError::Protocol)?;
        loop {
            check_deadline(deadline)?;
            let request = request(self.capabilities.host_epoch, key, revision, view)?;
            let reply = self.request_with_deadline(&request, deadline)?;
            if let Some(completion) = observe(&reply, key, revision, &mut view)? {
                return Ok(completion);
            }
            check_deadline(deadline)?;
            std::thread::sleep(
                Duration::from_millis(20).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }
}

pub(super) fn request(
    epoch: u64,
    key: &DataKey,
    revision: DatasetRevision,
    view: Option<ViewIdentity>,
) -> Result<DataRequest, DrawDataError> {
    key.validate().map_err(DrawDataError::Protocol)?;
    revision.validate().map_err(DrawDataError::Protocol)?;
    if let Some(view) = view {
        view.validate().map_err(DrawDataError::Protocol)?;
    }
    DataRequest {
        host_epoch: epoch,
        incarnation: revision.incarnation,
        base_generation: revision.generation,
        ..DataRequest::new(data_op::STATUS)
    }
    .with_key(key)
    .map_err(DrawDataError::Protocol)?
    .with_metadata(&DataStatusQuery {
        generation: Some(revision.generation),
        view,
    })
    .map_err(DrawDataError::Protocol)
}

pub(super) fn observe(
    reply: &DataReply,
    key: &DataKey,
    revision: DatasetRevision,
    view: &mut Option<ViewIdentity>,
) -> Result<Option<RenderCompletion>, WaitVisibleError> {
    if reply.code != data_code::OK {
        return Err(DrawDataError::Rejected {
            code: reply.code,
            message: String::from_utf8_lossy(&reply.message).into_owned(),
        }
        .into());
    }
    let status = DatasetStatus::from_json(&reply.metadata).map_err(DrawDataError::Protocol)?;
    if status.key != *key
        || status.requested != Some(revision)
        || reply.incarnation != revision.incarnation
    {
        return Err(DrawDataError::Protocol(
            "status did not answer the exact requested dataset".into(),
        )
        .into());
    }
    if let Some(target) = status.visibility.target() {
        if target.dataset != revision || view.is_some_and(|view| view != target.view) {
            return Err(
                DrawDataError::Protocol("status changed the frozen view identity".into()).into(),
            );
        }
        *view = Some(target.view);
    }
    match status.visibility {
        GenerationVisibility::Visible {
            target,
            frame,
            disposition,
            source_samples,
            selected_items,
        } => {
            if reply.stage != data_stage::VISIBLE
                || reply.visible_generation != revision.generation
                || (reply.view_id, reply.view_epoch) != (target.view.id, target.view.epoch)
            {
                return Err(DrawDataError::Protocol(
                    "visible status header disagrees with its receipt".into(),
                )
                .into());
            }
            Ok(Some(RenderCompletion {
                target,
                frame,
                disposition,
                source_samples,
                selected_items,
            }))
        }
        GenerationVisibility::Pending { .. } => Ok(None),
        terminal => Err(WaitVisibleError::Terminal(terminal)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const REV: DatasetRevision = DatasetRevision {
        incarnation: 1,
        generation: 2,
    };
    const VIEW: ViewIdentity = ViewIdentity { id: 3, epoch: 4 };
    fn reply(visibility: GenerationVisibility) -> (DataKey, DataReply) {
        let key = DataKey::new("test", "cloud").unwrap();
        let mut reply = DataReply::new(1);
        reply.incarnation = REV.incarnation;
        reply.stored_generation = REV.generation;
        reply.stage = data_stage::RETAINED;
        if let Some(target) = visibility.target() {
            reply.view_id = target.view.id;
            reply.view_epoch = target.view.epoch;
        }
        if matches!(visibility, GenerationVisibility::Visible { .. }) {
            reply.stage = data_stage::VISIBLE;
            reply.visible_generation = REV.generation;
        }
        reply.metadata = DatasetStatus {
            revision: SCHEMA_REVISION,
            key: key.clone(),
            kind: DatasetKind::Cloud,
            retained: Some(REV),
            requested: Some(REV),
            visibility,
            transfer: None,
            failure: None,
            resources: DataResourceUsage::default(),
            counters: DataCounters::default(),
            paging: None,
        }
        .to_json()
        .unwrap();
        (key, reply)
    }
    #[test]
    fn first_reply_freezes_primary_and_later_reply_cannot_follow_replacement() {
        let target = VisibilityTarget {
            dataset: REV,
            view: VIEW,
        };
        let (key, pending) = reply(GenerationVisibility::Pending { target });
        let mut view = None;
        assert!(observe(&pending, &key, REV, &mut view).unwrap().is_none());
        assert_eq!(view, Some(VIEW));
        let (_, changed) = reply(GenerationVisibility::Pending {
            target: VisibilityTarget {
                view: ViewIdentity { epoch: 5, ..VIEW },
                ..target
            },
        });
        assert!(matches!(
            observe(&changed, &key, REV, &mut view),
            Err(WaitVisibleError::Request(DrawDataError::Protocol(_)))
        ));
        let (_, gone) = reply(GenerationVisibility::ViewGone { target });
        assert!(matches!(
            observe(&gone, &key, REV, &mut view),
            Err(WaitVisibleError::Terminal(
                GenerationVisibility::ViewGone { .. }
            ))
        ));
    }
    #[test]
    fn only_matching_visible_header_and_payload_return_success() {
        let completion = RenderCompletion {
            target: VisibilityTarget {
                dataset: REV,
                view: VIEW,
            },
            frame: 7,
            disposition: FrontierDisposition::Drawn,
            source_samples: 12,
            selected_items: 3,
        };
        let (key, mut reply) = reply(completion.visibility());
        assert_eq!(
            observe(&reply, &key, REV, &mut None).unwrap(),
            Some(completion)
        );
        reply.visible_generation = 1;
        assert!(observe(&reply, &key, REV, &mut None).is_err());
        assert!(
            observe(
                &reply,
                &key,
                DatasetRevision {
                    generation: 1,
                    ..REV
                },
                &mut None
            )
            .is_err()
        );
    }
}
