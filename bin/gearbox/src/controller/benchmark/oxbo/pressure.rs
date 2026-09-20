use super::*;

fn read(fixture: &Fixture) -> Vec<gearbox_api::tyres::TyrePressure> {
    let mut tyres = wheel_forces::pressure_tyres(
        &fixture.machine,
        fixture
            .app
            .world()
            .resource::<crate::services::LinkValues>(),
    )
    .unwrap();
    tyres.sort_by(|a, b| a.link.cmp(&b.link));
    tyres
}

fn set(fixture: &mut Fixture, scope: &str, bar: f64) -> Result<(), String> {
    wheel_forces::set_pressure_group(
        &fixture.machine,
        &mut fixture
            .app
            .world_mut()
            .resource_mut::<crate::services::LinkValues>(),
        scope,
        bar,
    )
}

fn bodies(fixture: &mut Fixture, tyres: &[gearbox_api::tyres::TyrePressure]) -> Vec<BodyId> {
    let mut query = fixture.app.world_mut().query::<(Entity, &UsdPrimRef)>();
    let physics = fixture.app.world().resource::<PhysicsWorld>();
    tyres
        .iter()
        .map(|tyre| {
            let link = fixture
                .machine
                .links
                .links
                .iter()
                .find(|link| link.name == tyre.link)
                .unwrap();
            let path = link.body_prim.as_ref().unwrap();
            let entity = query
                .iter(fixture.app.world())
                .find(|(_, prim)| &prim.path == path)
                .unwrap()
                .0;
            let body = physics.entity_to_body[&entity];
            assert!(fixture.wheels.contains(&body));
            body
        })
        .collect()
}

fn check(fixture: &Fixture, bodies: &[BodyId], expected: &[f64], settled: bool) {
    let tyres = read(fixture);
    let physics = fixture.app.world().resource::<PhysicsWorld>();
    assert_eq!(tyres.len(), 6);
    let chassis = physics.body(fixture.chassis).unwrap();
    assert!(chassis.linvel().length() < 10.0 && chassis.angvel().length() < 10.0);
    for ((tyre, &body), &target) in tyres.iter().zip(bodies).zip(expected) {
        let output = physics.wheel_output(body).unwrap();
        let pressure = output.pressure.unwrap();
        assert!(
            (tyre.target - target).abs() < 1e-12,
            "{} telemetry target",
            tyre.link
        );
        assert!(
            (pressure.target_pressure_pa / 1e5 - target).abs() < 1e-12,
            "{} physical target",
            tyre.link
        );
        assert!(pressure.loaded_radius.is_finite() && pressure.loaded_radius > 0.0);
        assert!(pressure.deflection.is_finite() && pressure.deflection >= 0.0);
        assert!(output.normal_force.is_finite() && output.normal_force >= 0.0);
        if settled {
            assert!(
                (tyre.applied - target).abs() < 1e-9,
                "{} telemetry pressure",
                tyre.link
            );
            assert!(
                (pressure.pressure_pa / 1e5 - target).abs() < 1e-9,
                "{} physical pressure",
                tyre.link
            );
        }
    }
}

#[test]
#[ignore = "requires GEARBOX_BENCH_OXBO; independent six-wheel pressure lifecycle"]
fn imported_oxbo_independent_wheel_pressures() {
    let asset = std::env::var_os("GEARBOX_BENCH_OXBO").expect("set GEARBOX_BENCH_OXBO");
    let mut fixture = Fixture::load(Path::new(&asset));
    for _ in 0..360 {
        fixture.tick();
    }
    let initial = read(&fixture);
    assert_eq!(initial.len(), 6);
    let bodies = bodies(&mut fixture, &initial);
    assert_eq!(
        bodies
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        6
    );
    let mut expected = [0.8, 1.2, 1.6, 2.0, 2.4, 2.8];
    let pose = fixture
        .app
        .world()
        .resource::<PhysicsWorld>()
        .body(fixture.chassis)
        .unwrap()
        .position();
    fixture
        .app
        .world_mut()
        .resource_mut::<gearbox_api::PhysicsActive>()
        .0 = false;
    for (tyre, &bar) in initial.iter().zip(&expected) {
        set(&mut fixture, &format!("wheel:{}", tyre.link), bar).unwrap();
    }
    fixture.controllers.run(fixture.app.world_mut());
    check(&fixture, &bodies, &expected, false);
    for (before, after) in initial.iter().zip(read(&fixture)) {
        assert_eq!(before.applied.to_bits(), after.applied.to_bits());
    }
    assert_eq!(
        fixture
            .app
            .world()
            .resource::<PhysicsWorld>()
            .body(fixture.chassis)
            .unwrap()
            .position(),
        pose
    );

    let before_invalid = read(&fixture);
    for (scope, bar) in [
        ("all".to_string(), f64::NAN),
        ("all".to_string(), 100.0),
        (format!("wheel:{}", initial[0].link), -1.0),
        ("wheel:missing".to_string(), 1.0),
    ] {
        assert!(set(&mut fixture, &scope, bar).is_err());
        assert_eq!(read(&fixture), before_invalid);
    }

    fixture
        .app
        .world_mut()
        .resource_mut::<gearbox_api::PhysicsActive>()
        .0 = true;
    for _ in 0..600 {
        fixture.tick();
        check(&fixture, &bodies, &expected, false);
    }
    fixture.controllers.run(fixture.app.world_mut());
    check(&fixture, &bodies, &expected, true);
    for (tyre, &body) in read(&fixture).iter().zip(&bodies) {
        let pressure = fixture
            .app
            .world()
            .resource::<PhysicsWorld>()
            .wheel_output(body)
            .unwrap()
            .pressure
            .unwrap();
        eprintln!(
            "Oxbo independent wheel={} applied_bar={} target_bar={} deflection_m={} loaded_radius_m={}",
            tyre.link, tyre.applied, tyre.target, pressure.deflection, pressure.loaded_radius
        );
    }

    expected[0] = 3.2;
    set(
        &mut fixture,
        &format!("wheel:{}", initial[0].link),
        expected[0],
    )
    .unwrap();
    fixture.drive(1.0, 0.1);
    for _ in 0..600 {
        fixture.tick();
        check(&fixture, &bodies, &expected, false);
        for (tyre, &target) in read(&fixture).iter().zip(&expected).skip(1) {
            assert!(
                (tyre.applied - target).abs() < 1e-9,
                "unselected wheel {} changed pressure",
                tyre.link
            );
        }
    }
    fixture.controllers.run(fixture.app.world_mut());
    check(&fixture, &bodies, &expected, true);
}
