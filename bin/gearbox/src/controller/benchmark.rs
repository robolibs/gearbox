//! Imported-machine CPU benchmark without rendering, network agents or wall-clock stepping.

use super::*;
use crate::physics::{MollaBackend, PhysicsWorld, RapierBackend};
use crate::physics::backend::{ColliderDesc, ColliderId, DQuat, Pose, Shape};
use std::time::{Duration, Instant};

struct Fixture {
    app: App,
    controllers: bevy::ecs::schedule::Schedule,
    services: bevy::ecs::schedule::Schedule,
    chassis: BodyId,
    ground: ColliderId,
    wheels: Vec<BodyId>,
    machine: MachineInstanceSpec,
}

impl Fixture {
    fn load(path: &Path) -> Self {
        Self::load_backend(path, true)
    }

    fn load_backend(path: &Path, molla: bool) -> Self {
        let source = usd_bevy::UsdSource::from_file(path).expect("read benchmark asset");
        let stage = source.open_stage().expect("open benchmark stage");
        let mut machines = discover_machines_from_stage(&stage).unwrap();
        assert_eq!(machines.len(), 1, "benchmark requires one machine");
        let mut machine = machines.remove(0);
        let kubota = match path.file_name().unwrap().to_str().unwrap() {
            "kubota_tractor.usdz" => true,
            "krampe_trailer.usdz" => false,
            _ => panic!("benchmark requires the real Kubota or Krampe asset"),
        };
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
        app.insert_resource(PhysicsWorld::with_backend(if molla {
            Box::new(MollaBackend::default())
        } else { Box::new(RapierBackend::default()) }));
        if molla && let Some(value) = std::env::var_os("GEARBOX_BENCH_MOLLA_INTERNAL_ITERATIONS") {
            let iterations: usize = value.to_str().unwrap().parse().expect("invalid benchmark iterations");
            assert!((1..=64).contains(&iterations));
            let mut physics = app.world_mut().resource_mut::<PhysicsWorld>();
            let mut settings = physics.settings();
            settings.internal_iterations = iterations;
            physics.set_settings(settings);
        }
        app.finish();
        app.cleanup();
        let root = crate::physics::benchmark::project(&mut app, &stage);
        machine.scene_root = Some(root);
        app.init_resource::<ControllerInventory>()
            .init_resource::<ControllerCommands>()
            .init_resource::<ControllerRuntimeState>()
            .init_resource::<ControllerStates>()
            .init_resource::<MachineAgentKeys>()
            .init_resource::<UiDrive>()
            .init_resource::<crate::services::LinkValues>()
            .init_resource::<gearbox_fields::WheelContacts>()
            .insert_resource(gearbox_api::PhysicsActive(true));
        app.world_mut().resource_mut::<ControllerInventory>().machines.push(machine.clone());
        app.world_mut().resource_mut::<MachineAgentKeys>().0.insert(
            "benchmark".into(), ControllerKey::new(root, &machine.id, "drive"),
        );
        for link in &machine.links.links {
            for (name, value) in &link.values {
                app.world_mut().resource_mut::<crate::services::LinkValues>()
                    .set(&machine.id, &link.name, name, *value);
            }
        }
        let mut controllers = bevy::ecs::schedule::Schedule::default();
        controllers.add_systems((
            guard_chassis_inertia,
            prepare_machine_physics,
            wheel_forces::sync_machine_wheel_forces,
            apply_builtin_ackermann_cmd_vel,
            apply_builtin_diff_drive_cmd_vel,
            record_wheel_tracks,
        ).chain());
        controllers.run(app.world_mut());
        let runtime = app.world().resource::<ControllerRuntimeState>();
        let mut bodies = runtime.machine_bodies[&machine.id].clone();
        bodies.sort();
        let wheels = runtime.machine_wheels[&machine.id].clone();
        assert_eq!(bodies.len(), if kubota { 26 } else { 15 });
        assert_eq!(wheels.len(), 4);
        let chassis_entity = app.world_mut().query::<(Entity, &UsdPrimRef)>()
            .iter(app.world()).find(|(_, prim)| Some(&prim.path) == machine.body.as_ref()).unwrap().0;
        let mut physics = app.world_mut().resource_mut::<PhysicsWorld>();
        let chassis = physics.entity_to_body[&chassis_entity];
        let mass: f64 = bodies.iter().map(|id| physics.body(*id).unwrap().mass()).sum();
        if kubota {
            let expected_mass = if molla { 4916.010223 } else { 4913.449268 };
            assert!((mass - expected_mass).abs() < 0.001, "imported mass {mass}");
        } else {
            assert!((6500.0..7000.0).contains(&mass), "imported trailer mass {mass}");
        }
        let clearance = wheels.iter().flat_map(|id| physics.body(*id).unwrap().colliders())
            .map(|id| physics.collider(id).unwrap().aabb().mins.y).fold(f64::INFINITY, f64::min);
        for body in bodies {
            let body = physics.body_mut(body).unwrap();
            let mut pose = body.position();
            pose.translation.y += 0.03 - clearance;
            body.set_position(pose, true);
        }
        let ground = physics.insert_collider(ColliderDesc::new(Shape::Cuboid {
            half_extents: DVec3::new(10_000.0, 0.02, 10_000.0),
        }).translation(DVec3::new(0.0, -0.02, 0.0)).friction(1.0).restitution(0.0)).unwrap();
        physics.register_wheel_ground(ground, None).unwrap();
        eprintln!("imported fixture: bodies={} joints={} colliders={} tyres={} mass={mass} settings={:?}",
            physics.bodies().len(), physics.joints().len(), physics.colliders().len(), wheels.len(), physics.settings());
        assert_eq!(physics.joints().len(), if kubota { 31 } else { 14 });
        assert_eq!(physics.colliders().len(), if kubota { 18 } else { 19 });
        assert!((physics.dt() - 1.0 / 120.0).abs() < 1e-15, "benchmark requires 120 Hz");
        let services = crate::services::benchmark_schedule(&mut app);
        Self { app, controllers, services, chassis, ground, wheels, machine }
    }

    fn incline(&mut self, radians: f64) -> DVec3 {
        let rotation = DQuat::from_rotation_x(radians);
        let mut physics = self.app.world_mut().resource_mut::<PhysicsWorld>();
        let mut bodies = physics.bodies();
        bodies.sort();
        let poses: Vec<_> = bodies.into_iter().map(|id| (id, physics.body(id).unwrap().position())).collect();
        for (id, pose) in poses {
            physics.body_mut(id).unwrap().set_position(
                Pose::new(rotation * pose.translation, rotation * pose.rotation), true,
            );
        }
        physics.collider_mut(self.ground).unwrap().set_position(
            Pose::new(rotation * DVec3::new(0.0, -0.02, 0.0), rotation),
        );
        rotation * DVec3::Y
    }

    fn tick(&mut self) -> Duration {
        self.app.world_mut().resource_mut::<Time>().advance_by(Duration::from_secs_f64(1.0 / 120.0));
        let now = self.app.world().resource::<Time>().elapsed_secs_wrapped();
        {
            let mut contacts = self.app.world_mut().resource_mut::<gearbox_fields::WheelContacts>();
            contacts.contacts.clear();
            contacts.now = now;
        }
        self.controllers.run(self.app.world_mut());
        self.services.run(self.app.world_mut());
        let mut physics = self.app.world_mut().resource_mut::<PhysicsWorld>();
        let start = Instant::now();
        physics.step();
        let elapsed = start.elapsed();
        assert!(physics.quarantined.is_empty(), "physics quarantined an imported body");
        for id in physics.bodies() {
            let body = physics.body(id).unwrap();
            assert!(body.is_enabled() && body.position().translation.is_finite()
                && body.position().rotation.is_finite() && body.linvel().is_finite()
                && body.angvel().is_finite(), "invalid imported body {id:?}");
        }
        elapsed
    }

    fn pressure(&mut self, bar: f64) {
        wheel_forces::set_pressure_group(&self.machine,
            &mut self.app.world_mut().resource_mut::<crate::services::LinkValues>(), "all", bar).unwrap();
    }

    fn drive(&mut self, forward: f32, yaw: f32) {
        let key = ControllerKey::new(self.machine.scene_root.unwrap(), &self.machine.id, "drive");
        self.app.world_mut().resource_mut::<ControllerCommands>().cmd_vel.insert(
            key, CmdVel { linear_mps: forward, angular_rps: yaw },
        );
    }

    fn verify(&self, bar: f64, driving: bool) -> f64 {
        let physics = self.app.world().resource::<PhysicsWorld>();
        let chassis = physics.body(self.chassis).unwrap();
        let speed = chassis.linvel().length();
        if driving { assert!((speed - 2.0).abs() < 0.15, "driving speed {speed}"); }
        else { assert!(speed < 0.05, "parked speed {speed}"); }
        let mut radii = Vec::new();
        let mut deflections = Vec::new();
        let mut load = 0.0;
        for wheel in &self.wheels {
            let out = physics.wheel_output(*wheel).expect("registered wheel output");
            let pressure = out.pressure.expect("pressure mechanics enabled");
            assert!(out.in_contact && out.normal_force > 100.0, "unsupported wheel {wheel:?}");
            assert!((pressure.pressure_pa / 1e5 - bar).abs() < 1e-8);
            assert!(pressure.deflection > 0.0 && pressure.loaded_radius > 0.5);
            radii.push(pressure.radius);
            deflections.push(pressure.deflection);
            load += out.normal_force;
        }
        radii.sort_by(f64::total_cmp);
        for (actual, expected) in radii.iter().zip([0.691259358, 0.691259358, 0.888205080, 0.888205080]) {
            assert!((actual - expected).abs() < 1e-5, "reference rubber radius {actual} != {expected}");
        }
        assert!((load - 4916.0 * 9.81).abs() < 500.0, "unsupported total machine load {load}");
        eprintln!("state: pressure={bar} driving={driving} speed={speed:.6} load={load:.3} deflections_m={deflections:?} position={:?}", chassis.translation());
        deflections.iter().sum::<f64>() / deflections.len() as f64
    }

    fn snapshot(&self) -> Vec<f64> {
        let physics = self.app.world().resource::<PhysicsWorld>();
        let mut bodies: Vec<_> = physics.entity_to_body.iter().map(|(entity, id)| {
            (self.app.world().get::<UsdPrimRef>(*entity).unwrap().path.as_str(), *id)
        }).collect();
        bodies.sort_by_key(|(path, _)| *path);
        let mut values = Vec::new();
        for (_, id) in bodies {
            let body = physics.body(id).unwrap();
            values.extend(body.translation().to_array());
            values.extend(body.rotation().to_array());
            values.extend(body.linvel().to_array());
            values.extend(body.angvel().to_array());
            if let Some(output) = physics.wheel_output(id) {
                let pressure = output.pressure.unwrap();
                values.extend([
                    output.normal_force, output.grip_force, output.slip_ratio, output.slip_angle,
                    pressure.pressure_pa, pressure.target_pressure_pa, pressure.deflection,
                    pressure.loaded_radius, pressure.patch_length, pressure.patch_width,
                ]);
            }
        }
        values
    }

    fn trace_support(&self, step: usize) {
        let physics = self.app.world().resource::<PhysicsWorld>();
        let mut energy = 0.0;
        let mut potential = 0.0;
        for id in physics.bodies() {
            let body = physics.body(id).unwrap();
            if step == 120 {
                let path = body.entity().and_then(|e| self.app.world().get::<UsdPrimRef>(e));
                eprintln!("support body id={id:?} path={:?} mass={}", path.map(|p| &p.path), body.mass());
                for collider in body.colliders() {
                    let collider = physics.collider(collider).unwrap();
                    eprintln!("support shape body={id:?} shape={:?} pose={:?} bounds={:?}", collider.shape(), collider.position(), collider.aabb());
                }
            }
            let angular = body.rotation().inverse() * body.angvel();
            energy += 0.5 * (body.mass() * body.linvel().length_squared()
                + angular.dot(body.inertia_tensor() * angular));
            potential += body.mass() * 9.81 * body.center_of_mass().y;
        }
        let body = physics.body(self.chassis).unwrap();
        eprintln!("support trace step={step} backend={} kinetic={energy:.9} potential={potential:.9} chassis_y={:.9} velocity={:?} angular={:?}",
            physics.name(), body.translation().y, body.linvel(), body.angvel());
        for manifold in physics.contacts().into_iter().filter(|m| m.active) {
            let parents = [manifold.collider1, manifold.collider2]
                .map(|id| physics.collider(id).unwrap().parent());
            for point in manifold.points.iter().filter(|p| p.solved) {
                eprintln!("support contact step={step} parents={parents:?} normal={:?} step_mean_force={} depth={} point={:?}",
                    manifold.normal, point.impulse / physics.dt(), -point.dist, point.point);
            }
        }
        for wheel in &self.wheels {
            let body = physics.body(*wheel).unwrap();
            let out = physics.wheel_output(*wheel);
            eprintln!("support wheel step={step} body={wheel:?} y={} vy={} load={:?}",
                body.translation().y, body.linvel().y, out.map(|o| o.normal_force));
        }
    }
}

#[test]
#[ignore = "requires GEARBOX_BENCH_ASSET pointing to real kubota_tractor.usdz; CPU benchmark"]
fn imported_kubota_pressure_benchmark() {
    let asset = std::env::var_os("GEARBOX_BENCH_ASSET").expect("set GEARBOX_BENCH_ASSET");
    let mut fixture = Fixture::load(Path::new(&asset));
    for _ in 0..600 { fixture.tick(); }
    for driving in [false, true] {
        fixture.drive(if driving { 2.0 } else { 0.0 }, if driving { 0.12 } else { 0.0 });
        let mut previous_deflection = f64::INFINITY;
        for pressure in [0.5, 1.8, 4.0] {
            fixture.pressure(pressure);
            for _ in 0..600 { fixture.tick(); }
            let deflection = fixture.verify(pressure, driving);
            assert!(deflection < previous_deflection, "inflation must reduce loaded deflection");
            previous_deflection = deflection;
            let mut samples = Vec::new();
            for _ in 0..600 { samples.push(fixture.tick().as_secs_f64() * 1000.0); }
            fixture.verify(pressure, driving);
            samples.sort_by(f64::total_cmp);
            eprintln!("full imported Kubota physics pressure={pressure} driving={driving}: median={:.6} p95={:.6} max={:.6} ms/step; samples={} hz=120 renderer=none controller=outside-timing",
                samples[300], samples[570], samples[599], samples.len());
        }
    }
}

#[test]
#[ignore = "requires GEARBOX_BENCH_ASSET pointing to real kubota_tractor.usdz; paired CPU timing"]
fn imported_kubota_backend_timing() {
    let asset = std::env::var_os("GEARBOX_BENCH_ASSET").expect("set GEARBOX_BENCH_ASSET");
    for (run, molla) in [false, true, true, false].into_iter().enumerate() {
        let mut fixture = Fixture::load_backend(Path::new(&asset), molla);
        for driving in [false, true] {
            fixture.drive(if driving { 2.0 } else { 0.0 }, if driving { 0.12 } else { 0.0 });
            for _ in 0..600 { fixture.tick(); }
            let mut samples: Vec<_> = (0..1200)
                .map(|_| fixture.tick().as_secs_f64() * 1000.0).collect();
            samples.sort_by(f64::total_cmp);
            let physics = fixture.app.world().resource::<PhysicsWorld>();
            let speed = physics.body(fixture.chassis).unwrap().linvel().length();
            if driving { assert!((speed - 2.0).abs() < 0.2, "{} driving speed={speed}", physics.name()); }
            else { assert!(speed < 0.05, "{} parked speed={speed}", physics.name()); }
            let bodies = physics.bodies();
            let sleeping = bodies.iter().filter(|id| physics.body(**id).unwrap().is_sleeping()).count();
            eprintln!("paired backend timing run={run} backend={} driving={driving} speed={speed:.6} sleeping={sleeping}/{} median={:.6} p95={:.6} p99={:.6} max={:.6} ms/step; samples={} hz=120 renderer=none controller=outside-timing settings={:?}",
                physics.name(), bodies.len(), samples[600], samples[1140], samples[1188], samples[1199], samples.len(), physics.settings());
            if molla { fixture.verify(1.8, driving); }
        }
    }
}

#[test]
#[ignore = "requires GEARBOX_BENCH_ASSET pointing to real kubota_tractor.usdz; 60 s slope hold"]
fn imported_kubota_slope_parking() {
    let asset = std::env::var_os("GEARBOX_BENCH_ASSET").expect("set GEARBOX_BENCH_ASSET");
    let mut results = Vec::new();
    for degrees in [-10.0_f64, 10.0] {
        for bar in [0.5, 4.0] {
            let mut fixture = Fixture::load(Path::new(&asset));
            let normal = fixture.incline(degrees.to_radians());
            fixture.pressure(bar);
            fixture.drive(0.0, 0.0);
            for _ in 0..1200 { fixture.tick(); }
            let physics = fixture.app.world().resource::<PhysicsWorld>();
            let start = physics.body(fixture.chassis).unwrap().translation();
            let mut max_drift = 0.0_f64;
            let mut max_speed = 0.0_f64;
            for step in 0..7200 {
                fixture.tick();
                let physics = fixture.app.world().resource::<PhysicsWorld>();
                let body = physics.body(fixture.chassis).unwrap();
                let displacement = body.translation() - start;
                max_drift = max_drift.max((displacement - normal * displacement.dot(normal)).length());
                max_speed = max_speed.max(body.linvel().length());
                if step % 120 == 119 {
                    let mut load = 0.0;
                    for wheel in &fixture.wheels {
                        let output = physics.wheel_output(*wheel).unwrap();
                        assert!(output.in_contact && output.normal_force > 100.0);
                        let pressure = output.pressure.unwrap();
                        assert!((pressure.pressure_pa / 1e5 - bar).abs() < 1e-8);
                        assert!(pressure.ground.unwrap().normal.dot(normal) > 1.0 - 1e-10);
                        load += output.normal_force;
                    }
                    assert!((load - 4916.0 * 9.81 * normal.y).abs() < 500.0, "slope support {load}");
                }
            }
            let physics = fixture.app.world().resource::<PhysicsWorld>();
            for wheel in &fixture.wheels {
                let output = physics.wheel_output(*wheel).unwrap();
                let body = physics.body(*wheel).unwrap();
                let (axis, _, _) = body_tyre_geometry(physics, *wheel).unwrap();
                let spin = (body.angvel() - physics.body(fixture.chassis).unwrap().angvel())
                    .dot(body.rotation() * axis);
                eprintln!("slope wheel: id={wheel:?} spin_rad_s={spin:.9} slip_ratio={:.9} normal_force={:.3} grip_force={:.3}",
                    output.slip_ratio, output.normal_force, output.grip_force);
            }
            eprintln!("slope parking: degrees={degrees} bar={bar} duration=60s max_drift_m={max_drift:.9} max_speed_mps={max_speed:.9}");
            results.push((degrees, bar, max_drift, max_speed));
        }
    }
    for (degrees, bar, drift, speed) in results {
        assert!(drift < 0.01 && speed < 0.001,
            "slope creep at {degrees} degrees, {bar} bar: {drift} m, {speed} m/s");
    }
}

#[test]
#[ignore = "requires GEARBOX_BENCH_ASSET pointing to real kubota_tractor.usdz; drive and track inputs"]
fn imported_kubota_straight_turn_and_track_contacts() {
    let asset = std::env::var_os("GEARBOX_BENCH_ASSET").expect("set GEARBOX_BENCH_ASSET");
    let mut reference_headings = [0.0; 3];
    let mut reference_speeds = [0.0; 3];
    let mut straight_footprints = Vec::new();
    for pressure in [None, Some(0.5), Some(1.8), Some(4.0)] {
        for (command, turn) in [0.0_f32, 0.4, -0.4].into_iter().enumerate() {
            let mut fixture = Fixture::load_backend(Path::new(&asset), pressure.is_some());
            if let Some(bar) = pressure { fixture.pressure(bar); }
            for _ in 0..600 { fixture.tick(); }
            let physics = fixture.app.world().resource::<PhysicsWorld>();
            let start = physics.body(fixture.chassis).unwrap().translation();
            let heading = machine_heading_rad(physics.body(fixture.chassis).unwrap());
            fixture.drive(2.0, turn);
            let mut samples = 0;
            let mut footprints = Vec::new();
            for step in 0..480 {
                fixture.tick();
                if step < 120 { continue; }
                let contacts = fixture.app.world().resource::<gearbox_fields::WheelContacts>();
                assert_eq!(contacts.contacts.len(), 4, "all supported wheels need track input");
                for contact in &contacts.contacts {
                    assert!(contact.position.is_finite() && contact.position.y.abs() < 0.02);
                    assert!((contact.direction.length() - 1.0).abs() < 1e-5);
                    assert!(contact.width.is_finite() && contact.width > 0.1);
                    if pressure.is_some() {
                        let length = contact.length.expect("pressure footprint length");
                        assert!(length.is_finite() && length > 0.01);
                        footprints.push(length);
                    }
                    samples += 1;
                }
            }
            let physics = fixture.app.world().resource::<PhysicsWorld>();
            let body = physics.body(fixture.chassis).unwrap();
            let delta = machine_heading_rad(body) - heading;
            let displacement = body.translation() - start;
            let speed = body.linvel().length();
            let patch_mean = footprints.iter().sum::<f32>() / footprints.len().max(1) as f32;
            eprintln!("4s drive: backend={} pressure={pressure:?} command=(2,{turn}) heading_delta={delta:.9} displacement={displacement:?} speed={speed:.9} track_samples={samples} mean_patch_length={patch_mean}", physics.name());
            if pressure.is_none() {
                reference_headings[command] = delta;
                reference_speeds[command] = speed;
            } else {
                assert!((delta - reference_headings[command]).abs() < 0.1, "turn diverged from same-scene Rapier");
                assert!((speed - reference_speeds[command]).abs() < 0.1, "speed diverged from same-scene Rapier");
                if turn == 0.0 { straight_footprints.push(patch_mean); }
            }
            assert!((speed - 2.0).abs() < 0.2, "drive speed {speed}");
            assert!(displacement.length() > 5.0, "vehicle did not drive");
            if turn == 0.0 {
                assert!(delta.abs() < 0.02 && displacement.x.abs() < 0.1, "straight drive wandered");
            } else {
                assert!(delta * f64::from(turn.signum()) > 0.3 && delta.abs() < 1.8, "wrong turn response {delta}");
                assert!(displacement.x * f64::from(turn.signum()) > 0.5);
            }
        }
    }
    assert_eq!(straight_footprints.len(), 3);
    assert!(straight_footprints.windows(2).all(|pair| pair[0] > pair[1] + 0.02), "pressure must change track footprints: {straight_footprints:?}");
}

#[test]
#[ignore = "requires GEARBOX_BENCH_TRAILER pointing to real krampe_trailer.usdz"]
fn imported_krampe_parking_support() {
    let asset = std::env::var_os("GEARBOX_BENCH_TRAILER").expect("set GEARBOX_BENCH_TRAILER");
    let trace = std::env::var_os("GEARBOX_BENCH_TRACE_SUPPORT").is_some();
    let mut outcomes = Vec::new();
    for pressure in [None, Some(0.5), Some(1.8), Some(4.0)] {
        let mut fixture = Fixture::load_backend(Path::new(&asset), pressure.is_some());
        if let Some(bar) = pressure { fixture.pressure(bar); }
        for step in 0..1800 {
            fixture.tick();
            if trace && step % 120 == 119 { fixture.trace_support(step + 1); }
        }
        let physics = fixture.app.world().resource::<PhysicsWorld>();
        let body = physics.body(fixture.chassis).unwrap();
        let (roll, pitch) = machine_roll_pitch_rad(body);
        eprintln!("parked trailer: backend={} pressure={pressure:?} roll={roll} pitch={pitch} speed={} velocity={:?} angular={:?} position={:?}", physics.name(), body.linvel().length(), body.linvel(), body.angvel(), body.translation());
        for wheel in &fixture.wheels {
            let output = physics.wheel_output(*wheel);
            eprintln!("wheel {wheel:?}: position={:?} normal_force={:?} pressure={:?}", physics.body(*wheel).unwrap().translation(), output.map(|out| out.normal_force), output.and_then(|out| out.pressure).map(|p| (p.pressure_pa, p.deflection)));
        }
        outcomes.push((pressure, roll, pitch, body.linvel().length()));
    }
    for (pressure, roll, pitch, speed) in outcomes {
        assert!(roll.abs() < 0.1 && pitch.abs() < 0.2, "parked trailer tipped at {pressure:?}: roll={roll}, pitch={pitch}");
        assert!(speed < 0.05, "parked trailer moving at {pressure:?}: {speed}");
    }
}

#[test]
#[ignore = "requires GEARBOX_BENCH_ASSET pointing to real kubota_tractor.usdz; five controllers"]
fn imported_kubota_hitch_and_pto_controllers() {
    let asset = std::env::var_os("GEARBOX_BENCH_ASSET").expect("set GEARBOX_BENCH_ASSET");
    let mut fixture = Fixture::load(Path::new(&asset));
    assert_eq!(fixture.machine.controllers.len(), 5);
    let mut controls = Vec::new();
    for controller in &fixture.machine.controllers {
        eprintln!("imported controller: {} type={} target={:?}", controller.instance, controller.controller_type, controller.target);
        if !matches!(controller.controller_type.as_str(), "builtin:hitch" | "builtin:pto") { continue; }
        let paths = crate::services::controller_joints(&fixture.machine, controller);
        assert_eq!(paths.len(), 1);
        let link = crate::services::moved_link(&fixture.machine.links, paths[0]).expect("controlled link");
        let marker = fixture.app.world_mut().query::<(&UsdPrimRef, &crate::physics::markers::UsdPhysicsJoint)>()
            .iter(fixture.app.world()).find(|(prim, _)| prim.path == paths[0]).unwrap().1.clone();
        let physics = fixture.app.world().resource::<PhysicsWorld>();
        let a = physics.entity_to_body[&marker.body0.unwrap()];
        let b = physics.entity_to_body[&marker.body1.unwrap()];
        let joints = physics.joints_between(a, b);
        assert_eq!(joints.len(), 1, "ambiguous controller joint");
        eprintln!("controlled link: {} values={:?} joint={:?}", link.name, link.values, joints[0]);
        controls.push((controller.controller_type.clone(), link.name.clone(), joints[0]));
    }
    assert_eq!(controls.iter().filter(|(kind, _, _)| kind == "builtin:hitch").count(), 2);
    assert_eq!(controls.iter().filter(|(kind, _, _)| kind == "builtin:pto").count(), 2);
    for _ in 0..600 { fixture.tick(); }
    for (kind, link, _) in &controls {
        let id = &fixture.machine.id;
        let mut values = fixture.app.world_mut().resource_mut::<crate::services::LinkValues>();
        if kind == "builtin:hitch" {
            values.set(id, link, "position", 0.75);
        } else {
            assert!(values.get(id, link, "rpm").unwrap() > 0.0);
            values.set(id, link, "engaged", 1.0);
        }
    }
    let initial: Vec<_> = controls.iter().map(|(_, _, id)| fixture.app.world().resource::<PhysicsWorld>()
        .joint(*id).unwrap().motor_position(JointAxis::AngX).expect("joint coordinate")).collect();
    for _ in 0..600 { fixture.tick(); }
    let physics = fixture.app.world().resource::<PhysicsWorld>();
    let mut running = Vec::new();
    for ((kind, link, id), start) in controls.iter().zip(initial) {
        let joint = physics.joint(*id).unwrap();
        let position = joint.motor_position(JointAxis::AngX).unwrap();
        let motor = joint.motor(JointAxis::AngX).expect("service motor");
        eprintln!("service result: {kind} link={link} start={start} position={position} motor={motor:?}");
        assert!((position - start).abs() > if kind == "builtin:hitch" { 0.05 } else { 10.0 }, "controller did not move {link}");
        if kind == "builtin:hitch" {
            let range = fixture.app.world().resource::<crate::services::LinkValues>().get(&fixture.machine.id, link, "range").unwrap();
            assert!((motor.target_position - 0.75 * range).abs() < 1e-12);
            assert!((position - motor.target_position).abs() < 0.05, "hitch tracking error {link}");
            let limits = joint.limits(JointAxis::AngX).unwrap();
            assert!(position >= limits[0] - 1e-3 && position <= limits[1] + 1e-3);
        }
        running.push(position);
    }
    for _ in 0..120 { fixture.tick(); }
    for ((kind, link, id), before) in controls.iter().zip(&running) {
        if kind != "builtin:pto" { continue; }
        let rpm = fixture.app.world().resource::<crate::services::LinkValues>().get(&fixture.machine.id, link, "rpm").unwrap();
        let measured = fixture.app.world().resource::<PhysicsWorld>().joint(*id).unwrap()
            .motor_position(JointAxis::AngX).unwrap() - before;
        eprintln!("PTO measured: {link} rpm={rpm} angular_velocity={measured}");
        assert!((measured - rpm * std::f64::consts::TAU / 60.0).abs() < 0.5);
    }
    for (kind, link, _) in &controls {
        fixture.app.world_mut().resource_mut::<crate::services::LinkValues>()
            .set(&fixture.machine.id, link, if kind == "builtin:hitch" { "position" } else { "engaged" },
                if kind == "builtin:hitch" { 0.15 } else { 0.0 });
    }
    for _ in 0..600 { fixture.tick(); }
    let stopped: Vec<_> = controls.iter().map(|(_, _, id)| fixture.app.world().resource::<PhysicsWorld>()
        .joint(*id).unwrap().motor_position(JointAxis::AngX).unwrap()).collect();
    let mut previous = stopped.clone();
    let mut peak_speed = vec![0.0_f64; controls.len()];
    for _ in 0..120 {
        fixture.tick();
        let physics = fixture.app.world().resource::<PhysicsWorld>();
        for (index, (_, _, id)) in controls.iter().enumerate() {
            let current = physics.joint(*id).unwrap().motor_position(JointAxis::AngX).unwrap();
            peak_speed[index] = peak_speed[index].max((current - previous[index]).abs() * 120.0);
            previous[index] = current;
        }
    }
    for (index, (((kind, link, id), raised), stopped)) in controls.iter().zip(&running).zip(stopped).enumerate() {
        let physics = fixture.app.world().resource::<PhysicsWorld>();
        let joint = physics.joint(*id).unwrap();
        let final_position = joint.motor_position(JointAxis::AngX).unwrap();
        if kind == "builtin:hitch" {
            let range = fixture.app.world().resource::<crate::services::LinkValues>().get(&fixture.machine.id, link, "range").unwrap();
            assert!((final_position - 0.15 * range).abs() < 0.05, "hitch return {link}: {final_position}");
            assert!((final_position - raised).abs() > 0.05);
        } else {
            assert_eq!(joint.motor(JointAxis::AngX).unwrap().target_velocity, 0.0);
            assert!((final_position - stopped).abs() < 0.01, "PTO failed to disengage {link}");
            assert!(peak_speed[index] < 0.01, "PTO still moving after disengagement {link}: {}", peak_speed[index]);
        }
        eprintln!("service returned: {kind} link={link} position={final_position} peak_speed={}", peak_speed[index]);
    }
}

#[test]
#[ignore = "requires GEARBOX_BENCH_ASSET pointing to real kubota_tractor.usdz; deterministic replay"]
fn imported_kubota_pressure_replay() {
    let asset = std::env::var_os("GEARBOX_BENCH_ASSET").expect("set GEARBOX_BENCH_ASSET");
    let run = || {
        let mut fixture = Fixture::load(Path::new(&asset));
        let mut checkpoints = Vec::new();
        for step in 0..1800 {
            if step == 300 { fixture.pressure(0.5); }
            if step == 600 { fixture.drive(2.0, 0.12); }
            if step == 900 { fixture.pressure(4.0); }
            if step == 1500 { fixture.drive(0.0, 0.0); }
            fixture.tick();
            if step % 60 == 59 { checkpoints.push(fixture.snapshot()); }
        }
        checkpoints
    };
    let reference = run();
    let repeated = run();
    for (tick, (expected, actual)) in reference.iter().zip(&repeated).enumerate() {
        let delta = expected.iter().zip(actual).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        assert_eq!(expected, actual, "replay checkpoint {tick}: max absolute difference {delta}");
    }
}

#[cfg(feature = "profile")]
#[test]
#[ignore = "requires GEARBOX_BENCH_ASSET and GEARBOX_TRACE; one simulated second of solver spans"]
fn imported_kubota_solver_trace() {
    use tracing_subscriber::prelude::*;

    let asset = std::env::var_os("GEARBOX_BENCH_ASSET").expect("set GEARBOX_BENCH_ASSET");
    let path = std::env::var_os("GEARBOX_TRACE").expect("set GEARBOX_TRACE");
    let mut fixture = Fixture::load(Path::new(&asset));
    for _ in 0..600 { fixture.tick(); }
    fixture.drive(2.0, 0.12);
    for _ in 0..600 { fixture.tick(); }
    fixture.verify(1.8, true);
    let (layer, flush) = tracing_chrome::ChromeLayerBuilder::new().file(path).include_args(true).build();
    let subscriber = tracing_subscriber::registry().with(layer.with_filter(
        tracing_subscriber::EnvFilter::new("off,molla_solvers=debug"),
    ));
    let dispatch = tracing::Dispatch::new(subscriber);
    {
        let _guard = tracing::dispatcher::set_default(&dispatch);
        for _ in 0..120 { fixture.tick(); }
    }
    drop(flush);
    fixture.verify(1.8, true);
}
