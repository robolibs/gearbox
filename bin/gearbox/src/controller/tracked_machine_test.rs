use super::*;
use crate::physics::{MollaBackend, PhysicsWorld};
use crate::physics::backend::ColliderDesc;
#[path = "tracked_mass_export.rs"]
mod mass_export;

fn fem_mass_properties(rigid: &crate::physics::fem::rigid_machine::FemRigidMachine, belts: Option<&crate::physics::fem::track_mesh::FemTrackMeshes>) -> (f64,molla_math::Vec3,molla_math::Mat3) {
    use molla_math::{Mat3,Vec3 as Vector};
    let point_inertia = |m: f64,p: Vector| (Mat3::IDENTITY*p.length_squared()-Mat3::from_cols(p*p.x,p*p.y,p*p.z))*m;
    let mut mass=0.0; let mut first=Vector::ZERO; let mut inertia=Mat3::ZERO;
    for body in 0..rigid.model.body_count {
        let m=rigid.model.body_mass.host().unwrap()[body];
        let pose=rigid.state.body_q.host().unwrap()[body];
        let p=pose.transform_point(rigid.model.body_com.host().unwrap()[body]);
        let r=Mat3::from_quat(pose.rotation);
        mass+=m; first+=m*p;
        inertia+=r*rigid.model.body_inertia.host().unwrap()[body]*r.transpose()+point_inertia(m,p);
    }
    if let Some(belts)=belts {
        for (&m,&p) in belts.model.particle_mass.host().unwrap().iter().zip(belts.state.particle_q.host().unwrap()) {
            mass+=m; first+=m*p; inertia+=point_inertia(m,p);
        }
    }
    (mass,first,inertia)
}

fn assert_fem_mass_conserved(a: (f64,molla_math::Vec3,molla_math::Mat3),b: (f64,molla_math::Vec3,molla_math::Mat3)) {
    assert!((a.0-b.0).abs()<1e-8);
    assert!((a.1-b.1).length()<1e-8);
    assert!((a.2-b.2).to_cols_array().iter().all(|v| v.abs()<1e-8));
}

#[test]
#[ignore = "requires GEARBOX_TRACK_ASSET through oslo make test-fem-machine"]
fn ceol_fem_machine_binding() {
    use crate::physics::fem::machine::FemMachineLayout;
    use crate::physics::markers::UsdPhysicsJoint;
    let path = std::env::var("GEARBOX_TRACK_ASSET").expect("GEARBOX_TRACK_ASSET");
    let source = usd_bevy::UsdSource::from_file(Path::new(&path)).unwrap();
    let stage = source.open_stage().unwrap();
    let mut machine = discover_machines_from_stage(&stage).unwrap().remove(0);
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::transform::TransformPlugin,
        bevy::asset::AssetPlugin { file_path: "/".into(), unapproved_path_mode: bevy::asset::UnapprovedPathMode::Allow, ..default() },
        usd_bevy::UsdPlugin));
    app.init_asset::<Mesh>().init_asset::<StandardMaterial>().init_asset::<Image>();
    app.insert_resource(PhysicsWorld::with_backend(Box::new(MollaBackend::default())));
    app.finish(); app.cleanup();
    let first_root = crate::physics::benchmark::project(&mut app, &stage);
    machine.scene_root = Some(first_root);
    let layout = FemMachineLayout::inspect(app.world_mut(), &machine).unwrap();
    let prepared = crate::physics::fem::rigid_machine::FemRigidMachine::prepare(app.world(), &layout).unwrap();
    let wheel_contacts = crate::physics::fem::track_contacts::FemTrackContacts::prepare(app.world(), &machine, &layout, &prepared).unwrap();
    assert_eq!(wheel_contacts.shapes.len(), 66);
    assert_eq!(prepared.model.body_count, layout.bodies.len());
    assert_eq!(prepared.model.joint_count, layout.tree_joints.len() + 1);
    assert_eq!(prepared.loops.len(), layout.loop_joints.len());
    let belts = crate::physics::fem::track_mesh::FemTrackMeshes::prepare(app.world(), &machine, &layout, &prepared).unwrap();
    assert_eq!(belts.tracks.len(), 2);
    assert_eq!(belts.model.particle_count, 10752);
    assert_eq!(belts.model.tet_count, 32256);
    let mut contact_features = 0usize;
    for track in &belts.tracks {
        let vertices = track
            .surface
            .iter()
            .flatten()
            .copied()
            .collect::<HashSet<_>>();
        let edges = track
            .surface
            .iter()
            .flat_map(|t| [[t[0], t[1]], [t[1], t[2]], [t[2], t[0]]])
            .map(|mut edge| {
                edge.sort();
                edge
            })
            .collect::<HashSet<_>>();
        contact_features += vertices.len() + edges.len() + track.surface.len();
    }
    eprintln!(
        "authored FEM contact budget: features={contact_features}, 13-shape dense response={} bytes, four-contacts-per-feature response={} bytes",
        contact_features * 13 * 3 * prepared.model.joint_dof_total * 4,
        contact_features * 4 * 3 * prepared.model.joint_dof_total * 4
    );
    let points = belts.state.particle_q.host().unwrap();
    for track in &belts.tracks {
        assert!(track.minimum_j > 0.5);
        assert!((track.neutral_length - 3.1887).abs() < 0.001);
        assert!(track.mass > 10.0 && track.mass < 20.0);
        assert!(track.pitch_radius_difference > 0.002 && track.pitch_radius_difference < 0.003);
        assert!(track.surface.iter().flatten().all(|node| track.nodes.contains(node)));
        let body = prepared.bodies[&track.carrier];
        let pose = prepared.state.body_q.host().unwrap()[body.index()];
        let local = track.nodes.iter().map(|&node| pose.inverse().transform_point(points[node as usize])).collect::<Vec<_>>();
        assert!(local.iter().all(|p| p.x.abs() <= 0.090001));
        eprintln!("authored belt: nodes={} tets={} mass={} kg neutral={} m minJ={} pitch-radius mismatch={} m",
            track.nodes.len(), belts.model.tet_count/2, track.mass, track.neutral_length, track.minimum_j, track.pitch_radius_difference);
    }
    assert!(belts.tracks[0].nodes.iter().all(|node| !belts.tracks[1].nodes.contains(node)));
    let saved_fem = machine.tracks[0].fem.take();
    assert!(crate::physics::fem::track_mesh::FemTrackMeshes::prepare(app.world(), &machine, &layout, &prepared).is_err());
    machine.tracks[0].fem = saved_fem;
    let mut partitioned = crate::physics::fem::rigid_machine::FemRigidMachine::prepare(app.world(), &layout).unwrap();
    partitioned.partition_belt_mass(&belts).unwrap();
    let combined_mass = partitioned.model.body_mass.host().unwrap().iter().sum::<f64>() + belts.model.particle_mass.host().unwrap().iter().sum::<f64>();
    assert!((combined_mass - 750.0).abs() < 1e-5);
    assert_fem_mass_conserved(fem_mass_properties(&prepared,None),fem_mass_properties(&partitioned,Some(&belts)));
    for (authored,track) in machine.tracks.iter().zip(&belts.tracks) {
        use molla_math::{Vec3 as Vector,Mat3};
        let path=openusd::sdf::path(&authored.carrier).unwrap();
        let report: serde_json::Value=serde_json::from_str(&read_string(&stage,&path,"ceol:femMassComposition").unwrap()).unwrap();
        let mut m=0.0; let mut first=Vector::ZERO; let mut inertia=Mat3::ZERO;
        let outer=|v: Vector| Mat3::IDENTITY*v.length_squared()-Mat3::from_cols(v*v.x,v*v.y,v*v.z);
        for part in report["rigid_components"].as_array().unwrap() {
            let mass=part["mass"].as_f64().unwrap();
            let vec=|key: &str| Vector::from_array(std::array::from_fn(|i| part[key][i].as_f64().unwrap()));
            let center=vec("center"); let size=vec("size");
            m+=mass; first+=mass*center;
            inertia+=Mat3::from_diagonal((Vector::splat(size.length_squared())-size*size)*(mass/12.0))+outer(center)*mass;
        }
        let center=first/m;
        inertia-=outer(center)*m;
        let body=partitioned.bodies[&track.carrier].index();
        assert!((partitioned.model.body_mass.host().unwrap()[body]-m).abs()<1e-8);
        assert!((partitioned.model.body_com.host().unwrap()[body]-center).length()<1e-6);
        assert!((partitioned.model.body_inertia.host().unwrap()[body]-inertia).to_cols_array().iter().all(|v| v.abs()<1e-5));
    }
    assert!(partitioned.partition_belt_mass(&belts).is_err());
    let mut invalid_partition = crate::physics::fem::rigid_machine::FemRigidMachine::prepare(app.world(), &layout).unwrap();
    let last_carrier=invalid_partition.bodies[&belts.tracks[1].carrier].index();
    invalid_partition.model.body_mass.host_mut().unwrap()[last_carrier]=1.0;
    let saved_masses=invalid_partition.model.body_mass.host().unwrap().to_vec();
    let saved_com=invalid_partition.model.body_com.host().unwrap().to_vec();
    let saved_inertia=invalid_partition.model.body_inertia.host().unwrap().to_vec();
    assert!(invalid_partition.partition_belt_mass(&belts).is_err());
    assert_eq!(invalid_partition.model.body_mass.host().unwrap(),saved_masses);
    assert_eq!(invalid_partition.model.body_com.host().unwrap(),saved_com);
    assert_eq!(invalid_partition.model.body_inertia.host().unwrap(),saved_inertia);
    let mass: f64 = prepared.model.body_mass.host().unwrap().iter().sum();
    assert!((mass - 750.0).abs() < 1e-5, "authored CEOL mass: {mass}");
    assert!(prepared.control.joint_motor_max_force.host().unwrap().iter().any(|&f| f == 20000.0));
    assert!(layout.bodies.len() > 13);
    assert_eq!(layout.tree_joints.len() + 1, layout.bodies.len());
    assert_eq!(layout.loop_joints.len(), 2, "hitch closure constraints were dropped");
    assert_eq!(layout.tracks.len(), 2);
    assert_eq!(layout.tracks.iter().map(|t| t.passive_joints.len()).sum::<usize>(), 10);
    for track in &layout.tracks {
        assert!(layout.bodies.contains(&track.carrier));
        assert!(layout.bodies.contains(&track.sprocket));
        assert!(layout.tree_joints.contains(&track.drive_joint));
        assert_eq!(track.treads.len(), 56);
        assert_eq!(track.contact_patches.len(), 6);
    }
    let second_root = crate::physics::benchmark::project(&mut app, &stage);
    machine.scene_root = Some(second_root);
    let second = FemMachineLayout::inspect(app.world_mut(), &machine).unwrap();
    app.world_mut().get_mut::<Transform>(second_root).unwrap().translation = Vec3::new(10.0, 2.0, -4.0);
    app.world_mut().get_mut::<Transform>(second_root).unwrap().rotation = Quat::from_rotation_y(0.3);
    app.update();
    let relocated = crate::physics::fem::rigid_machine::FemRigidMachine::prepare(app.world(), &second).unwrap();
    assert!((relocated.origin - prepared.origin).length() > 10.0);
    assert_eq!(relocated.model.body_mass.host().unwrap(), prepared.model.body_mass.host().unwrap());
    let relocated_belts=crate::physics::fem::track_mesh::FemTrackMeshes::prepare(app.world(),&machine,&second,&relocated).unwrap();
    let relocated_contacts=crate::physics::fem::track_contacts::FemTrackContacts::prepare(app.world(),&machine,&second,&relocated).unwrap();
    for (a,b) in wheel_contacts.shapes.iter().zip(&relocated_contacts.shapes) {
        assert_eq!(a.position,b.position); assert_eq!(a.rotation,b.rotation);
        assert_eq!(a.data,b.data); assert_eq!(a.ids,b.ids);
    }
    assert!(crate::physics::fem::track_contacts::FemTrackContacts::prepare(app.world(),&machine,&second,&prepared).is_err());
    let original_properties=fem_mass_properties(&relocated,None);
    let mut relocated_partition=relocated;
    relocated_partition.partition_belt_mass(&relocated_belts).unwrap();
    assert_fem_mass_conserved(original_properties,fem_mass_properties(&relocated_partition,Some(&relocated_belts)));
    assert!(layout.bodies.iter().all(|e| !second.bodies.contains(e)));
    assert_ne!(layout.chassis, second.chassis);
    let joint = second.loop_joints[0];
    let original = app.world().get::<UsdPhysicsJoint>(joint).unwrap().clone();
    app.world_mut().get_mut::<UsdPhysicsJoint>(joint).unwrap().body1 = Some(layout.chassis);
    assert!(FemMachineLayout::inspect(app.world_mut(), &machine).unwrap_err().contains("foreign"));
    app.world_mut().entity_mut(joint).insert(original);
    let saved = app.world().get::<UsdPhysicsJoint>(joint).unwrap().clone();
    app.world_mut().get_mut::<UsdPhysicsJoint>(joint).unwrap().local_pos1.x += 0.01;
    assert!(crate::physics::fem::rigid_machine::FemRigidMachine::prepare(app.world(), &second)
        .err().unwrap().to_string().contains("loop anchors"));
    app.world_mut().entity_mut(joint).insert(saved);
    let saved_mass = app.world().get::<crate::physics::markers::UsdMass>(second.chassis).unwrap().clone();
    app.world_mut().get_mut::<crate::physics::markers::UsdMass>(second.chassis).unwrap().mass = Some(f32::NAN);
    assert!(crate::physics::fem::rigid_machine::FemRigidMachine::prepare(app.world(), &second).is_err());
    app.world_mut().entity_mut(second.chassis).insert(saved_mass);
    machine.tracks[0].sprocket = "/foreign/sprocket".into();
    assert!(FemMachineLayout::inspect(app.world_mut(), &machine).unwrap_err().contains("outside"));
    eprintln!("actual CEOL binding: {} bodies, {} tree joints, {} closed-loop joints, 2 isolated instances",
        layout.bodies.len(), layout.tree_joints.len(), layout.loop_joints.len());
    eprintln!("prepared CEOL: mass={mass} kg, {} DOFs, {} retained loops",
        prepared.model.joint_dof_total, prepared.loops.len());
}

#[test]
#[ignore = "requires GEARBOX_TRACK_ASSET pointing to a tracked machine USDZ"]
fn real_tracked_machine() {
    let path = std::env::var("GEARBOX_TRACK_ASSET").expect("GEARBOX_TRACK_ASSET");
    let source = usd_bevy::UsdSource::from_file(Path::new(&path)).unwrap();
    let stage = source.open_stage().unwrap();
    let mut machine = discover_machines_from_stage(&stage).unwrap().remove(0);
    assert!(machine.links.errors.is_empty(), "{:?}", machine.links.errors);
    assert_eq!(machine.tracks.len(), 2);
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::transform::TransformPlugin,
        bevy::asset::AssetPlugin { file_path: "/".into(), unapproved_path_mode: bevy::asset::UnapprovedPathMode::Allow, ..default() },
        usd_bevy::UsdPlugin));
    app.init_asset::<Mesh>().init_asset::<StandardMaterial>().init_asset::<Image>();
    app.insert_resource(PhysicsWorld::with_backend(Box::new(MollaBackend::default())));
    app.finish(); app.cleanup();
    let root = crate::physics::benchmark::project(&mut app, &stage);
    machine.scene_root = Some(root);
    app.init_resource::<ControllerInventory>().init_resource::<ControllerCommands>()
        .init_resource::<ControllerRuntimeState>().init_resource::<ControllerStates>()
        .init_resource::<MachineAgentKeys>().init_resource::<crate::services::LinkValues>()
        .insert_resource(gearbox_api::PhysicsActive(true));
    app.world_mut().resource_mut::<ControllerInventory>().machines.push(machine.clone());
    let key = ControllerKey::new(root, &machine.id, "drive");
    app.world_mut().resource_mut::<MachineAgentKeys>().0.insert("tracked-test".into(), key.clone());
    let mut controllers = bevy::ecs::schedule::Schedule::default();
    controllers.add_systems((prepare_machine_physics, apply).chain());
    let mut visuals = bevy::ecs::schedule::Schedule::default();
    visuals.add_systems(animate);
    controllers.run(app.world_mut());
    let map: HashMap<_, _> = app.world_mut().query::<(Entity, &UsdPrimRef)>().iter(app.world())
        .map(|(e, p)| (p.path.clone(), e)).collect();
    let chassis;
    let rotors;
    {
        let mut physics = app.world_mut().resource_mut::<PhysicsWorld>();
        chassis = physics.entity_to_body[&map[machine.body.as_ref().unwrap()]];
        rotors = machine.tracks.iter().map(|s| physics.entity_to_body[&map[&s.sprocket]]).collect::<Vec<_>>();
        assert!(rotors.iter().all(|&r| physics.track_output(r).is_some()), "track registration failed");
        physics.insert_collider(ColliderDesc::new(Shape::Cuboid { half_extents: DVec3::new(100.0, 0.02, 100.0) })
            .translation(DVec3::new(0., -0.02, 0.)).friction(0.85)).unwrap();
        eprintln!("actual asset: {} bodies, {} joints", physics.bodies().len(), physics.joints().len());
    }
    let tick = |app: &mut App, schedule: &mut bevy::ecs::schedule::Schedule, n| {
        for _ in 0..n {
            app.world_mut().resource_mut::<PhysicsWorld>().pending_steps = 1;
            schedule.run(app.world_mut());
            let mut physics = app.world_mut().resource_mut::<PhysicsWorld>();
            physics.step();
            assert!(physics.quarantined.is_empty());
            for id in physics.bodies() {
                let body = physics.body(id).unwrap();
                assert!(body.translation().is_finite() && body.linvel().length() < 30.0, "unstable {id:?}");
            }
        }
    };
    tick(&mut app, &mut controllers, 240);
    let forward = body_forward_vector(app.world().resource::<PhysicsWorld>().body(chassis).unwrap()).unwrap();
    for (label, v, w, n) in [("forward", 0.4, 0.0, 360), ("brake", 0.0, 0.0, 240), ("reverse", -0.4, 0.0, 360), ("pivot", 0.0, 0.6, 360), ("pivot-right", 0.0, -0.6, 600), ("brake", 0.0, 0.0, 240)] {
        let start = app.world().resource::<PhysicsWorld>().body(chassis).unwrap().translation();
        app.world_mut().resource_mut::<ControllerCommands>().cmd_vel.insert(key.clone(), CmdVel { linear_mps: v, angular_rps: w });
        tick(&mut app, &mut controllers, n);
        visuals.run(app.world_mut());
        let physics = app.world().resource::<PhysicsWorld>();
        let body = physics.body(chassis).unwrap();
        let distance = (body.translation() - start).dot(forward);
        let outputs: Vec<_> = rotors.iter().map(|&id| physics.track_output(id).unwrap()).collect();
        eprintln!("{label}: distance={distance:.4} speed={:.4} yaw={:.4} tracks={outputs:?}", body.linvel().length(), body.angvel().y);
        assert!(outputs.iter().all(|s| s.motor_torque.abs() <= 300.0001));
        match label {
            "forward" => assert!(distance > 0.3),
            "reverse" => assert!(distance < -0.3),
            "brake" => assert!(body.linvel().length() < 0.08),
            "pivot" => assert!(body.angvel().y > 0.3),
            "pivot-right" => assert!(body.angvel().y < -0.3),
            _ => unreachable!(),
        }
        for spec in &machine.tracks {
            let sprocket = physics.entity_to_body[&map[&spec.sprocket]];
            let output = physics.track_output(sprocket).unwrap();
            let actual = app.world().get::<Transform>(map[&spec.treads[0]]).unwrap();
            let expected = sample(&spec.path, output.travel).0;
            assert!((actual.translation - expected).length() < 1e-5);
        }
    }
    {
        let mut physics = app.world_mut().resource_mut::<PhysicsWorld>();
        physics.set_gravity(DVec3::ZERO);
        let poses: Vec<_> = physics.bodies().iter().map(|&id| {
            let mut pose = physics.body(id).unwrap().position(); pose.translation.y += 3.; (id, pose)
        }).collect();
        physics.set_body_poses(&poses, true).unwrap();
        for (id, _) in poses { let b = physics.body_mut(id).unwrap(); b.set_linvel(DVec3::ZERO, true); b.set_angvel(DVec3::ZERO, true); }
    }
    app.world_mut().resource_mut::<ControllerCommands>().cmd_vel.insert(key, CmdVel { linear_mps: 0.4, angular_rps: 0.0 });
    tick(&mut app, &mut controllers, 240);
    let physics = app.world().resource::<PhysicsWorld>();
    let momentum: DVec3 = physics.bodies().iter().map(|&id| { let b = physics.body(id).unwrap(); b.linvel() * b.mass() }).sum();
    eprintln!("airborne momentum={momentum:?}");
    assert!(momentum.length() < 0.01);
    assert!(rotors.iter().all(|&id| physics.track_output(id).unwrap().contacts == 0));
    app.world_mut().resource_mut::<ControllerInventory>().machines.clear();
    controllers.run(app.world_mut());
    let physics = app.world().resource::<PhysicsWorld>();
    assert!(rotors.iter().all(|&id| physics.track_output(id).is_none()));
    assert!(app.world().resource::<ControllerStates>().states.is_empty());
}
