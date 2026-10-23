//! Command, ownership, scheduling and lifecycle cases of plan 001 §9.2 that
//! need no physics. Physics-backed cases live with the commit in
//! `physics::world`.

use std::time::{Duration, Instant};

use super::*;
use super::policy::{CadenceError, CapPolicy, NormalizedRange};
use crate::machine_model::{DofToken, DriveType, Interface};

fn key(machine: &str, joint: &str) -> DofKey {
    DofKey { machine: machine.into(), generation: 1, joint: joint.into(), dof: DofToken::Angular }
}

fn spec(machine: &str, joint: &str, owner: Option<OwnerId>) -> DofSpec {
    DofSpec {
        key: key(machine, joint),
        interfaces: Interface::ALL.to_vec(),
        lower: Some(-1.0),
        upper: Some(1.0),
        drive: Some(DriveSpec { stiffness: 100.0, damping: 10.0, drive: DriveType::Force, max_force: Some(100.0) }),
        hardware_cap: None,
        default_owner: owner,
    }
}

fn controller(machine: &str) -> OwnerId {
    OwnerId::new(OwnerClass::Controller, machine, "drive")
}

fn batch(owner: &OwnerId, epoch: u64, sequence: u64, commands: Vec<(DofKey, Setpoint)>, validity: Validity) -> Batch {
    Batch { owner: owner.clone(), epoch, sequence, commands, validity, requested_cap: None }
}

fn state(arbiter: &mut Arbiter, now: Instant, q: f64) -> Vec<(DofKey, ActuatorState)> {
    arbiter.resolve(now, |_| Some(q))
}

pub(crate) fn c06() {
    for order in [[0, 1], [1, 0]] {
        let mut arbiter = Arbiter::new();
        let owners = [controller("m"), OwnerId::new(OwnerClass::Service, "m", "hitch")];
        let specs = vec![spec("m", "/j", Some(owners[0].clone()))];
        arbiter.register(specs).unwrap();
        let now = Instant::now();
        let mut results = Vec::new();
        for &i in &order {
            results.push(arbiter.submit(batch(&owners[i], 0, 1, vec![(key("m", "/j"), Setpoint::Velocity { qdot: 1.0 })], Validity::Latched), now));
        }
        let service = if order[0] == 1 { &results[0] } else { &results[1] };
        assert!(matches!(service, Err(Rejection::NotOwner { .. })), "{service:?}");
        let resolved = state(&mut arbiter, now, 0.0);
        assert_eq!(resolved.len(), 1, "one state per DOF per tick");
    }
    let mut arbiter = Arbiter::new();
    let err = arbiter.register(vec![spec("m", "/j", Some(controller("m"))), spec("m", "/j", None)]).unwrap_err();
    assert!(matches!(err[0], Rejection::DuplicateDof { .. }));
}

pub(crate) fn c07() {
    let mut arbiter = Arbiter::new();
    let drive = controller("m");
    arbiter.register(vec![spec("m", "/j", Some(drive.clone()))]).unwrap();
    let now = Instant::now();
    arbiter.submit(batch(&drive, 0, 1, vec![(key("m", "/j"), Setpoint::Velocity { qdot: 2.0 })], Validity::Latched), now).unwrap();
    let grant = arbiter.acquire("client", "m", &[key("m", "/j")]).unwrap();
    assert!(matches!(
        arbiter.submit(batch(&drive, 0, 2, vec![(key("m", "/j"), Setpoint::Velocity { qdot: 3.0 })], Validity::Latched), now),
        Err(Rejection::NotOwner { .. })
    ));
    let held = state(&mut arbiter, now, 0.3);
    assert!(matches!(held[0].1.effective, Effective::Servo { q: Some(q), .. } if (q - 0.3).abs() < 1e-12), "override starts by holding: {held:?}");
    assert!(matches!(arbiter.acquire("other", "m", &[key("m", "/j")]), Err(Rejection::Conflict { .. })));
    arbiter
        .submit(batch(&grant.owner, grant.epoch, 1, vec![(key("m", "/j"), Setpoint::Position { q: 0.5, qdot: None })], Validity::Latched), now)
        .unwrap();
    assert!(matches!(
        arbiter.submit(batch(&grant.owner, grant.epoch + 99, 2, vec![(key("m", "/j"), Setpoint::Position { q: 0.1, qdot: None })], Validity::Latched), now),
        Err(Rejection::StaleEpoch { .. })
    ));
    arbiter.release(&grant, false);
    assert_eq!(arbiter.owner(&key("m", "/j")), None, "release without resume leaves the DOF unowned");
    assert!(matches!(
        arbiter.submit(batch(&drive, 0, 3, vec![(key("m", "/j"), Setpoint::Velocity { qdot: 3.0 })], Validity::Latched), now),
        Err(Rejection::NotOwner { .. })
    ));
    let mut arbiter = Arbiter::new();
    arbiter.register(vec![spec("m", "/j", Some(drive.clone()))]).unwrap();
    let grant = arbiter.acquire("client", "m", &[key("m", "/j")]).unwrap();
    arbiter.release(&grant, true);
    assert_eq!(arbiter.owner(&key("m", "/j")), Some(&drive), "explicit resume returns the DOF to its owner");
}

pub(crate) fn c08() {
    let reversed = NormalizedRange { at_zero: 0.5, at_one: -0.25 };
    assert!((reversed.coordinate(0.0) - 0.5).abs() < 1e-12);
    assert!((reversed.coordinate(1.0) + 0.25).abs() < 1e-12);
    assert!((reversed.coordinate(0.5) - 0.125).abs() < 1e-12);
    assert_eq!(reversed.coordinate(2.0), reversed.coordinate(1.0));
    assert_eq!(reversed.coordinate(-1.0), reversed.coordinate(0.0));
    assert_eq!(reversed.coordinate(f64::NAN), reversed.coordinate(0.0));
}

pub(crate) fn c09() {
    let mut arbiter = Arbiter::new();
    let owner = OwnerId::new(OwnerClass::TestDriver, "m", "replay");
    arbiter.register(vec![spec("m", "/a", None), spec("m", "/b", None)]).unwrap();
    let now = Instant::now();
    let bad = batch(
        &owner,
        0,
        1,
        vec![(key("m", "/a"), Setpoint::Position { q: 0.2, qdot: None }), (key("m", "/b"), Setpoint::Position { q: f64::NAN, qdot: None })],
        Validity::Latched,
    );
    assert!(matches!(arbiter.submit(bad, now), Err(Rejection::NonFinite { .. })));
    assert_eq!(arbiter.requested(&key("m", "/a")), None, "nothing of a rejected batch is applied");
    assert!(state(&mut arbiter, now, 0.0).is_empty());
    let twice = batch(&owner, 0, 2, vec![(key("m", "/a"), Setpoint::Free), (key("m", "/a"), Setpoint::Free)], Validity::Latched);
    assert!(matches!(arbiter.submit(twice, now), Err(Rejection::DuplicateDof { .. })));
}

pub(crate) fn c10() {
    let mut arbiter = Arbiter::new();
    let owner = OwnerId::new(OwnerClass::TestDriver, "m", "replay");
    arbiter.register(vec![spec("m", "/j", None)]).unwrap();
    let now = Instant::now();
    let send = |arbiter: &mut Arbiter, seq: u64, s: Setpoint| arbiter.submit(batch(&owner, 0, seq, vec![(key("m", "/j"), s)], Validity::Latched), now).unwrap();
    send(&mut arbiter, 1, Setpoint::Hold);
    let first = arbiter.resolve(now, |_| Some(0.4));
    let later = arbiter.resolve(now, |_| Some(0.9));
    let q = |s: &[(DofKey, ActuatorState)]| match s[0].1.effective {
        Effective::Servo { q: Some(q), .. } => q,
        other => panic!("{other:?}"),
    };
    assert_eq!((q(&first), q(&later)), (0.4, 0.4), "a hold captures once");
    send(&mut arbiter, 2, Setpoint::Velocity { qdot: 0.5 });
    send(&mut arbiter, 3, Setpoint::Effort { force: 5.0 });
    assert!(matches!(arbiter.resolve(now, |_| Some(0.0))[0].1.effective, Effective::Effort { force } if force == 5.0));
    send(&mut arbiter, 4, Setpoint::Position { q: 0.2, qdot: None });
    match arbiter.resolve(now, |_| Some(0.0))[0].1.effective {
        Effective::Servo { q, stiffness, damping, drive, .. } => {
            assert_eq!((q, stiffness, damping, drive), (Some(0.2), 100.0, 10.0, DriveType::Force), "authored drive kept across modes");
        }
        other => panic!("{other:?}"),
    }
    send(&mut arbiter, 5, Setpoint::Hold);
    assert_eq!(q(&arbiter.resolve(now, |_| Some(0.15))), 0.15, "a new hold captures anew");
    send(&mut arbiter, 6, Setpoint::Free);
    assert_eq!(arbiter.resolve(now, |_| Some(0.0))[0].1.effective, Effective::Free);
}

pub(crate) fn c11() {
    let cases = [
        (CapPolicy { authored: Some(100.0), requested: Some(1000.0), ..Default::default() }, Some(100.0)),
        (CapPolicy { authored: Some(100.0), requested: Some(40.0), ..Default::default() }, Some(40.0)),
        (CapPolicy { authored: None, requested: Some(40.0), ..Default::default() }, Some(40.0)),
        (CapPolicy::default(), None),
        (CapPolicy { authored: Some(100.0), hardware: Some(80.0), safety: Some(90.0), ..Default::default() }, Some(80.0)),
        (CapPolicy { authored: Some(100.0), safety: Some(-5.0), ..Default::default() }, Some(0.0)),
    ];
    for (policy, expected) in cases {
        assert_eq!(policy.effective(), expected, "{policy:?}");
    }

    let mut arbiter = Arbiter::new();
    let owner = OwnerId::new(OwnerClass::TestDriver, "m", "replay");
    arbiter.register(vec![spec("m", "/j", None)]).unwrap();
    let mut b = batch(&owner, 0, 1, vec![(key("m", "/j"), Setpoint::Effort { force: 500.0 })], Validity::Latched);
    b.requested_cap = Some(1000.0);
    arbiter.submit(b, Instant::now()).unwrap();
    let s = arbiter.resolve(Instant::now(), |_| None)[0].1;
    assert_eq!((s.cap, s.effective), (Some(100.0), Effective::Effort { force: 100.0 }), "authored 100, requested 1000 gives 100");
}

pub(crate) fn c13() {
    for (rate, physics, ticks, expected) in [(50.0, 240.0, 240, 50), (60.0, 240.0, 240, 60), (240.0, 240.0, 240, 240), (7.0, 120.0, 1200, 70)] {
        let mut cadence = Cadence::new(rate, physics).unwrap();
        let due: Vec<usize> = (0..ticks).filter(|_| cadence.tick()).collect();
        assert_eq!(due.len(), expected, "{rate} Hz on {physics} Hz");
        assert_eq!(due[0], 0, "first tick due");
        let mut again = Cadence::new(rate, physics).unwrap();
        let replay: Vec<usize> = (0..ticks).filter(|_| again.tick()).collect();
        assert_eq!(due, replay, "deterministic");
        let gaps: Vec<usize> = due.windows(2).map(|w| w[1] - w[0]).collect();
        let (lo, hi) = (gaps.iter().min().copied().unwrap_or(0), gaps.iter().max().copied().unwrap_or(0));
        assert!(hi - lo <= 1, "zero-order hold between evenly spread due ticks: {gaps:?}");
    }
}

pub(crate) fn c14() {
    for bad in [0.0, -5.0, f64::NAN, f64::INFINITY] {
        assert_eq!(Cadence::new(bad, 120.0), Err(CadenceError::NotPositive), "{bad}");
    }
    assert_eq!(Cadence::new(240.0, 120.0), Err(CadenceError::AbovePhysics));
}

pub(crate) fn c16() {
    let mut arbiter = Arbiter::new();
    let drive = controller("m");
    arbiter.register(vec![spec("m", "/j", Some(drive))]).unwrap();
    let grant = arbiter.acquire("client", "m", &[key("m", "/j")]).unwrap();
    let t0 = Instant::now();
    let latched = batch(&grant.owner, grant.epoch, 1, vec![(key("m", "/j"), Setpoint::Velocity { qdot: 1.0 })], Validity::Latched);
    assert!(matches!(arbiter.submit(latched, t0), Err(Rejection::UnboundedLiveCommand { .. })), "no indefinite live velocity");
    let live = batch(&grant.owner, grant.epoch, 2, vec![(key("m", "/j"), Setpoint::Velocity { qdot: 1.0 })], Validity::WallMs(500));
    arbiter.submit(live, t0).unwrap();
    let before = arbiter.resolve(t0 + Duration::from_millis(100), |_| Some(0.0));
    assert!(matches!(before[0].1.effective, Effective::Servo { qdot, .. } if qdot == 1.0));
    let after = arbiter.resolve(t0 + Duration::from_millis(600), |_| Some(0.0));
    assert!(matches!(after[0].1.effective, Effective::Servo { qdot, q: None, .. } if qdot == 0.0), "a lapsed velocity stops, it does not jump: {after:?}");

    let mut replay = Arbiter::new();
    let driver = OwnerId::new(OwnerClass::TestDriver, "m", "replay");
    replay.register(vec![spec("m", "/j", None)]).unwrap();
    replay.submit(batch(&driver, 0, 1, vec![(key("m", "/j"), Setpoint::Effort { force: 3.0 })], Validity::Ticks(3)), t0).unwrap();
    let far = t0 + Duration::from_secs(3600);
    let modes: Vec<bool> = (0..5).map(|_| matches!(replay.resolve(far, |_| None)[0].1.effective, Effective::Effort { force } if force == 3.0)).collect();
    assert_eq!(modes, vec![true, true, false, false, false], "tick validity ignores wall time");

    let mut revoked = Arbiter::new();
    revoked.register(vec![spec("m", "/j", Some(controller("m")))]).unwrap();
    let grant = revoked.acquire("client", "m", &[key("m", "/j")]).unwrap();
    revoked.submit(batch(&grant.owner, grant.epoch, 1, vec![(key("m", "/j"), Setpoint::Effort { force: 9.0 })], Validity::WallMs(10_000)), t0).unwrap();
    revoked.revoke(&grant.owner);
    assert_eq!(revoked.resolve(t0, |_| Some(0.2))[0].1.effective, Effective::Free, "revocation ends effort before the next step");
    assert_eq!(revoked.owner(&key("m", "/j")), None, "revocation does not resume the suspended owner");
}

pub(crate) fn c17() {
    let mut arbiter = Arbiter::new();
    let owner = OwnerId::new(OwnerClass::TestDriver, "m", "replay");
    arbiter.register(vec![spec("m", "/j", None)]).unwrap();
    let mut next = spec("m", "/j", None);
    next.key.generation = 2;
    arbiter.register(vec![next]).unwrap();
    let stale = batch(&owner, 0, 1, vec![(key("m", "/j"), Setpoint::Free)], Validity::Latched);
    assert!(matches!(arbiter.submit(stale, Instant::now()), Err(Rejection::StaleGeneration { have: 2, given: 1, .. })));
}

pub(crate) fn c18() {
    let mut arbiter = Arbiter::new();
    let (a, b) = (controller("a"), controller("b"));
    arbiter.register(vec![spec("a", "/j", Some(a.clone()))]).unwrap();
    arbiter.register(vec![spec("b", "/j", Some(b.clone()))]).unwrap();
    let now = Instant::now();
    for (owner, m) in [(&a, "a"), (&b, "b")] {
        arbiter.submit(batch(owner, 0, 1, vec![(key(m, "/j"), Setpoint::Velocity { qdot: 1.0 })], Validity::Latched), now).unwrap();
    }
    arbiter.reset_machine("a");
    assert_eq!(arbiter.requested(&key("a", "/j")), None);
    assert_eq!(arbiter.requested(&key("b", "/j")), Some(Setpoint::Velocity { qdot: 1.0 }), "the other robot keeps its command");
    assert_eq!(arbiter.owner(&key("a", "/j")), Some(&a));
}

pub(crate) const COMMAND_CASES: &[(&str, fn())] = &[
    ("C06", c06),
    ("C07", c07),
    ("C08", c08),
    ("C09", c09),
    ("C10", c10),
    ("C11", c11),
    ("C13", c13),
    ("C14", c14),
    ("C16", c16),
    ("C17", c17),
    ("C18", c18),
];

#[test]
fn command_inventory_passes() {
    let mut failed = Vec::new();
    for (id, case) in COMMAND_CASES {
        if std::panic::catch_unwind(case).is_err() {
            failed.push(*id);
        }
    }
    assert!(failed.is_empty(), "failed command cases: {failed:?}");
}

#[test]
fn c06_conflicts_fail_the_same_way_in_any_order() {
    c06()
}
#[test]
fn c07_override_is_an_explicit_authority_transfer() {
    c07()
}
#[test]
fn c08_normalized_endpoints_map_affinely() {
    c08()
}
#[test]
fn c09_an_invalid_member_rejects_the_whole_batch() {
    c09()
}
#[test]
fn c10_hold_captures_once_across_mode_changes() {
    c10()
}
#[test]
fn c11_caps_take_the_lowest_bound() {
    c11()
}
#[test]
fn c13_controller_cadence_is_deterministic() {
    c13()
}
#[test]
fn c14_invalid_rates_are_rejected() {
    c14()
}
#[test]
fn c16_live_expiry_and_tick_replay_keep_their_clocks() {
    c16()
}
#[test]
fn c17_a_stale_generation_is_rejected() {
    c17()
}
#[test]
fn c18_reset_touches_one_robot() {
    c18()
}
