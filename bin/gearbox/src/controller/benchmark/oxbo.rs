use super::*;

#[path = "oxbo/pressure.rs"]
mod pressure;

#[test]
#[ignore = "requires GEARBOX_BENCH_OXBO; real six-wheel harvester settling"]
fn imported_oxbo_settling() {
    settling(false, false);
}

#[test]
#[ignore = "requires GEARBOX_BENCH_OXBO; viewer terrain and spawn alignment"]
fn imported_oxbo_viewer_spawn() {
    settling(true, false);
}

#[test]
#[ignore = "requires GEARBOX_BENCH_OXBO; equivalent quaternion teleport stability"]
fn imported_oxbo_antipodal_teleports() {
    settling(true, true);
}

fn settling(viewer: bool, antipodal: bool) {
    let asset = std::env::var_os("GEARBOX_BENCH_OXBO").expect("set GEARBOX_BENCH_OXBO");
    let mut fixture = Fixture::load(Path::new(&asset));
    if viewer {
        let meadow = crate::terrain::benchmark_meadow_collider();
        {
            let mut physics = fixture.app.world_mut().resource_mut::<PhysicsWorld>();
            let ground = physics.collider_mut(fixture.ground).unwrap();
            ground.set_shape(meadow.shape);
            ground.set_position(meadow.pose);
            ground.set_friction(meadow.friction.unwrap());
        }
        crate::load::benchmark_align_machine(
            &mut fixture.app,
            fixture.machine.scene_root.unwrap(),
        );
        if antipodal {
            let mut physics = fixture.app.world_mut().resource_mut::<PhysicsWorld>();
            for body in physics.bodies() {
                let body = physics.body_mut(body).unwrap();
                let mut pose = body.position();
                pose.rotation = -pose.rotation;
                body.set_position(pose, true);
            }
        }
    }
    let initial = fixture
        .app
        .world()
        .resource::<PhysicsWorld>()
        .body(fixture.chassis)
        .unwrap()
        .translation();
    for step in 0..1200 {
        fixture.tick();
        let physics = fixture.app.world().resource::<PhysicsWorld>();
        let body = physics.body(fixture.chassis).unwrap();
        if step % 30 == 0 {
            eprintln!(
                "Oxbo step={step} position={:?} velocity={:?} angular={:?}",
                body.translation(),
                body.linvel(),
                body.angvel()
            );
        }
        assert!(
            body.linvel().length() < 10.0 && body.angvel().length() < 10.0,
            "runaway Oxbo at step {step}: v={:?}, w={:?}",
            body.linvel(),
            body.angvel()
        );
        assert!(
            (body.translation() - initial).length() < 3.0,
            "Oxbo left spawn at step {step}: {:?}",
            body.translation()
        );
    }
    let physics = fixture.app.world().resource::<PhysicsWorld>();
    assert!(physics.body(fixture.chassis).unwrap().linvel().length() < 0.05);
    for &wheel in &fixture.wheels {
        let output = physics.wheel_output(wheel).expect("Oxbo tyre output");
        assert!(output.in_contact && output.normal_force > 100.0);
        assert!(output.pressure.is_some());
    }
}
