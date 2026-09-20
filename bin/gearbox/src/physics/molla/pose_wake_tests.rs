use super::*;

#[test]
fn teleport_wake_flag_matches_rapier_for_uncontacted_bodies() {
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
            let pose = Pose::new(DVec3::new(4.0, 5.0, 6.0), DQuat::from_rotation_y(0.4));
            backend.body_mut(a).unwrap().set_position(pose, wake);
            assert_eq!(backend.body(a).unwrap().is_sleeping(), !wake);
            assert_eq!(backend.body(a).unwrap().position(), pose);
            for _ in 0..5 {
                backend.step(&|_, _| false);
            }
            near(
                backend.body(a).unwrap().position().translation,
                pose.translation,
            );
            assert_eq!(
                backend.body(a).unwrap().is_sleeping(),
                !wake,
                "{}",
                backend.name()
            );
            assert!(backend.body(other).unwrap().is_sleeping());
        }
    }
}
