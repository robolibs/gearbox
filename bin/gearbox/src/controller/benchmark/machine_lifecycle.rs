use super::*;

fn first_snapshot(fixture: &Fixture) -> String {
    let physics = fixture.app.world().resource::<PhysicsWorld>();
    let values: Vec<_> = fixture.fleet[0]
        .bodies
        .iter()
        .map(|&id| {
            let body = physics.body(id).unwrap();
            (
                body.position(),
                body.linvel(),
                body.angvel(),
                physics.wheel_output(id),
            )
        })
        .collect();
    format!("{values:?}")
}

fn first_values(fixture: &Fixture) -> Vec<f64> {
    let physics = fixture.app.world().resource::<PhysicsWorld>();
    let mut values = Vec::new();
    for &id in &fixture.fleet[0].bodies {
        let body = physics.body(id).unwrap();
        values.extend(body.translation().to_array());
        values.extend(body.rotation().to_array());
        values.extend(body.linvel().to_array());
        values.extend(body.angvel().to_array());
        if let Some(out) = physics.wheel_output(id) {
            let p = out.pressure.unwrap();
            values.extend([
                f64::from(out.in_contact),
                f64::from(out.held_support),
                out.normal_force,
                out.grip_force,
                out.slip_ratio,
                out.slip_angle,
                p.pressure_pa,
                p.target_pressure_pa,
                p.loaded_radius,
                p.deflection,
                p.patch_length,
                p.patch_width,
                p.patch_area,
            ]);
            for vector in [
                out.normal,
                out.contact_point,
                out.force,
                out.aligning_moment,
                p.hub,
                p.forward,
                p.rolling_moment,
            ] {
                values.extend(vector.to_array());
            }
        }
    }
    values
}

fn compare_layouts(reference: &Fixture, actual: &Fixture, tick: usize) -> f64 {
    let expected = first_values(reference);
    let actual = first_values(actual);
    assert_eq!(actual.len(), expected.len());
    let mut peak = 0.0_f64;
    for (field, (a, b)) in actual.into_iter().zip(expected).enumerate() {
        let error = (a - b).abs() / b.abs().max(1.0);
        assert!(
            error.is_finite() && error <= 1e-9,
            "cross-layout field {field} tick {tick}: {a} != {b}; normalized error {error}"
        );
        peak = peak.max(error);
    }
    peak
}

fn spawn_second(fixture: &mut Fixture, path: &Path) -> (Entity, String) {
    let path = path.canonicalize().unwrap();
    let asset = path.to_str().unwrap();
    assert!(!asset.contains(['@', '\n', '\r']));
    let text = format!(
        r#"#usda 1.0
(defaultPrim = "World"
 upAxis = "Z"
 metersPerUnit = 1)
def Xform "World" {{
    def Xform "Second" {{
        double3 xformOp:translate = (25, 0, 0)
        uniform token[] xformOpOrder = ["xformOp:translate"]
        def Xform "Tractor" (prepend references = @{asset}@</robot>) {{
            token gearbox:machine:id = "lifecycle_second"
        }}
    }}
}}
"#
    );
    let source = usd_bevy::UsdSource::new(
        std::env::temp_dir().join("gearbox-live-lifecycle.usda"),
        text.into_bytes(),
    )
    .unwrap();
    let stage = source.open_stage().unwrap();
    let mut machines = discover_machines_from_stage(&stage).unwrap();
    assert_eq!(machines.len(), 1);
    let before = first_snapshot(fixture);
    let clock = *fixture.app.world().resource::<Time>();
    let materials: Vec<_> = {
        let physics = fixture.app.world().resource::<PhysicsWorld>();
        fixture.fleet[0]
            .bodies
            .iter()
            .flat_map(|id| physics.body(*id).unwrap().colliders())
            .map(|id| {
                let c = physics.collider(id).unwrap();
                (id, c.friction(), c.restitution())
            })
            .collect()
    };
    let root = crate::physics::benchmark::project(&mut fixture.app, &stage);
    fixture.app.insert_resource(clock);
    for (id, friction, restitution) in materials {
        let physics = fixture.app.world().resource::<PhysicsWorld>();
        let c = physics.collider(id).unwrap();
        assert_eq!(
            (friction, restitution),
            (c.friction(), c.restitution()),
            "spawn changed existing material {id:?}"
        );
    }
    assert_eq!(
        first_snapshot(fixture),
        before,
        "USD projection changed first machine"
    );
    let mut machine = machines.remove(0);
    machine.scene_root = Some(root);
    let name = machine.id.clone();
    fixture
        .app
        .world_mut()
        .resource_mut::<MachineAgentKeys>()
        .0
        .insert(
            "lifecycle_second".into(),
            ControllerKey::new(root, &name, "drive"),
        );
    for link in &machine.links.links {
        for (key, value) in &link.values {
            fixture
                .app
                .world_mut()
                .resource_mut::<crate::services::LinkValues>()
                .set(&name, &link.name, key, *value);
        }
    }
    fixture
        .app
        .world_mut()
        .resource_mut::<ControllerInventory>()
        .machines
        .push(machine.clone());
    fixture.controllers.run(fixture.app.world_mut());
    let runtime = fixture.app.world().resource::<ControllerRuntimeState>();
    let bodies = runtime.machine_bodies[&name].clone();
    let wheels = runtime.machine_wheels[&name].clone();
    assert_eq!(bodies.len(), 26);
    assert_eq!(wheels.len(), 4);
    let before_alignment = first_snapshot(fixture);
    let mut physics = fixture.app.world_mut().resource_mut::<PhysicsWorld>();
    let clearance = wheels
        .iter()
        .flat_map(|id| physics.body(*id).unwrap().colliders())
        .map(|id| physics.collider(id).unwrap().aabb().mins.y)
        .fold(f64::INFINITY, f64::min);
    let aligned: Vec<_> = bodies
        .iter()
        .map(|&id| {
            let mut pose = physics.body(id).unwrap().position();
            pose.translation.y += 0.03 - clearance;
            (id, pose)
        })
        .collect();
    physics.set_body_poses(&aligned, true).unwrap();
    drop(physics);
    assert_eq!(
        first_snapshot(fixture),
        before_alignment,
        "spawn alignment changed first machine"
    );
    wheel_forces::set_pressure_group(
        &machine,
        &mut fixture
            .app
            .world_mut()
            .resource_mut::<crate::services::LinkValues>(),
        "all",
        0.8,
    )
    .unwrap();
    (root, name)
}

#[test]
#[ignore = "requires GEARBOX_BENCH_ASSET; live imported spawn/despawn during pressure transition"]
fn imported_kubota_live_machine_lifecycle_preserves_first_machine() {
    let asset = std::env::var_os("GEARBOX_BENCH_ASSET").expect("set GEARBOX_BENCH_ASSET");
    let mut reference = Fixture::load(Path::new(&asset));
    let mut actual = Fixture::load(Path::new(&asset));
    let mut replay = Fixture::load(Path::new(&asset));
    for _ in 0..600 {
        reference.tick();
        actual.tick();
        replay.tick();
    }
    for fixture in [&mut reference, &mut actual, &mut replay] {
        fixture.pressure(3.2);
        fixture.drive(1.2, 0.15);
    }
    let mut second = [None, None];
    let mut peak_error = 0.0_f64;
    for tick in 0..480 {
        if tick == 30 {
            second[0] = Some(spawn_second(&mut actual, Path::new(&asset)));
            second[1] = Some(spawn_second(&mut replay, Path::new(&asset)));
            reference.controllers.run(reference.app.world_mut());
        }
        if tick == 240 {
            for (actual, second) in [&mut actual, &mut replay].into_iter().zip(&mut second) {
                let (root, name) = second.take().unwrap();
                let before = first_snapshot(actual);
                actual
                    .app
                    .world_mut()
                    .resource_mut::<MachineAgentKeys>()
                    .0
                    .remove("lifecycle_second");
                actual
                    .app
                    .world_mut()
                    .resource_mut::<ControllerInventory>()
                    .machines
                    .retain(|m| m.id != name);
                crate::physics::benchmark::despawn(&mut actual.app, root);
                assert_eq!(
                    first_snapshot(actual),
                    before,
                    "despawn changed first machine"
                );
            }
        }
        peak_error = peak_error.max(compare_layouts(&reference, &actual, tick));
        reference.tick();
        actual.tick();
        replay.tick();
        if (31..240).contains(&tick) {
            let runtime = actual.app.world().resource::<ControllerRuntimeState>();
            let physics = actual.app.world().resource::<PhysicsWorld>();
            for &wheel in &runtime.machine_wheels["lifecycle_second"] {
                let pressure = physics.wheel_output(wheel).unwrap().pressure.unwrap();
                assert_eq!(pressure.target_pressure_pa, 80_000.0);
                assert!(pressure.pressure_pa < 180_000.0);
                if tick == 239 {
                    assert_eq!(pressure.pressure_pa, 80_000.0);
                }
            }
        }
        peak_error = peak_error.max(compare_layouts(&reference, &actual, tick));
        let values = actual.snapshot();
        let expected = replay.snapshot();
        assert_eq!(values.len(), expected.len());
        for (field, (a, b)) in values.into_iter().zip(expected).enumerate() {
            assert_eq!(
                a.to_bits(),
                b.to_bits(),
                "replay tick {tick}, field {field}: {a} != {b}"
            );
        }
    }
    eprintln!(
        "live-machine lifecycle maximum normalized cross-layout error={peak_error:e}; replay=bitwise"
    );
    assert!(second.iter().all(Option::is_none));
    assert_eq!(
        actual.app.world().resource::<PhysicsWorld>().bodies().len(),
        26
    );
    let physics = actual.app.world().resource::<PhysicsWorld>();
    assert_eq!(physics.joints().len(), 31);
    assert_eq!(physics.colliders().len(), 18);
    assert!(
        physics
            .entity_to_body
            .keys()
            .all(|&e| actual.app.world().get_entity(e).is_ok())
    );
    assert!(
        physics
            .entity_to_collider
            .keys()
            .all(|&e| actual.app.world().get_entity(e).is_ok())
    );
}
