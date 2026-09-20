use super::*;
use crate::physics::backend::BodyDesc;

fn pressures(fixture: &Fixture) -> Vec<gearbox_api::tyres::TyrePressure> {
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

fn command(fixture: &mut Fixture, scope: &str, bar: f64) {
    wheel_forces::set_pressure_group(
        &fixture.machine,
        &mut fixture
            .app
            .world_mut()
            .resource_mut::<crate::services::LinkValues>(),
        scope,
        bar,
    )
    .unwrap();
}

fn compare(reference: &Fixture, actual: &Fixture, step: usize) {
    let expected = reference.snapshot();
    let actual_state = actual.snapshot();
    assert_eq!(
        actual_state.len(),
        expected.len(),
        "snapshot size at {step}"
    );
    for (index, (a, b)) in actual_state.iter().zip(&expected).enumerate() {
        assert_eq!(
            a.to_bits(),
            b.to_bits(),
            "physical state at step {step}, field {index}: {a} != {b}"
        );
    }
    assert_eq!(
        pressures(reference),
        pressures(actual),
        "pressure telemetry at {step}"
    );
}

fn insert_prop(fixture: &mut Fixture, dynamic: bool) -> BodyId {
    let mut physics = fixture.app.world_mut().resource_mut::<PhysicsWorld>();
    let desc = if dynamic {
        BodyDesc::dynamic()
    } else {
        BodyDesc::fixed()
    };
    let body =
        physics.insert_body(desc.pose(Pose::from_translation(DVec3::new(100.0, 4.0, 100.0))));
    physics
        .insert_collider(
            ColliderDesc::new(Shape::Ball { radius: 0.2 })
                .parent(body)
                .density(1000.0),
        )
        .unwrap();
    body
}

fn snapshot_bits(fixture: &Fixture) -> Vec<u64> {
    fixture.snapshot().into_iter().map(f64::to_bits).collect()
}

#[test]
#[ignore = "requires GEARBOX_BENCH_ASSET; imported pressure state through topology edits"]
fn imported_kubota_pressure_survives_unrelated_bodies() {
    let asset = std::env::var_os("GEARBOX_BENCH_ASSET").expect("set GEARBOX_BENCH_ASSET");
    let mut reference = Fixture::load(Path::new(&asset));
    let mut edited = Fixture::load(Path::new(&asset));
    let tyres = pressures(&reference);
    assert_eq!(tyres.len(), 4);
    for fixture in [&mut reference, &mut edited] {
        for (tyre, target) in tyres.iter().zip([0.9, 1.4, 2.3, 3.1]) {
            command(fixture, &format!("wheel:{}", tyre.link), target);
        }
    }
    let mut props = Vec::new();
    for step in 0..960 {
        if step == 60 || step == 300 || step == 660 {
            let before = snapshot_bits(&edited);
            props.push(insert_prop(&mut edited, step != 60));
            assert_eq!(
                snapshot_bits(&edited),
                before,
                "insert must preserve machine state"
            );
        }
        if step == 120 || step == 480 || step == 720 {
            let before = snapshot_bits(&edited);
            edited
                .app
                .world_mut()
                .resource_mut::<PhysicsWorld>()
                .remove_body(props.pop().unwrap());
            assert_eq!(
                snapshot_bits(&edited),
                before,
                "remove must preserve machine state"
            );
        }
        if step == 240 {
            reference.drive(1.2, 0.15);
            edited.drive(1.2, 0.15);
        }
        if step == 420 {
            for fixture in [&mut reference, &mut edited] {
                command(fixture, &format!("wheel:{}", tyres[0].link), 3.5);
                command(fixture, &format!("wheel:{}", tyres[3].link), 0.7);
            }
        }
        reference.tick();
        edited.tick();
        compare(&reference, &edited, step);
    }
    assert!(props.is_empty());
    let final_tyres = pressures(&edited);
    for (tyre, expected) in final_tyres.iter().zip([3.5, 1.4, 2.3, 0.7]) {
        assert!((tyre.applied - expected).abs() < 1e-9);
        assert_eq!(tyre.target, expected);
        eprintln!(
            "Kubota topology-preserved wheel={} applied_bar={} target_bar={}",
            tyre.link, tyre.applied, tyre.target
        );
    }
}
