use gearbox_api::wire::{draw_data::*, json, pack};

#[test]
fn request_json_preserves_raw_section_order_spacing_and_u64_header_fields() {
    let mut request = DataRequest::new(data_op::STATUS);
    request.host_epoch = u64::MAX;
    request.key = br#"{ "id":"cloud", "namespace":"mapping" }"#.to_vec();
    request.metadata = br#"{ "view":null,"generation":18446744073709551615 }"#.to_vec();
    let env = request.pack_checked().unwrap();
    let value = json::env_to_json(&env).unwrap();
    assert_eq!(value["host_epoch"].as_u64(), Some(u64::MAX));
    let restored = json::env_from_json("gearbox.draw.data.request.v1", &value).unwrap();
    assert_eq!(restored.wire(), env.wire());
    assert_eq!(restored.type_hash(), env.type_hash());
}

#[test]
fn default_json_builds_a_valid_discovery_request_and_schema_is_registered() {
    let env = json::env_from_json("gearbox.draw.data.request.v1", &serde_json::json!({})).unwrap();
    assert_eq!(
        DataRequest::unpack_checked(env.type_hash(), env.wire()).unwrap(),
        DataRequest::new(data_op::CAPABILITIES)
    );
    for name in ["gearbox.draw.data.request.v1", "gearbox.draw.data.reply.v1"] {
        assert_eq!(json::find_by_name(name).unwrap().name, name);
    }
}

#[test]
fn json_chunk_payload_has_explicit_bounded_hex_and_exact_u64_offsets() {
    let mut request = DataRequest::new(data_op::CHUNK)
        .with_key(&DataKey::new("mapping", "cloud").unwrap())
        .unwrap();
    request.host_epoch = u64::MAX;
    request.lease = u64::MAX;
    request.transaction = u64::MAX;
    request.page_id = u64::MAX;
    request.scan_key = u64::MAX;
    request.payload = vec![0, 255, 128, 1];
    let env = request.pack_checked().unwrap();
    let mut value = json::env_to_json(&env).unwrap();
    assert_eq!(value["payload_hex"], "0x00ff8001");
    assert_eq!(
        json::env_from_json("gearbox.draw.data.request.v1", &value)
            .unwrap()
            .wire(),
        env.wire()
    );
    for invalid in ["0x0", "00", "0xzz", "0xéé"] {
        value["payload_hex"] = invalid.into();
        assert!(json::env_from_json("gearbox.draw.data.request.v1", &value).is_err());
    }
    value["payload_hex"] = format!("0x{}", "ff".repeat(DATA_CHUNK_BYTES + 1)).into();
    assert!(json::env_from_json("gearbox.draw.data.request.v1", &value).is_err());
}

#[test]
fn reply_json_preserves_metadata_and_utf8_and_rejects_invalid_fields() {
    let mut reply = DataReply::error(u64::MAX, data_code::UNSUPPORTED, "énergie");
    reply.metadata = br#"{ "supported":false }"#.to_vec();
    let env = reply.pack_checked().unwrap();
    let mut value = json::env_to_json(&env).unwrap();
    assert_eq!(
        json::env_from_json("gearbox.draw.data.reply.v1", &value)
            .unwrap()
            .wire(),
        env.wire()
    );
    value["message"] = "x".repeat(513).into();
    assert!(json::env_from_json("gearbox.draw.data.reply.v1", &value).is_err());
    value["message"] = "bad\n".into();
    assert!(json::env_from_json("gearbox.draw.data.reply.v1", &value).is_err());
}

#[test]
fn json_entrypoints_enforce_typed_metadata_and_do_not_hide_unknown_fields_in_props() {
    let mut request = DataRequest::new(data_op::LIST);
    request.host_epoch = 1;
    request.metadata = br#"{"namespace":"", "limit":0, "cursor":null}"#.to_vec();
    assert!(json::env_to_json(&pack(&request)).is_err());
    for value in [
        serde_json::json!({"unexpected":0}),
        serde_json::json!({"operation":-1}),
        serde_json::json!({"host_epoch":1.5}),
        serde_json::json!({"metadata_json":"x".repeat(DATA_METADATA_BYTES + 1)}),
    ] {
        assert!(json::env_from_json("gearbox.draw.data.request.v1", &value).is_err());
    }
}
