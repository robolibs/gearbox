//! Scheduling cases of plan 001 §9.2: the effective target of every physics
//! tick is the same whatever the render rate, controllers run at their own
//! rate on simulated time, and pause, single-step and catch-up move only the
//! simulated clock.

use std::time::Duration;

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use glam::DVec3;

use super::runtime::ControlClock;
use super::{DofKey, DofSpec, OwnerClass, OwnerId, Setpoint};
use crate::machine_model::Interface;
use crate::machine_model::{DofToken, DriveType};
use crate::physics::backend::{BodyDesc, Inertia, JointAxis, JointDesc, JointId, JointKind, MassProps, Pose};
use crate::physics::{ControlTick, PhysicsWorld};

#[derive(Resource)]
struct Rig {
    joint: JointId,
    rate_hz: f64,
}

#[derive(Resource, Default)]
struct Ramp {
    x: f64,
    runs: u32,
}

fn clock(mut clock: ResMut<ControlClock>, physics: Res<PhysicsWorld>, rig: Res<Rig>) {
    let (dt, hz) = (physics.dt(), physics.step_hz);
    clock.advance(dt, hz, [(("rig".to_string(), "ramp".to_string()), rig.rate_hz)]);
}

/// A controller that slews its target at 0.7 rad/s on its own cadence.
fn ramp(clock: Res<ControlClock>, mut ramp: ResMut<Ramp>, mut physics: ResMut<PhysicsWorld>, rig: Res<Rig>) {
    let Some(dt) = clock.due("rig", "ramp") else { return };
    ramp.runs += 1;
    ramp.x += 0.7 * dt;
    let owner = OwnerId::new(OwnerClass::Controller, "rig", "ramp");
    let setpoint = Setpoint::Servo { q: Some(ramp.x), qdot: 0.7, stiffness: 300.0, damping: 40.0, drive: DriveType::Force };
    physics.submit(&owner, rig.joint, JointAxis::AngX, setpoint, None).unwrap();
}

fn app(rate_hz: f64, physics_hz: f64) -> App {
    let mut world = PhysicsWorld::default();
    world.step_hz = physics_hz;
    let mut settings = world.settings();
    settings.dt = 1.0 / physics_hz;
    world.set_settings(settings);
    world.set_gravity(DVec3::ZERO);
    world.control.arbiter.trace.enabled = true;
    let post = world.insert_body(BodyDesc::fixed());
    let mut arm = BodyDesc::dynamic();
    arm.additional_mass = Some(MassProps { mass: 1.0, local_com: DVec3::ZERO, inertia: Inertia::Principal(DVec3::splat(1.0)) });
    let arm = world.insert_body(arm);
    let joint = world.insert_joint(post, arm, JointDesc::new(JointKind::Revolute { axis: DVec3::Z }, Pose::IDENTITY, Pose::IDENTITY));
    let owner = OwnerId::new(OwnerClass::Controller, "rig", "ramp");
    world.bind_dof(
        DofSpec {
            key: DofKey { machine: "rig".into(), generation: 1, joint: "/arm".into(), dof: DofToken::Angular },
            interfaces: vec![Interface::Position],
            lower: None,
            upper: None,
            drive: None,
            hardware_cap: None,
            default_owner: Some(owner),
        },
        joint,
        JointAxis::AngX,
    );
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(world)
        .insert_resource(gearbox_api::PhysicsActive(true))
        .init_resource::<gearbox_api::PhysicsStepBudget>()
        .init_resource::<ControlClock>()
        .init_resource::<Ramp>()
        .insert_resource(Rig { joint, rate_hz })
        .init_schedule(ControlTick)
        .add_systems(First, crate::physics::plan_physics_steps.after(bevy::time::TimeSystems))
        .add_systems(Update, crate::physics::step_physics)
        .add_systems(ControlTick, (clock, ramp).chain());
    app
}

fn run(app: &mut App, frames: &[f64]) {
    for &frame in frames {
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(frame)));
        app.update();
    }
}

/// The effective target of each of the first `ticks` physics ticks.
fn targets(app: &App, ticks: u64) -> Vec<(u64, String)> {
    app.world()
        .resource::<PhysicsWorld>()
        .control
        .arbiter
        .trace
        .effective_targets()
        .into_iter()
        .filter(|(tick, ..)| *tick <= ticks)
        .map(|(tick, _, state)| (tick, format!("{:?}", state.effective)))
        .collect()
}

fn frames(hz: f64, seconds: f64) -> Vec<f64> {
    vec![1.0 / hz; (seconds * hz).ceil() as usize + 2]
}

pub(crate) fn c12() {
    let ticks = 240;
    let mut reference = app(50.0, 120.0);
    run(&mut reference, &frames(60.0, 2.5));
    let expected = targets(&reference, ticks);
    assert_eq!(expected.len(), ticks as usize, "the reference ran {} ticks", expected.len());
    let irregular: Vec<f64> = (0..400).map(|i| [0.004, 0.031, 0.017, 0.0, 0.052, 0.009][i % 6]).collect();
    for (label, schedule) in [("30 Hz", frames(30.0, 2.5)), ("144 Hz", frames(144.0, 2.5)), ("irregular", irregular)] {
        let mut other = app(50.0, 120.0);
        run(&mut other, &schedule);
        let got = targets(&other, ticks);
        assert_eq!(got.len(), ticks as usize, "{label} ran {} ticks", got.len());
        assert_eq!(got, expected, "{label} rendering changed the per-tick targets");
    }
}

pub(crate) fn c13() {
    for (rate, physics_hz) in [(50.0, 240.0), (60.0, 240.0)] {
        let mut app = app(rate, physics_hz);
        run(&mut app, &frames(60.0, 1.0));
        let ticks = app.world().resource::<PhysicsWorld>().ticks;
        let runs = app.world().resource::<Ramp>().runs as f64;
        let expected = (ticks as f64 * rate / physics_hz).ceil();
        assert!((runs - expected).abs() <= 1.0, "{rate} Hz on {physics_hz} Hz: {runs} runs in {ticks} ticks");
        let held = targets(&app, ticks);
        let distinct = held.windows(2).filter(|w| w[0].1 != w[1].1).count() as f64;
        assert!(distinct <= runs, "targets change only on due ticks: {distinct} changes, {runs} runs");
    }
}

pub(crate) fn c15() {
    let mut app = app(50.0, 120.0);
    run(&mut app, &frames(60.0, 0.5));
    app.world_mut().resource_mut::<gearbox_api::PhysicsActive>().0 = false;
    let (ticks, x) = (app.world().resource::<PhysicsWorld>().ticks, app.world().resource::<Ramp>().x);
    run(&mut app, &frames(60.0, 1.0));
    assert_eq!(app.world().resource::<PhysicsWorld>().ticks, ticks, "paused: no tick");
    assert_eq!(app.world().resource::<Ramp>().x, x, "paused: no controller progress");

    app.world_mut().resource_mut::<gearbox_api::PhysicsActive>().0 = true;
    app.world_mut().resource_mut::<gearbox_api::PhysicsStepBudget>().0 = Some(1);
    run(&mut app, &[0.5]);
    assert_eq!(app.world().resource::<PhysicsWorld>().ticks, ticks + 1, "a single step runs exactly one tick");
    assert!(!app.world().resource::<gearbox_api::PhysicsActive>().0, "and pauses");
    run(&mut app, &frames(60.0, 0.5));
    assert_eq!(app.world().resource::<PhysicsWorld>().ticks, ticks + 1);

    let mut smooth = app_after(&frames(120.0, 1.0));
    let mut lumpy = app_after(&vec![0.1; 10]);
    let (a, b) = (targets(&smooth, 108), targets(&lumpy, 108));
    assert_eq!(a.len(), 108);
    assert_eq!(a, b, "catch-up steps run the same pipeline per tick");
    let _ = (&mut smooth, &mut lumpy);
}

fn app_after(schedule: &[f64]) -> App {
    let mut app = app(50.0, 120.0);
    run(&mut app, schedule);
    app
}

pub(crate) const SCHEDULE_CASES: &[(&str, fn())] = &[("C12", c12), ("C13-runtime", c13), ("C15", c15)];

#[test]
fn schedule_inventory_passes() {
    let mut failed = Vec::new();
    for (id, case) in SCHEDULE_CASES {
        if std::panic::catch_unwind(case).is_err() {
            failed.push(*id);
        }
    }
    assert!(failed.is_empty(), "failed schedule cases: {failed:?}");
}

#[test]
fn c12_render_rate_does_not_change_tick_targets() {
    c12()
}
#[test]
fn c13_controllers_run_at_their_rate_on_simulated_time() {
    c13()
}
#[test]
fn c15_pause_single_step_and_catch_up() {
    c15()
}
