//! Lifecycle cases of plan 001 §9.2: a rejected machine activates nothing,
//! detached joints stop, v2 commands are validated before the arbiter, state
//! never reports a commanded effort as measured, and a failed activation
//! leaves no binding behind.

use std::time::Instant;

use bevy::prelude::*;
use gearbox_api::{JointCommandV2, code, joint_mode, names_to_bytes};
use glam::DVec3;

use super::api::{batch_from_command, sample_joint_state};
use super::{Arbiter, Batch, DofKey, DofSpec, Grant, OwnerClass, OwnerId, Setpoint, Validity};
use crate::controller::{ControllerInventory, ExternalControllerPolicy, ExternalControllerProcesses, MachineAgentKeys};
use crate::machine_model::{CompileOptions, DofToken, Interface};
use crate::physics::PhysicsWorld;
use crate::physics::backend::{BodyDesc, Inertia, JointDesc, JointKind, MassProps, Pose};

fn fixture(name: &str) -> std::path::PathBuf {
    crate::machine_model::tests::fixture(name)
}

fn machines(name: &str) -> Vec<crate::controller::MachineInstanceSpec> {
    let path = fixture(name);
    let stage = openusd::usd::Stage::open(path.to_str().unwrap()).unwrap();
    let model = crate::machine_model::compile(&stage, &CompileOptions::default());
    crate::controller::discover_machines_with_model(&stage, &model).unwrap()
}

/// A rejected machine with a motor device and an external process: no
/// process, no device, no binding, no agent key.
pub(crate) fn c19() {
    let mut machines = machines("c19_rejected_machine.usda");
    assert_eq!(machines.len(), 1);
    assert!(!machines[0].accepted(), "{:?}", machines[0].links.errors);
    assert!(!machines[0].devices.is_empty(), "the fixture has a motor device");
    let mut app = App::new();
    let root = app.world_mut().spawn_empty().id();
    machines[0].scene_root = Some(root);
    let sh = std::path::Path::new("/bin/sh").canonicalize().unwrap();
    app.insert_resource(ControllerInventory { machines })
        .insert_resource(ExternalControllerPolicy { allow_processes: true, allowlist_dirs: vec![sh.parent().unwrap().to_path_buf()] })
        .init_resource::<ExternalControllerProcesses>()
        .init_resource::<crate::devices::MachineDevices>()
        .init_resource::<MachineAgentKeys>()
        .init_resource::<super::runtime::BindingWaits>()
        .insert_resource(PhysicsWorld::default())
        .add_systems(
            Update,
            (
                crate::controller::reconcile_external_process_controllers,
                crate::devices::drive_devices,
                super::runtime::bind_accepted_machines,
            ),
        );
    for _ in 0..3 {
        app.update();
    }
    let processes = app.world().resource::<ExternalControllerProcesses>();
    assert!(processes.children.is_empty(), "no process spawned for a rejected machine: {:?}", processes.status);
    assert_eq!(app.world().resource::<crate::devices::MachineDevices>().count("broken"), 0);
    let physics = app.world().resource::<PhysicsWorld>();
    assert!(physics.control.machines.is_empty());
    assert_eq!(physics.control.arbiter.dofs().count(), 0);
}

/// A slave's master-driven DOFs stop when it is detached and keep their owner.
pub(crate) fn c20() {
    let mut arbiter = Arbiter::new();
    let coupling = OwnerId::new(OwnerClass::Service, "slave", "coupling");
    let key = DofKey { machine: "slave".into(), generation: 1, joint: "/pto".into(), dof: DofToken::Angular };
    arbiter.register(vec![DofSpec {
        key: key.clone(),
        interfaces: vec![Interface::Velocity],
        lower: None,
        upper: None,
        drive: None,
        hardware_cap: None,
        default_owner: Some(coupling.clone()),
        legacy_caps: true,
    }])
    .unwrap();
    let batch = Batch { owner: coupling.clone(), epoch: 0, sequence: 1, commands: vec![(key.clone(), Setpoint::Velocity { qdot: 56.5 })], validity: Validity::Latched, requested_cap: None };
    arbiter.submit(batch, Instant::now()).unwrap();
    arbiter.quiesce(&coupling);
    assert_eq!(arbiter.requested(&key), Some(Setpoint::Velocity { qdot: 0.0 }), "the PTO stops, it is not left spinning");
    assert_eq!(arbiter.owner(&key), Some(&coupling));
    for _ in 0..3 {
        arbiter.quiesce(&coupling);
    }
    assert_eq!(arbiter.requested(&key), Some(Setpoint::Velocity { qdot: 0.0 }), "repeated detach leaks nothing");

    let servo = |qdot| Setpoint::Servo { q: None, qdot, stiffness: 0.0, damping: 200.0, drive: crate::machine_model::DriveType::Acceleration };
    let batch = Batch { owner: coupling.clone(), epoch: 0, sequence: 2, commands: vec![(key.clone(), servo(56.5))], validity: Validity::Latched, requested_cap: None };
    arbiter.submit(batch, Instant::now()).unwrap();
    arbiter.quiesce(&coupling);
    assert_eq!(arbiter.requested(&key), Some(servo(0.0)), "a service's velocity servo stops on its own damping");
}

fn rig_model() -> crate::machine_model::MachineDescription {
    let result = crate::machine_model::compile_file(&fixture("c22_commanded_rig.usda"), &CompileOptions::default()).unwrap();
    let model = result.machines.into_iter().next().unwrap();
    assert!(model.accepted(), "{:?}", model.diagnostics);
    model
}

/// v2 commands convert whole or not at all; unknown modes, names and stale
/// generations are rejected with their code.
pub(crate) fn c22() {
    let model = rig_model();
    let grant = Grant { owner: OwnerId::new(OwnerClass::Direct, "rig", "client#1"), epoch: 1, dofs: vec![] };
    let command = |modes: Vec<u8>, names: &[&str], generation: u64| JointCommandV2 {
        session: 1,
        epoch: 1,
        sequence: 1,
        generation,
        max_effort: f64::NAN,
        validity_ms: 0,
        _pad: 0,
        values: vec![0.25; modes.len()],
        modes,
        feedforward: Vec::new(),
        names: names_to_bytes(names),
    };
    let ok = batch_from_command(&model, "rig", &grant, &command(vec![joint_mode::POSITION], &["shoulder"], model.generation)).unwrap();
    assert_eq!(ok.commands[0].1, Setpoint::Position { q: 0.25, qdot: None });
    assert_eq!(ok.validity, Validity::WallMs(model.command_validity_ms));
    let reject = |c: JointCommandV2| batch_from_command(&model, "rig", &grant, &c).unwrap_err().0;
    assert_eq!(reject(command(vec![9], &["shoulder"], model.generation)), code::UNSUPPORTED);
    assert_eq!(reject(command(vec![joint_mode::EFFORT], &["elbow"], model.generation)), code::NOT_FOUND);
    assert_eq!(reject(command(vec![joint_mode::EFFORT], &["shoulder"], model.generation + 1)), code::REFUSED);
    assert_eq!(reject(command(vec![joint_mode::EFFORT, joint_mode::FREE], &["shoulder"], model.generation)), code::USAGE);
}

/// Measured effort is NaN where the backend has none; the committed effort
/// is reported as commanded, never as measured.
pub(crate) fn c23() {
    let model = rig_model();
    let mut world = PhysicsWorld::default();
    world.set_gravity(DVec3::ZERO);
    let post = world.insert_body(BodyDesc::fixed());
    let mut arm = BodyDesc::dynamic();
    arm.additional_mass = Some(MassProps { mass: 1.0, local_com: DVec3::ZERO, inertia: Inertia::Principal(DVec3::splat(1.0)) });
    let arm = world.insert_body(arm);
    let joint = world.insert_joint(post, arm, JointDesc::new(JointKind::Revolute { axis: DVec3::Z }, Pose::IDENTITY, Pose::IDENTITY));
    world.bind_machine("rig", &model, |_| Some(joint)).unwrap();
    let grant = world.control.arbiter.acquire("client", "rig", &[DofKey { machine: "rig".into(), generation: model.generation, joint: "/rig/shoulder".into(), dof: DofToken::Angular }]).unwrap();
    let key = grant.dofs[0].clone();
    world
        .submit_batch(Batch { owner: grant.owner.clone(), epoch: grant.epoch, sequence: 1, commands: vec![(key, Setpoint::Effort { force: 4.0 })], validity: Validity::WallMs(1000), requested_cap: None }, Instant::now())
        .unwrap();
    let mut last = std::collections::HashMap::new();
    world.step();
    let first = sample_joint_state("rig", &model, &world, &mut last);
    world.step();
    let state = sample_joint_state("rig", &model, &world, &mut last);
    assert_eq!(state.names(), vec!["shoulder".to_string()]);
    assert!(state.measured_effort[0].is_nan(), "no measured effort on a plain joint");
    assert_eq!(state.commanded_effort[0], 4.0);
    assert_eq!((state.generation, state.tick, state.valid), (model.generation, 2, 1));
    assert!(first.velocity[0].is_nan() && state.velocity[0] > 0.0, "rates need two samples");
}

/// The v2 server: a lease belongs to its claimed session and generation, a
/// command needs it, a new session revokes it, a reset clears every lease
/// and command, and an overlong validity is refused.
pub(crate) fn c21() {
    use super::api::{DirectLeases, handle_control, handle_joint_command};
    use gearbox_api::{ControlRequestV2, command_status, control_op};
    let model = rig_model();
    let mut world = PhysicsWorld::default();
    world.set_gravity(DVec3::ZERO);
    let post = world.insert_body(BodyDesc::fixed());
    let mut arm = BodyDesc::dynamic();
    arm.additional_mass = Some(MassProps { mass: 1.0, local_com: DVec3::ZERO, inertia: Inertia::Principal(DVec3::splat(1.0)) });
    let arm = world.insert_body(arm);
    let joint = world.insert_joint(post, arm, JointDesc::new(JointKind::Revolute { axis: DVec3::Z }, Pose::IDENTITY, Pose::IDENTITY));
    world.bind_machine("rig", &model, |_| Some(joint)).unwrap();
    let mut leases = DirectLeases::default();
    let key = DofKey { machine: "rig".into(), generation: model.generation, joint: "/rig/shoulder".into(), dof: DofToken::Angular };
    let control = |op, session, generation, epoch| ControlRequestV2 { session, generation, epoch, op, resume: 0, names: names_to_bytes(&["shoulder"]) };
    let command = |session, epoch, sequence, validity_ms| JointCommandV2 {
        session,
        epoch,
        sequence,
        generation: model.generation,
        max_effort: f64::NAN,
        validity_ms,
        _pad: 0,
        modes: vec![joint_mode::VELOCITY],
        values: vec![0.5],
        feedforward: Vec::new(),
        names: names_to_bytes(&["shoulder"]),
    };
    let now = Instant::now();

    let unclaimed = handle_control(&mut world, &mut leases, &model, "rig", 7, "client", &control(control_op::ACQUIRE, 0, model.generation, 0));
    assert_eq!(unclaimed.code, code::REFUSED, "no lease without the claimed session");
    let stale = handle_control(&mut world, &mut leases, &model, "rig", 7, "client", &control(control_op::ACQUIRE, 7, model.generation + 1, 0));
    assert_eq!(stale.code, code::REFUSED, "no lease on a stale generation");
    let grant = handle_control(&mut world, &mut leases, &model, "rig", 7, "client", &control(control_op::ACQUIRE, 7, model.generation, 0));
    assert_eq!(grant.code, code::OK);
    let ack = handle_joint_command(&mut world, &leases, &model, "rig", 7, &command(7, grant.epoch, 1, 200), now);
    assert_eq!((ack.status, ack.code), (command_status::ACCEPTED, code::OK));
    assert_eq!(world.control.arbiter.requested(&key), Some(Setpoint::Velocity { qdot: 0.5 }));
    let overlong = handle_joint_command(&mut world, &leases, &model, "rig", 7, &command(7, grant.epoch, 2, 60_000), now);
    assert_eq!((overlong.status, overlong.code), (command_status::REJECTED, code::USAGE), "a live command is bounded");
    let other = handle_joint_command(&mut world, &leases, &model, "rig", 8, &command(8, grant.epoch, 3, 200), now);
    assert_eq!(other.status, command_status::REJECTED, "another session holds no lease");
    super::api::revoke_lapsed(&mut world, &mut leases, "rig", 7);
    assert_eq!(leases.0.len(), 1, "the current session keeps its lease");
    super::api::revoke_lapsed(&mut world, &mut leases, "rig", 8);
    assert!(leases.0.is_empty(), "a new session revokes the old lease");
    assert_eq!(world.control.arbiter.requested(&key), Some(Setpoint::Velocity { qdot: 0.0 }), "a revoked velocity stops");
    let grant = handle_control(&mut world, &mut leases, &model, "rig", 7, "client", &control(control_op::ACQUIRE, 7, model.generation, 0));
    assert_eq!(grant.code, code::OK, "the DOFs can be acquired again after a revocation");

    let reset = handle_control(&mut world, &mut leases, &model, "rig", 7, "client", &control(control_op::RESET, 7, model.generation, 0));
    assert_eq!(reset.code, code::OK);
    assert!(leases.0.is_empty(), "a reset ends every lease of the machine");
    assert_eq!(world.control.arbiter.requested(&key), None, "a reset clears the command");
    let after = handle_joint_command(&mut world, &leases, &model, "rig", 7, &command(7, grant.epoch, 4, 200), now);
    assert_eq!(after.status, command_status::REJECTED, "the old lease is gone");
    let again = handle_control(&mut world, &mut leases, &model, "rig", 7, "client", &control(control_op::ACQUIRE, 7, model.generation, 0));
    assert_eq!(again.code, code::OK, "the DOFs can be acquired again after a reset");
    assert!(again.epoch > grant.epoch);
}

/// A machine whose joints are not all built binds nothing.
pub(crate) fn c24() {
    let model = rig_model();
    let mut world = PhysicsWorld::default();
    let missing = world.bind_machine("rig", &model, |_| None).unwrap_err();
    assert_eq!(missing.len(), 1, "{missing:?}");
    assert!(world.control.machines.is_empty());
    assert_eq!(world.control.arbiter.dofs().count(), 0);
}

pub(crate) const LIFECYCLE_CASES: &[(&str, fn())] = &[("C19", c19), ("C20", c20), ("C21", c21), ("C22", c22), ("C23", c23), ("C24", c24)];

#[test]
fn lifecycle_inventory_passes() {
    let mut failed = Vec::new();
    for (id, case) in LIFECYCLE_CASES {
        if std::panic::catch_unwind(case).is_err() {
            failed.push(*id);
        }
    }
    assert!(failed.is_empty(), "failed lifecycle cases: {failed:?}");
}

#[test]
fn c19_a_rejected_machine_activates_nothing() {
    c19()
}
#[test]
fn c20_detached_joints_stop_and_keep_their_owner() {
    c20()
}
#[test]
fn c22_v2_commands_are_validated_whole() {
    c22()
}
#[test]
fn c23_commanded_effort_is_not_measured_effort() {
    c23()
}
#[test]
fn c24_a_failed_activation_binds_nothing() {
    c24()
}

/// TF v2 edges follow moving bodies: ball_tip rides the probe, tool_tip the
/// arm, and composing the edges from the root gives each site's world pose.
#[test]
fn tf_v2_sites_follow_their_moving_bodies() {
    use crate::machine_model::RigidTransform;
    use bevy::math::{DQuat, DVec3};
    let result = crate::machine_model::compile_file(&fixture("s09_sites.usda"), &CompileOptions::default()).unwrap();
    let model = &result.machines[0];
    let index = |p: &str| model.body_index(p).unwrap();
    let mut bodies: Vec<Option<RigidTransform>> = model.bodies.iter().map(|b| Some(b.initial_pose)).collect();
    let arm = RigidTransform::new(DVec3::new(1.0, 0.0, 0.6), DQuat::from_rotation_y(0.4));
    let probe = arm.mul(&RigidTransform::new(DVec3::new(0.3, 0.0, -0.4 - 0.1), DQuat::IDENTITY));
    bodies[index("/weeder/arm")] = Some(arm);
    bodies[index("/weeder/probe")] = Some(probe);
    let edges = super::api::tf_edges(model, &bodies);
    let parent_of = |child: &str| edges.iter().find(|(_, c, _)| c == child).map(|(p, _, t)| (p.clone(), *t)).unwrap();
    assert_eq!(parent_of("tool_tip").0, "arm");
    assert_eq!(parent_of("ball_tip").0, "probe");
    let world = |name: &str| {
        let mut pose = RigidTransform::IDENTITY;
        let mut current = name.to_string();
        while let Some((parent, t)) = edges.iter().find(|(_, c, _)| *c == current).map(|(p, _, t)| (p.clone(), *t)) {
            pose = t.mul(&pose);
            current = parent;
        }
        let root = model.frames.iter().position(|f| f.name == current).unwrap();
        let (body, offset) = model.frame_in_body(root).unwrap();
        bodies[body].unwrap().mul(&offset).mul(&pose)
    };
    let tool = world("tool_tip");
    let ball = world("ball_tip");
    assert!((tool.translation - arm.transform_point(DVec3::new(0.3, 0.0, -0.2))).length() < 1e-9, "{tool:?}");
    assert!((ball.translation - probe.transform_point(DVec3::new(0.0, 0.0, -0.15))).length() < 1e-9, "{ball:?}");
    assert!((tool.translation - ball.translation).length() > 0.05, "two distinct sites");
}
#[test]
fn c21_v2_leases_follow_the_session_and_a_reset_clears_them() {
    c21()
}
