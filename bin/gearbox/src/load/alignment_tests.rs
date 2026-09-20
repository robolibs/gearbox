use super::*;
use crate::physics::backend::{
    BodyDesc, BodyId, ColliderDesc, JointDesc, JointKind, PhysicsBackend, Shape,
};
use crate::physics::{MollaBackend, PhysicsWorld, RapierBackend};

fn fixture(molla: bool, reverse: bool) -> (App, Entity, [Entity; 3], [BodyId; 3], BodyId) {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::transform::TransformPlugin));
    let root = app
        .world_mut()
        .spawn(Transform::from_xyz(0.0, 1.0, 0.0))
        .id();
    let entities = [1.0, 0.5, 0.5].map(|y| {
        app.world_mut()
            .spawn((Transform::from_xyz(0.0, y, 0.0), ChildOf(root)))
            .id()
    });
    let mut physics = PhysicsWorld::with_backend(if molla {
        Box::new(MollaBackend::default()) as Box<dyn PhysicsBackend>
    } else {
        Box::new(RapierBackend::default())
    });
    let mut ids = [BodyId(0); 3];
    for index in if reverse { [2, 1, 0] } else { [0, 1, 2] } {
        ids[index] = physics.insert_body(BodyDesc::dynamic().entity(entities[index]).pose(
            Pose::from_translation(DVec3::Y * if index == 0 { 1.0 } else { 0.5 }),
        ));
        physics.entity_to_body.insert(entities[index], ids[index]);
    }
    physics.insert_joint(
        ids[0],
        ids[1],
        JointDesc::new(
            JointKind::Prismatic { axis: DVec3::Y },
            Pose::from_translation(-DVec3::Y * 0.5),
            Pose::IDENTITY,
        ),
    );
    physics.insert_joint(
        ids[1],
        ids[2],
        JointDesc::new(
            JointKind::Revolute { axis: DVec3::X },
            Pose::IDENTITY,
            Pose::IDENTITY,
        ),
    );
    let collider_entity = app
        .world_mut()
        .spawn((Transform::IDENTITY, ChildOf(entities[2]), Name::new("tyre")))
        .id();
    let collider = physics
        .insert_collider(
            ColliderDesc::new(Shape::Ball { radius: 0.5 })
                .parent(ids[2])
                .entity(collider_entity),
        )
        .unwrap();
    physics.entity_to_collider.insert(collider_entity, collider);
    let other =
        physics.insert_body(BodyDesc::dynamic().pose(Pose::from_translation(DVec3::X * 10.0)));
    physics.body_mut(other).unwrap().set_linvel(DVec3::X, true);
    app.insert_resource(physics);
    app.update();
    (app, root, entities, ids, other)
}

#[test]
fn terrain_alignment_translates_the_whole_suspension_once() {
    for molla in [false, true] {
        for reverse in [false, true] {
            let (mut app, root, _, ids, other) = fixture(molla, reverse);
            benchmark_align_machine(&mut app, root);
            let physics = app.world().resource::<PhysicsWorld>();
            for (id, y) in ids.into_iter().zip([1.03, 0.53, 0.53]) {
                let body = physics.body(id).unwrap();
                assert!(
                    (body.translation().y - y).abs() < 1e-6,
                    "{} reverse={reverse} body={id:?}: {:?}, expected y={y}",
                    physics.name(),
                    body.translation()
                );
                assert_eq!(body.linvel(), DVec3::ZERO);
                assert_eq!(body.angvel(), DVec3::ZERO);
            }
            assert_eq!(physics.body(other).unwrap().translation(), DVec3::X * 10.0);
            assert_eq!(physics.body(other).unwrap().linvel(), DVec3::X);
            assert!(
                (app.world().get::<Transform>(root).unwrap().translation.y - 0.03).abs() < 1e-6
            );
            assert!(app.world().get::<MachinePhysicsSyncPending>(root).is_none());
        }
    }
}

#[test]
fn invalid_projected_pose_keeps_the_entire_machine_pending_and_unchanged() {
    for molla in [false, true] {
        let (mut app, root, entities, ids, _) = fixture(molla, false);
        let before: Vec<_> = ids
            .iter()
            .map(|&id| {
                app.world()
                    .resource::<PhysicsWorld>()
                    .body(id)
                    .unwrap()
                    .position()
            })
            .collect();
        app.world_mut()
            .entity_mut(entities[1])
            .insert(GlobalTransform::from_translation(Vec3::splat(f32::NAN)));
        benchmark_align_machine(&mut app, root);
        let physics = app.world().resource::<PhysicsWorld>();
        for (id, pose) in ids.into_iter().zip(before) {
            assert_eq!(physics.body(id).unwrap().position(), pose);
        }
        assert_eq!(
            app.world().get::<Transform>(root).unwrap().translation.y,
            1.0
        );
        assert!(app.world().get::<MachinePhysicsSyncPending>(root).is_some());
        assert!(!app.world().contains_resource::<PhysicsActivationPending>());
        app.world_mut()
            .entity_mut(entities[1])
            .insert(GlobalTransform::from_translation(Vec3::Y * 1.5));
        app.world_mut()
            .get_mut::<MachinePhysicsSyncPending>(root)
            .unwrap()
            .activate_after_sync = true;
        let mut schedule = bevy::ecs::schedule::Schedule::default();
        schedule.add_systems(sync_pending_machine_physics_to_scene_transforms);
        schedule.run(app.world_mut());
        assert!(app.world().get::<MachinePhysicsSyncPending>(root).is_none());
        assert!(app.world().contains_resource::<PhysicsActivationPending>());
        for (id, y) in ids.into_iter().zip([1.03, 0.53, 0.53]) {
            assert!(
                (app.world()
                    .resource::<PhysicsWorld>()
                    .body(id)
                    .unwrap()
                    .translation()
                    .y
                    - y)
                    .abs()
                    < 1e-6
            );
        }
    }
}

#[test]
fn missing_transforms_or_colliders_do_not_partially_align_a_machine() {
    for molla in [false, true] {
        for missing_transform in [false, true] {
            let (mut app, root, entities, ids, _) = fixture(molla, false);
            if missing_transform {
                app.world_mut()
                    .entity_mut(entities[1])
                    .remove::<GlobalTransform>();
            } else {
                app.world_mut()
                    .resource_mut::<PhysicsWorld>()
                    .entity_to_collider
                    .clear();
            }
            benchmark_align_machine(&mut app, root);
            for (id, y) in ids.into_iter().zip([1.0, 0.5, 0.5]) {
                assert!(
                    (app.world()
                        .resource::<PhysicsWorld>()
                        .body(id)
                        .unwrap()
                        .translation()
                        .y
                        - y)
                        .abs()
                        < 1e-6
                );
            }
            assert!(app.world().get::<MachinePhysicsSyncPending>(root).is_some());
            assert!(!app.world().contains_resource::<PhysicsActivationPending>());
            assert_eq!(
                app.world().get::<Transform>(root).unwrap().translation.y,
                1.0
            );
        }
    }
}
