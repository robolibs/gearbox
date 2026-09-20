use super::*;

#[cfg(feature = "profile")]
#[test]
#[ignore = "requires GEARBOX_BENCH_ASSET and GEARBOX_TRACE; Molla meadow trace"]
fn imported_kubota_meadow_trace() {
    use tracing_subscriber::prelude::*;

    let asset = std::env::var_os("GEARBOX_BENCH_ASSET").expect("set GEARBOX_BENCH_ASSET");
    let path = std::env::var_os("GEARBOX_TRACE").expect("set GEARBOX_TRACE");
    let mut fixture = Fixture::load_backend(Path::new(&asset), true);
    let meadow = crate::terrain::benchmark_meadow_collider();
    {
        let mut physics = fixture.app.world_mut().resource_mut::<PhysicsWorld>();
        let ground = physics.collider_mut(fixture.ground).unwrap();
        ground.set_shape(meadow.shape);
        ground.set_position(meadow.pose);
        ground.set_friction(meadow.friction.unwrap());
    }
    for _ in 0..600 {
        fixture.tick();
    }
    fixture.drive(2.0, 0.12);
    for _ in 0..600 {
        fixture.tick();
    }
    fixture.verify(1.8, true);
    let (layer, flush) = tracing_chrome::ChromeLayerBuilder::new()
        .file(path)
        .include_args(true)
        .build();
    let subscriber = tracing_subscriber::registry().with(layer.with_filter(
        tracing_subscriber::EnvFilter::new("off,molla_solvers=debug,molla_geometry=debug"),
    ));
    let dispatch = tracing::Dispatch::new(subscriber);
    {
        let _guard = tracing::dispatcher::set_default(&dispatch);
        for _ in 0..120 {
            fixture.tick();
        }
    }
    drop(flush);
    fixture.verify(1.8, true);
}

#[test]
#[ignore = "requires GEARBOX_BENCH_ASSET; full-resolution viewer terrain timing"]
fn imported_kubota_meadow_timing() {
    meadow_timing(false);
}

#[test]
#[ignore = "requires GEARBOX_BENCH_ASSET; full-resolution field-friction timing"]
fn imported_kubota_meadow_field_friction_timing() {
    meadow_timing(true);
}

fn meadow_timing(field_friction: bool) {
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
    let field_grid = if field_friction {
        let grid = gearbox_fields::HeightGrid::sample(800.0, 1.0, |_, _| 0.0);
        let mut layout = gearbox_fields::FieldLayout::default();
        layout.default_friction = meadow.friction;
        Some(crate::physics::backend::TerrainFrictionGrid {
            origin: [grid.min_x as f64, grid.min_z as f64],
            cell_size: [grid.cell as f64; 2], cols: grid.cols, rows: grid.rows,
            values: layout.friction_samples(&grid, meadow.friction.unwrap()).unwrap().unwrap(),
        })
    } else { None };
    for (run, molla) in [false, true, true, false].into_iter().enumerate() {
        let mut fixture = Fixture::load_backend(Path::new(&asset), molla);
        {
            let mut physics = fixture.app.world_mut().resource_mut::<PhysicsWorld>();
            let ground = physics.collider_mut(fixture.ground).unwrap();
            ground.set_shape(meadow.shape.clone());
            ground.set_position(meadow.pose);
            ground.set_friction(meadow.friction.unwrap());
            if molla && let Some(grid) = &field_grid {
                physics.register_wheel_ground(fixture.ground, Some(grid.clone())).unwrap();
            }
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
                "meadow timing run={run} backend={} driving={driving} speed={speed:.9} median={:.6} p95={:.6} p99={:.6} max={:.6} ms/step; samples=1200 hz=120 terrain=801x801 controller=outside-timing position={position:?} field_friction={}",
                physics.name(),
                samples[600],
                samples[1140],
                samples[1188],
                samples[1199],
                molla && field_friction,
            );
            if molla {
                fixture.verify(1.8, driving);
            }
        }
    }
}
