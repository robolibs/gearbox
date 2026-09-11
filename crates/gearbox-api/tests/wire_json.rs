use gearbox_api::wire::json::{env_from_json, env_to_json, find_by_name, types};
use gearbox_api::{UsdLoad, category, pack};
use serde_json::json;

#[test]
fn every_type_round_trips_through_json() {
    for t in types() {
        let env = (t.from_json)(&json!({})).unwrap_or_else(|e| panic!("{}: {e}", t.name));
        assert_eq!(env.type_hash(), t.type_hash, "{}", t.name);
        let value = env_to_json(&env).unwrap_or_else(|e| panic!("{}: {e}", t.name));
        assert!(value.is_object(), "{}", t.name);
        let schema = (t.schema)();
        assert_eq!(schema.canonical_name, t.name);
    }
}

#[test]
fn flat_keys_become_props_and_names_are_short_or_long() {
    let env = env_from_json(
        "usd_load",
        &json!({ "id": "bale_1", "path": "markers/bale.usdz", "x": 1.5, "category": "machine", "namespace": "oxbo" }),
    )
    .expect("usd_load from json");
    let load: UsdLoad = gearbox_api::unpack(env.type_hash(), env.wire()).expect("decode");
    assert_eq!(load.id(), "bale_1");
    assert_eq!(load.path().as_deref(), Some("markers/bale.usdz"));
    assert_eq!(load.x, 1.5);
    assert_eq!(load.category, category::MACHINE);
    assert_eq!(load.namespace().as_deref(), Some("oxbo"));

    let back = env_to_json(&pack(&load)).expect("to json");
    assert_eq!(back["props"]["id"], "bale_1");
    assert_eq!(back["category_name"], "machine");
    assert!(find_by_name("gearbox.usd_load.v1").is_some());
    assert!(find_by_name("no_such").is_none());
}
