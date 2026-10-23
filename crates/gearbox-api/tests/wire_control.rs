//! The robot-contract messages round-trip.

use gearbox_api::*;

#[test]
fn control_messages_round_trip() {
    let command = JointCommand {
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
    let back: JointCommand = unpack(env.type_hash(), env.wire()).unwrap();
    assert_eq!((back.session, back.epoch, back.sequence, back.generation, back.validity_ms), (3, 9, 41, 2, 250));
    assert_eq!(back.max_effort, 12.5);
    assert_eq!(back.modes, command.modes);
    assert_eq!(back.values, command.values);
    assert_eq!(back.feedforward[0], 0.1);
    assert!(back.feedforward[1].is_nan());
    assert_eq!(back.names(), vec!["shoulder".to_string(), "slide:transX".to_string()]);

    let state = JointState {
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
    let back: JointState = unpack(env.type_hash(), env.wire()).unwrap();
    assert_eq!((back.generation, back.tick, back.valid), (2, 77, 1));
    assert!(back.measured_effort[0].is_nan());
    assert_eq!(back.commanded_effort, vec![3.0]);

    let tf = Tf {
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
    let back: Tf = unpack(env.type_hash(), env.wire()).unwrap();
    assert_eq!((back.parents(), back.children()), (vec!["base_link".to_string()], vec!["tool_tip".to_string()]));
    assert_eq!(back.props().get("convention").as_deref(), Some("right_handed_z_up"));

    let request = ControlRequest { session: 3, generation: 2, epoch: 0, op: control_op::ACQUIRE, resume: 0, names: names_to_bytes(&["shoulder"]) };
    let env = pack(&request);
    let back: ControlRequest = unpack(env.type_hash(), env.wire()).unwrap();
    assert_eq!(names_from_bytes(&back.names), vec!["shoulder".to_string()]);

    let ack = JointCommandAck { sequence: 41, generation: 2, tick: 78, status: command_status::REJECTED, code: code::UNSUPPORTED, message: b"unknown joint mode 9".to_vec() };
    let env = pack(&ack);
    let back: JointCommandAck = unpack(env.type_hash(), env.wire()).unwrap();
    assert_eq!((back.status, back.code, back.message()), (command_status::REJECTED, code::UNSUPPORTED, "unknown joint mode 9".to_string()));
}
