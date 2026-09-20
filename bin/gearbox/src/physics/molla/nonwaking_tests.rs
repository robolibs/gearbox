use super::*;

#[test]
fn nonwaking_velocity_force_and_impulse_edits_match_rapier() {
    for edit in 0..8 {
        let backends: [Box<dyn PhysicsBackend>; 2] = [
            Box::new(MollaBackend::default()),
            Box::new(crate::physics::rapier::RapierBackend::default()),
        ];
        for mut backend in backends {
            backend.set_gravity(DVec3::ZERO);
            let body = backend.insert_body(dynamic());
            let unrelated = backend.insert_body(dynamic());
            for _ in 0..480 {
                backend.step(&|_, _| false);
            }
            assert!(backend.body(body).unwrap().is_sleeping());
            let pose = backend.body(body).unwrap().position();
            {
                let target = backend.body_mut(body).unwrap();
                match edit {
                    0 => target.set_linvel(DVec3::X * 2.0, false),
                    1 => target.set_angvel(DVec3::Z * 2.0, false),
                    2 => target.add_force(DVec3::X * 3.0, false),
                    3 => target.add_torque(DVec3::Z * 4.0, false),
                    5 => target.apply_impulse(DVec3::X * 4.0, false),
                    6 => target.apply_torque_impulse(DVec3::Z * 4.0, false),
                    7 => target.apply_impulse_at_point(
                        DVec3::X * 4.0,
                        target.center_of_mass() + DVec3::Y,
                        false,
                    ),
                    _ => {
                        target.add_force(DVec3::X * 3.0, false);
                        target.add_torque(DVec3::Z * 4.0, false);
                        target.reset_forces(false);
                    }
                }
            }
            for _ in 0..5 {
                assert!(
                    backend.body(body).unwrap().is_sleeping(),
                    "{} edit={edit}",
                    backend.name()
                );
                backend.step(&|_, _| false);
                assert_eq!(backend.body(body).unwrap().position(), pose);
            }
            if edit == 0 {
                near(backend.body(body).unwrap().linvel(), DVec3::X * 2.0);
            }
            if edit == 1 {
                near(backend.body(body).unwrap().angvel(), DVec3::Z * 2.0);
            }
            if edit == 5 || edit == 7 {
                near(backend.body(body).unwrap().linvel(), DVec3::X * 4.0);
            }
            if edit == 6 || edit == 7 {
                near(
                    backend.body(body).unwrap().angvel(),
                    DVec3::Z * if edit == 6 { 2.0 } else { -2.0 },
                );
            }
            backend.body_mut(body).unwrap().wake_up(true);
            backend.step(&|_, _| false);
            let changed = backend.body(body).unwrap();
            match edit {
                0 | 2 | 5 => assert!(changed.linvel().x > 0.0),
                1 | 3 | 6 => assert!(changed.angvel().z > 0.0),
                7 => {
                    assert!(changed.linvel().x > 0.0);
                    assert!(changed.angvel().z < 0.0);
                }
                _ => {
                    near(changed.linvel(), DVec3::ZERO);
                    near(changed.angvel(), DVec3::ZERO);
                }
            }
            assert!(backend.body(unrelated).unwrap().is_sleeping());
        }
    }
}

#[test]
fn waking_edits_activate_only_the_target_and_invalid_edits_do_not_wake() {
    for edit in 0..8 {
        let mut backend = MollaBackend::default();
        backend.set_gravity(DVec3::ZERO);
        let body = backend.insert_body(dynamic());
        let unrelated = backend.insert_body(dynamic());
        for id in [body, unrelated] {
            backend.body_mut(id).unwrap().sleep();
        }
        let target = backend.body_mut(body).unwrap();
        target.set_linvel(DVec3::splat(f64::NAN), true);
        target.set_angvel(DVec3::splat(f64::INFINITY), true);
        target.add_force(DVec3::splat(f64::NAN), true);
        target.add_torque(DVec3::splat(f64::INFINITY), true);
        target.apply_impulse(DVec3::splat(f64::NAN), true);
        target.apply_torque_impulse(DVec3::splat(f64::INFINITY), true);
        target.apply_impulse_at_point(DVec3::X, DVec3::splat(f64::NAN), true);
        assert!(target.is_sleeping());
        near(target.linvel(), DVec3::ZERO);
        near(target.angvel(), DVec3::ZERO);
        match edit {
            0 => target.set_linvel(DVec3::X, true),
            1 => target.set_angvel(DVec3::Z, true),
            2 => target.add_force(DVec3::X, true),
            3 => target.add_torque(DVec3::Z, true),
            5 => target.apply_impulse(DVec3::X, true),
            6 => target.apply_torque_impulse(DVec3::Z, true),
            7 => target.apply_impulse_at_point(DVec3::X, target.center_of_mass() + DVec3::Y, true),
            _ => {
                target.add_force(DVec3::X, false);
                target.reset_forces(true);
            }
        }
        assert!(!target.is_sleeping(), "edit={edit}");
        backend.step(&|_, _| false);
        assert!(backend.body(unrelated).unwrap().is_sleeping());
    }
}
