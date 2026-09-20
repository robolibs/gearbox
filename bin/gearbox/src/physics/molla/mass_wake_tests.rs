use super::*;

#[test]
fn additional_mass_wake_flag_matches_rapier_and_preserves_unrelated_sleepers() {
    for wake in [false, true] {
        let backends: [Box<dyn PhysicsBackend>; 2] = [
            Box::new(MollaBackend::default()),
            Box::new(crate::physics::rapier::RapierBackend::default()),
        ];
        for mut backend in backends {
            backend.set_gravity(DVec3::ZERO);
            let a = backend.insert_body(dynamic());
            let other = backend.insert_body(dynamic());
            for _ in 0..480 {
                backend.step(&|_, _| false);
            }
            assert!(backend.body(a).unwrap().is_sleeping());
            let pose = backend.body(a).unwrap().position();
            backend.body_mut(a).unwrap().set_additional_mass(
                MassProps {
                    mass: 4.0,
                    local_com: DVec3::X * 0.25,
                    inertia: Inertia::Tensor(DMat3::IDENTITY * 8.0),
                },
                wake,
            );
            assert_eq!(
                backend.body(a).unwrap().is_sleeping(),
                !wake,
                "{}",
                backend.name()
            );
            assert!(backend.body(other).unwrap().is_sleeping());
            for _ in 0..5 {
                backend.step(&|_, _| false);
            }
            assert_eq!(
                backend.body(a).unwrap().is_sleeping(),
                !wake,
                "{}",
                backend.name()
            );
            assert!(backend.body(other).unwrap().is_sleeping());
            assert_eq!(backend.body(a).unwrap().position(), pose);
            assert_eq!(backend.body(a).unwrap().mass(), 4.0);
            near(
                backend.body(a).unwrap().local_center_of_mass(),
                DVec3::X * 0.25,
            );
            backend
                .body_mut(a)
                .unwrap()
                .apply_impulse(DVec3::X * 4.0, true);
            near(backend.body(a).unwrap().linvel(), DVec3::X);
            assert!(!backend.body(a).unwrap().is_sleeping());
            assert!(backend.body(other).unwrap().is_sleeping());
        }
    }
}

#[test]
fn invalid_or_unchanged_additional_mass_does_not_wake_molla() {
    let mut backend = MollaBackend::default();
    let a = backend.insert_body(dynamic());
    backend.body_mut(a).unwrap().sleep();
    for mass in [-1.0, f64::NAN, 1.0] {
        backend.body_mut(a).unwrap().set_additional_mass(
            MassProps {
                mass,
                local_com: DVec3::ZERO,
                inertia: Inertia::Principal(DVec3::splat(2.0)),
            },
            true,
        );
        assert!(backend.body(a).unwrap().is_sleeping());
        assert_eq!(backend.body(a).unwrap().mass(), 1.0);
    }
}
