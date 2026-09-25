//! A tractor carrying a mounted implement on its three-point hitch.

use super::*;
use bevy::ecs::system::RunSystemOnce;
use crate::physics::backend::JointId;

/// A coupling point: its body and its place in that body's frame.
type Pin = (BodyId, DVec3);

struct Hitched {
    app: App,
    schedules: [bevy::ecs::schedule::Schedule; 3],
    tractor: BodyId,
    implement: BodyId,
    /// The rear hitch, the top link's end, the headstock and its mast.
    pins: [Pin; 4],
}

impl Hitched {
    fn load(tractor: &Path, implement: &Path) -> Self {
        let [tractor, implement] = [tractor, implement].map(|p| p.canonicalize().unwrap());
        let text = format!(
            r#"#usda 1.0
(defaultPrim = "World"
 upAxis = "Z"
 metersPerUnit = 1)
def Xform "World" {{
    def Xform "Tractor" (
        prepend references = @{}@</robot>
        variants = {{ string hitchRear = "linked" }}
    ) {{
        token gearbox:machine:id = "tractor"
    }}
    def Xform "Implement" (prepend references = @{}@</robot>) {{
        token gearbox:machine:id = "implement"
        double3 xformOp:translate = (0, 6, 0)
        uniform token[] xformOpOrder = ["xformOp:translate"]
    }}
}}
"#,
            tractor.display(),
            implement.display(),
        );
        let source = usd_bevy::UsdSource::new(std::env::temp_dir().join("gearbox-hitching.usda"), text.into_bytes()).unwrap();
        let stage = source.open_stage().expect("compose tractor and implement");
        let mut machines = discover_machines_from_stage(&stage).unwrap();
        assert_eq!(machines.len(), 2);
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::transform::TransformPlugin,
            bevy::asset::AssetPlugin {
                file_path: "/".into(),
                unapproved_path_mode: bevy::asset::UnapprovedPathMode::Allow,
                ..default()
            },
            usd_bevy::UsdPlugin,
        ));
        app.init_asset::<Mesh>().init_asset::<StandardMaterial>().init_asset::<Image>();
        app.insert_resource(PhysicsWorld::default());
        app.finish();
        app.cleanup();
        let root = crate::physics::benchmark::project(&mut app, &stage);
        let pins = pins(&mut app, &machines);
        app.init_resource::<ControllerInventory>()
            .init_resource::<ControllerCommands>()
            .init_resource::<ControllerRuntimeState>()
            .init_resource::<ControllerStates>()
            .init_resource::<MachineAgentKeys>()
            .init_resource::<UiDrive>()
            .init_resource::<crate::services::LinkValues>()
            .init_resource::<gearbox_fields::WheelContacts>()
            .insert_resource(gearbox_api::PhysicsActive(true));
        for machine in &mut machines {
            machine.scene_root = Some(root);
            app.world_mut().resource_mut::<MachineAgentKeys>().0
                .insert(machine.id.clone(), ControllerKey::new(root, &machine.id, "drive"));
            for link in &machine.links.links {
                for (name, value) in &link.values {
                    app.world_mut().resource_mut::<crate::services::LinkValues>().set(&machine.id, &link.name, name, *value);
                }
            }
        }
        app.world_mut().resource_mut::<ControllerInventory>().machines = machines.clone();
        let mut controllers = bevy::ecs::schedule::Schedule::default();
        controllers.add_systems((
            guard_chassis_inertia,
            prepare_machine_physics,
            wheel_forces::sync_machine_wheel_forces,
            apply_builtin_ackermann_cmd_vel,
        ).chain());
        controllers.run(app.world_mut());

        // Everything onto the ground, standing on the tractor's tyres.
        let (bodies, wheels) = {
            let runtime = app.world().resource::<ControllerRuntimeState>();
            (runtime.machine_bodies.clone(), runtime.machine_wheels["tractor"].clone())
        };
        let mut physics = app.world_mut().resource_mut::<PhysicsWorld>();
        let clearance = wheels.iter().flat_map(|id| physics.body(*id).unwrap().colliders())
            .map(|id| physics.collider(id).unwrap().aabb().mins.y).fold(f64::INFINITY, f64::min);
        let everything: Vec<BodyId> = bodies.values().flatten().copied().collect();
        let aligned: Vec<_> = everything.iter().map(|&body| {
            let mut pose = physics.body(body).unwrap().position();
            pose.translation.y += 0.03 - clearance;
            (body, pose)
        }).collect();
        physics.set_body_poses(&aligned, true).unwrap();
        // The headstock onto the rear hitch.
        let at = |(body, local): Pin| physics.body(body).unwrap().position().transform_point(local);
        let mut reach = at(pins[0]) - at(pins[2]);
        reach.y = 0.0;
        let behind: Vec<_> = bodies["implement"].iter().map(|&body| {
            let mut pose = physics.body(body).unwrap().position();
            pose.translation += reach;
            (body, pose)
        }).collect();
        physics.set_body_poses(&behind, true).unwrap();
        let ground = physics.insert_collider(ColliderDesc::new(Shape::Cuboid {
            half_extents: DVec3::new(10_000.0, 0.02, 10_000.0),
        }).translation(DVec3::new(0.0, -0.02, 0.0)).friction(1.0).restitution(0.0)).unwrap();
        physics.register_wheel_ground(ground, None).unwrap();
        for &a in &bodies["tractor"] {
            for &b in &bodies["implement"] {
                physics.attachment_filtered_pairs.extend([(a, b), (b, a)]);
            }
        }
        let services = crate::services::benchmark_schedule(&mut app);
        let devices = crate::devices::benchmark_schedule(&mut app);
        let chassis = |id: &str| {
            let physics = app.world().resource::<PhysicsWorld>();
            let path = machines.iter().find(|m| m.id == id).unwrap().body.clone().unwrap();
            app.world().iter_entities().find(|e| e.get::<usd_bevy::UsdPrimRef>().is_some_and(|p| p.path == path))
                .and_then(|e| physics.entity_to_body.get(&e.id()).copied()).unwrap()
        };
        let (tractor, implement) = (chassis("tractor"), chassis("implement"));
        Self { app, schedules: [controllers, devices, services], tractor, implement, pins }
    }

    /// The three-point hitch as a Connect makes it; the top link.
    fn connect(&mut self) -> JointId {
        let [hitch, top_end, coupler, mast] = self.pins;
        let mut physics = self.app.world_mut().resource_mut::<PhysicsWorld>();
        crate::attach::join_hitch(
            &mut physics,
            "three_point_mounted",
            (hitch.0, Pose::new(hitch.1, DQuat::IDENTITY)),
            (coupler.0, Pose::new(coupler.1, DQuat::IDENTITY)),
            Some((top_end, mast)),
        )
        .and_then(|(_, top)| top)
        .expect("a three-point hitch with its top link")
    }

    fn tick(&mut self) {
        self.app.world_mut().resource_mut::<Time>().advance_by(Duration::from_secs_f64(1.0 / 120.0));
        for schedule in &mut self.schedules {
            schedule.run(self.app.world_mut());
        }
        let mut physics = self.app.world_mut().resource_mut::<PhysicsWorld>();
        physics.step();
        assert!(physics.quarantined.is_empty(), "physics quarantined a body");
    }

    fn pose(&self, body: BodyId) -> Pose {
        self.app.world().resource::<PhysicsWorld>().body(body).unwrap().position()
    }

    fn hitch(&mut self, position: f64, seconds: f64) {
        self.app.world_mut().resource_mut::<crate::services::LinkValues>()
            .set("tractor", "hitch_lower_left", "position", position);
        for _ in 0..(seconds * 120.0) as usize {
            self.tick();
        }
    }

    /// How far a joint's two anchors have come apart.
    fn gap(&self, joint: JointId) -> f64 {
        let physics = self.app.world().resource::<PhysicsWorld>();
        let (j, (a, b)) = (physics.joint(joint).unwrap(), physics.joint_bodies(joint).unwrap());
        let (pa, pb) = (physics.body(a).unwrap().position(), physics.body(b).unwrap().position());
        (pa.translation + pa.rotation * j.frame1().translation).distance(pb.translation + pb.rotation * j.frame2().translation)
    }
}

#[test]
#[ignore = "requires GEARBOX_BENCH_ASSET (kubota_tractor.usdz) and GEARBOX_BENCH_TRAILER (knoche_disc_harrow.usdz)"]
fn imported_tractor_lifts_and_lowers_a_mounted_implement() {
    let tractor = std::env::var_os("GEARBOX_BENCH_ASSET").expect("set GEARBOX_BENCH_ASSET");
    let implement = std::env::var_os("GEARBOX_BENCH_TRAILER").expect("set GEARBOX_BENCH_TRAILER");
    let mut hitched = Hitched::load(Path::new(&tractor), Path::new(&implement));
    hitched.hitch(0.0, 2.0);
    let top_link = hitched.connect();
    hitched.hitch(0.0, 4.0);
    let (parked, level) = (hitched.pose(hitched.implement), hitched.pose(hitched.tractor).rotation);
    hitched.hitch(1.0, 12.0);
    let (lifted, carrying) = (hitched.pose(hitched.implement), hitched.pose(hitched.tractor).rotation);
    let rise = lifted.translation.y - parked.translation.y;
    let tilt = level.angle_between(carrying).to_degrees();
    eprintln!("raised {rise:.3} m, tractor tilted {tilt:.1}°, top link apart {:.3} m", hitched.gap(top_link));
    assert!(rise > 0.8, "the implement rose {rise} m");
    assert!(tilt < 2.0, "the tractor tilted {tilt}° under the implement");
    assert!(hitched.gap(top_link) < 0.02, "the top link stretched");
    hitched.hitch(0.0, 12.0);
    let lowered = hitched.pose(hitched.implement).translation.y - parked.translation.y;
    eprintln!("lowered back to {lowered:.3} m");
    assert!(lowered.abs() < 0.05, "the implement came back down to {lowered} m");
}

/// The tractor's rear three-point hitch and top-link end, and the
/// implement's headstock and mast, as the stage was projected.
fn pins(app: &mut App, machines: &[MachineInstanceSpec]) -> [Pin; 4] {
    let coupling = |id: &str, side: crate::links::CouplingSide| {
        let machine = machines.iter().find(|m| m.id == id).unwrap();
        let (link, coupling) = machine.links.couplings()
            .find(|(_, c)| c.side == side && c.kind == "three_point_mounted"
                && (side == crate::links::CouplingSide::Coupler || c.name.starts_with("rear")))
            .unwrap();
        [link.prim_path.clone(), coupling.top_link.clone().expect("a top-link pin")]
    };
    let [hitch, top_end] = coupling("tractor", crate::links::CouplingSide::Hitch);
    let [coupler, mast] = coupling("implement", crate::links::CouplingSide::Coupler);
    app.world_mut().run_system_once(
        move |prims: Query<(Entity, &usd_bevy::UsdPrimRef)>, parents: Query<&'static ChildOf>,
              transforms: Query<&'static GlobalTransform>, physics: Res<PhysicsWorld>| {
            [&hitch, &top_end, &coupler, &mast].map(|path| {
                let entity = prims.iter().find(|(_, p)| p.path == *path).unwrap().0;
                crate::attach::pin(entity, &parents, &transforms, &physics).unwrap()
            })
        },
    ).unwrap()
}
