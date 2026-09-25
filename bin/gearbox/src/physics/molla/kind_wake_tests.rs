use super::*;

#[test]
fn body_kind_wake_flag_and_noops_leave_other_bodies_asleep() {
    for intermediate in [BodyKind::Fixed, BodyKind::Kinematic] {
        for wake in [false, true] {
            let mut backend = MollaBackend::default();
            backend.set_gravity(DVec3::ZERO);
            let a = backend.insert_body(dynamic());
            let other = backend.insert_body(dynamic().pose(Pose::from_translation(DVec3::X * 3.0)));
            for _ in 0..480 { backend.step(&|_, _| false); }
            assert!(backend.body(a).unwrap().is_sleeping());
            assert!(backend.body(other).unwrap().is_sleeping());
            let pose = backend.body(a).unwrap().position();
            backend.body_mut(a).unwrap().set_kind(BodyKind::Dynamic, true);
            assert!(backend.body(a).unwrap().is_sleeping(), "no-op");
            backend.body_mut(a).unwrap().set_kind(intermediate, false);
            for _ in 0..3 { backend.step(&|_, _| false); }
            backend.body_mut(a).unwrap().set_kind(BodyKind::Dynamic, wake);
            assert_eq!(backend.body(a).unwrap().is_sleeping(), !wake, "{intermediate:?}");
            assert!(backend.body(other).unwrap().is_sleeping(), "unrelated");
            for _ in 0..3 { backend.step(&|_, _| false); }
            assert_eq!(backend.body(a).unwrap().is_sleeping(), !wake, "after step");
            assert!(backend.body(other).unwrap().is_sleeping());
            assert_eq!(backend.body(a).unwrap().position(), pose);
            assert_eq!(backend.body(a).unwrap().mass(), 1.0);
            backend.body_mut(a).unwrap().apply_impulse(DVec3::X, true);
            near(backend.body(a).unwrap().linvel(), DVec3::X);
            assert!(!backend.body(a).unwrap().is_sleeping());
            assert!(backend.body(other).unwrap().is_sleeping());
        }
    }
}
