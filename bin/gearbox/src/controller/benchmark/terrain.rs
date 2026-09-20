use super::*;

#[test]
#[ignore = "requires GEARBOX_BENCH_ASSET; full-resolution viewer terrain timing"]
fn imported_kubota_meadow_timing() {
    let asset = std::env::var_os("GEARBOX_BENCH_ASSET").expect("set GEARBOX_BENCH_ASSET");
    let meadow = crate::terrain::benchmark_meadow_collider();
    let Shape::Heightfield {
        rows,
        cols,
        heights,
        scale,
    } = &meadow.shape
    else {
        panic!("viewer terrain must be a heightfield");
    };
    assert_eq!((*rows, *cols, heights.len()), (801, 801, 801 * 801));
    assert_eq!(*scale, DVec3::new(800.0, 1.0, 800.0));
    for (run, molla) in [false, true, true, false].into_iter().enumerate() {
        let mut fixture = Fixture::load_backend(Path::new(&asset), molla);
        {
            let mut physics = fixture.app.world_mut().resource_mut::<PhysicsWorld>();
            let ground = physics.collider_mut(fixture.ground).unwrap();
            ground.set_shape(meadow.shape.clone());
            ground.set_position(meadow.pose);
            ground.set_friction(meadow.friction.unwrap());
        }
        for driving in [false, true] {
            fixture.drive(
                if driving { 2.0 } else { 0.0 },
                if driving { 0.12 } else { 0.0 },
            );
            for _ in 0..600 {
                fixture.tick();
            }
            let mut samples: Vec<_> = (0..1200)
                .map(|_| fixture.tick().as_secs_f64() * 1000.0)
                .collect();
            samples.sort_by(f64::total_cmp);
            let physics = fixture.app.world().resource::<PhysicsWorld>();
            let body = physics.body(fixture.chassis).unwrap();
            let speed = body.linvel().length();
            assert!(
                if driving {
                    (speed - 2.0).abs() < 0.2
                } else {
                    speed < 0.05
                },
                "{} driving={driving} speed={speed}",
                physics.name()
            );
            let position = body.position().translation;
            assert!(
                DVec3::new(position.x, 0.0, position.z).length() < 28.0,
                "fixture left the meadow's flat spawn region: {position:?}"
            );
            eprintln!(
                "meadow timing run={run} backend={} driving={driving} speed={speed:.9} median={:.6} p95={:.6} p99={:.6} max={:.6} ms/step; samples=1200 hz=120 terrain=801x801 controller=outside-timing position={position:?}",
                physics.name(),
                samples[600],
                samples[1140],
                samples[1188],
                samples[1199]
            );
            if molla {
                fixture.verify(1.8, driving);
            }
        }
    }
}
