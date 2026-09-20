use super::*;
use crate::physics::backend::{BodyDesc, Inertia, JointDesc, JointKind, MassProps};

#[derive(Clone, Copy, Debug, Default)]
struct LoadedWheel {
    force: f64,
    deflection: f64,
    radius: f64,
    area: f64,
}

fn settled(fixture: &mut Fixture, mass: f64) -> Vec<LoadedWheel> {
    for _ in 0..960 {
        fixture.tick();
    }
    let mut average = vec![LoadedWheel::default(); fixture.wheels.len()];
    for _ in 0..120 {
        fixture.tick();
        let physics = fixture.app.world().resource::<PhysicsWorld>();
        let chassis = physics.body(fixture.chassis).unwrap();
        assert!(chassis.linvel().length() < 0.05);
        assert!(chassis.angvel().length() < 0.05);
        for (&wheel, mean) in fixture.wheels.iter().zip(&mut average) {
            let output = physics.wheel_output(wheel).unwrap();
            let tyre = output.pressure.unwrap();
            assert!(output.in_contact && output.normal_force > 100.0);
            assert_eq!(tyre.pressure_pa, 180_000.0);
            assert_eq!(tyre.target_pressure_pa, 180_000.0);
            assert!(tyre.deflection > 0.0 && tyre.loaded_radius > 0.0);
            assert!(tyre.patch_area > 0.0 && tyre.patch_area.is_finite());
            assert!((tyre.loaded_radius + tyre.deflection - tyre.radius).abs() < 1e-12);
            mean.force += output.normal_force / 120.0;
            mean.deflection += tyre.deflection / 120.0;
            mean.radius += tyre.loaded_radius / 120.0;
            mean.area += tyre.patch_area / 120.0;
        }
    }
    let physics = fixture.app.world().resource::<PhysicsWorld>();
    let weight = -physics.gravity().y * mass;
    let support: f64 = average.iter().map(|wheel| wheel.force).sum();
    assert!(
        (support - weight).abs() < weight * 0.01,
        "support {support}, weight {weight}"
    );
    eprintln!("Kubota load sweep mass_kg={mass} support_N={support} wheels={average:?}");
    average
}

#[test]
#[ignore = "requires GEARBOX_BENCH_ASSET; imported fixed-pressure payload response"]
fn imported_kubota_payload_changes_each_tyres_load_and_footprint() {
    let asset = std::env::var_os("GEARBOX_BENCH_ASSET").expect("set GEARBOX_BENCH_ASSET");
    let mut fixture = Fixture::load(Path::new(&asset));
    fixture.pressure(1.8);
    let initial_mass = {
        let physics = fixture.app.world().resource::<PhysicsWorld>();
        fixture.fleet[0]
            .bodies
            .iter()
            .map(|&id| physics.body(id).unwrap().mass())
            .sum::<f64>()
    };
    let baseline = settled(&mut fixture, initial_mass);
    let payload_mass = 1500.0;
    let (payload, forward_indices) = {
        let mut physics = fixture.app.world_mut().resource_mut::<PhysicsWorld>();
        let tyres: Vec<_> = fixture
            .wheels
            .iter()
            .map(|&wheel| physics.wheel_output(wheel).unwrap().pressure.unwrap())
            .collect();
        let heading = (tyres[0].forward * DVec3::new(1.0, 0.0, 1.0)).normalize();
        let mut order: Vec<_> = (0..tyres.len()).collect();
        order.sort_by(|&a, &b| {
            tyres[a]
                .hub
                .dot(heading)
                .total_cmp(&tyres[b].hub.dot(heading))
        });
        let rear = (tyres[order[0]].hub + tyres[order[1]].hub) * 0.5;
        let front = (tyres[order[2]].hub + tyres[order[3]].hub) * 0.5;
        assert!((front - rear).dot(heading) > 1.0);
        let chassis = physics.body(fixture.chassis).unwrap();
        let parent_pose = chassis.position();
        let mut location = rear.lerp(front, 0.75);
        location.y = chassis.center_of_mass().y + 0.5;
        let local = parent_pose.rotation.conjugate() * (location - parent_pose.translation);
        let mut desc = BodyDesc::dynamic().pose(Pose::new(location, parent_pose.rotation));
        desc.additional_mass = Some(MassProps {
            mass: payload_mass,
            local_com: DVec3::ZERO,
            inertia: Inertia::Principal(DVec3::splat(payload_mass / 6.0)),
        });
        let payload = physics.insert_body(desc);
        physics.insert_joint(
            fixture.chassis,
            payload,
            JointDesc::new(
                JointKind::Fixed,
                Pose::from_translation(local),
                Pose::IDENTITY,
            ),
        );
        (payload, [order[2], order[3]])
    };
    let loaded = settled(&mut fixture, initial_mass + payload_mass);
    let mut increase = Vec::new();
    for (before, after) in baseline.iter().zip(&loaded) {
        increase.push(after.force - before.force);
        assert!(after.force > before.force + 100.0);
        assert!(after.deflection > before.deflection + 0.0001);
        assert!(after.radius < before.radius - 0.0001);
    }
    let total: f64 = increase.iter().sum();
    let forward = forward_indices.iter().map(|&i| increase[i]).sum::<f64>() / total;
    assert!(
        (0.60..0.90).contains(&forward),
        "forward axle payload share {forward}"
    );
    eprintln!("Kubota payload per-wheel increase_N={increase:?} forward_share={forward}");
    fixture
        .app
        .world_mut()
        .resource_mut::<PhysicsWorld>()
        .remove_body(payload);
    let returned = settled(&mut fixture, initial_mass);
    for (before, after) in baseline.iter().zip(&returned) {
        assert!((after.force - before.force).abs() < before.force * 0.01);
        assert!((after.deflection - before.deflection).abs() < 0.001);
        assert!((after.radius - before.radius).abs() < 0.001);
        assert!((after.area - before.area).abs() < before.area * 0.02);
    }
    for (before, after) in baseline.iter().zip(&loaded) {
        assert!(
            after.area > before.area,
            "loaded footprint must grow: {before:?} -> {after:?}"
        );
    }
}
