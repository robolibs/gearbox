//! E03 native half: each declared drive controller maps one held cmd_vel to
//! DOF targets while the chassis moves exactly as commanded, so the
//! closed-loop trims rest at zero. The Isaac half evaluates Isaac's own
//! controllers on the same inputs; `scripts/robot_contract/compare.py`
//! compares the two.

use super::*;
use crate::physics::PhysicsWorld;
use serde_json::{Value, json};

fn committed(physics: &PhysicsWorld, joint: crate::physics::backend::JointId) -> (&'static str, Option<f64>) {
    let state = physics.control.committed.iter().find(|((j, _), _)| *j == joint).map(|(_, s)| s.effective);
    match state {
        Some(crate::joint_control::Effective::Servo { q: Some(q), .. }) => ("position", Some(q)),
        Some(crate::joint_control::Effective::Servo { q: None, qdot, .. }) => ("velocity", Some(qdot)),
        Some(crate::joint_control::Effective::Effort { force }) => ("effort", Some(force)),
        Some(crate::joint_control::Effective::Free) => ("free", None),
        None => ("uncommanded", None),
    }
}

fn run_case(case: &Value, hz: f64, ticks: u64) -> Value {
    let path = crate::machine_model::tests::fixture(case["fixture"].as_str().unwrap());
    let stage = openusd::usd::Stage::open(path.to_str().unwrap()).unwrap();
    let model = crate::machine_model::compile(&stage, &crate::machine_model::CompileOptions::default());
    let mut machines = discover_machines_with_model(&stage, &model).unwrap();
    assert_eq!(machines.len(), 1, "{}", case["name"]);
    assert!(machines[0].accepted(), "{}: {:?}", case["name"], machines[0].links.errors);
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::transform::TransformPlugin,
        bevy::asset::AssetPlugin { file_path: "/".into(), unapproved_path_mode: bevy::asset::UnapprovedPathMode::Allow, ..default() },
        usd_bevy::UsdPlugin,
    ));
    app.init_asset::<Mesh>().init_asset::<StandardMaterial>().init_asset::<Image>();
    let mut physics = PhysicsWorld::default();
    physics.set_gravity(glam::DVec3::ZERO);
    physics.step_hz = hz;
    let mut settings = physics.settings();
    settings.dt = 1.0 / hz;
    physics.set_settings(settings);
    app.insert_resource(physics);
    app.finish();
    app.cleanup();
    let root = crate::physics::benchmark::project(&mut app, &stage);
    machines[0].scene_root = Some(root);
    let machine = machines[0].clone();
    let key = ControllerKey::new(root, &machine.id, "drive");
    let requested = CmdVel {
        linear_mps: case["cmd"]["linear_mps"].as_f64().unwrap() as f32,
        angular_rps: case["cmd"]["angular_rps"].as_f64().unwrap() as f32,
    };
    let mut commands = ControllerCommands::default();
    commands.cmd_vel.insert(key.clone(), requested);
    app.insert_resource(ControllerInventory { machines: machines.clone() })
        .insert_resource(commands)
        .init_resource::<ControllerRuntimeState>()
        .init_resource::<ControllerStates>()
        .init_resource::<MachineAgentKeys>()
        .init_resource::<UiDrive>()
        .init_resource::<crate::services::LinkValues>()
        .init_resource::<gearbox_fields::WheelContacts>()
        .init_resource::<crate::joint_control::runtime::ControlClock>()
        .init_resource::<crate::joint_control::runtime::BindingWaits>()
        .insert_resource(gearbox_api::PhysicsActive(true));
    let mut tick = bevy::ecs::schedule::Schedule::default();
    tick.add_systems(
        (
            prepare_machine_physics,
            crate::joint_control::runtime::bind_accepted_machines,
            crate::joint_control::runtime::advance_control_clock,
            apply_builtin_ackermann_cmd_vel,
            apply_builtin_diff_drive_cmd_vel,
        )
            .chain(),
    );
    let entities: HashMap<String, Entity> = {
        let world = app.world_mut();
        let mut prims = world.query::<(Entity, &UsdPrimRef)>();
        prims.iter(world).map(|(e, p)| (p.path.clone(), e)).collect()
    };
    let bodies: Vec<_> = {
        let physics = app.world().resource::<PhysicsWorld>();
        let description = machine.model.as_ref().unwrap();
        description.bodies.iter().filter_map(|b| physics.entity_to_body.get(&entities[&b.path]).copied()).collect()
    };
    let chassis = app.world().resource::<PhysicsWorld>().entity_to_body[&entities[machine.body.as_ref().unwrap()]];
    let reference = DVec3::from_array(serde_json::from_value(case["speed_reference"].clone()).unwrap());
    app.world_mut().resource_mut::<ControllerRuntimeState>().applied_cmd_vel.insert(key.clone(), requested);
    let mut applied = requested;
    for _ in 0..ticks {
        applied = app.world().resource::<ControllerRuntimeState>().applied_cmd_vel.get(&key).copied().unwrap_or_default();
        let mut physics = app.world_mut().resource_mut::<PhysicsWorld>();
        let (origin, forward) = physics.chassis(chassis).map(|b| (b.translation() + b.rotation() * reference, body_forward_vector(&b).unwrap())).unwrap();
        let yaw = DVec3::Y * applied.angular_rps as f64;
        for &id in &bodies {
            let body = physics.body_mut(id).unwrap();
            let lever = body.translation() - origin;
            body.set_linvel(forward * applied.linear_mps as f64 + yaw.cross(lever), true);
            if id == chassis {
                body.set_angvel(yaw, true);
            }
        }
        tick.run(app.world_mut());
        app.world_mut().resource_mut::<PhysicsWorld>().step();
    }
    let physics = app.world().resource::<PhysicsWorld>();
    let mut dofs = serde_json::Map::new();
    for (role, joint) in case["dofs"].as_object().unwrap() {
        let path = joint.as_str().unwrap();
        let id = physics.entity_to_joint.get(&entities[path]).copied();
        let (mode, target) = id.map_or(("unbuilt", None), |id| committed(physics, id));
        dofs.insert(role.clone(), json!({ "joint": path, "mode": mode, "target": target }));
    }
    json!({
        "name": case["name"],
        "controller": case["controller"],
        "requested": case["cmd"],
        "applied": { "linear_mps": applied.linear_mps, "angular_rps": applied.angular_rps },
        "dofs": Value::Object(dofs),
    })
}

/// Every E03 case emits a target on each listed DOF from its held command.
#[test]
fn e03_native_mapping_cases() {
    let path = crate::machine_model::tests::fixture("schedules/e03_mapping.json");
    let schedule: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let hz = schedule["physics_hz"].as_f64().unwrap();
    let ticks = schedule["settle_ticks"].as_u64().unwrap();
    let cases: Vec<Value> = schedule["cases"].as_array().unwrap().iter().map(|c| run_case(c, hz, ticks)).collect();
    for case in &cases {
        let requested = &case["requested"];
        let applied = &case["applied"];
        for k in ["linear_mps", "angular_rps"] {
            let (a, b) = (applied[k].as_f64().unwrap(), requested[k].as_f64().unwrap());
            assert!((a - b).abs() < 1e-5, "{} {k}: applied {a} vs requested {b}", case["name"]);
        }
        for (role, dof) in case["dofs"].as_object().unwrap() {
            assert!(dof["target"].is_number(), "{} {role}: {dof}", case["name"]);
        }
    }
    if let Ok(out) = std::env::var("GEARBOX_CONFORMANCE_OUT") {
        let dir = std::path::PathBuf::from(out).join("e03_mapping");
        std::fs::create_dir_all(&dir).unwrap();
        let text = serde_json::to_string_pretty(&json!({ "harness": "gearbox-native", "cases": cases })).unwrap();
        std::fs::write(dir.join("mapping.json"), text).unwrap();
    }
}
