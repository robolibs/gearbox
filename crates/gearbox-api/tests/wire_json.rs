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
        &json!({ "id": "bale_1", "path": "markers/bale.usdz", "x": 1.5, "category": "machine", "machine_id": "oxbo" }),
    )
    .expect("usd_load from json");
    let load: UsdLoad = gearbox_api::unpack(env.type_hash(), env.wire()).expect("decode");
    assert_eq!(load.id(), "bale_1");
    assert_eq!(load.path().as_deref(), Some("markers/bale.usdz"));
    assert_eq!(load.x, 1.5);
    assert_eq!(load.category, category::MACHINE);
    assert_eq!(load.machine_id().as_deref(), Some("oxbo"));

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

#[test]
fn measurements_and_emit_requests_round_trip_through_json() {
    use gearbox_api::{EmitRequest, Measurement, Props, measurement_kind};
    let radar = Measurement {
        sim_time_s: 2.5,
        link_index: 4,
        kind: measurement_kind::RADAR,
        stamp_ms: 10,
        sample: 7,
        count: 2,
        values: vec![3.0, 0.1, -0.2, 0.0, -60.0, f64::INFINITY, 0.0, 0.0, 1.5, -70.0],
        props: Props::from_pairs(&[("name", "radar_link")]).into_bytes(),
        ..Default::default()
    };
    let value = env_to_json(&pack(&radar)).expect("measurement to json");
    assert_eq!(value["kind"], "radar");
    assert_eq!(value["records"][0][0], 3.0);
    assert!(value["records"][1][0].is_null());
    let back = env_from_json("gearbox.measurement.v1", &value).expect("measurement from json");
    let back: Measurement = gearbox_api::unpack(back.type_hash(), back.wire()).expect("decode");
    assert_eq!((back.kind, back.count, back.sample, back.name()), (radar.kind, 2, 7, "radar_link".into()));
    assert!(back.values[5].is_infinite() && back.values[9] == -70.0);

    let emit = env_from_json("gearbox.emit_request.v1", &json!({ "data": "hello", "link": "emitter_link" }))
        .expect("emit from json");
    let emit: EmitRequest = gearbox_api::unpack(emit.type_hash(), emit.wire()).expect("decode");
    assert_eq!((emit.link().as_str(), emit.data.as_slice()), ("emitter_link", &b"hello"[..]));
}

#[test]
fn actuator_commands_and_readings_round_trip_through_json() {
    use gearbox_api::{ActuatorCommand, Measurement, Props, measurement_kind};
    let command = env_from_json("gearbox.actuator_command.v1", &json!({ "device": "boom", "position": "0.8", "pid": "4,0,0" }))
        .expect("actuator command from json");
    let command: ActuatorCommand = gearbox_api::unpack(command.type_hash(), command.wire()).expect("decode");
    assert_eq!(command.device(), "boom");
    assert_eq!(command.props().get("position").as_deref(), Some("0.8"));
    assert_eq!(command.props().get("pid").as_deref(), Some("4,0,0"));

    let motor = Measurement {
        kind: measurement_kind::MOTOR,
        count: 1,
        values: vec![0.4, 0.0, 0.0, 19.6, 300.0, -1.5, 0.0, 0.4],
        props: Props::from_pairs(&[("name", "boom")]).into_bytes(),
        ..Default::default()
    };
    let value = env_to_json(&pack(&motor)).expect("motor to json");
    assert_eq!(value["kind"], "motor");
    assert_eq!(value["records"][0][3], 19.6);
    for (kind, width) in [
        (measurement_kind::MOTOR, 8),
        (measurement_kind::PROPELLER, 4),
        (measurement_kind::BELT, 2),
        (measurement_kind::CONNECTOR, 5),
    ] {
        assert_eq!(measurement_kind::width(kind), width, "{}", measurement_kind::name(kind));
    }
}
