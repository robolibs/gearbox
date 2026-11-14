use super::MollaBackend;
use super::backend::*;

fn fixture(grounded: bool) -> (MollaBackend, BodyId, Vec<BodyId>) {
    let mut physics = MollaBackend::default();
    physics.set_gravity(if grounded {
        -DVec3::Y * 9.81
    } else {
        DVec3::ZERO
    });
    if grounded {
        let ground =
            physics.insert_body(BodyDesc::fixed().pose(Pose::from_translation(-DVec3::Y * 0.1)));
        physics
            .insert_collider(
                ColliderDesc::new(Shape::Cuboid {
                    half_extents: DVec3::new(10.0, 0.1, 10.0),
                })
                .parent(ground)
                .friction(0.85),
            )
            .unwrap();
    }
    let mut desc = BodyDesc::dynamic().pose(Pose::from_translation(DVec3::Y * 0.20));
    desc.additional_mass = Some(MassProps {
        local_com: DVec3::ZERO,
        mass: 734.0,
        inertia: Inertia::Principal(DVec3::splat(120.0)),
    });
    let chassis = physics.insert_body(desc);
    let mut rotors = Vec::new();
    for side in [-1.0, 1.0] {
        let mut collider = ColliderDesc::new(Shape::Cuboid {
            half_extents: DVec3::new(0.09, 0.06, 0.54),
        })
        .parent(chassis)
        .density(0.0)
        .friction(0.0);
        collider.pose = Pose::from_translation(DVec3::new(side * 0.41, -0.08, 0.0));
        let contact = physics.insert_collider(collider).unwrap();
        let offset = DVec3::new(side * 0.41, 0.28, -0.40);
        let mut rotor = BodyDesc::dynamic().pose(Pose::from_translation(DVec3::Y * 0.20 + offset));
        rotor.additional_mass = Some(MassProps {
            local_com: DVec3::ZERO,
            mass: 8.0,
            inertia: Inertia::Principal(DVec3::splat(0.3)),
        });
        let sprocket = physics.insert_body(rotor);
        let joint = physics.insert_joint(
            chassis,
            sprocket,
            JointDesc::new(
                JointKind::Revolute { axis: DVec3::X },
                Pose::from_translation(offset),
                Pose::IDENTITY,
            ),
        );
        physics
            .configure_track(TrackForceDesc {
                carrier: chassis,
                sprocket,
                joint,
                contact_colliders: vec![contact],
                local_axle: DVec3::X,
                local_forward: DVec3::Z,
                pitch_radius: 0.111,
                longitudinal_friction: 0.85,
                lateral_friction: 0.65,
                slip_damping: 8000.0,
                max_torque: 300.0,
                max_power: 5000.0,
                speed_gain: 120.0,
            })
            .unwrap();
        rotors.push(sprocket);
    }
    (physics, chassis, rotors)
}

fn run(physics: &mut dyn PhysicsBackend, count: usize) {
    for _ in 0..count {
        physics.step(&|_, _| false);
    }
}

#[test]
fn ground_contact_converts_sprocket_torque_into_propulsion() {
    let (mut physics, chassis, rotors) = fixture(true);
    run(&mut physics, 120);
    let start = physics.body(chassis).unwrap().translation();
    for &r in &rotors {
        physics.set_track_speed(r, 0.4).unwrap();
    }
    run(&mut physics, 480);
    let displacement = physics.body(chassis).unwrap().translation() - start;
    assert!(displacement.z > 0.4, "{displacement:?}");
    assert!(rotors.iter().all(|&r| {
        physics
            .track_output(r)
            .is_some_and(|s| s.contacts > 0 && s.travel > 0.5)
    }));
}

#[test]
fn airborne_tracks_spin_without_linear_propulsion() {
    let (mut physics, _, rotors) = fixture(false);
    for &r in &rotors {
        physics.set_track_speed(r, 0.4).unwrap();
    }
    run(&mut physics, 240);
    assert!(rotors.iter().all(|&r| {
        physics
            .track_output(r)
            .is_some_and(|s| s.travel > 0.5 && s.contacts == 0)
    }));
    let momentum = physics
        .bodies()
        .iter()
        .filter_map(|id| physics.body(*id))
        .fold(DVec3::ZERO, |sum, body| sum + body.linvel() * body.mass());
    assert!(momentum.length() < 1e-5, "{momentum:?}");
    physics.remove_track(rotors[0]);
    assert!(physics.track_output(rotors[0]).is_none());
    assert!(physics.set_track_speed(rotors[0], 0.4).is_err());
}
