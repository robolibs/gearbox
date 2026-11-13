use gearbox_api::wire::{draw_data::*, json};
use serde_json::Value;

#[path = "support/draw_data_wire.rs"]
mod fixtures;

fn frozen() -> Value {
    serde_json::from_str(include_str!("fixtures/draw_data/wire-v1.json")).unwrap()
}

fn bytes(packet: &Value) -> Vec<u8> {
    packet["wire_hex"]
        .as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}

#[test]
fn rust_packets_and_page_bytes_match_the_frozen_revision() {
    assert_eq!(fixtures::fixture(), frozen());
}

#[test]
fn every_frozen_packet_roundtrips_through_the_actual_cli_json_descriptors() {
    for packet in frozen()["packets"].as_array().unwrap() {
        let name = packet["canonical"].as_str().unwrap();
        let env = json::env_from_json(name, &packet["json"]).unwrap();
        assert_eq!(env.type_hash(), packet["type_hash"].as_u64().unwrap());
        assert_eq!(env.wire(), bytes(packet), "{}", packet["name"]);
        assert_eq!(json::env_to_json(&env).unwrap(), packet["json"]);
        if name == DataRequest::CANONICAL_NAME {
            DataRequest::unpack_checked(env.type_hash(), env.wire())
                .unwrap()
                .operation()
                .unwrap();
        } else if name == DataReply::CANONICAL_NAME {
            DataReply::unpack_checked(env.type_hash(), env.wire()).unwrap();
        }
    }
}

#[test]
fn frozen_pages_decode_with_their_typed_schema_and_checksum() {
    for page in frozen()["pages"].as_array().unwrap() {
        let bytes = bytes(page);
        assert_eq!(
            u64::from(page_crc32(&bytes)),
            page["crc32"].as_u64().unwrap()
        );
        if page["schema"].is_object() {
            let schema: CloudSchema = serde_json::from_value(page["schema"].clone()).unwrap();
            let decoded = CloudPage::decode(&schema, &bytes).unwrap();
            assert_eq!(decoded.count, 2);
            assert_eq!(decoded.encode(&schema).unwrap(), bytes);
        } else {
            let config: OctreeConfig = serde_json::from_value(page["config"].clone()).unwrap();
            let leaves = decode_occupancy_page(&config, &bytes).unwrap();
            assert_eq!(serde_json::to_value(&leaves).unwrap(), page["leaves"]);
            assert_eq!(encode_occupancy_page(&config, &leaves).unwrap(), bytes);
        }
    }
}
