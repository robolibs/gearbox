//! Command cases of plan 001 §9.2 that need the physics: real motion under
//! position, velocity and effort; drive types; exact targets on parallel
//! joints; one commit write per DOF per tick.

use std::time::Instant;

use glam::DVec3;

use super::backend::{BodyDesc, BodyId, Inertia, JointAxis, JointDesc, JointId, JointKind, MassProps, Pose};
use super::world::PhysicsWorld;
use crate::joint_control::{Batch, DofKey, DofSpec, DriveSpec, OwnerClass, OwnerId, Setpoint, Validity};
use crate::machine_model::{DofToken, DriveType, Interface};

struct Rig {
    world: PhysicsWorld,
    joint: JointId,
    body: BodyId,
    axis: JointAxis,
    key: DofKey,
}

fn driver() -> OwnerId {
    OwnerId::new(OwnerClass::TestDriver, "rig", "replay")
}

/// A body on a joint to a fixed post, the joint through its centre of mass,
/// in zero gravity: the joint coordinate obeys `effort = inertia × q̈`.
fn rig(prismatic: bool, inertia: f64, drive: Option<DriveSpec>, joint_name: &str) -> Rig {
    let mut world = PhysicsWorld::default();
    world.set_gravity(DVec3::ZERO);
    let post = world.insert_body(BodyDesc::fixed());
    let mut desc = BodyDesc::dynamic();
    desc.additional_mass = Some(MassProps { mass: inertia, local_com: DVec3::ZERO, inertia: Inertia::Principal(DVec3::splat(inertia)) });
    let body = world.insert_body(desc);
    let kind = if prismatic { JointKind::Prismatic { axis: DVec3::X } } else { JointKind::Revolute { axis: DVec3::Z } };
    let joint = world.insert_joint(post, body, JointDesc::new(kind, Pose::IDENTITY, Pose::IDENTITY));
    let axis = if prismatic { JointAxis::LinX } else { JointAxis::AngX };
    let key = DofKey { machine: "rig".into(), generation: 1, joint: joint_name.into(), dof: if prismatic { DofToken::Linear } else { DofToken::Angular } };
    world.bind_dof(
        DofSpec { key: key.clone(), interfaces: Interface::ALL.to_vec(), lower: None, upper: None, drive, hardware_cap: None, default_owner: None, legacy_caps: false },
        joint,
        axis,
    );
    Rig { world, joint, body, axis, key }
}

impl Rig {
    fn send(&mut self, sequence: u64, setpoint: Setpoint) {
        let batch = Batch { owner: driver(), epoch: 0, sequence, commands: vec![(self.key.clone(), setpoint)], validity: Validity::Latched, requested_cap: None };
        self.world.submit_batch(batch, Instant::now()).unwrap();
    }

    fn q(&self) -> f64 {
        self.world.joint(self.joint).unwrap().motor_position(self.axis).unwrap()
    }

    fn steps(&mut self, n: usize) {
        for _ in 0..n {
            self.world.step();
        }
    }

    fn rate(&mut self) -> f64 {
        let dt = self.world.dt();
        let a = self.q();
        self.world.step();
        (self.q() - a) / dt
    }
}

fn servo_drive(stiffness: f64, damping: f64, drive: DriveType) -> Option<DriveSpec> {
    Some(DriveSpec { stiffness, damping, drive, max_force: None })
}

pub(crate) fn c01_motion() {
    for prismatic in [false, true] {
        let mut rig = rig(prismatic, 2.0, servo_drive(400.0, 80.0, DriveType::Force), "/j");
        rig.send(1, Setpoint::Position { q: 0.3, qdot: None });
        rig.steps(600);
        assert!((rig.q() - 0.3).abs() < 1e-2, "position: {} (prismatic {prismatic})", rig.q());
        rig.send(2, Setpoint::Velocity { qdot: -0.5 });
        rig.steps(240);
        let rate = rig.rate();
        assert!((rate + 0.5).abs() < 2e-2, "velocity: {rate} (prismatic {prismatic})");
    }
}

/// Effort accelerates the coordinate at effort / inertia whatever its rate,
/// and the motor is in effort mode: no target speed, no damping.
pub(crate) fn c01_effort() {
    for prismatic in [false, true] {
        for start in [-2.0, 0.0, 2.0] {
            let inertia = 2.0;
            let mut rig = rig(prismatic, inertia, servo_drive(0.0, 400.0, DriveType::Force), "/j");
            rig.send(1, Setpoint::Velocity { qdot: start });
            rig.steps(240);
            let before = rig.rate();
            assert!((before - start).abs() < 0.05, "reached {before} (wanted {start})");
            rig.send(2, Setpoint::Effort { force: -3.0 });
            let window = 60;
            let dt = rig.world.dt();
            let v0 = rig.rate();
            rig.steps(window);
            let v1 = rig.rate();
            let accel = (v1 - v0) / ((window + 1) as f64 * dt);
            assert!((accel + 3.0 / inertia).abs() < 0.05, "accel {accel} from rate {start} (prismatic {prismatic})");
            let motor = rig.world.joint(rig.joint).unwrap().motor(rig.axis).unwrap();
            assert_eq!((motor.target_position, motor.target_velocity, motor.damping, motor.stiffness), (-3.0, 0.0, 0.0, 0.0), "{motor:?}");
        }
    }
}

/// Force drives oscillate at sqrt(k / I), acceleration drives at sqrt(k)
/// whatever the inertia; the commit keeps the authored drive type.
pub(crate) fn c02() {
    let half_period = |drive: DriveType, inertia: f64| {
        let mut rig = rig(false, inertia, servo_drive(100.0, 0.0, drive), "/j");
        rig.send(1, Setpoint::Position { q: 0.2, qdot: None });
        let dt = rig.world.dt();
        let mut peak = (0.0, 0.0);
        for i in 0..1200 {
            rig.world.step();
            let q = rig.q();
            if q > peak.1 {
                peak = (i as f64 * dt, q);
            } else if q < peak.1 - 1e-4 {
                break;
            }
        }
        let motor = rig.world.joint(rig.joint).unwrap().motor(JointAxis::AngX).unwrap();
        let model = motor.model;
        (peak.0, model)
    };
    for inertia in [0.5, 2.0] {
        let (t, model) = half_period(DriveType::Force, inertia);
        let expected = std::f64::consts::PI / (100.0 / inertia).sqrt();
        assert!((t - expected).abs() < 0.1 * expected, "force drive I={inertia}: {t} vs {expected}");
        assert_eq!(model, super::backend::MotorModel::Force);
        let (t, model) = half_period(DriveType::Acceleration, inertia);
        let expected = std::f64::consts::PI / 10.0;
        assert!((t - expected).abs() < 0.1 * expected, "acceleration drive I={inertia}: {t} vs {expected}");
        assert_eq!(model, super::backend::MotorModel::Acceleration);
    }
}

pub(crate) fn c03() {
    let mut rig = rig(false, 1.0, servo_drive(400.0, 80.0, DriveType::Force), "/j");
    rig.send(1, Setpoint::Position { q: 0.3, qdot: Some(0.5) });
    rig.world.commit_pending();
    let motor = rig.world.joint(rig.joint).unwrap().motor(JointAxis::AngX).unwrap();
    assert_eq!((motor.target_position, motor.target_velocity), (0.3, 0.5), "position and feed-forward velocity both applied");
}

/// No drive, zero gains, a spring and a commanded motor stay distinct: only
/// a commanded DOF is written.
pub(crate) fn c04() {
    let mut world = PhysicsWorld::default();
    world.set_gravity(DVec3::ZERO);
    let post = world.insert_body(BodyDesc::fixed());
    let arm = |world: &mut PhysicsWorld, motor: Option<super::backend::MotorDesc>| {
        let mut desc = BodyDesc::dynamic();
        desc.additional_mass = Some(MassProps { mass: 1.0, local_com: DVec3::ZERO, inertia: Inertia::Principal(DVec3::splat(1.0)) });
        let body = world.insert_body(desc);
        let mut joint = JointDesc::new(JointKind::Revolute { axis: DVec3::Z }, Pose::IDENTITY, Pose::IDENTITY);
        joint.motors.extend(motor);
        let id = world.insert_joint(post, body, joint);
        world.body_mut(body).unwrap().set_angvel(DVec3::new(0.0, 0.0, 1.0), true);
        (id, body)
    };
    let spring = |k: f64| super::backend::MotorDesc {
        axis: JointAxis::AngX,
        target: Some(super::backend::MotorTarget::Position { target: 0.0, stiffness: k, damping: 0.0 }),
        max_force: None,
        model: Some(super::backend::MotorModel::Force),
    };
    let (free, _) = arm(&mut world, None);
    let (zero, _) = arm(&mut world, Some(spring(0.0)));
    let (sprung, _) = arm(&mut world, Some(spring(50.0)));
    let (driven, _) = arm(&mut world, None);
    let key = DofKey { machine: "rig".into(), generation: 1, joint: "/driven".into(), dof: DofToken::Angular };
    world.bind_dof(
        DofSpec { key: key.clone(), interfaces: vec![Interface::Effort], lower: None, upper: None, drive: None, hardware_cap: None, default_owner: None, legacy_caps: false },
        driven,
        JointAxis::AngX,
    );
    world
        .submit_batch(Batch { owner: driver(), epoch: 0, sequence: 1, commands: vec![(key, Setpoint::Effort { force: 2.0 })], validity: Validity::Latched, requested_cap: None }, Instant::now())
        .unwrap();
    for _ in 0..120 {
        world.step();
    }
    assert!(world.joint(free).unwrap().motor(JointAxis::AngX).is_none(), "no drive: no motor");
    let q = |w: &PhysicsWorld, j| w.joint(j).unwrap().motor_position(JointAxis::AngX).unwrap();
    assert!(q(&world, free) > 0.9 && (q(&world, zero) - q(&world, free)).abs() < 0.05, "zero gains act like none: {} {}", q(&world, zero), q(&world, free));
    assert!(q(&world, sprung).abs() < 0.3, "the spring pulls back: {}", q(&world, sprung));
    assert!(q(&world, driven) > q(&world, free) + 0.5, "the commanded motor pushes: {}", q(&world, driven));
    assert_eq!(world.control.writes.len(), 1, "only the commanded DOF is written");
}

/// Two joints between the same bodies: the owner's setpoint reaches its own
/// joint only.
pub(crate) fn c05() {
    let mut world = PhysicsWorld::default();
    let a = world.insert_body(BodyDesc::fixed());
    let b = world.insert_body(BodyDesc::dynamic());
    let tree = world.insert_joint(a, b, JointDesc::new(JointKind::Revolute { axis: DVec3::Z }, Pose::IDENTITY, Pose::IDENTITY));
    let mut closure = JointDesc::new(JointKind::Revolute { axis: DVec3::Z }, Pose::IDENTITY, Pose::IDENTITY);
    closure.loop_closure = true;
    let closure = world.insert_joint(a, b, closure);
    let owner = OwnerId::new(OwnerClass::Controller, "rig", "drive");
    for (joint, name, owned) in [(tree, "/tree", true), (closure, "/closure", false)] {
        let key = DofKey { machine: "rig".into(), generation: 1, joint: name.into(), dof: DofToken::Angular };
        world.bind_dof(
            DofSpec {
                key,
                interfaces: vec![],
                lower: None,
                upper: None,
                drive: None,
                hardware_cap: None,
                default_owner: owned.then(|| owner.clone()),
                legacy_caps: true,
            },
            joint,
            JointAxis::AngX,
        );
    }
    let targets = world.owned_joints_between(&owner, a, b, JointAxis::AngX);
    assert_eq!(targets, vec![tree]);
    let servo = Setpoint::Servo { q: None, qdot: 3.0, stiffness: 0.0, damping: 10.0, drive: DriveType::Force };
    world.submit(&owner, tree, JointAxis::AngX, servo, Some(5.0)).unwrap();
    assert!(world.submit(&owner, closure, JointAxis::AngX, servo, Some(5.0)).is_err(), "the closure is not this owner's");
    world.step();
    assert_eq!(world.joint(tree).unwrap().motor(JointAxis::AngX).unwrap().target_velocity, 3.0);
    assert!(world.joint(closure).unwrap().motor(JointAxis::AngX).is_none(), "no second motor write");
}

/// However many times producers submit in a frame, each DOF is written once
/// per tick, and an outside write does not survive the next commit.
pub(crate) fn c06_commit() {
    let mut rig = rig(false, 1.0, servo_drive(400.0, 80.0, DriveType::Force), "/j");
    let owner = OwnerId::new(OwnerClass::Controller, "rig", "drive");
    rig.world.bind_dof(
        DofSpec {
            key: rig.key.clone(),
            interfaces: vec![],
            lower: None,
            upper: None,
            drive: servo_drive(400.0, 80.0, DriveType::Force),
            hardware_cap: None,
            default_owner: Some(owner.clone()),
            legacy_caps: true,
        },
        rig.joint,
        rig.axis,
    );
    for q in [0.1, 0.2, 0.3] {
        let servo = Setpoint::Servo { q: Some(q), qdot: 0.0, stiffness: 400.0, damping: 80.0, drive: DriveType::Force };
        rig.world.submit(&owner, rig.joint, rig.axis, servo, None).unwrap();
    }
    rig.world.step();
    assert_eq!(rig.world.control.writes.get(&(rig.joint, rig.axis)), Some(&1));
    rig.world.joint_mut(rig.joint, true).unwrap().set_motor_velocity(rig.axis, 9.0, 1.0e6);
    rig.world.step();
    let motor = rig.world.joint(rig.joint).unwrap().motor(rig.axis).unwrap();
    assert_eq!((motor.target_position, motor.target_velocity), (0.3, 0.0), "the commit overwrites a stray write: {motor:?}");
    let _ = rig.body;
}

/// A DOF whose command is cleared (a machine reset) loses its actuator on
/// the next tick: an effort stops accelerating it, it coasts.
pub(crate) fn c18_reset_frees() {
    let mut free = rig(false, 1.0, None, "/j");
    free.send(1, Setpoint::Effort { force: 2.0 });
    free.steps(60);
    let driven = free.rate();
    free.world.control.arbiter.reset_machine("rig");
    free.steps(1);
    let start = free.rate();
    free.steps(60);
    let later = free.rate();
    assert!(driven > 1.0, "the effort spun the joint up: {driven}");
    assert!((later - start).abs() < 0.02 * start.abs(), "freed after reset, the joint coasts: {start} -> {later}");

    let mut spring = rig(true, 2.0, servo_drive(400.0, 80.0, DriveType::Force), "/j");
    let authored = super::world::AuthoredDrive { stiffness: 400.0, damping: 80.0, force: true, target_position: Some(0.0), ..Default::default() };
    spring.world.authored_drives.insert(spring.joint, authored);
    spring.send(1, Setpoint::Position { q: 0.3, qdot: None });
    spring.steps(600);
    assert!((spring.q() - 0.3).abs() < 1e-2, "commanded to 0.3: {}", spring.q());
    spring.world.control.arbiter.reset_machine("rig");
    spring.steps(600);
    assert!(spring.q().abs() < 1e-2, "a reset returns the authored drive to its target 0: {}", spring.q());

    let mut axis = rig(true, 2.0, servo_drive(400.0, 80.0, DriveType::Force), "/j");
    axis.world.authored_axis_drives.insert((axis.joint, axis.axis), authored);
    axis.send(1, Setpoint::Position { q: 0.3, qdot: None });
    axis.steps(600);
    axis.world.control.arbiter.reset_machine("rig");
    axis.steps(600);
    assert!(axis.q().abs() < 1e-2, "a D6 axis gets its authored drive back too: {}", axis.q());
}

pub(crate) const PHYSICS_CASES: &[(&str, fn())] = &[
    ("C01", c01_motion),
    ("C01-effort", c01_effort),
    ("C02", c02),
    ("C03", c03),
    ("C04", c04),
    ("C05", c05),
    ("C06-commit", c06_commit),
    ("C18-reset", c18_reset_frees),
];

#[test]
fn physics_command_inventory_passes() {
    let mut failed = Vec::new();
    for (id, case) in PHYSICS_CASES {
        if std::panic::catch_unwind(case).is_err() {
            failed.push(*id);
        }
    }
    assert!(failed.is_empty(), "failed physics command cases: {failed:?}");
}

#[test]
fn c01_revolute_and_prismatic_commands_move_their_joints() {
    c01_motion()
}
#[test]
fn c01_true_effort_is_a_generalized_force() {
    c01_effort()
}
#[test]
fn c02_force_and_acceleration_drives_keep_their_type() {
    c02()
}
#[test]
fn c03_position_with_feed_forward_velocity() {
    c03()
}
#[test]
fn c04_no_drive_zero_gain_spring_and_motor_are_distinct() {
    c04()
}
#[test]
fn c05_parallel_joints_get_exact_targets() {
    c05()
}
#[test]
fn c06_one_commit_write_per_dof_per_tick() {
    c06_commit()
}
#[test]
fn c18_a_reset_dof_returns_to_its_load_time_actuator() {
    c18_reset_frees()
}
