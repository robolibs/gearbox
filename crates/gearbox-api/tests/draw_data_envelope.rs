use gearbox_api::MAX_PAYLOAD_BYTES;
use gearbox_api::wire::{DrawEntity, draw_data::*, pack};

#[allow(dead_code)]
#[path = "support/draw_data_contracts.rs"]
mod contracts;

fn request(op: u32) -> DataRequest {
    use data_op::*;
    let mut request = DataRequest::new(op);
    if op != CAPABILITIES {
        request.host_epoch = 123;
    }
    if matches!(op, CLOSE_WRITER..=CLEAR | ATTACH_PROVIDER) {
        request.lease = 3;
    }
    if matches!(op, BEGIN..=CANCEL) {
        request.transaction = 4;
    }
    if matches!(op, BEGIN..=DELETE | STATUS | ATTACH_PROVIDER) {
        request = request
            .with_key(&DataKey::new("mapping", "data").unwrap())
            .unwrap();
    }
    if matches!(
        op,
        BEGIN
            | DECLARE_SCAN
            | DECLARE_PAGE
            | PATCH_PRESENTATION
            | CLEAR
            | LIST
            | STATUS
            | ATTACH_PROVIDER
    ) {
        request = request.with_metadata(&serde_json::json!({})).unwrap();
    }
    if op == CHUNK {
        request.payload = vec![1];
    }
    request
}

fn reply() -> DataReply {
    DataReply {
        revision: 1,
        host_epoch: 123,
        ..Default::default()
    }
}

#[test]
fn wire_operations_have_frozen_discriminants_and_distinct_identities() {
    use data_op::*;
    assert_eq!(
        [
            CAPABILITIES,
            OPEN_WRITER,
            CLOSE_WRITER,
            BEGIN,
            DECLARE_SCAN,
            DECLARE_PAGE,
            CHUNK,
            REMOVE_PAGE,
            REMOVE_SCAN,
            COMMIT,
            CANCEL,
            PATCH_PRESENTATION,
            DELETE,
            CLEAR,
            LIST,
            STATUS,
            ATTACH_PROVIDER
        ],
        std::array::from_fn::<_, 17, _>(|i| i as u32)
    );
    let legacy = pack(&DrawEntity::clear("mapping"));
    let req = request(CAPABILITIES).pack_checked().unwrap();
    let ans = reply().pack_checked().unwrap();
    assert_ne!(legacy.type_hash(), req.type_hash());
    assert_ne!(legacy.type_hash(), ans.type_hash());
    assert_ne!(req.type_hash(), ans.type_hash());
    assert!(DataRequest::unpack_checked(legacy.type_hash(), legacy.wire()).is_err());
    assert!(DataReply::unpack_checked(req.type_hash(), req.wire()).is_err());
}

#[test]
fn each_envelope_operation_roundtrips_under_host_payload_limit() {
    for op in data_op::CAPABILITIES..=data_op::ATTACH_PROVIDER {
        let request = request(op);
        let env = request.pack_checked().unwrap();
        assert!(env.wire().len() <= MAX_PAYLOAD_BYTES);
        assert_eq!(
            DataRequest::unpack_checked(env.type_hash(), env.wire()).unwrap(),
            request
        );
    }
}

#[test]
fn maximum_chunk_and_metadata_use_real_packed_message_size() {
    let mut chunk = request(data_op::CHUNK);
    chunk.scan_key = u64::MAX;
    chunk.page_id = u64::MAX;
    chunk.offset = 65_536;
    chunk.payload = vec![255; DATA_CHUNK_BYTES];
    let env = chunk.pack_checked().unwrap();
    assert!(env.wire().len() > DATA_CHUNK_BYTES);
    assert!(env.wire().len() <= MAX_PAYLOAD_BYTES);
    chunk.payload.push(1);
    assert!(chunk.pack_checked().is_err());
    let begin = request(data_op::BEGIN)
        .with_metadata(&"x".repeat(DATA_METADATA_BYTES - 2))
        .unwrap();
    assert_eq!(begin.metadata.len(), DATA_METADATA_BYTES);
    assert!(begin.pack_checked().unwrap().wire().len() <= MAX_PAYLOAD_BYTES);
    assert!(
        request(data_op::BEGIN)
            .with_metadata(&"x".repeat(DATA_METADATA_BYTES))
            .is_err()
    );
}

#[test]
fn envelope_checks_reject_wrong_routes_and_extraneous_fields() {
    for field in 0..9 {
        let mut request = request(data_op::CAPABILITIES);
        match field {
            0 => request.revision = 2,
            1 => request.operation = u32::MAX,
            2 => request.flags = 1,
            3 => request.reserved = 1,
            4 => request.host_epoch = 1,
            5 => request.lease = 1,
            6 => request.transaction = 1,
            7 => request.metadata = b"{}".to_vec(),
            _ => request.payload = vec![1],
        }
        assert!(request.pack_checked().is_err(), "case {field}");
    }
    let mut begin = request(data_op::BEGIN);
    begin.lease = 0;
    assert!(begin.validate().is_err());
    begin = request(data_op::BEGIN);
    begin.transaction = 0;
    assert!(begin.validate().is_err());
    begin = request(data_op::BEGIN);
    begin.key.clear();
    assert!(begin.validate().is_err());
    let mut query = request(data_op::STATUS);
    query.lease = 2;
    assert!(query.validate().is_err());
    query = request(data_op::STATUS);
    query.page_id = 1;
    assert!(query.validate().is_err());
}

#[test]
fn chunks_reject_empty_overflowing_and_out_of_page_ranges() {
    let mut chunk = request(data_op::CHUNK);
    chunk.payload.clear();
    assert!(chunk.validate().is_err());
    chunk.payload = vec![0];
    chunk.offset = u64::MAX;
    assert!(chunk.validate().is_err());
    chunk.offset = MAX_ENCODED_PAGE_BYTES as u64;
    assert!(chunk.validate().is_err());
    chunk.offset -= 1;
    chunk.validate().unwrap();
    chunk.total_bytes = 1;
    assert!(chunk.validate().is_err());
    chunk.total_bytes = 0;
    chunk.metadata = b"{}".to_vec();
    assert!(chunk.validate().is_err());
}

#[test]
fn malformed_and_noncanonical_packets_fail_before_dispatch() {
    let env = request(data_op::BEGIN).pack_checked().unwrap();
    for end in 0..env.wire().len() {
        assert!(
            DataRequest::unpack_checked(env.type_hash(), &env.wire()[..end]).is_err(),
            "prefix {end}"
        );
    }
    let mut trailing = env.wire().to_vec();
    trailing.push(0);
    assert!(DataRequest::unpack_checked(env.type_hash(), &trailing).is_err());
    assert!(DataRequest::unpack_checked(env.type_hash(), &vec![0; MAX_PAYLOAD_BYTES + 1]).is_err());
    let mut bad = request(data_op::BEGIN);
    bad.metadata = vec![0xff];
    assert!(bad.pack_checked().is_err());
    bad.metadata = b"{} garbage".to_vec();
    assert!(bad.pack_checked().is_err());
    bad = request(data_op::BEGIN);
    bad.key = br#"{"namespace":"n","id":"i","extra":1}"#.to_vec();
    assert!(bad.pack_checked().is_err());
}

#[test]
fn begin_metadata_is_bounded_and_separate_from_page_payload() {
    let mut metadata = BeginMetadata {
        descriptor: contracts::cloud(),
        mode: CommitMode::Replace,
        scans: 2,
        pages: 10,
        removed_scans: 0,
        removed_pages: 0,
    };
    metadata.validate().unwrap();
    let request = request(data_op::BEGIN).with_metadata(&metadata).unwrap();
    let decoded: BeginMetadata = request.metadata_as().unwrap();
    assert_eq!(decoded, metadata);
    decoded.validate().unwrap();
    metadata.removed_pages = 1;
    assert!(metadata.validate().is_err());
    metadata.mode = CommitMode::Delta;
    metadata.validate().unwrap();
    metadata.pages = MAX_DECLARED_PAGES + 1;
    assert!(metadata.validate().is_err());
    metadata.pages = 1;
    metadata.scans = MAX_SCANS + 1;
    assert!(metadata.validate().is_err());
}

#[test]
fn replies_separate_received_retained_and_view_scoped_visibility() {
    let mut response = reply();
    response.stage = data_stage::RECEIVED;
    response.validate().unwrap();
    response.stage = data_stage::RETAINED;
    assert!(response.validate().is_err());
    response.incarnation = 1;
    response.stored_generation = 2;
    response.validate().unwrap();
    response.stage = data_stage::VISIBLE;
    assert!(response.validate().is_err());
    response.visible_generation = 1;
    response.view_id = 10;
    response.view_epoch = 20;
    let env = response.pack_checked().unwrap();
    assert_eq!(
        DataReply::unpack_checked(env.type_hash(), env.wire()).unwrap(),
        response
    );
    response.visible_generation = 3;
    assert!(response.validate().is_err());
    response.visible_generation = 1;
    response.view_epoch = 0;
    assert!(response.validate().is_err());
}

#[test]
fn reply_errors_are_bounded_utf8_and_keep_terminal_failure_codes() {
    let mut response = reply();
    response.code = data_code::SUPERSEDED;
    response.metadata = br#"{"successor_generation":3}"#.to_vec();
    response.message = b"generation superseded before rendering".to_vec();
    response.pack_checked().unwrap();
    for message in [vec![b'x'; 513], vec![0xff], b"line\nbreak".to_vec()] {
        response.message = message;
        assert!(response.validate().is_err());
    }
    response.message.clear();
    response.code = u32::MAX;
    assert!(response.validate().is_err());
    response.code = data_code::OK;
    response.metadata = vec![b' '; DATA_METADATA_BYTES + 1];
    assert!(response.validate().is_err());
}
