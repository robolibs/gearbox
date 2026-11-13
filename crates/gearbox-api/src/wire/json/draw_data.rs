use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::TypeDesc;
use crate::wire::{Env, draw_data::*};

#[derive(Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RequestJson {
    revision: u32,
    operation: u32,
    flags: u32,
    reserved: u32,
    host_epoch: u64,
    lease: u64,
    transaction: u64,
    incarnation: u64,
    base_generation: u64,
    scan_key: u64,
    page_id: u64,
    offset: u64,
    total_bytes: u64,
    key_json: String,
    metadata_json: String,
    payload_hex: String,
}

impl Default for RequestJson {
    fn default() -> Self {
        Self {
            revision: SCHEMA_REVISION,
            operation: 0,
            flags: 0,
            reserved: 0,
            host_epoch: 0,
            lease: 0,
            transaction: 0,
            incarnation: 0,
            base_generation: 0,
            scan_key: 0,
            page_id: 0,
            offset: 0,
            total_bytes: 0,
            key_json: String::new(),
            metadata_json: String::new(),
            payload_hex: "0x".into(),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ReplyJson {
    revision: u32,
    code: u32,
    stage: u32,
    reserved: u32,
    host_epoch: u64,
    lease: u64,
    transaction: u64,
    incarnation: u64,
    stored_generation: u64,
    visible_generation: u64,
    view_id: u64,
    view_epoch: u64,
    metadata_json: String,
    message: String,
}

impl Default for ReplyJson {
    fn default() -> Self {
        Self {
            revision: SCHEMA_REVISION,
            code: 0,
            stage: 0,
            reserved: 0,
            host_epoch: 1,
            lease: 0,
            transaction: 0,
            incarnation: 0,
            stored_generation: 0,
            visible_generation: 0,
            view_id: 0,
            view_epoch: 0,
            metadata_json: String::new(),
            message: String::new(),
        }
    }
}

pub(super) fn request_desc() -> TypeDesc {
    TypeDesc {
        name: DataRequest::CANONICAL_NAME,
        type_hash: DataRequest::TYPE_HASH,
        schema: || datapod::schema::describe::<DataRequest>(DataRequest::CANONICAL_NAME),
        to_json: request_to_json,
        from_json: request_from_json,
    }
}

pub(super) fn reply_desc() -> TypeDesc {
    TypeDesc {
        name: DataReply::CANONICAL_NAME,
        type_hash: DataReply::TYPE_HASH,
        schema: || datapod::schema::describe::<DataReply>(DataReply::CANONICAL_NAME),
        to_json: reply_to_json,
        from_json: reply_from_json,
    }
}

fn request_to_json(wire: &[u8]) -> Result<Value, String> {
    let r = DataRequest::unpack_checked(DataRequest::TYPE_HASH, wire)?;
    r.operation()?;
    serde_json::to_value(RequestJson {
        revision: r.revision,
        operation: r.operation,
        flags: r.flags,
        reserved: r.reserved,
        host_epoch: r.host_epoch,
        lease: r.lease,
        transaction: r.transaction,
        incarnation: r.incarnation,
        base_generation: r.base_generation,
        scan_key: r.scan_key,
        page_id: r.page_id,
        offset: r.offset,
        total_bytes: r.total_bytes,
        key_json: utf8(r.key)?,
        metadata_json: utf8(r.metadata)?,
        payload_hex: format!("0x{}", super::hex_encode(&r.payload)),
    })
    .map_err(|e| e.to_string())
}

fn request_from_json(value: &Value) -> Result<Env, String> {
    lengths(
        value,
        &[
            ("key_json", DATA_KEY_BYTES),
            ("metadata_json", DATA_METADATA_BYTES),
            ("payload_hex", 2 + 2 * DATA_CHUNK_BYTES),
        ],
    )?;
    let r = RequestJson::deserialize(value).map_err(|e| e.to_string())?;
    let request = DataRequest {
        revision: r.revision,
        operation: r.operation,
        flags: r.flags,
        reserved: r.reserved,
        host_epoch: r.host_epoch,
        lease: r.lease,
        transaction: r.transaction,
        incarnation: r.incarnation,
        base_generation: r.base_generation,
        scan_key: r.scan_key,
        page_id: r.page_id,
        offset: r.offset,
        total_bytes: r.total_bytes,
        key: r.key_json.into_bytes(),
        metadata: r.metadata_json.into_bytes(),
        payload: hex_decode(&r.payload_hex)?,
    };
    request.operation()?;
    request.pack_checked()
}

fn reply_to_json(wire: &[u8]) -> Result<Value, String> {
    let r = DataReply::unpack_checked(DataReply::TYPE_HASH, wire)?;
    serde_json::to_value(ReplyJson {
        revision: r.revision,
        code: r.code,
        stage: r.stage,
        reserved: r.reserved,
        host_epoch: r.host_epoch,
        lease: r.lease,
        transaction: r.transaction,
        incarnation: r.incarnation,
        stored_generation: r.stored_generation,
        visible_generation: r.visible_generation,
        view_id: r.view_id,
        view_epoch: r.view_epoch,
        metadata_json: utf8(r.metadata)?,
        message: utf8(r.message)?,
    })
    .map_err(|e| e.to_string())
}

fn reply_from_json(value: &Value) -> Result<Env, String> {
    lengths(
        value,
        &[("metadata_json", DATA_METADATA_BYTES), ("message", 512)],
    )?;
    let r = ReplyJson::deserialize(value).map_err(|e| e.to_string())?;
    DataReply {
        revision: r.revision,
        code: r.code,
        stage: r.stage,
        reserved: r.reserved,
        host_epoch: r.host_epoch,
        lease: r.lease,
        transaction: r.transaction,
        incarnation: r.incarnation,
        stored_generation: r.stored_generation,
        visible_generation: r.visible_generation,
        view_id: r.view_id,
        view_epoch: r.view_epoch,
        metadata: r.metadata_json.into_bytes(),
        message: r.message.into_bytes(),
    }
    .pack_checked()
}

fn utf8(bytes: Vec<u8>) -> Result<String, String> {
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

fn lengths(value: &Value, fields: &[(&str, usize)]) -> Result<(), String> {
    for (name, limit) in fields {
        if value
            .get(*name)
            .and_then(Value::as_str)
            .is_some_and(|s| s.len() > *limit)
        {
            return Err(format!("{name} exceeds its encoded data limit"));
        }
    }
    Ok(())
}

fn hex_decode(value: &str) -> Result<Vec<u8>, String> {
    let value = value
        .strip_prefix("0x")
        .ok_or("payload_hex requires a 0x prefix")?;
    if value.len() % 2 != 0 || value.len() / 2 > DATA_CHUNK_BYTES {
        return Err("invalid data hex payload length".into());
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |b: u8| match b {
                b'0'..=b'9' => Some(b - b'0'),
                b'a'..=b'f' => Some(b - b'a' + 10),
                b'A'..=b'F' => Some(b - b'A' + 10),
                _ => None,
            };
            Ok(digit(pair[0]).ok_or("invalid payload hex digit")? * 16
                + digit(pair[1]).ok_or("invalid payload hex digit")?)
        })
        .collect()
}
