//! Imported-machine CPU benchmark without rendering, network agents or wall-clock stepping.

use super::*;
use crate::physics::{MollaBackend, PhysicsWorld};
use crate::physics::backend::{ColliderDesc, ColliderId, DQuat, Pose, Shape};
use std::time::{Duration, Instant};

struct Fixture {
    app: App,
    controllers: bevy::ecs::schedule::Schedule,
    chassis: BodyId,
    ground: ColliderId,
    wheels: Vec<BodyId>,
    machine: MachineInstanceSpec,
}

impl Fixture {
    fn load(path: &Path) -> Self {
        let source = usd_bevy::UsdSource::from_file(path).expect("read benchmark asset");
        let stage = source.open_stage().expect("open benchmark stage");
        let mut machines = discover_machines_from_stage(&stage).unwrap();
        assert_eq!(machines.len(), 1, "benchmark requires one machine");
        let mut machine = machines.remove(0);
        assert_eq!(path.file_name().unwrap(), "kubota_tractor.usdz", "this gate is the real Kubota fixture");
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
        app.insert_resource(PhysicsWorld::with_backend(Box::new(MollaBackend::default())));
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
        ).chain());
        controllers.run(app.world_mut());
        let runtime = app.world().resource::<ControllerRuntimeState>();
        let mut bodies = runtime.machine_bodies[&machine.id].clone();
        bodies.sort();
        let wheels = runtime.machine_wheels[&machine.id].clone();
        assert_eq!(bodies.len(), 26);
        assert_eq!(wheels.len(), 4);
        let chassis_entity = app.world_mut().query::<(Entity, &UsdPrimRef)>()
            .iter(app.world()).find(|(_, prim)| Some(&prim.path) == machine.body.as_ref()).unwrap().0;
        let mut physics = app.world_mut().resource_mut::<PhysicsWorld>();
        let chassis = physics.entity_to_body[&chassis_entity];
        let mass: f64 = bodies.iter().map(|id| physics.body(*id).unwrap().mass()).sum();
        assert!((mass - 4917.0).abs() < 1.0, "imported mass {mass}");
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
        eprintln!("imported fixture: bodies={} joints={} colliders={} tyres={} mass={mass} dt={}",
            physics.bodies().len(), physics.joints().len(), physics.colliders().len(), wheels.len(), physics.dt());
        assert_eq!(physics.joints().len(), 31);
        assert_eq!(physics.colliders().len(), 18);
        assert!((physics.dt() - 1.0 / 120.0).abs() < 1e-15, "benchmark requires 120 Hz");
        Self { app, controllers, chassis, ground, wheels, machine }
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
        self.controllers.run(self.app.world_mut());
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
