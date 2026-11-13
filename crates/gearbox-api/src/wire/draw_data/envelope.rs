use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::host::MAX_PAYLOAD_BYTES;
use crate::wire::{Env, pack, unpack};

use super::{DataDescriptor, DataKey, MAX_ENCODED_PAGE_BYTES, MAX_SCANS, SCHEMA_REVISION};

pub const DATA_CHUNK_BYTES: usize = 12 * 1024;
pub const DATA_METADATA_BYTES: usize = 12 * 1024;
pub const DATA_KEY_BYTES: usize = 1024;
pub const MAX_DECLARED_PAGES: u32 = 65_536;

pub mod data_op {
    pub const CAPABILITIES: u32 = 0;
    pub const OPEN_WRITER: u32 = 1;
    pub const CLOSE_WRITER: u32 = 2;
    pub const BEGIN: u32 = 3;
    pub const DECLARE_SCAN: u32 = 4;
    pub const DECLARE_PAGE: u32 = 5;
    pub const CHUNK: u32 = 6;
    pub const REMOVE_PAGE: u32 = 7;
    pub const REMOVE_SCAN: u32 = 8;
    pub const COMMIT: u32 = 9;
    pub const CANCEL: u32 = 10;
    pub const PATCH_PRESENTATION: u32 = 11;
    pub const DELETE: u32 = 12;
    pub const CLEAR: u32 = 13;
    pub const LIST: u32 = 14;
    pub const STATUS: u32 = 15;
    pub const ATTACH_PROVIDER: u32 = 16;
    pub const FIELD_QUERY: u32 = 17;
}

pub mod data_code {
    pub const OK: u32 = 0;
    pub const UNSUPPORTED: u32 = 1;
    pub const INVALID: u32 = 2;
    pub const CONFLICT: u32 = 3;
    pub const BUDGET: u32 = 4;
    pub const EXPIRED: u32 = 5;
    pub const NOT_FOUND: u32 = 6;
    pub const STALE_EPOCH: u32 = 7;
    pub const SUPERSEDED: u32 = 8;
    pub const VISIBILITY_UNAVAILABLE: u32 = 9;
    pub const VIEW_GONE: u32 = 10;
    pub const HISTORY_EXPIRED: u32 = 11;
    pub const INTERNAL: u32 = 12;
    pub const STALE_CURSOR: u32 = 13;
}

pub mod data_stage {
    pub const NONE: u32 = 0;
    pub const RECEIVED: u32 = 1;
    pub const RETAINED: u32 = 2;
    pub const VISIBLE: u32 = 3;
}

#[datapod::datapod(name = "gearbox.draw.data.request.v1")]
#[derive(Default)]
pub struct DataRequest {
    pub revision: u32,
    pub operation: u32,
    pub flags: u32,
    pub reserved: u32,
    pub host_epoch: u64,
    pub lease: u64,
    pub transaction: u64,
    pub incarnation: u64,
    pub base_generation: u64,
    pub scan_key: u64,
    pub page_id: u64,
    pub offset: u64,
    pub total_bytes: u64,
    #[dp(bytes, section = "key")]
    pub key: Vec<u8>,
    #[dp(bytes, section = "metadata")]
    pub metadata: Vec<u8>,
    #[dp(bytes, section = "payload")]
    pub payload: Vec<u8>,
}

impl DataRequest {
    pub fn new(operation: u32) -> Self {
        Self {
            revision: SCHEMA_REVISION,
            operation,
            ..Default::default()
        }
    }

    pub fn with_key(mut self, key: &DataKey) -> Result<Self, String> {
        key.validate()?;
        self.key = serde_json::to_vec(key).map_err(|e| e.to_string())?;
        if self.key.len() > DATA_KEY_BYTES {
            return Err("encoded key exceeds 1 KiB".into());
        }
        Ok(self)
    }

    pub fn data_key(&self) -> Result<DataKey, String> {
        let key: DataKey = json(&self.key, DATA_KEY_BYTES)?;
        key.validate()?;
        Ok(key)
    }

    pub fn with_metadata<T: Serialize>(mut self, metadata: &T) -> Result<Self, String> {
        self.metadata = serde_json::to_vec(metadata).map_err(|e| e.to_string())?;
        if self.metadata.len() > DATA_METADATA_BYTES {
            return Err("metadata exceeds 12 KiB".into());
        }
        Ok(self)
    }

    pub fn metadata_as<T: DeserializeOwned>(&self) -> Result<T, String> {
        json(&self.metadata, DATA_METADATA_BYTES)
    }

    /// Validates envelope routing and lengths; the store validates operation metadata.
    pub fn validate(&self) -> Result<(), String> {
        use data_op::*;
        if self.revision != SCHEMA_REVISION
            || self.operation > FIELD_QUERY
            || self.flags != 0
            || self.reserved != 0
        {
            return Err("unsupported request revision/operation/flags".into());
        }
        if self.key.len() > DATA_KEY_BYTES
            || self.metadata.len() > DATA_METADATA_BYTES
            || self.payload.len() > DATA_CHUNK_BYTES
        {
            return Err("request section exceeds byte limit".into());
        }
        let object = matches!(self.operation, BEGIN..=DELETE | STATUS | ATTACH_PROVIDER | FIELD_QUERY);
        if object {
            self.data_key()?;
        } else if !self.key.is_empty() {
            return Err("operation does not accept an object key".into());
        }
        if self.operation != CAPABILITIES && self.host_epoch == 0 {
            return Err("request requires host epoch".into());
        }
        let writer = matches!(self.operation, CLOSE_WRITER..=CLEAR | ATTACH_PROVIDER);
        if writer != (self.lease != 0) {
            return Err("writer lease does not match operation".into());
        }
        let transaction = matches!(self.operation, BEGIN..=CANCEL);
        if transaction != (self.transaction != 0) {
            return Err("transaction ID does not match operation".into());
        }
        if !object && (self.incarnation != 0 || self.base_generation != 0) {
            return Err("operation does not accept dataset incarnation/generation".into());
        }
        let page = matches!(self.operation, DECLARE_PAGE | CHUNK | REMOVE_PAGE);
        let scan = page || matches!(self.operation, DECLARE_SCAN | REMOVE_SCAN);
        if (!page && self.page_id != 0) || (!scan && self.scan_key != 0) {
            return Err("page/scan address does not match operation".into());
        }
        if self.operation == CHUNK {
            if self.payload.is_empty()
                || !self.metadata.is_empty()
                || self.total_bytes != 0
                || self
                    .offset
                    .checked_add(self.payload.len() as u64)
                    .is_none_or(|n| n > MAX_ENCODED_PAGE_BYTES as u64)
            {
                return Err("invalid chunk range or sections".into());
            }
        } else if self.offset != 0 || !self.payload.is_empty() {
            return Err("only chunks carry offsets and payloads".into());
        }
        if self.operation != BEGIN && self.total_bytes != 0 {
            return Err("only BEGIN declares transaction bytes".into());
        }
        let metadata = matches!(
            self.operation,
            BEGIN
                | DECLARE_SCAN
                | DECLARE_PAGE
                | PATCH_PRESENTATION
                | CLEAR
                | LIST
                | STATUS
                | ATTACH_PROVIDER
                | FIELD_QUERY
        );
        if metadata != !self.metadata.is_empty() {
            return Err("metadata presence does not match operation".into());
        }
        if metadata {
            json::<serde_json::Value>(&self.metadata, DATA_METADATA_BYTES)?;
        }
        if self.operation == CAPABILITIES && self.host_epoch != 0 {
            return Err("capability discovery must not assume a host epoch".into());
        }
        Ok(())
    }

    pub fn pack_checked(&self) -> Result<Env, String> {
        self.validate()?;
        let env = pack(self);
        if env.wire().len() > MAX_PAYLOAD_BYTES {
            return Err("packed data request exceeds host payload limit".into());
        }
        Ok(env)
    }

    pub fn unpack_checked(type_hash: u64, wire: &[u8]) -> Result<Self, String> {
        let value = Self::unpack_bounded(type_hash, wire)?;
        value.validate()?;
        Ok(value)
    }

    pub(crate) fn unpack_bounded(type_hash: u64, wire: &[u8]) -> Result<Self, String> {
        if wire.len() > MAX_PAYLOAD_BYTES {
            return Err("data request exceeds host payload limit".into());
        }
        let value: Self = unpack(type_hash, wire).map_err(|e| e.to_string())?;
        if value.key.len() > DATA_KEY_BYTES
            || value.metadata.len() > DATA_METADATA_BYTES
            || value.payload.len() > DATA_CHUNK_BYTES
        {
            return Err("request section exceeds byte limit".into());
        }
        if pack(&value).wire() != wire {
            return Err("noncanonical data request encoding".into());
        }
        Ok(value)
    }
}

#[datapod::datapod(name = "gearbox.draw.data.reply.v1")]
#[derive(Default)]
pub struct DataReply {
    pub revision: u32,
    pub code: u32,
    pub stage: u32,
    pub reserved: u32,
    pub host_epoch: u64,
    pub lease: u64,
    pub transaction: u64,
    pub incarnation: u64,
    pub stored_generation: u64,
    pub visible_generation: u64,
    pub view_id: u64,
    pub view_epoch: u64,
    #[dp(bytes, section = "metadata")]
    pub metadata: Vec<u8>,
    #[dp(bytes, section = "message")]
    pub message: Vec<u8>,
}

impl DataReply {
    pub fn new(host_epoch: u64) -> Self {
        Self {
            revision: SCHEMA_REVISION,
            host_epoch,
            ..Default::default()
        }
    }

    pub fn error(host_epoch: u64, code: u32, message: &str) -> Self {
        let mut bounded = String::new();
        for ch in message.chars().filter(|ch| !ch.is_control()) {
            if bounded.len() + ch.len_utf8() > 512 {
                break;
            }
            bounded.push(ch);
        }
        Self {
            code,
            message: bounded.into_bytes(),
            ..Self::new(host_epoch)
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.revision != SCHEMA_REVISION
            || self.code > data_code::STALE_CURSOR
            || self.stage > data_stage::VISIBLE
            || self.reserved != 0
            || self.host_epoch == 0
        {
            return Err("invalid data reply header".into());
        }
        if self.metadata.len() > DATA_METADATA_BYTES || self.message.len() > 512 {
            return Err("reply section exceeds byte limit".into());
        }
        let message =
            std::str::from_utf8(&self.message).map_err(|_| "reply message is not UTF-8")?;
        if message.chars().any(char::is_control) {
            return Err("reply message contains control characters".into());
        }
        if !self.metadata.is_empty() {
            json::<serde_json::Value>(&self.metadata, DATA_METADATA_BYTES)?;
        }
        if self.visible_generation > self.stored_generation
            || (self.view_id == 0) != (self.view_epoch == 0)
        {
            return Err("invalid visible generation or view identity".into());
        }
        if self.stage >= data_stage::RETAINED
            && (self.incarnation == 0 || self.stored_generation == 0)
        {
            return Err("retained reply requires dataset identity/generation".into());
        }
        if self.stage == data_stage::VISIBLE && (self.visible_generation == 0 || self.view_id == 0)
        {
            return Err("visible reply requires a generation and specific view".into());
        }
        Ok(())
    }

    pub fn pack_checked(&self) -> Result<Env, String> {
        self.validate()?;
        let env = pack(self);
        if env.wire().len() > MAX_PAYLOAD_BYTES {
            return Err("packed reply exceeds host payload limit".into());
        }
        Ok(env)
    }

    pub fn unpack_checked(type_hash: u64, wire: &[u8]) -> Result<Self, String> {
        if wire.len() > MAX_PAYLOAD_BYTES {
            return Err("data reply exceeds host payload limit".into());
        }
        let value: Self = unpack(type_hash, wire).map_err(|e| e.to_string())?;
        value.validate()?;
        if pack(&value).wire() != wire {
            return Err("noncanonical data reply encoding".into());
        }
        Ok(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitMode {
    Replace,
    Delta,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeginMetadata {
    pub descriptor: DataDescriptor,
    pub mode: CommitMode,
    pub scans: u32,
    pub pages: u32,
    pub removed_scans: u32,
    pub removed_pages: u32,
}

impl BeginMetadata {
    pub fn validate(&self) -> Result<(), String> {
        self.descriptor.validate()?;
        if self.scans > MAX_SCANS
            || self.removed_scans > MAX_SCANS
            || self.pages > MAX_DECLARED_PAGES
            || self.removed_pages > MAX_DECLARED_PAGES
        {
            return Err("transaction declaration count exceeds limit".into());
        }
        if self.mode == CommitMode::Replace && (self.removed_scans != 0 || self.removed_pages != 0)
        {
            return Err("replacement must not carry removals".into());
        }
        Ok(())
    }
}

fn json<T: DeserializeOwned>(bytes: &[u8], limit: usize) -> Result<T, String> {
    if bytes.len() > limit {
        return Err("JSON section exceeds byte limit".into());
    }
    serde_json::from_slice(bytes).map_err(|e| format!("invalid data metadata: {e}"))
}
