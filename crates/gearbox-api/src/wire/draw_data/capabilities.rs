use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::{
    DATA_CHUNK_BYTES, DATA_METADATA_BYTES, MAX_ENCODED_PAGE_BYTES, MAX_LIST_ITEMS, MAX_PAGE_POINTS,
    MAX_SCANS, SCHEMA_REVISION,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatasetKind {
    Cloud,
    Octree,
    Surfels,
    MeshBlocks,
    Volume,
    Elevation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataLimits {
    pub cpu_bytes: u64,
    pub gpu_bytes: u64,
    pub incomplete_bytes: u64,
    pub writer_incomplete_bytes: u64,
    pub writer_staged_bytes: u64,
    pub client_queue_bytes: u64,
    pub page_bytes: u64,
    pub page_points: u32,
    pub visible_points: u32,
    pub upload_bytes_per_frame: u64,
    pub lease_timeout_ms: u64,
    pub writers: u32,
    pub transactions: u32,
    pub writer_transactions: u32,
    pub datasets: u32,
    pub scans: u32,
    pub pages: u32,
    pub completion_history: u32,
    pub cursors: u32,
    pub cursor_timeout_ms: u64,
    pub list_items: u32,
    pub chunk_bytes: u32,
}

impl Default for DataLimits {
    fn default() -> Self {
        Self::reference()
    }
}

impl DataLimits {
    pub fn reference() -> Self {
        Self {
            cpu_bytes: 1 << 30,
            gpu_bytes: 256 << 20,
            incomplete_bytes: 128 << 20,
            writer_incomplete_bytes: 32 << 20,
            writer_staged_bytes: 256 << 20,
            client_queue_bytes: 32 << 20,
            page_bytes: 32 << 20,
            page_points: MAX_PAGE_POINTS,
            visible_points: 1_000_000,
            upload_bytes_per_frame: 4 << 20,
            lease_timeout_ms: 10_000,
            writers: 32,
            transactions: 64,
            writer_transactions: 4,
            datasets: 4096,
            scans: MAX_SCANS,
            pages: super::MAX_DECLARED_PAGES,
            completion_history: 256,
            cursors: 64,
            cursor_timeout_ms: 10_000,
            list_items: MAX_LIST_ITEMS,
            chunk_bytes: DATA_CHUNK_BYTES as u32,
        }
    }

    pub fn low_memory() -> Self {
        Self {
            cpu_bytes: 128 << 20,
            gpu_bytes: 32 << 20,
            incomplete_bytes: 16 << 20,
            writer_incomplete_bytes: 8 << 20,
            writer_staged_bytes: 32 << 20,
            client_queue_bytes: 8 << 20,
            page_bytes: 8 << 20,
            visible_points: 100_000,
            upload_bytes_per_frame: 1 << 20,
            writers: 8,
            transactions: 16,
            datasets: 256,
            completion_history: 64,
            cursors: 16,
            ..Self::reference()
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        for (value, limit) in [
            (self.cpu_bytes, usize::MAX as u64),
            (self.gpu_bytes, usize::MAX as u64),
            (self.incomplete_bytes, self.cpu_bytes),
            (self.writer_incomplete_bytes, self.incomplete_bytes),
            (self.writer_staged_bytes, self.cpu_bytes),
            (self.client_queue_bytes, self.cpu_bytes),
            (
                self.page_bytes,
                self.writer_incomplete_bytes
                    .min(MAX_ENCODED_PAGE_BYTES as u64),
            ),
            (self.upload_bytes_per_frame, self.gpu_bytes),
            (u64::from(self.chunk_bytes), DATA_CHUNK_BYTES as u64),
        ] {
            if value == 0 || value > limit {
                return Err("data byte limit exceeds its containing budget".into());
            }
        }
        if self.page_bytes < 16
            || u64::from(self.chunk_bytes) > self.page_bytes
            || self.lease_timeout_ms == 0
            || self.cursor_timeout_ms == 0
        {
            return Err("invalid page/chunk or timeout limits".into());
        }
        for (value, limit) in [
            (self.page_points, MAX_PAGE_POINTS),
            (self.scans, MAX_SCANS),
            (self.pages, super::MAX_DECLARED_PAGES),
            (self.list_items, MAX_LIST_ITEMS),
            (self.writer_transactions, self.transactions),
            (self.writers, 4096),
            (self.transactions, 65_536),
            (self.datasets, 65_536),
            (self.completion_history, 65_536),
            (self.cursors, 4096),
            (self.visible_points, u32::MAX),
        ] {
            if value == 0 || value > limit {
                return Err("data item limit exceeds its bound".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataFeatures {
    pub atomic_generations: bool,
    pub delta_updates: bool,
    pub accumulation: bool,
    pub presentation_updates: bool,
    pub unified_listing: bool,
    pub per_view_visibility: bool,
    pub picking: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataCapabilities {
    pub revision: u32,
    pub host_epoch: u64,
    pub schemas: Vec<u32>,
    pub decodable: Vec<DatasetKind>,
    pub retained: Vec<DatasetKind>,
    pub rendered: Vec<DatasetKind>,
    pub features: DataFeatures,
    pub limits: DataLimits,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub page_providers: Vec<String>,
}

impl DataCapabilities {
    pub fn wire_only(host_epoch: u64) -> Self {
        Self {
            revision: SCHEMA_REVISION,
            host_epoch,
            schemas: vec![SCHEMA_REVISION],
            decodable: vec![DatasetKind::Cloud, DatasetKind::Octree, DatasetKind::Surfels, DatasetKind::MeshBlocks, DatasetKind::Volume, DatasetKind::Elevation],
            retained: vec![],
            rendered: vec![],
            features: DataFeatures::default(),
            limits: DataLimits::reference(),
            page_providers: vec![],
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.revision != SCHEMA_REVISION
            || self.host_epoch == 0
            || self.schemas.is_empty()
            || self.schemas.len() > 16
            || self.schemas.contains(&0)
            || self.schemas.iter().collect::<HashSet<_>>().len() != self.schemas.len()
        {
            return Err("unsupported capability revision/schema or zero host epoch".into());
        }
        self.limits.validate()?;
        for kinds in [&self.decodable, &self.retained, &self.rendered] {
            if kinds.len() > 16 || kinds.iter().collect::<HashSet<_>>().len() != kinds.len() {
                return Err("duplicate or excessive capability kinds".into());
            }
        }
        if self.retained.iter().any(|k| !self.decodable.contains(k))
            || self.rendered.iter().any(|k| !self.retained.contains(k))
        {
            return Err("render/retention capabilities require their data foundation".into());
        }
        let store = !self.retained.is_empty();
        if self.features.atomic_generations != store
            || (self.features.delta_updates
                || self.features.presentation_updates
                || self.features.unified_listing)
                && !store
            || self.features.accumulation && !self.retained.contains(&DatasetKind::Cloud)
            || (self.features.per_view_visibility || self.features.picking)
                && self.rendered.is_empty()
        {
            return Err("feature flags disagree with available backends".into());
        }
        if self.page_providers.len() > 16
            || !self.page_providers.is_empty() && !store
            || self.page_providers.iter().collect::<HashSet<_>>().len() != self.page_providers.len()
        {
            return Err("page providers require retained storage and unique names".into());
        }
        for name in &self.page_providers {
            super::frames::bounded_text(name, super::MAX_PROVIDER_NAME_BYTES, false)?;
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if bytes.len() > DATA_METADATA_BYTES {
            return Err("capabilities exceed metadata limit".into());
        }
        Ok(bytes)
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > DATA_METADATA_BYTES {
            return Err("capabilities exceed metadata limit".into());
        }
        let value: Self =
            serde_json::from_slice(bytes).map_err(|e| format!("invalid capabilities: {e}"))?;
        value.validate()?;
        Ok(value)
    }
}
