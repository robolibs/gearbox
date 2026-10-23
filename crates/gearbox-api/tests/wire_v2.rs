//! The versioned robot-contract messages round-trip, and the v1 payloads the
//! compatibility endpoints carry keep their exact bytes.

use gearbox_api::*;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Fixed v1 payloads: name, type hash and wire bytes.
fn v1_payloads() -> Vec<(&'static str, Env)> {
    let props = |pairs: &[(&str, &str)]| Props::from_pairs(pairs).into_bytes();
    vec![
        ("twist_cmd", pack(&TwistCmd::new(7, 1.5, -0.25))),
        ("claim_request", pack(&ClaimRequest::new("client", 500))),
        ("controller_command", pack(&ControllerCommand { session: 7, value: 0.5, element: 2, _pad: 0, props: props(&[("position", "0.5")]) })),
        ("actuator_command", pack(&ActuatorCommand { stamp_ms: 12, _pad: 0, props: props(&[("device", "boom"), ("position", "0.4")]) })),
        (
            "link_pose",
            pack(&LinkPose { x: 1.0, y: 2.0, z: 3.0, qw: 1.0, qx: 0.0, qy: 0.0, qz: 0.0, index: 4, stamp_ms: 9, props: props(&[("name", "base_link")]) }),
        ),
        (
            "link_record",
            pack(&LinkRecord {
                x: 0.5,
                y: 0.0,
                z: 1.0,
                qw: 1.0,
                qx: 0.0,
                qy: 0.0,
                qz: 0.0,
                index: 1,
                parent_index: 0,
                props: props(&[("name", "tool_tip"), ("parent", "arm"), ("role", "tool")]),
            }),
        ),
    ]
}

fn golden_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/v1_payloads.txt")
}

fn render() -> String {
    v1_payloads().into_iter().map(|(name, env)| format!("{name} {:016x} {}\n", env.type_hash(), hex(env.wire()))).collect()
}

/// Writes the v1 golden payloads; run on purpose only.
#[test]
#[ignore]
fn write_v1_golden_payloads() {
    assert_eq!(std::env::var("GEARBOX_WRITE_GOLDEN").as_deref(), Ok("1"), "set GEARBOX_WRITE_GOLDEN=1");
    std::fs::create_dir_all(golden_path().parent().unwrap()).unwrap();
    std::fs::write(golden_path(), render()).unwrap();
}

#[test]
fn v1_payloads_keep_their_bytes() {
    let golden = std::fs::read_to_string(golden_path()).expect("golden v1 payloads");
    assert_eq!(render(), golden, "a v1 payload changed shape");
}

#[test]
fn v2_messages_round_trip() {
    let command = JointCommandV2 {
        session: 3,
        epoch: 9,
        sequence: 41,
        generation: 2,
        max_effort: 12.5,
        validity_ms: 250,
        _pad: 0,
        modes: vec![joint_mode::POSITION, joint_mode::EFFORT],
        values: vec![0.3, -4.0],
        feedforward: vec![0.1, f64::NAN],
        names: names_to_bytes(&["shoulder", "slide:transX"]),
    };
    let env = pack(&command);
    let back: JointCommandV2 = unpack(env.type_hash(), env.wire()).unwrap();
    assert_eq!((back.session, back.epoch, back.sequence, back.generation, back.validity_ms), (3, 9, 41, 2, 250));
    assert_eq!(back.max_effort, 12.5);
    assert_eq!(back.modes, command.modes);
    assert_eq!(back.values, command.values);
    assert_eq!(back.feedforward[0], 0.1);
    assert!(back.feedforward[1].is_nan());
    assert_eq!(back.names(), vec!["shoulder".to_string(), "slide:transX".to_string()]);

    let state = JointStateV2 {
        generation: 2,
        tick: 77,
        sim_time_s: 0.64,
        valid: 1,
        _pad: 0,
        position: vec![0.1],
        velocity: vec![0.2],
        measured_effort: vec![f64::NAN],
        commanded_effort: vec![3.0],
        target: vec![0.3],
        names: names_to_bytes(&["shoulder"]),
    };
    let env = pack(&state);
    let back: JointStateV2 = unpack(env.type_hash(), env.wire()).unwrap();
    assert_eq!((back.generation, back.tick, back.valid), (2, 77, 1));
    assert!(back.measured_effort[0].is_nan());
    assert_eq!(back.commanded_effort, vec![3.0]);

    let tf = TfV2 {
        generation: 2,
        tick: 77,
        sim_time_s: 0.64,
        translation: vec![1.0, 2.0, 3.0],
        rotation: vec![1.0, 0.0, 0.0, 0.0],
        parents: names_to_bytes(&["base_link"]),
        children: names_to_bytes(&["tool_tip"]),
        props: Props::from_pairs(&[("convention", "right_handed_z_up")]).into_bytes(),
    };
    let env = pack(&tf);
    let back: TfV2 = unpack(env.type_hash(), env.wire()).unwrap();
    assert_eq!((back.parents(), back.children()), (vec!["base_link".to_string()], vec!["tool_tip".to_string()]));
    assert_eq!(back.props().get("convention").as_deref(), Some("right_handed_z_up"));

    let request = ControlRequestV2 { session: 3, generation: 2, epoch: 0, op: control_op::ACQUIRE, resume: 0, names: names_to_bytes(&["shoulder"]) };
    let env = pack(&request);
    let back: ControlRequestV2 = unpack(env.type_hash(), env.wire()).unwrap();
    assert_eq!(names_from_bytes(&back.names), vec!["shoulder".to_string()]);

    let ack = JointCommandAckV2 { sequence: 41, generation: 2, tick: 78, status: command_status::REJECTED, code: code::UNSUPPORTED, message: b"unknown joint mode 9".to_vec() };
    let env = pack(&ack);
    let back: JointCommandAckV2 = unpack(env.type_hash(), env.wire()).unwrap();
    assert_eq!((back.status, back.code, back.message()), (command_status::REJECTED, code::UNSUPPORTED, "unknown joint mode 9".to_string()));
}
