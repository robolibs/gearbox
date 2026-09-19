use super::*;

fn near(a: DVec3, b: DVec3) {
    assert!((a - b).length() < 1e-8, "{a:?} != {b:?}");
}

fn dynamic() -> BodyDesc {
    let mut body = BodyDesc::dynamic();
    body.additional_mass = Some(MassProps {
        mass: 1.0,
        local_com: DVec3::ZERO,
        inertia: Inertia::Principal(DVec3::splat(2.0)),
    });
    body
}

#[test]
fn backend_is_send_sync_and_preserves_body_mass_and_tags() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<MollaBackend>();
    let mut backend = MollaBackend::default();
    let mut ecs = bevy::prelude::World::new();
    let entity = ecs.spawn_empty().id();
    let body = backend.insert_body(BodyDesc::dynamic().entity(entity));
    let collider = backend
        .insert_collider(
            ColliderDesc::new(Shape::Cuboid {
                half_extents: DVec3::splat(0.5),
            })
            .parent(body)
            .density(10.0)
            .entity(entity),
        )
        .unwrap();
    assert!((backend.body(body).unwrap().mass() - 10.0).abs() < 1e-10);
    assert_eq!(backend.body(body).unwrap().entity(), Some(entity));
    assert_eq!(backend.collider(collider).unwrap().entity(), Some(entity));
    assert_eq!(backend.body(body).unwrap().colliders(), vec![collider]);
    backend
        .body_mut(body)
        .unwrap()
        .apply_impulse(DVec3::X * 20.0, true);
    near(backend.body(body).unwrap().linvel(), DVec3::X * 2.0);
    backend.collider_mut(collider).unwrap().set_mass(5.0);
    assert!((backend.body(body).unwrap().mass() - 5.0).abs() < 1e-10);
    backend.body_mut(body).unwrap().set_additional_mass(
        MassProps {
            mass: 2.0,
            local_com: DVec3::ZERO,
            inertia: Inertia::Principal(DVec3::ONE),
        },
        true,
    );
    assert!((backend.body(body).unwrap().mass() - 7.0).abs() < 1e-10);
    backend.remove_collider(collider, true);
    assert!((backend.body(body).unwrap().mass() - 2.0).abs() < 1e-10);
}

#[test]
fn both_joint_requests_run_actual_reduced_motors_with_caps() {
    for reduced in [false, true] {
        let mut backend = MollaBackend::default();
        backend.set_gravity(DVec3::ZERO);
        let fixed = backend.insert_body(BodyDesc::fixed());
        let moving = backend.insert_body(dynamic());
        let mut desc = JointDesc::new(
            JointKind::Revolute { axis: DVec3::Z },
            Pose::IDENTITY,
            Pose::IDENTITY,
        );
        desc.reduced = reduced;
        let joint = backend.insert_joint(fixed, moving, desc);
        let motor = backend.joint_mut(joint, true).unwrap();
        motor.set_motor_model(JointAxis::AngX, MotorModel::Force);
        motor.set_motor_velocity(JointAxis::AngX, 5.0, 4.0);
        motor.set_motor_max_force(JointAxis::AngX, 2.0);
        for _ in 0..120 {
            backend.step(&|_, _| false);
        }
        assert!(backend.joint_is_reduced(joint));
        near(backend.body(moving).unwrap().angvel(), DVec3::Z);
        assert!(backend.body(moving).unwrap().rotation().z > 0.0);
        assert_eq!(
            backend
                .joint(joint)
                .unwrap()
                .motor(JointAxis::AngX)
                .unwrap()
                .max_force,
            2.0
        );
    }
}

#[test]
fn heightfield_rays_and_live_collider_queries_use_current_poses() {
    let mut backend = MollaBackend::default();
    let heightfield = backend
        .insert_collider(ColliderDesc::new(Shape::Heightfield {
            rows: 2,
            cols: 3,
            heights: vec![1.0; 6],
            scale: DVec3::new(6.0, 2.0, 4.0),
        }))
        .unwrap();
    let (hit, distance) = backend
        .cast_ray(DVec3::Y * 5.0, -DVec3::Y * 2.0, 10.0)
        .unwrap();
    assert_eq!(hit, heightfield);
    assert!((distance - 3.0).abs() < 1e-10);
    let body = backend.insert_body(BodyDesc::fixed().pose(Pose::from_translation(DVec3::X * 10.0)));
    let collider = backend
        .insert_collider(ColliderDesc::new(Shape::Ball { radius: 1.0 }).parent(body))
        .unwrap();
    backend
        .body_mut(body)
        .unwrap()
        .set_translation(DVec3::X * 20.0, true);
    backend.sync_collider_positions();
    near(
        backend.collider(collider).unwrap().position().translation,
        DVec3::X * 20.0,
    );
    near(
        backend.collider(collider).unwrap().aabb().center(),
        DVec3::X * 20.0,
    );
    assert_eq!(
        backend
            .cast_ray(DVec3::new(20.0, 5.0, 0.0), -DVec3::Y, 10.0)
            .unwrap()
            .0,
        collider
    );
    backend
        .collider_mut(collider)
        .unwrap()
        .set_shape(Shape::ConvexHull {
            points: vec![DVec3::ZERO],
        });
    assert!(matches!(
        backend.collider(collider).unwrap().shape(),
        ShapeView::Ball { .. }
    ));
}

#[test]
fn contacts_expose_impulses_material_and_per_step_pair_filter() {
    let mut backend = MollaBackend::default();
    backend.set_gravity(DVec3::ZERO);
    let ground = backend.insert_body(BodyDesc::fixed());
    backend
        .insert_collider(
            ColliderDesc::new(Shape::Cuboid {
                half_extents: DVec3::new(5.0, 0.5, 5.0),
            })
            .parent(ground)
            .friction(0.2),
        )
        .unwrap();
    let moving = backend.insert_body(dynamic().pose(Pose::from_translation(DVec3::Y * 0.9)));
    let mut desc = ColliderDesc::new(Shape::Ball { radius: 0.5 })
        .parent(moving)
        .density(0.0)
        .friction(0.9);
    desc.friction_combine = Some(CombineRule::Min);
    let ball = backend.insert_collider(desc).unwrap();
    backend.step(&|a, b| (a == ground && b == moving) || (a == moving && b == ground));
    assert!(backend.contacts().is_empty());
    backend.step(&|_, _| false);
    let contacts = backend.contacts_with(ball);
    assert!(!contacts.is_empty());
    assert!(
        contacts
            .iter()
            .flat_map(|m| &m.points)
            .any(|p| p.solved && p.impulse > 0.0)
    );
    for p in contacts.iter().flat_map(|m| &m.points).filter(|p| p.solved) {
        assert_eq!(p.friction, 0.2);
        assert!(p.dist <= 0.0);
    }
    assert!(backend.body(moving).unwrap().linvel().y > 0.0);
}

#[test]
fn nonroot_fixed_transitions_and_removals_preserve_stable_ids() {
    let mut backend = MollaBackend::default();
    backend.set_gravity(DVec3::ZERO);
    let a = backend.insert_body(dynamic());
    let b = backend.insert_body(dynamic());
    let joint = backend.insert_joint(
        a,
        b,
        JointDesc::new(
            JointKind::Revolute { axis: DVec3::Z },
            Pose::IDENTITY,
            Pose::IDENTITY,
        ),
    );
    let collider = backend
        .insert_collider(
            ColliderDesc::new(Shape::Ball { radius: 0.2 })
                .parent(a)
                .density(0.0),
        )
        .unwrap();
    backend.body_mut(b).unwrap().set_kind(BodyKind::Fixed, true);
    backend
        .body_mut(a)
        .unwrap()
        .apply_torque_impulse(DVec3::Z * 2.0, true);
    backend.step(&|_, _| false);
    assert!(backend.body(a).unwrap().rotation().z > 0.0);
    assert_eq!(backend.joint_bodies(joint), Some((a, b)));
    backend.remove_body(a);
    assert!(backend.body(a).is_none());
    assert!(backend.collider(collider).is_none());
    assert!(backend.joint(joint).is_none());
    assert!(backend.body(b).is_some());
    assert_ne!(backend.insert_body(dynamic()), a);
}

#[test]
fn generalized_corruption_is_quarantined_before_step() {
    let mut backend = MollaBackend::default();
    let body = backend.insert_body(dynamic());
    backend
        .shared
        .world()
        .scene
        .state_mut()
        .joint_qd
        .host_mut()
        .unwrap()[0] = f64::NAN;
    backend.step(&|_, _| false);
    assert!(!backend.body(body).unwrap().is_enabled());
    assert!(backend.body(body).unwrap().linvel().is_finite());
    assert_eq!(backend.quarantined_bodies(), vec![body]);
}

#[test]
fn internal_quarantine_reaches_the_gearbox_entity_report() {
    let mut backend = MollaBackend::default();
    let mut ecs = bevy::prelude::World::new();
    let entity = ecs.spawn_empty().id();
    let body = backend.insert_body(dynamic().entity(entity));
    backend
        .shared
        .world()
        .scene
        .state_mut()
        .joint_qd
        .host_mut()
        .unwrap()[0] = f64::NAN;
    let mut world = crate::physics::PhysicsWorld::with_backend(Box::new(backend));
    world.step();
    assert!(!world.body(body).unwrap().is_enabled());
    assert_eq!(world.quarantined, vec![entity]);
}

#[test]
fn softness_and_frame_edits_produce_compliant_motion() {
    let mut backend = MollaBackend::default();
    backend.set_gravity(DVec3::ZERO);
    let fixed = backend.insert_body(BodyDesc::fixed());
    let moving = backend.insert_body(dynamic().pose(Pose::from_translation(DVec3::X * 0.1)));
    let mut desc = JointDesc::new(
        JointKind::Fixed,
        Pose::IDENTITY,
        Pose::from_translation(-DVec3::X * 0.1),
    );
    desc.softness = Some((8.0, 1.0));
    let j = backend.insert_joint(fixed, moving, desc);
    backend
        .joint_mut(j, true)
        .unwrap()
        .set_frame2(Pose::IDENTITY);
    near(
        backend.body(moving).unwrap().position().translation,
        DVec3::X * 0.1,
    );
    backend.step(&|_, _| false);
    let body = backend.body(moving).unwrap();
    assert!(body.position().translation.x > 0.05 && body.position().translation.x < 0.1);
    assert!(body.linvel().x < 0.0);
    for _ in 0..120 {
        backend.step(&|_, _| false);
    }
    near(
        backend.body(moving).unwrap().position().translation,
        DVec3::ZERO,
    );
    assert!(backend.quarantined_bodies().is_empty());
}

#[test]
fn physics_world_hitch_capture_uses_live_softness() {
    let backend = MollaBackend::default();
    let mut world = crate::physics::PhysicsWorld::with_backend(Box::new(backend));
    world.set_gravity(DVec3::ZERO);
    let fixed = world.insert_body(BodyDesc::fixed());
    let moving = world.insert_body(dynamic().pose(Pose::from_translation(DVec3::X * 0.1)));
    let mut desc = JointDesc::new(
        JointKind::Generic {
            locked: JointAxes(63),
        },
        Pose::IDENTITY,
        Pose::from_translation(-DVec3::X * 0.1),
    );
    desc.softness = Some((8.0, 1.0));
    let j = world.insert_joint(fixed, moving, desc);
    world.capture_hitch(j, Pose::IDENTITY);
    for _ in 0..240 {
        let before = world.body(moving).unwrap().position().translation;
        world.step();
        let body = world.body(moving).unwrap();
        assert!(body.is_enabled());
        assert!((body.position().translation - before).length() < 0.01);
        assert!(world.quarantined.is_empty());
    }
    near(world.joint(j).unwrap().frame2().translation, DVec3::ZERO);
    near(
        world.body(moving).unwrap().position().translation,
        DVec3::ZERO,
    );
}
