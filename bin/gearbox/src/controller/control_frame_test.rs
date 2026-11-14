//! Rotated-equivalent machines drive the same way in their own axes: each
//! chassis forward along the +X of its declared control frame, one turned
//! −90° about Z (forward −Y), one not.

use super::*;
use crate::physics::PhysicsWorld;

#[test]
fn the_declared_control_frame_sets_the_drive_direction() {
    let path = crate::machine_model::tests::fixture("c10_two_carts.usda");
    let stage = openusd::usd::Stage::open(path.to_str().unwrap()).unwrap();
    let model = crate::machine_model::compile(&stage, &crate::machine_model::CompileOptions::default());
    let mut machines = discover_machines_with_model(&stage, &model).unwrap();
    assert_eq!(machines.len(), 2);
    assert!(machines.iter().all(|m| m.accepted()), "{:?}", machines.iter().map(|m| &m.links.errors).collect::<Vec<_>>());
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
    app.insert_resource(physics);
    app.finish();
    app.cleanup();
    let root = crate::physics::benchmark::project(&mut app, &stage);
    for machine in &mut machines {
        machine.scene_root = Some(root);
    }
    let mut commands = ControllerCommands::default();
    for machine in &machines {
        commands.cmd_vel.insert(ControllerKey::new(root, &machine.id, "drive"), CmdVel { linear_mps: 1.0, angular_rps: 0.0 });
    }
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
            apply_builtin_diff_drive_cmd_vel,
        )
            .chain(),
    );
    let chassis = |app: &mut App, name: &str| {
        let world = app.world_mut();
        let mut prims = world.query::<(Entity, &UsdPrimRef)>();
        let entity = prims.iter(world).find(|(_, p)| p.path == format!("/World/{name}/chassis")).unwrap().0;
        world.resource::<PhysicsWorld>().entity_to_body[&entity]
    };
    let (turned, plain) = (chassis(&mut app, "turned"), chassis(&mut app, "plain"));
    let start = |app: &App, id| app.world().resource::<PhysicsWorld>().body(id).unwrap().translation();
    let (turned0, plain0) = (start(&app, turned), start(&app, plain));
    for _ in 0..240 {
        tick.run(app.world_mut());
        app.world_mut().resource_mut::<PhysicsWorld>().step();
    }
    let moved = |app: &App, id, from: glam::DVec3| {
        let d = start(app, id) - from;
        glam::DVec3::new(d.x, 0.0, d.z)
    };
    let (dt, dp) = (moved(&app, turned, turned0), moved(&app, plain, plain0));
    // USD −Y is Bevy +Z; USD +X is Bevy +X.
    assert!(dt.length() > 1.0 && dt.normalize().dot(glam::DVec3::Z) > 0.99, "turned cart moved {dt:?}");
    assert!(dp.length() > 1.0 && dp.normalize().dot(glam::DVec3::X) > 0.99, "plain cart moved {dp:?}");
    let states = app.world().resource::<ControllerStates>();
    let heading = |id: &str| states.states[&ControllerKey::new(root, id, "drive")].heading_rad;
    assert!((heading("world_turned") - 0.0).abs() < 1e-3, "turned heading {}", heading("world_turned"));
    assert!((heading("world_plain") - std::f64::consts::FRAC_PI_2).abs() < 1e-3, "plain heading {}", heading("world_plain"));
}
