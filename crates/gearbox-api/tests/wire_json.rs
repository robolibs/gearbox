use datapod::robot::{Imu, TurnRadius, WheelEncoder, WheelEncoders};
use datapod::{Acceleration, Quaternion, Velocity};
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

/// New sensor types carry no hand-written `TypeDesc`; this exercises the
/// generic schema-driven fallback the CLI's `sub`/`call` commands rely on.
#[test]
fn sensor_types_reflect_generically_with_no_hand_written_desc() {
    let imu = Imu::new(
        Velocity {
            vx: 0.0,
            vy: 0.0,
            vz: 0.3,
        },
        Acceleration {
            ax: 0.1,
            ay: 0.0,
            az: 9.8,
        },
        Quaternion::new(1.0, 0.0, 0.0, 0.0),
    );
    let value = env_to_json(&pack(&imu)).expect("imu to json");
    assert_eq!(value["angular_velocity"]["vz"], 0.3);
    assert_eq!(value["linear_acceleration"]["az"], 9.8);

    let radius = TurnRadius::new(4.2);
    let value = env_to_json(&pack(&radius)).expect("turn radius to json");
    assert_eq!(value["radius_m"], 4.2);

    let encoders = WheelEncoders::new(vec![
        WheelEncoder::new(0, 1.5, 0.4),
        WheelEncoder::new(1, 1.6, 0.42),
    ]);
    let value = env_to_json(&pack(&encoders)).expect("wheel encoders to json");
    let wheels = value["wheels"].as_array().expect("wheels array");
    assert_eq!(wheels.len(), 2);
    assert_eq!(wheels[0]["angle_rad"], 1.5);
    assert_eq!(wheels[1]["velocity_rad_s"], 0.42);
}
