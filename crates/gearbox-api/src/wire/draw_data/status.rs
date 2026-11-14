use std::io::{self, Write};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use super::frames::bounded_text;
use super::{
    DATA_METADATA_BYTES, DataKey, DataLimits, DatasetKind, DatasetRevision, MAX_CURSOR_BYTES,
    MAX_DECLARED_PAGES, MAX_ENCODED_PAGE_BYTES, MAX_LIST_ITEMS, MAX_PAGE_POINTS, MAX_SCANS,
    SCHEMA_REVISION, ViewIdentity, data_code,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "owner", rename_all = "snake_case", deny_unknown_fields)]
pub enum SummaryOwner {
    Legacy {
        kind: u32,
        generation: u64,
        revision: u64,
        points: u64,
        instances: u64,
    },
    Dataset {
        kind: DatasetKind,
        revision: DatasetRevision,
        scans: u32,
        pages: u32,
        source_samples: u64,
        valid_samples: u64,
        payload_bytes: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrawSummary {
    pub namespace: String,
    pub id: String,
    pub object: SummaryOwner,
}

impl DrawSummary {
    pub fn key(&self) -> (&str, &str) {
        (&self.namespace, &self.id)
    }

    pub fn validate(&self) -> Result<(), String> {
        match &self.object {
            SummaryOwner::Legacy {
                kind,
                generation,
                revision,
                ..
            } => {
                if !(crate::wire::draw_kind::LINES..=crate::wire::draw_kind::LAST).contains(kind)
                    || *generation == 0
                    || generation > revision
                    || self.id.is_empty()
                    || self.namespace.trim_matches('/') != self.namespace
                    || self.namespace.len().saturating_add(self.id.len())
                        > crate::host::MAX_PAYLOAD_BYTES
                {
                    return Err("invalid or unrepresentable legacy summary".into());
                }
            }
            SummaryOwner::Dataset {
                kind,
                revision,
                scans,
                pages,
                source_samples,
                valid_samples,
                payload_bytes,
            } => {
                DataKey {
                    namespace: self.namespace.clone(),
                    id: self.id.clone(),
                }
                .validate()?;
                revision.validate()?;
                if *scans > MAX_SCANS
                    || *pages > MAX_DECLARED_PAGES
                    || (*kind != DatasetKind::Cloud && *scans != 0)
                    || valid_samples > source_samples
                    || *source_samples > u64::from(*pages) * u64::from(MAX_PAGE_POINTS)
                    || *payload_bytes < u64::from(*pages) * 16
                    || *payload_bytes > u64::from(*pages) * MAX_ENCODED_PAGE_BYTES as u64
                {
                    return Err("invalid dataset summary counts".into());
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrawSummaryPage {
    pub revision: u32,
    pub listing_revision: u64,
    pub items: Vec<DrawSummary>,
    pub next_cursor: Option<String>,
}

impl DrawSummaryPage {
    pub fn validate(&self) -> Result<(), String> {
        if self.revision != SCHEMA_REVISION || self.items.len() > MAX_LIST_ITEMS as usize {
            return Err("unsupported listing revision or excessive items".into());
        }
        if let Some(cursor) = &self.next_cursor {
            bounded_text(cursor, MAX_CURSOR_BYTES, false)?;
        }
        for item in &self.items {
            item.validate()?;
        }
        if self
            .items
            .windows(2)
            .any(|items| items[0].key() >= items[1].key())
        {
            return Err("summary keys must be unique and sorted".into());
        }
        if self.items.is_empty() && self.next_cursor.is_some() {
            return Err("empty listing page cannot advance a cursor".into());
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        encode(self)
    }
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        let value: Self = decode(bytes)?;
        value.validate()?;
        Ok(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataPhase {
    Receiving,
    Validating,
    Retaining,
    Preparing,
    Rendering,
    Picking,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataFailure {
    pub phase: DataPhase,
    pub code: u32,
    pub reason: String,
}

impl DataFailure {
    pub fn validate(&self) -> Result<(), String> {
        Self::validate_parts(self.code, &self.reason)
    }

    pub(crate) fn validate_parts(code: u32, reason: &str) -> Result<(), String> {
        if code == data_code::OK || code > data_code::STALE_CURSOR {
            return Err("failure requires a known error code".into());
        }
        bounded_text(reason, 512, false)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisibilityTarget {
    pub dataset: DatasetRevision,
    pub view: ViewIdentity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrontierDisposition {
    Drawn,
    Culled,
    Empty,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum GenerationVisibility {
    Unavailable {
        reason: String,
    },
    Pending {
        target: VisibilityTarget,
    },
    Visible {
        target: VisibilityTarget,
        frame: u64,
        disposition: FrontierDisposition,
        source_samples: u64,
        selected_items: u64,
    },
    Superseded {
        target: VisibilityTarget,
        successor: DatasetRevision,
    },
    ViewGone {
        target: VisibilityTarget,
    },
    HistoryExpired {
        target: VisibilityTarget,
    },
    Failed {
        target: VisibilityTarget,
        failure: DataFailure,
    },
}

impl GenerationVisibility {
    pub fn target(&self) -> Option<VisibilityTarget> {
        match self {
            Self::Unavailable { .. } => None,
            Self::Pending { target }
            | Self::Visible { target, .. }
            | Self::Superseded { target, .. }
            | Self::ViewGone { target }
            | Self::HistoryExpired { target }
            | Self::Failed { target, .. } => Some(*target),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if let Some(target) = self.target() {
            target.dataset.validate()?;
            target.view.validate()?;
        }
        match self {
            Self::Unavailable { reason } => bounded_text(reason, 512, false)?,
            Self::Visible {
                frame,
                disposition,
                selected_items,
                ..
            } => {
                if *frame == 0
                    || (*disposition == FrontierDisposition::Drawn && *selected_items == 0)
                    || (*disposition == FrontierDisposition::Empty && *selected_items != 0)
                {
                    return Err("visible frontier has inconsistent completion/counts".into());
                }
            }
            Self::Superseded { target, successor } => {
                successor.validate()?;
                if target.dataset.incarnation != successor.incarnation
                    || target.dataset.generation >= successor.generation
                {
                    return Err("supersession must advance the same dataset incarnation".into());
                }
            }
            Self::Failed { failure, .. } => failure.validate()?,
            _ => {}
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferProgress {
    pub transaction: u64,
    pub phase: DataPhase,
    pub received_bytes: u64,
    pub expected_bytes: u64,
    pub staged_bytes: u64,
    pub declared_pages: u32,
    pub complete_pages: u32,
}

impl TransferProgress {
    pub fn validate(&self) -> Result<(), String> {
        if self.transaction == 0
            || self.received_bytes > self.expected_bytes
            || self.staged_bytes > self.received_bytes
            || self.declared_pages > MAX_DECLARED_PAGES
            || self.complete_pages > self.declared_pages
            || self.expected_bytes < u64::from(self.declared_pages) * 16
            || self.expected_bytes > u64::from(self.declared_pages) * MAX_ENCODED_PAGE_BYTES as u64
            || !matches!(
                self.phase,
                DataPhase::Receiving | DataPhase::Validating | DataPhase::Retaining
            )
        {
            return Err("invalid transfer progress".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataResourceUsage {
    pub cpu_bytes: u64,
    pub gpu_bytes: u64,
    pub incomplete_bytes: u64,
    pub staged_bytes: u64,
    pub resident_page_bytes: u64,
    pub pinned_retired_bytes: u64,
    pub metadata_bytes: u64,
    pub pending_transactions: u32,
    pub writer_leases: u32,
    pub retained_datasets: u32,
}

impl DataResourceUsage {
    pub fn validate(&self) -> Result<(), String> {
        if [
            self.incomplete_bytes,
            self.staged_bytes,
            self.resident_page_bytes,
            self.metadata_bytes,
        ]
        .iter()
        .any(|bytes| *bytes > self.cpu_bytes)
            || self.pinned_retired_bytes > self.resident_page_bytes
        {
            return Err("resource category exceeds its containing allocation total".into());
        }
        Ok(())
    }

    pub fn validate_against(&self, limits: &DataLimits) -> Result<(), String> {
        self.validate()?;
        limits.validate()?;
        if self.cpu_bytes > limits.cpu_bytes
            || self.gpu_bytes > limits.gpu_bytes
            || self.incomplete_bytes > limits.incomplete_bytes
            || self.pending_transactions > limits.transactions
            || self.writer_leases > limits.writers
            || self.retained_datasets > limits.datasets
        {
            return Err("resource usage exceeds advertised limits".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataCounters {
    pub received_bytes: u64,
    pub rejected_bytes: u64,
    pub rejected_transactions: u64,
    pub dropped_scans: u64,
    pub superseded_scans: u64,
    pub evicted_scans: u64,
}

/// Provider-backed CPU residency of the prepared generation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageResidency {
    pub pages: u32,
    pub resident: u32,
    pub evicted: u32,
    /// Demanded pages whose provider could not reload them; shown at coarse detail.
    pub unavailable: u32,
    pub misses: u64,
    pub max_load_us: u64,
}

impl PageResidency {
    pub fn validate(&self) -> Result<(), String> {
        if u64::from(self.resident) + u64::from(self.evicted) != u64::from(self.pages)
            || self.unavailable > self.evicted
        {
            return Err("page residency counts disagree".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetStatus {
    pub revision: u32,
    pub key: DataKey,
    pub kind: DatasetKind,
    pub retained: Option<DatasetRevision>,
    pub requested: Option<DatasetRevision>,
    pub visibility: GenerationVisibility,
    pub transfer: Option<TransferProgress>,
    pub failure: Option<DataFailure>,
    pub resources: DataResourceUsage,
    pub counters: DataCounters,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paging: Option<PageResidency>,
}

impl DatasetStatus {
    pub fn validate(&self) -> Result<(), String> {
        if self.revision != SCHEMA_REVISION {
            return Err("unsupported status revision".into());
        }
        self.key.validate()?;
        for revision in [self.retained, self.requested].into_iter().flatten() {
            revision.validate()?;
        }
        if let (Some(retained), Some(requested)) = (self.retained, self.requested) {
            if retained.incarnation != requested.incarnation
                || requested.generation > retained.generation
            {
                return Err("status target is not in the retained incarnation/history".into());
            }
        }
        self.visibility.validate()?;
        if let Some(target) = self.visibility.target() {
            if self.requested != Some(target.dataset) {
                return Err("visibility does not answer the exact requested generation".into());
            }
        }
        if matches!(
            self.visibility,
            GenerationVisibility::Visible { .. }
                | GenerationVisibility::Pending { .. }
                | GenerationVisibility::Superseded { .. }
        ) && self.retained.is_none()
        {
            return Err("visibility progress requires retained data".into());
        }
        if let GenerationVisibility::Superseded { successor, .. } = self.visibility {
            if self.retained.is_none_or(|retained| {
                retained.incarnation != successor.incarnation
                    || retained.generation < successor.generation
            }) {
                return Err("supersession successor was not retained".into());
            }
        }
        if let Some(transfer) = &self.transfer {
            transfer.validate()?;
        }
        if let Some(failure) = &self.failure {
            failure.validate()?;
        }
        self.resources.validate()
    }

    pub fn to_json(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        encode(self)
    }
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        let value: Self = decode(bytes)?;
        value.validate()?;
        Ok(value)
    }
}

fn encode(value: &impl Serialize) -> Result<Vec<u8>, String> {
    struct Bounded(Vec<u8>);
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.0.len().saturating_add(bytes.len()) > DATA_METADATA_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::FileTooLarge,
                    "data summary/status exceeds metadata limit",
                ));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut out = Bounded(Vec::with_capacity(DATA_METADATA_BYTES));
    serde_json::to_writer(&mut out, value).map_err(|e| e.to_string())?;
    Ok(out.0)
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    if bytes.len() > DATA_METADATA_BYTES {
        return Err("data summary/status exceeds metadata limit".into());
    }
    serde_json::from_slice(bytes).map_err(|e| format!("invalid data summary/status: {e}"))
}
