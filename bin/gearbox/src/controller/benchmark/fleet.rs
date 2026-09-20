use super::*;

impl Fixture {
    fn load_fleet(path: &Path, molla: bool, count: usize) -> Self {
        assert!((1..=8).contains(&count));
        assert_eq!(path.file_name().unwrap(), "kubota_tractor.usdz");
        let path = path.canonicalize().unwrap();
        let asset = path.to_str().unwrap();
        assert!(!asset.contains(['@', '\n', '\r']));
        let mut text = String::from(
            "#usda 1.0\n(defaultPrim = \"World\"\n upAxis = \"Z\"\n metersPerUnit = 1)\ndef Xform \"World\" {\n",
        );
        for index in 0..count {
            let x = index * 25;
            text.push_str(&format!(
                r#"
def Xform "Slot{index}" {{
    double3 xformOp:translate = ({x}, 0, 0)
    uniform token[] xformOpOrder = ["xformOp:translate"]
    def Xform "Tractor" (prepend references = @{asset}@</robot>) {{
        token gearbox:machine:id = "fleet_{index}"
    }}
}}
"#
            ));
        }
        text.push_str("}\n");
        let source = usd_bevy::UsdSource::new(
            std::env::temp_dir().join("gearbox-fleet-benchmark.usda"),
            text.into_bytes(),
        )
        .unwrap();
        let stage = source
            .open_stage()
            .expect("compose referenced Kubota fleet");
        Self::from_stage(&stage, molla, true, count)
    }

    fn drive_fleet(&mut self, active: usize) {
        let mut commands = self.app.world_mut().resource_mut::<ControllerCommands>();
        for (index, machine) in self.fleet.iter().enumerate() {
            commands.cmd_vel.insert(
                ControllerKey::new(
                    machine.machine.scene_root.unwrap(),
                    &machine.machine.id,
                    "drive",
                ),
                CmdVel {
                    linear_mps: if index < active { 2.0 } else { 0.0 },
                    angular_rps: 0.0,
                },
            );
        }
    }

    fn verify_fleet(&self, active: usize, require_sleep: bool, verify_speed: bool) {
        let physics = self.app.world().resource::<PhysicsWorld>();
        for (index, machine) in self.fleet.iter().enumerate() {
            let body = physics.body(machine.chassis).unwrap();
            if index < active {
                if verify_speed || require_sleep {
                    let tolerance = if require_sleep { 0.15 } else { 0.2 };
                    assert!(
                        (body.linvel().length() - 2.0).abs() < tolerance,
                        "{} machine {index} driving speed {} position {:?}",
                        physics.name(),
                        body.linvel().length(),
                        body.translation()
                    );
                }
            } else {
                assert!(body.linvel().length() < 0.05);
                if require_sleep {
                    assert!(
                        machine
                            .bodies
                            .iter()
                            .all(|&body| physics.body(body).unwrap().is_sleeping()),
                        "parked machine {index} woke unexpectedly"
                    );
                }
            }
        }
    }
}

#[test]
#[ignore = "requires GEARBOX_BENCH_ASSET; real tractors sharing one physics world"]
fn imported_kubota_fleet() {
    let asset = std::env::var_os("GEARBOX_BENCH_ASSET").expect("set GEARBOX_BENCH_ASSET");
    for count in [1, 2, 4] {
        for molla in [false, true] {
            let mut fixture = Fixture::load_fleet(Path::new(&asset), molla, count);
            fixture.drive_fleet(0);
            for _ in 0..7200 {
                fixture.tick();
            }
            let phases = if count == 1 {
                vec![0, 1]
            } else {
                vec![0, 1, count]
            };
            for active in phases {
                fixture.drive_fleet(active);
                for _ in 0..600 {
                    fixture.tick();
                }
                let mut samples = Vec::new();
                let mut speed_ranges = vec![(f64::INFINITY, 0.0_f64); count];
                for _ in 0..1200 {
                    samples.push(fixture.tick().as_secs_f64() * 1000.0);
                    fixture.verify_fleet(active, molla, false);
                    let physics = fixture.app.world().resource::<PhysicsWorld>();
                    for (machine, range) in fixture.fleet.iter().zip(&mut speed_ranges) {
                        let speed = physics.body(machine.chassis).unwrap().linvel().length();
                        range.0 = range.0.min(speed);
                        range.1 = range.1.max(speed);
                    }
                }
                fixture.verify_fleet(active, molla, true);
                samples.sort_by(f64::total_cmp);
                let physics = fixture.app.world().resource::<PhysicsWorld>();
                let sleeping = physics
                    .bodies()
                    .iter()
                    .filter(|&&id| physics.body(id).unwrap().is_sleeping())
                    .count();
                let reference_excursion = !molla
                    && speed_ranges[..active]
                        .iter()
                        .any(|&(min, max)| min < 1.8 || max > 2.2);
                eprintln!(
                    "fleet timing backend={} machines={count} active={active} sleeping={sleeping}/{} median_ms={:.6} p95_ms={:.6} p99_ms={:.6} speed_ranges={speed_ranges:?} reference_speed_excursion={reference_excursion} renderer=none controllers=enabled outside-timing",
                    physics.name(),
                    physics.bodies().len(),
                    samples[600],
                    samples[1140],
                    samples[1188]
                );
                if molla {
                    for (index, machine) in fixture.fleet.iter().enumerate() {
                        fixture.verify_machine(
                            machine.chassis,
                            &machine.wheels,
                            1.8,
                            index < active,
                        );
                    }
                }
            }
        }
    }
}

#[cfg(feature = "profile")]
#[test]
#[ignore = "requires GEARBOX_BENCH_ASSET and GEARBOX_TRACE; four active tractors"]
fn imported_kubota_fleet_trace() {
    use tracing_subscriber::prelude::*;

    let asset = std::env::var_os("GEARBOX_BENCH_ASSET").expect("set GEARBOX_BENCH_ASSET");
    let path = std::env::var_os("GEARBOX_TRACE").expect("set GEARBOX_TRACE");
    let mut fixture = Fixture::load_fleet(Path::new(&asset), true, 4);
    for _ in 0..600 {
        fixture.tick();
    }
    fixture.drive_fleet(4);
    for _ in 0..600 {
        fixture.tick();
    }
    fixture.verify_fleet(4, true, true);
    let (layer, flush) = tracing_chrome::ChromeLayerBuilder::new()
        .file(path)
        .include_args(true)
        .build();
    let subscriber = tracing_subscriber::registry().with(layer.with_filter(
        tracing_subscriber::EnvFilter::new("off,molla_solvers=debug"),
    ));
    let dispatch = tracing::Dispatch::new(subscriber);
    {
        let _guard = tracing::dispatcher::set_default(&dispatch);
        for _ in 0..120 {
            fixture.tick();
        }
    }
    drop(flush);
    fixture.verify_fleet(4, true, true);
}
