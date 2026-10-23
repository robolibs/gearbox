//! The native half of the conformance comparison: a fixture loaded through
//! projection, physics attach, the machine compiler and DOF binding, a
//! command schedule replayed tick by tick through the arbiter as the trusted
//! test driver, and the manifest and per-tick trace written for comparison.

use std::path::{Path, PathBuf};
use std::time::Instant;

use bevy::prelude::*;
use serde_json::{Value, json};
use usd_bevy::UsdPrimRef;

use super::{Batch, DofKey, Effective, OwnerClass, OwnerId, Setpoint, Validity};
use crate::machine_model::{CompileOptions, MachineDescription, RigidTransform, manifest};
use crate::physics::PhysicsWorld;

pub struct Schedule {
    pub physics_hz: f64,
    pub ticks: u64,
    pub commands: Vec<(u64, String, Setpoint)>,
    /// Lifecycle events before a tick: `pause` for some frames, `reset`.
    pub events: Vec<(u64, String, u64)>,
    pub tolerances: Value,
}

pub fn read_schedule(path: &Path) -> Schedule {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).expect("schedule")).expect("schedule json");
    let commands = v["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .map(|c| {
            let value = c["value"].as_f64().unwrap_or(0.0);
            let setpoint = match c["mode"].as_str().expect("mode") {
                "position" => Setpoint::Position { q: value, qdot: None },
                "velocity" => Setpoint::Velocity { qdot: value },
                "effort" => Setpoint::Effort { force: value },
                "hold" => Setpoint::Hold,
                "free" => Setpoint::Free,
                other => panic!("unknown schedule mode {other}"),
            };
            (c["tick"].as_u64().expect("tick"), c["dof"].as_str().expect("dof").to_string(), setpoint)
        })
        .collect();
    let events = v["events"]
        .as_array()
        .map(|events| {
            events
                .iter()
                .map(|e| (e["tick"].as_u64().expect("tick"), e["event"].as_str().expect("event").to_string(), e["frames"].as_u64().unwrap_or(0)))
                .collect()
        })
        .unwrap_or_default();
    Schedule { physics_hz: v["physics_hz"].as_f64().unwrap(), ticks: v["ticks"].as_u64().unwrap(), commands, events, tolerances: v["tolerances"].clone() }
}

/// The gravity the stage's PhysicsScene authors (m/s², stage axes).
fn scene_gravity(stage: &openusd::usd::Stage) -> Option<(f64, [f64; 3])> {
    let scene = crate::machine_model::scene::Scene::new(stage, "/", crate::machine_model::StageUnits::si(), Vec::new());
    let prim = scene.prims.iter().find(|p| p.type_name == "PhysicsScene")?;
    let magnitude = crate::machine_model::read::float(&scene, &prim.path, "physics:gravityMagnitude").unwrap_or(9.81);
    let direction = crate::machine_model::read::vec3(&scene, &prim.path, "physics:gravityDirection").unwrap_or(bevy::math::DVec3::NEG_Z);
    Some((magnitude, [direction.x, direction.y, direction.z]))
}

pub struct Run {
    pub model: MachineDescription,
    pub trace: Vec<Value>,
    pub run: Value,
}

fn to_z_up(pose: crate::physics::backend::Pose) -> RigidTransform {
    let r = bevy::math::DQuat::from_rotation_x(std::f64::consts::FRAC_PI_2);
    let (t, q) = (pose.translation, pose.rotation);
    RigidTransform::new(r * bevy::math::DVec3::new(t.x, t.y, t.z), (r * bevy::math::DQuat::from_xyzw(q.x, q.y, q.z, q.w)).normalize())
}

/// A headless app with the stage projected, attached and its physics built.
fn simulate(stage: &openusd::usd::Stage, physics_hz: f64, gravity: Option<(f64, [f64; 3])>) -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::transform::TransformPlugin,
        bevy::asset::AssetPlugin { file_path: "/".into(), unapproved_path_mode: bevy::asset::UnapprovedPathMode::Allow, ..default() },
        usd_bevy::UsdPlugin,
    ));
    app.init_asset::<Mesh>().init_asset::<StandardMaterial>().init_asset::<Image>();
    let mut physics = PhysicsWorld::default();
    physics.step_hz = physics_hz;
    let mut settings = physics.settings();
    settings.dt = 1.0 / physics_hz;
    physics.set_settings(settings);
    let up = |direction: [f64; 3]| bevy::math::DQuat::from_rotation_x(-std::f64::consts::FRAC_PI_2) * bevy::math::DVec3::from_array(direction);
    let (magnitude, direction) = gravity.unwrap_or((0.0, [0.0, 0.0, -1.0]));
    let g = up(direction) * magnitude;
    physics.set_gravity(glam::DVec3::new(g.x, g.y, g.z));
    app.insert_resource(physics);
    app.finish();
    app.cleanup();
    crate::physics::benchmark::project(&mut app, stage);
    app
}

/// Prim path → entity of every projected prim.
fn entities(app: &mut App) -> std::collections::HashMap<String, Entity> {
    let world = app.world_mut();
    let mut prims = world.query::<(Entity, &UsdPrimRef)>();
    prims.iter(world).map(|(e, p)| (p.path.clone(), e)).collect()
}

/// Bind every built joint of `model` in the app's physics world.
fn bind(app: &mut App, model: &MachineDescription, paths: &std::collections::HashMap<String, Entity>) {
    let mut physics = app.world_mut().resource_mut::<PhysicsWorld>();
    let built: std::collections::HashMap<String, crate::physics::backend::JointId> = model
        .joints
        .iter()
        .filter_map(|j| paths.get(&j.path).and_then(|e| physics.entity_to_joint.get(e).copied()).map(|id| (j.path.clone(), id)))
        .collect();
    physics.bind_machine(&model.machine_id, model, |p| built.get(p).copied()).expect("bind");
}

/// World pose (Z-up SI) of every body of `model` in the app's physics world.
fn body_poses(app: &App, model: &MachineDescription, paths: &std::collections::HashMap<String, Entity>) -> Vec<Option<RigidTransform>> {
    let physics = app.world().resource::<PhysicsWorld>();
    model
        .bodies
        .iter()
        .map(|b| {
            let id = paths.get(&b.path).and_then(|e| physics.entity_to_body.get(e))?;
            Some(to_z_up(physics.body(*id)?.position()))
        })
        .collect()
}

/// Rendered frames two physics ticks long, through the simulator's own step
/// planner and stepper, with physics active or paused.
fn run_frames(app: &mut App, frames: u64, physics_hz: f64, active: bool) {
    app.world_mut().insert_resource(gearbox_api::PhysicsActive(active));
    let mut frame = bevy::ecs::schedule::Schedule::default();
    frame.add_systems((crate::physics::plan_physics_steps, crate::physics::step_physics).chain());
    for _ in 0..frames {
        app.world_mut().resource_mut::<Time>().advance_by(std::time::Duration::from_secs_f64(2.0 / physics_hz));
        frame.run(app.world_mut());
    }
    app.world_mut().insert_resource(gearbox_api::PhysicsActive(true));
}

fn pause_frames(app: &mut App, frames: u64, physics_hz: f64) {
    run_frames(app, frames, physics_hz, false);
}

pub fn run(fixture: &Path, schedule: &Schedule) -> Run {
    let stage = openusd::usd::Stage::open(fixture.to_str().unwrap()).expect("fixture");
    let result = crate::machine_model::compile(&stage, &CompileOptions { physics_rate_hz: schedule.physics_hz, ..Default::default() });
    let model = result.machines.first().cloned().expect("one machine");
    assert!(model.accepted(), "{:?}", model.diagnostics);

    let gravity = scene_gravity(&stage);
    let mut app = simulate(&stage, schedule.physics_hz, gravity);
    let paths = entities(&mut app);
    bind(&mut app, &model, &paths);
    let machine = model.machine_id.clone();
    let driver = OwnerId::new(OwnerClass::TestDriver, &machine, "schedule");
    let body_of = |path: &str, physics: &PhysicsWorld| paths.get(path).and_then(|e| physics.entity_to_body.get(e).copied());
    let tool = model.frames.iter().position(|f| f.name == "tool_tip");

    let mut trace = Vec::new();
    let mut previous: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let coordinates = |physics: &PhysicsWorld| -> serde_json::Map<String, Value> {
        model
            .dofs
            .iter()
            .filter_map(|dof| {
                let key = DofKey { machine: machine.clone(), generation: model.generation, joint: model.joints[dof.joint].path.clone(), dof: dof.token };
                let (joint, axis) = physics.dof_binding(&key)?;
                Some((dof.name.clone(), json!(physics.joint(joint)?.motor_position(axis)?)))
            })
            .collect()
    };
    for tick in 0..schedule.ticks {
        for (_, event, frames) in schedule.events.iter().filter(|(t, ..)| *t == tick) {
            match event.as_str() {
                "pause" => {
                    let physics = app.world().resource::<PhysicsWorld>();
                    let (before, ticks_before) = (coordinates(physics), physics.ticks);
                    pause_frames(&mut app, *frames, schedule.physics_hz);
                    let physics = app.world().resource::<PhysicsWorld>();
                    trace.push(json!({ "tick": tick, "event": "pause", "frames": frames, "q_before": before, "q_after": coordinates(physics), "ticks_advanced": physics.ticks - ticks_before }));
                }
                "reset" => {
                    app.world_mut().resource_mut::<PhysicsWorld>().control.arbiter.reset_machine(&machine);
                    trace.push(json!({ "tick": tick, "event": "reset" }));
                }
                other => panic!("unknown schedule event {other}"),
            }
        }
        let mut physics = app.world_mut().resource_mut::<PhysicsWorld>();
        let commands: Vec<(DofKey, Setpoint)> = schedule
            .commands
            .iter()
            .filter(|(t, ..)| *t == tick)
            .map(|(_, name, setpoint)| {
                let r = model.dof_by_name(name).unwrap_or_else(|| panic!("no DOF {name}"));
                (DofKey { machine: machine.clone(), generation: model.generation, joint: model.joints[r.joint].path.clone(), dof: model.dofs[r.dof].token }, *setpoint)
            })
            .collect();
        if !commands.is_empty() {
            let batch = Batch { owner: driver.clone(), epoch: 0, sequence: tick + 1, commands, validity: Validity::Latched, requested_cap: None };
            physics.submit_batch(batch, Instant::now()).expect("schedule batch");
        }
        physics.step();
        let dt = physics.dt();
        for dof in &model.dofs {
            let key = DofKey { machine: machine.clone(), generation: model.generation, joint: model.joints[dof.joint].path.clone(), dof: dof.token };
            let Some(binding) = physics.dof_binding(&key) else { continue };
            let q = physics.joint(binding.0).and_then(|j| j.motor_position(binding.1)).unwrap_or(f64::NAN);
            let qdot = previous.insert(dof.name.clone(), q).map_or(f64::NAN, |p| (q - p) / dt);
            let state = physics.control.committed.get(&binding).copied();
            let (mode, target) = match state.map(|s| s.effective) {
                None => ("uncommanded", f64::NAN),
                Some(Effective::Free) => ("free", f64::NAN),
                Some(Effective::Effort { force }) => ("effort", force),
                Some(Effective::Servo { q: Some(q), .. }) => ("position", q),
                Some(Effective::Servo { q: None, qdot, .. }) => ("velocity", qdot),
            };
            trace.push(json!({ "tick": tick + 1, "dof": dof.name, "mode": mode, "target": manifest::canon(target), "q": q, "qdot": manifest::canon(qdot) }));
        }
        if let Some(frame) = tool
            && let Some((body, offset)) = model.frame_in_body(frame)
            && let Some(handle) = body_of(&model.bodies[body].path, &physics)
        {
            let pose = to_z_up(physics.body(handle).unwrap().position()).mul(&offset);
            trace.push(json!({ "tick": tick + 1, "frame": "tool_tip", "translation": [pose.translation.x, pose.translation.y, pose.translation.z] }));
        }
    }
    let run = json!({
        "harness": "gearbox-native",
        "fixture": fixture.file_name().map(|f| f.to_string_lossy().to_string()),
        "physics_hz": schedule.physics_hz,
        "ticks": schedule.ticks,
        "gravity": gravity.map(|(m, d)| json!({ "magnitude": m, "direction_stage": d })),
        "engine": app.world().resource::<PhysicsWorld>().name(),
        "owner": driver.to_string(),
        "validity": "latched, simulation ticks",
        "overlay": "e01_arm_gearbox.usda adds GearboxMachineAPI, schemaVersion 2, control body, tool_tip link, command interfaces",
    });
    Run { model, trace, run }
}

pub fn write(out: &Path, run: &Run) {
    std::fs::create_dir_all(out).unwrap();
    std::fs::write(out.join("manifest.json"), manifest::to_string(&manifest::machine(&run.model))).unwrap();
    std::fs::write(out.join("run.json"), manifest::to_string(&run.run)).unwrap();
    let lines: String = run.trace.iter().map(|v| v.to_string() + "\n").collect();
    std::fs::write(out.join("trace.jsonl"), lines).unwrap();
}

fn series<'a>(trace: &'a [Value], dof: &'a str) -> impl Iterator<Item = &'a Value> {
    trace.iter().filter(move |v| v["dof"] == dof)
}

/// Least-squares fit of `q̈ = k·(target − q) + c·(target_velocity − q̇)`
/// over ticks `from..to` of a sampled coordinate, with central differences.
pub fn fit_gains(q: &[f64], dt: f64, from: usize, to: usize, target: Option<f64>, target_velocity: f64) -> (f64, f64) {
    let (mut see, mut sev, mut svv, mut sae, mut sav) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for n in from.max(1)..to.min(q.len() - 1) {
        let a = (q[n + 1] - 2.0 * q[n] + q[n - 1]) / (dt * dt);
        let v = target_velocity - (q[n + 1] - q[n - 1]) / (2.0 * dt);
        let e = target.map_or(0.0, |t| t - q[n]);
        (see, sev, svv, sae, sav) = (see + e * e, sev + e * v, svv + v * v, sae + a * e, sav + a * v);
    }
    if target.is_none() {
        return (0.0, sav / svv);
    }
    let det = see * svv - sev * sev;
    ((sae * svv - sav * sev) / det, (sav * see - sae * sev) / det)
}

pub fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/robot_contract")
}

/// E02 native run of the arm: targets follow the schedule exactly, and the
/// arm moves the way each mode asks.
#[test]
fn native_conformance_arm_schedule() {
    let dir = fixture_dir();
    let schedule = read_schedule(&dir.join("schedules/e02_arm.json"));
    let run = run(&dir.join("e01_arm_gearbox.usda"), &schedule);
    if let Ok(out) = std::env::var("GEARBOX_CONFORMANCE_OUT") {
        write(&PathBuf::from(out).join("e02_arm"), &run);
    }
    let tol = |k: &str| schedule.tolerances[k].as_f64().unwrap();
    let at = |dof: &str, tick: u64| series(&run.trace, dof).find(|v| v["tick"] == tick).cloned().unwrap();
    let modes: Vec<(u64, &str)> = vec![(1, "position"), (121, "velocity"), (241, "effort"), (361, "position"), (481, "free")];
    for (tick, mode) in modes {
        assert_eq!(at("shoulder", tick)["mode"], mode, "shoulder mode at tick {tick}");
    }
    assert_eq!(at("shoulder", 121)["target"], 0.5);
    assert_eq!(at("shoulder", 241)["target"], 2.0);
    assert_eq!(at("slide", 241)["target"], -0.5);
    let q = |dof: &str, tick: u64| at(dof, tick)["q"].as_f64().unwrap();
    assert!((q("shoulder", 120) - 0.5).abs() < tol("position_settle_rad_or_m"), "shoulder settles: {}", q("shoulder", 120));
    assert!((q("slide", 120) - 0.1).abs() < tol("position_settle_rad_or_m"), "slide settles: {}", q("slide", 120));
    let rate = (q("shoulder", 240) - q("shoulder", 200)) / (40.0 / schedule.physics_hz);
    assert!((rate - 0.5).abs() < tol("velocity_relative") * 0.5, "velocity phase rate {rate}");
    let r0 = (q("shoulder", 252) - q("shoulder", 250)) / (2.0 / schedule.physics_hz);
    let r1 = (q("shoulder", 302) - q("shoulder", 300)) / (2.0 / schedule.physics_hz);
    assert!(r1 > r0 + 0.2, "effort accelerates the shoulder: {r0} -> {r1}");
    let hold = at("shoulder", 361)["target"].as_f64().unwrap();
    assert!((hold - q("shoulder", 360)).abs() < 0.05, "hold captures the coordinate at entry");
    assert!((q("shoulder", 479) - hold).abs() < tol("hold_drift_rad_or_m"), "hold returns to {hold}: {}", q("shoulder", 479));
}

/// An angular acceleration drive authored at stiffness 1 and damping 0.05
/// behaves as 180/π times that per radian, as Isaac/PhysX runs it.
#[test]
fn native_acceleration_drive_follows_the_degree_convention() {
    let dir = fixture_dir();
    let path = dir.join("schedules/e01_accel.json");
    let schedule = read_schedule(&path);
    let spec: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let run = run(&dir.join("e01_accel.usda"), &schedule);
    if let Ok(out) = std::env::var("GEARBOX_CONFORMANCE_OUT") {
        write(&PathBuf::from(out).join("e01_accel"), &run);
    }
    let mut q = vec![0.0];
    q.extend(series(&run.trace, "hinge").map(|v| v["q"].as_f64().unwrap()));
    let window = |k: &str| (spec["measure"][k][0].as_u64().unwrap() as usize, spec["measure"][k][1].as_u64().unwrap() as usize);
    let dt = 1.0 / schedule.physics_hz;
    let (from, to) = window("stiffness_window");
    let (k, _) = fit_gains(&q, dt, from + 2, to, Some(0.2), 0.0);
    let (from, to) = window("damping_window");
    let (_, c_velocity) = fit_gains(&q, dt, from + 2, to, None, 0.5);
    let tol = schedule.tolerances["gain_relative"].as_f64().unwrap();
    let expect = |key: &str, got: f64| {
        let want = spec["expected"][key].as_f64().unwrap();
        assert!((got - want).abs() <= tol * want, "{key}: native {got}, expected {want}");
    };
    expect("stiffness_per_rad", k);
    expect("damping_per_rad", c_velocity);
}

/// A real weed unit (default `machines/usd/weedunit.usdz`, or
/// GEARBOX_TIP_ASSET): each tool_z joint is driven through a direct lease,
/// and every ball_tip and tool_tip site rides its tool body at its own
/// offset, published through the TF v2 edges.
#[test]
#[ignore]
fn real_weedunit_tips_follow_their_tool_bodies() {
    let path = std::env::var("GEARBOX_TIP_ASSET")
        .map(PathBuf::from)
        .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../machines/usd/weedunit.usdz"));
    let source = usd_bevy::UsdSource::from_file(&path).expect("tip asset");
    let stage = source.open_stage().expect("tip stage");
    let model = crate::machine_model::compile(&stage, &CompileOptions::default()).machines.first().cloned().expect("machine");
    assert!(model.accepted(), "{:?}", model.errors().collect::<Vec<_>>());
    let mut app = simulate(&stage, 120.0, None);
    let paths = entities(&mut app);
    bind(&mut app, &model, &paths);

    let tools: Vec<usize> = model.dofs.iter().enumerate().filter(|(_, d)| model.joints[d.joint].path.ends_with("/tool_z")).map(|(i, _)| i).collect();
    assert_eq!(tools.len(), 3, "a triple weed unit has three tool joints");
    let key = |i: usize| DofKey { machine: model.machine_id.clone(), generation: model.generation, joint: model.joints[model.dofs[i].joint].path.clone(), dof: model.dofs[i].token };
    let targets: Vec<(DofKey, Setpoint)> = tools
        .iter()
        .map(|&i| {
            let dof = &model.dofs[i];
            let (lower, upper) = (dof.limit.lower.unwrap_or(-0.1), dof.limit.upper.unwrap_or(0.1));
            (key(i), Setpoint::Position { q: lower + 0.8 * (upper - lower), qdot: None })
        })
        .collect();
    let before = body_poses(&app, &model, &paths);
    {
        let mut physics = app.world_mut().resource_mut::<PhysicsWorld>();
        let keys: Vec<DofKey> = targets.iter().map(|(k, _)| k.clone()).collect();
        let grant = physics.control.arbiter.acquire("tips", &model.machine_id, &keys).expect("lease");
        let batch = Batch { owner: grant.owner, epoch: grant.epoch, sequence: 1, commands: targets.clone(), validity: Validity::Ticks(600), requested_cap: None };
        physics.submit_batch(batch, Instant::now()).expect("tool batch");
    }
    for _ in 0..480 {
        app.world_mut().resource_mut::<PhysicsWorld>().step();
    }
    let after = body_poses(&app, &model, &paths);
    let edges = super::api::tf_edges(&model, &after);
    let world = |name: &str| {
        let mut pose = RigidTransform::IDENTITY;
        let mut current = name.to_string();
        while let Some((parent, t)) = edges.iter().find(|(_, c, _)| *c == current).map(|(p, _, t)| (p.clone(), *t)) {
            pose = t.mul(&pose);
            current = parent;
        }
        let root = model.frames.iter().position(|f| f.name == current).unwrap();
        let (body, offset) = model.frame_in_body(root).unwrap();
        after[body].unwrap().mul(&offset).mul(&pose)
    };
    let physics = app.world().resource::<PhysicsWorld>();
    for &i in &tools {
        let joint = &model.joints[model.dofs[i].joint];
        let binding = physics.dof_binding(&key(i)).expect("bound tool");
        let q = physics.joint(binding.0).and_then(|j| j.motor_position(binding.1)).unwrap();
        let target = match targets.iter().find(|(k, _)| *k == key(i)).unwrap().1 { Setpoint::Position { q, .. } => q, _ => unreachable!() };
        assert!((q - target).abs() < 0.01, "{} at {q}, asked {target}", joint.path);
        let body = joint.body1.endpoint.body().expect("tool body");
        let moved = (after[body].unwrap().translation - before[body].unwrap().translation).length();
        assert!(moved > 0.02, "{} moved its body {moved} m", joint.path);
        let sites: Vec<&crate::machine_model::FrameDescription> = model
            .frames
            .iter()
            .enumerate()
            .filter(|(index, f)| model.frame_body(*index) == Some(body) && (f.name.ends_with("ball_tip") || f.name.ends_with("tool_tip")))
            .map(|(_, f)| f)
            .collect();
        assert_eq!(sites.len(), 2, "{} carries a ball_tip and a tool_tip", joint.path);
        for site in &sites {
            let index = model.frames.iter().position(|f| f.path == site.path).unwrap();
            let (_, offset) = model.frame_in_body(index).unwrap();
            let expected = after[body].unwrap().mul(&offset).translation;
            let published = world(&site.name).translation;
            assert!((published - expected).length() < 1e-6, "{} published at {published:?}, rides at {expected:?}", site.name);
        }
        assert!((world(&sites[0].name).translation - world(&sites[1].name).translation).length() > 0.05, "two distinct sites on {}", joint.path);
    }
}

/// E02 lifecycle: a pause advances no tick and the motion resumes where it
/// stopped; a machine reset returns every DOF to its load-time actuator,
/// here the authored drives, which pull both back to their target 0.
#[test]
fn native_conformance_arm_lifecycle() {
    let dir = fixture_dir();
    let schedule = read_schedule(&dir.join("schedules/e02_lifecycle.json"));
    let run = run(&dir.join("e01_arm_gearbox.usda"), &schedule);
    if let Ok(out) = std::env::var("GEARBOX_CONFORMANCE_OUT") {
        write(&PathBuf::from(out).join("e02_lifecycle"), &run);
    }
    let tol = |k: &str| schedule.tolerances[k].as_f64().unwrap();
    let dt = 1.0 / schedule.physics_hz;
    let q = |dof: &str, tick: u64| series(&run.trace, dof).find(|v| v["tick"] == tick).unwrap()["q"].as_f64().unwrap();
    let mode = |dof: &str, tick: u64| series(&run.trace, dof).find(|v| v["tick"] == tick).unwrap()["mode"].as_str().unwrap().to_string();
    let pause = run.trace.iter().find(|v| v["event"] == "pause").expect("pause row");
    assert_eq!(pause["q_before"], pause["q_after"], "a pause moves nothing");
    assert_eq!(pause["ticks_advanced"], 0);
    let before = (q("shoulder", 120) - q("shoulder", 117)) / (3.0 * dt);
    let after = (q("shoulder", 124) - q("shoulder", 121)) / (3.0 * dt);
    assert!((after - before).abs() <= tol("velocity_relative") * before.abs(), "the shoulder resumes at its speed: {before} -> {after}");
    assert!(q("shoulder", 240) > 0.5, "the effort turned the shoulder: {}", q("shoulder", 240));
    for dof in ["shoulder", "slide"] {
        assert_eq!(mode(dof, 239), if dof == "shoulder" { "effort" } else { "position" });
        assert_eq!(mode(dof, 241), "uncommanded", "{dof} after the reset");
        let target = schedule_targets(&dir.join("schedules/e02_lifecycle.json"))[dof];
        assert!((q(dof, schedule.ticks) - target).abs() < tol("reset_return_rad_or_m"), "{dof} back at its load-time target {target}: {}", q(dof, schedule.ticks));
    }
}

fn schedule_targets(path: &Path) -> std::collections::HashMap<String, f64> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    v["load_time_targets"].as_object().unwrap().iter().map(|(k, t)| (k.clone(), t.as_f64().unwrap())).collect()
}

/// The pause path is the real one: the same frames step physics once it is active.
#[test]
fn paused_frames_step_nothing_and_active_frames_step() {
    let dir = fixture_dir();
    let stage = openusd::usd::Stage::open(dir.join("e01_arm_gearbox.usda").to_str().unwrap()).unwrap();
    let mut app = simulate(&stage, 120.0, None);
    let ticks = |app: &App| app.world().resource::<PhysicsWorld>().ticks;
    let start = ticks(&app);
    run_frames(&mut app, 10, 120.0, false);
    assert_eq!(ticks(&app), start, "paused frames step nothing");
    run_frames(&mut app, 10, 120.0, true);
    assert_eq!(ticks(&app), start + 20, "active frames of two ticks step twice each");
}
