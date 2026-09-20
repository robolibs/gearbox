use super::*;

#[path = "nonwaking_tests.rs"]
mod nonwaking;

#[path = "mass_wake_tests.rs"]
mod mass_wake;

#[path = "pose_wake_tests.rs"]
mod pose_wake;

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
fn authored_sleep_and_explicit_wake_advance_the_real_backend() {
    let mut backend = MollaBackend::default();
    let mut desc = dynamic();
    desc.sleeping = true;
    desc.linvel = DVec3::X;
    let body = backend.insert_body(desc);
    assert!(backend.body(body).unwrap().is_sleeping());
    near(backend.body(body).unwrap().linvel(), DVec3::ZERO);
    let pose = backend.body(body).unwrap().position();
    for _ in 0..20 { backend.step(&|_, _| false); }
    assert_eq!(backend.body(body).unwrap().position(), pose);
    backend.body_mut(body).unwrap().wake_up(true);
    assert!(!backend.body(body).unwrap().is_sleeping());
    backend.step(&|_, _| false);
    assert!(backend.body(body).unwrap().linvel().y < 0.0);
    backend.body_mut(body).unwrap().sleep();
    assert!(backend.body(body).unwrap().is_sleeping());
    backend.body_mut(body).unwrap().add_force(DVec3::X, true);
    assert!(!backend.body(body).unwrap().is_sleeping());
    backend.step(&|_, _| false);
    assert!(backend.body(body).unwrap().linvel().x > 0.0);
}

#[test]
fn weak_and_strong_wakes_match_rapier_sleep_eligibility() {
    let backends: [Box<dyn PhysicsBackend>; 2] = [
        Box::new(MollaBackend::default()),
        Box::new(crate::physics::rapier::RapierBackend::default()),
    ];
    for mut backend in backends {
        check_wake_strength(&mut *backend, false);
    }
    check_wake_strength(&mut MollaBackend::default(), true);
}

#[test]
#[ignore = "Rapier 0.32 panics when explicitly sleeping a mixed-wake active island"]
fn rapier_explicit_sleep_mixed_wake_regression() {
    check_wake_strength(&mut crate::physics::rapier::RapierBackend::default(), true);
}

fn check_wake_strength(backend: &mut dyn PhysicsBackend, explicit: bool) {
    backend.set_gravity(DVec3::ZERO);
    let weak = backend.insert_body(dynamic());
    let strong = backend.insert_body(dynamic());
    let untouched = backend.insert_body(dynamic());
    backend.step(&|_, _| false);
    if explicit {
        for body in [weak, strong, untouched] {
            backend.body_mut(body).unwrap().sleep();
        }
    } else {
        for _ in 0..480 { backend.step(&|_, _| false); }
    }
    for body in [weak, strong, untouched] {
        assert!(backend.body(body).unwrap().is_sleeping());
    }
    backend.body_mut(weak).unwrap().wake_up(false);
    backend.body_mut(strong).unwrap().wake_up(true);
    assert!(!backend.body(weak).unwrap().is_sleeping());
    assert!(!backend.body(strong).unwrap().is_sleeping());
    backend.step(&|_, _| false);
    assert!(backend.body(weak).unwrap().is_sleeping(), "{} weak", backend.name());
    assert!(!backend.body(strong).unwrap().is_sleeping(), "{} strong", backend.name());
    assert!(backend.body(untouched).unwrap().is_sleeping(), "{} unrelated", backend.name());
}

#[test]
fn backend_sleep_and_motor_wake_cover_the_whole_tree() {
    let mut backend = MollaBackend::default();
    let a = backend.insert_body(dynamic());
    let b = backend.insert_body(dynamic());
    let joint = backend.insert_joint(a, b, JointDesc::new(
        JointKind::Revolute { axis: DVec3::Z }, Pose::IDENTITY, Pose::IDENTITY,
    ));
    backend.body_mut(b).unwrap().sleep();
    assert!(backend.body(a).unwrap().is_sleeping());
    assert!(backend.body(b).unwrap().is_sleeping());
    backend.joint_mut(joint, false).unwrap().set_motor_velocity(JointAxis::AngX, 2.0, 5.0);
    backend.step(&|_, _| false);
    assert!(backend.body(a).unwrap().is_sleeping());
    assert!(backend.body(b).unwrap().is_sleeping());
    backend.joint_mut(joint, true).unwrap().set_motor_velocity(JointAxis::AngX, 1.0, 5.0);
    assert!(!backend.body(a).unwrap().is_sleeping());
    assert!(!backend.body(b).unwrap().is_sleeping());
    backend.step(&|_, _| false);
    assert!(backend.body(b).unwrap().angvel().length() > 0.0);
}

#[test]
fn authored_loop_closure_keeps_tree_and_joint_access() {
    let mut backend = MollaBackend::default();
    backend.set_gravity(DVec3::ZERO);
    let a = backend.insert_body(BodyDesc::fixed());
    let b = backend.insert_body(dynamic());
    let hinge = JointDesc::new(
        JointKind::Revolute { axis: DVec3::Z },
        Pose::IDENTITY,
        Pose::IDENTITY,
    );
    let tree = backend.insert_joint(a, b, hinge.clone());
    let mut desc = hinge;
    desc.loop_closure = true;
    let closure = backend.insert_joint(a, b, desc);
    assert_ne!(tree, closure);
    assert_eq!(backend.joints_between(a, b).len(), 2);
    assert!(!backend.joint(closure).unwrap().contacts_enabled());
    backend
        .joint_mut(closure, true)
        .unwrap()
        .set_motor_velocity(JointAxis::AngX, 0.2, 5.0);
    for _ in 0..30 {
        backend.step(&|_, _| false);
    }
    assert!(backend.body(b).unwrap().angvel().z > 0.01);
    backend.remove_joint(closure);
    assert!(backend.joint(tree).is_some());
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

fn wheel_rig(axis: DVec3) -> (MollaBackend, BodyId, BodyId, JointId, ColliderId) {
    let mut backend = MollaBackend::default();
    backend.set_gravity(DVec3::ZERO);
    let fixed = backend.insert_body(BodyDesc::fixed());
    let chassis = backend.insert_body(dynamic().pose(Pose::from_translation(DVec3::Y * 0.49)));
    let wheel = backend.insert_body(dynamic().pose(Pose::from_translation(DVec3::Y * 0.49)));
    backend.insert_joint(
        fixed,
        chassis,
        JointDesc::new(
            JointKind::Prismatic { axis: DVec3::X },
            Pose::from_translation(DVec3::Y * 0.49),
            Pose::IDENTITY,
        ),
    );
    let joint = backend.insert_joint(
        chassis,
        wheel,
        JointDesc::new(JointKind::Revolute { axis }, Pose::IDENTITY, Pose::IDENTITY),
    );
    backend
        .insert_collider(ColliderDesc::new(Shape::Ball { radius: 0.5 }).parent(wheel))
        .unwrap();
    let ground = backend
        .insert_collider(
            ColliderDesc::new(Shape::Cuboid {
                half_extents: DVec3::new(10.0, 0.1, 10.0),
            })
            .translation(-DVec3::Y * 0.1)
            .friction(0.6),
        )
        .unwrap();
    backend.register_wheel_ground(ground, None).unwrap();
    backend
        .configure_wheel(WheelForceDesc {
            body: wheel,
            joint,
            local_hub: DVec3::ZERO,
            forward: DVec3::X,
            radius: 0.5,
            supported_mass: 100.0,
            tyre: None,
        })
        .unwrap();
    (backend, chassis, wheel, joint, ground)
}

#[test]
fn sleeping_wheel_retains_pressure_visuals_and_static_support() {
    let (mut backend, chassis, wheel, joint, _) = wheel_rig(-DVec3::Z);
    backend.configure_wheel(WheelForceDesc {
        body: wheel, joint, local_hub: DVec3::ZERO, forward: DVec3::X,
        radius: 0.5, supported_mass: 100.0, tyre: Some(PressureTyreDesc::reference(0.3)),
    }).unwrap();
    backend.step(&|_, _| false);
    let before = backend.wheel_output(wheel).unwrap();
    backend.body_mut(chassis).unwrap().sleep();
    for _ in 0..20 { backend.step(&|_, _| false); }
    assert!(backend.body(chassis).unwrap().is_sleeping());
    let after = backend.wheel_output(wheel).unwrap();
    assert!(after.in_contact && after.normal_force > 0.0 && after.grip_force > 0.0);
    assert!((after.normal_force - before.normal_force).abs() < 1e-8);
    assert_eq!(after.pressure.unwrap().patch_area, before.pressure.unwrap().patch_area);
    backend.set_wheel_pressures(&[(wheel, 220_000.0)]).unwrap();
    assert!(!backend.body(chassis).unwrap().is_sleeping());
    backend.step(&|_, _| false);
    assert!(backend.wheel_output(wheel).unwrap().pressure.unwrap().pressure_pa > 180_000.0);
}

#[test]
fn tyre_readout_replaces_ground_manifolds_and_respects_material_cells() {
    let (mut backend, _, wheel, _, ground) = wheel_rig(-DVec3::Z);
    backend
        .register_wheel_ground(
            ground,
            Some(TerrainFrictionGrid {
                origin: [-10.0, -10.0],
                cell_size: [20.0, 20.0],
                cols: 1,
                rows: 1,
                values: vec![0.2],
            }),
        )
        .unwrap();
    backend.step(&|_, _| false);
    let output = backend.wheel_output(wheel).unwrap();
    assert!(output.in_contact);
    near(output.normal, DVec3::Y);
    assert!((output.normal_force - 490.5).abs() < 1e-6);
    assert!((output.grip_force - output.normal_force * 0.2).abs() < 1e-8);
    assert!(backend.contacts_with(ground).is_empty());
    let mut settings = backend.settings();
    settings.dt *= 2.0;
    backend.set_settings(settings);
    assert_eq!(
        backend.wheel_output(wheel).unwrap().normal_force,
        output.normal_force
    );
    backend.collider_mut(ground).unwrap().set_enabled(false);
    backend.step(&|_, _| false);
    assert!(!backend.wheel_output(wheel).unwrap().in_contact);
}

#[test]
fn unchanged_material_sync_preserves_tyre_readout_and_scene() {
    let (mut backend, _, wheel, _, ground) = wheel_rig(-DVec3::Z);
    backend.step(&|_, _| false);
    let output = backend.wheel_output(wheel).unwrap();
    assert!(output.in_contact && output.normal_force > 0.0);
    let stamp = backend.shared.world().scene.stamp();
    let friction = backend.collider(ground).unwrap().friction();
    let restitution = backend.collider(ground).unwrap().restitution();
    for _ in 0..3 {
        let collider = backend.collider_mut(ground).unwrap();
        collider.set_friction(friction);
        collider.set_restitution(restitution);
        collider.set_friction_combine_rule(CombineRule::Average);
        assert_eq!(backend.shared.world().scene.stamp(), stamp);
        assert_eq!(
            backend.wheel_output(wheel).unwrap().normal_force,
            output.normal_force
        );
    }
    backend
        .collider_mut(ground)
        .unwrap()
        .set_friction(friction * 0.5);
    assert_ne!(backend.shared.world().scene.stamp(), stamp);
    assert!(!backend.wheel_output(wheel).unwrap().in_contact);
    backend.step(&|_, _| false);
    assert!(backend.wheel_output(wheel).unwrap().in_contact);
}

#[test]
fn opposite_authored_axles_use_opposite_motor_signs_for_forward_motion() {
    for axis in [-DVec3::Z, DVec3::Z] {
        let (mut backend, chassis, wheel, joint, _) = wheel_rig(axis);
        let sign = backend.wheel_drive_sign(joint);
        assert_eq!(sign, -axis.z);
        let motor = backend.joint_mut(joint, true).unwrap();
        motor.set_motor_model(JointAxis::AngX, MotorModel::Force);
        motor.set_motor_velocity(JointAxis::AngX, 2.0 * sign, 2.0);
        motor.set_motor_max_force(JointAxis::AngX, 10.0);
        for _ in 0..120 {
            backend.step(&|_, _| false);
        }
        assert!(backend.body(chassis).unwrap().translation().x > 0.01);
        assert!(backend.wheel_output(wheel).unwrap().normal_force > 0.0);
        assert!(backend.quarantined_bodies().is_empty());
    }
}

#[test]
fn failed_wheel_registration_preserves_active_tyre_and_removal_clears_it() {
    let (mut backend, _, wheel, joint, _) = wheel_rig(-DVec3::Z);
    assert!(
        backend
            .configure_wheel(WheelForceDesc {
                body: wheel,
                joint,
                local_hub: DVec3::ZERO,
                forward: DVec3::X,
                radius: f64::NAN,
                supported_mass: 100.0,
                tyre: None,
            })
            .is_err()
    );
    backend.step(&|_, _| false);
    assert!(backend.wheel_output(wheel).unwrap().in_contact);
    backend.remove_joint(joint);
    assert!(backend.wheel_output(wheel).is_none());
    backend.remove_body(wheel);
    assert!(backend.wheel_output(wheel).is_none());
}

#[test]
fn pressure_backend_preserves_targets_and_reports_brush_grip() {
    let (mut backend, chassis, wheel, joint, ground) = wheel_rig(-DVec3::Z);
    let desc = WheelForceDesc {
        body: wheel,
        joint,
        local_hub: DVec3::ZERO,
        forward: DVec3::X,
        radius: 0.5,
        supported_mass: 100.0,
        tyre: Some(PressureTyreDesc::reference(0.3)),
    };
    backend.configure_wheel(desc).unwrap();
    assert!(
        backend
            .set_wheel_pressures(&[(wheel, 220_000.0), (chassis, 220_000.0)])
            .is_err()
    );
    assert_eq!(
        backend
            .wheel_output(wheel)
            .unwrap()
            .pressure
            .unwrap()
            .target_pressure_pa,
        180_000.0
    );
    backend.set_wheel_pressures(&[(wheel, 220_000.0)]).unwrap();
    let before = backend.body(wheel).unwrap().translation();
    backend.step(&|_, _| false);
    near(backend.body(wheel).unwrap().translation(), before);
    let output = backend.wheel_output(wheel).unwrap();
    let tyre = output.pressure.unwrap();
    near(tyre.hub, backend.body(wheel).unwrap().translation());
    near(tyre.forward, DVec3::X);
    assert_eq!(tyre.radius, desc.radius);
    assert_eq!(tyre.width, desc.tyre.unwrap().width);
    assert!((tyre.pressure_pa - (180_000.0 + desc.tyre.unwrap().pressure_rate_pa_s * backend.settings().dt)).abs() < 1e-6);
    assert!(output.in_contact && tyre.patch_area > 0.0 && tyre.deflection > 0.0);
    assert!(
        output.grip_force > 0.0 && output.grip_force <= 1.12 * 0.6 * output.normal_force + 1e-8,
        "{output:?}"
    );
    assert!((output.grip_force - 0.6 * output.normal_force).abs() > 1.0);
    assert!(backend.contacts_with(ground).is_empty());
    backend.configure_wheel(desc).unwrap();
    assert_eq!(
        backend
            .wheel_output(wheel)
            .unwrap()
            .pressure
            .unwrap()
            .pressure_pa,
        tyre.pressure_pa
    );
    assert_eq!(
        backend
            .wheel_output(wheel)
            .unwrap()
            .pressure
            .unwrap()
            .target_pressure_pa,
        220_000.0
    );
    let mut bad = desc;
    bad.tyre.as_mut().unwrap().width = f64::NAN;
    assert!(backend.configure_wheel(bad).is_err());
    assert_eq!(
        backend
            .wheel_output(wheel)
            .unwrap()
            .pressure
            .unwrap()
            .target_pressure_pa,
        220_000.0
    );
    backend.remove_body(wheel);
    assert!(backend.wheel_output(wheel).is_none());
}

#[test]
fn inflation_lifts_supported_body_and_keeps_final_envelope_on_ground() {
    let mut backend = MollaBackend::default();
    let ground = backend.insert_collider(ColliderDesc::new(Shape::Cuboid {
        half_extents: DVec3::new(5.0, 0.1, 5.0),
    }).translation(-DVec3::Y * 0.1)).unwrap();
    backend.register_wheel_ground(ground, None).unwrap();
    let anchor = backend.insert_body(BodyDesc::fixed());
    let mut chassis_desc = dynamic().pose(Pose::from_translation(DVec3::Y * 0.48));
    chassis_desc.additional_mass.as_mut().unwrap().mass = 100.0;
    let chassis = backend.insert_body(chassis_desc);
    let wheel = backend.insert_body(dynamic().pose(Pose::from_translation(DVec3::Y * 0.48)));
    backend.insert_joint(anchor, chassis, JointDesc::new(
        JointKind::Prismatic { axis: DVec3::Y },
        Pose::from_translation(DVec3::Y * 0.48), Pose::IDENTITY,
    ));
    let joint = backend.insert_joint(chassis, wheel, JointDesc::new(
        JointKind::Revolute { axis: DVec3::Z }, Pose::IDENTITY, Pose::IDENTITY,
    ));
    let mut tyre = PressureTyreDesc::reference(0.3);
    tyre.pressure_pa = 50_000.0;
    backend.configure_wheel(WheelForceDesc {
        body: wheel, joint, local_hub: DVec3::ZERO, forward: DVec3::X,
        radius: 0.5, supported_mass: 101.0, tyre: Some(tyre),
    }).unwrap();
    let steps = (8.0 / backend.settings().dt).ceil() as usize;
    for _ in 0..steps { backend.step(&|_, _| false); }
    let low = backend.body(chassis).unwrap().translation().y;
    backend.set_wheel_pressures(&[(wheel, 400_000.0)]).unwrap();
    assert_eq!(backend.body(chassis).unwrap().translation().y, low);
    let mut previous = low;
    for _ in 0..steps {
        backend.step(&|_, _| false);
        let output = backend.wheel_output(wheel).unwrap();
        let pressure = output.pressure.unwrap();
        assert!(output.in_contact);
        let bottom = pressure.hub - output.normal * pressure.loaded_radius;
        assert!(bottom.y.abs() < 1e-8, "{output:?}");
        let height = backend.body(chassis).unwrap().translation().y;
        assert!((height - previous).abs() < 0.005);
        previous = height;
    }
    let high = backend.body(chassis).unwrap().translation().y;
    assert!(high > low + 0.005, "low={low} high={high}");
    backend.set_wheel_pressures(&[(wheel, 50_000.0)]).unwrap();
    for _ in 0..steps { backend.step(&|_, _| false); }
    assert!((backend.body(chassis).unwrap().translation().y - low).abs() < 1e-5);
    assert!(backend.quarantined_bodies().is_empty());
}
