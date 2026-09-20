//! Copy solved world poses into local transforms, ancestors before descendants.

use bevy::prelude::*;

use super::convert::quat_from_d;
use super::world::PhysicsWorld;
use crate::globe::{Site, transform_in_site};

// A body's pose is in its site's region of the physics world; its transform
// is in its site's frame, so the chain stops at the site's grid.
pub(super) struct PublishedTransform {
    pub body: super::backend::BodyId,
    pub pose: super::backend::Pose,
    pub global: GlobalTransform,
}
pub fn writeback_transforms(
    mut world: ResMut<PhysicsWorld>,
    active: Res<super::PhysicsActive>,
    parents: Query<&ChildOf>,
    sites: Query<&Site>,
    mut transforms: ParamSet<(Query<&Transform>, Query<&mut Transform>)>,
) {
    let mut entities: Vec<_> = world.entity_to_body.keys().copied().collect();
    entities.sort_by_cached_key(|entity| parents.iter_ancestors(*entity).count());
    let mut written = std::collections::HashSet::new();
    for entity in entities {
        let handle = world.entity_to_body[&entity];
        let Some(rb) = world.body(handle) else {
            continue;
        };
        let pose = rb.position();
        if !active.0
            && world
                .published_transforms
                .get(&entity)
                .is_some_and(|old| old.body == handle && old.pose == pose)
            && !parents
                .iter_ancestors(entity)
                .any(|parent| written.contains(&parent))
        {
            continue;
        }
        let parent_world = match parents.get(entity) {
            Ok(parent) if !sites.contains(parent.parent()) => {
                transform_in_site(parent.parent(), &parents, &transforms.p0(), &sites)
            }
            _ => GlobalTransform::IDENTITY,
        };
        // Physics steps at its own fixed rate, so a frame falls somewhere between
        // two steps. What is drawn is the pose carried on by the time left over:
        // drawn as stepped, a moving body would stutter against the frame rate.
        let ahead = if active.0 && rb.is_dynamic() { world.accumulator } else { 0.0 };
        let at = pose.translation + rb.linvel() * ahead;
        let (_, local) = crate::globe::site_local(at.x, at.y, at.z);
        let translation = parent_world
            .affine()
            .inverse()
            .transform_point3(Vec3::new(local[0] as f32, local[1] as f32, local[2] as f32));
        let turned = rb.angvel() * ahead;
        let spin = Quat::from_scaled_axis(Vec3::new(turned.x as f32, turned.y as f32, turned.z as f32));
        let rotation = parent_world.compute_transform().rotation.inverse()
            * (spin * quat_from_d(pose.rotation));
        if let Ok(mut transform) = transforms.p1().get_mut(entity) {
            transform.translation = translation;
            transform.rotation = rotation;
            written.insert(entity);
        }
        if written.contains(&entity) {
            let global = transform_in_site(entity, &parents, &transforms.p0(), &sites);
            world.published_transforms.insert(
                entity,
                PublishedTransform {
                    body: handle,
                    pose,
                    global,
                },
            );
        }
    }
    let live: std::collections::HashSet<_> = world.entity_to_body.keys().copied().collect();
    world
        .published_transforms
        .retain(|entity, _| live.contains(entity));
}

#[cfg(test)]
mod tests {
    use super::super::convert::{quat_to_d, vec3_to_d};
    use super::*;
    use crate::physics::backend::{BodyDesc, Pose};
    use bevy::transform::helper::TransformHelper;
    use bevy::ecs::system::{RunSystemOnce, SystemState};

    #[test]
    fn paused_teleports_publish_and_resume_only_applies_external_edits() {
        use crate::physics::backend::{DQuat, DVec3, JointDesc, JointKind, PhysicsBackend};
        for backend in [
            Box::new(crate::physics::molla::MollaBackend::default()) as Box<dyn PhysicsBackend>,
            Box::new(crate::physics::rapier::RapierBackend::default()),
        ] {
            let mut app = App::new();
            let root = app.world_mut().spawn(Transform::IDENTITY).id();
            let chassis = app
                .world_mut()
                .spawn((Transform::IDENTITY, ChildOf(root)))
                .id();
            let wheel = app
                .world_mut()
                .spawn((Transform::from_xyz(1.0, 0.0, 0.0), ChildOf(chassis)))
                .id();
            let mut physics = PhysicsWorld::with_backend(backend);
            let a = physics.insert_body(BodyDesc::dynamic().entity(chassis));
            let b = physics.insert_body(
                BodyDesc::dynamic()
                    .entity(wheel)
                    .pose(Pose::from_translation(DVec3::X)),
            );
            let extra = physics.insert_body(BodyDesc::dynamic());
            physics.entity_to_body.insert(chassis, a);
            physics.entity_to_body.insert(wheel, b);
            physics.insert_joint(
                a,
                b,
                JointDesc::new(
                    JointKind::Revolute { axis: DVec3::X },
                    Pose::from_translation(DVec3::X),
                    Pose::IDENTITY,
                ),
            );
            physics.body_mut(extra).unwrap().set_linvel(DVec3::Z, true);
            app.insert_resource(physics)
                .insert_resource(super::super::PhysicsActive(false))
                .add_systems(Update, super::super::sync_bodies_to_transforms_on_resume)
                .add_systems(PostUpdate, writeback_transforms);
            app.update();

            let rotation = DQuat::from_rotation_y(0.4);
            let target = Pose::new(DVec3::new(4.0, 2.0, -3.0), rotation);
            let wheel_target = Pose::new(target.transform_point(DVec3::X), rotation);
            app.world_mut()
                .resource_mut::<PhysicsWorld>()
                .set_body_poses(&[(b, wheel_target), (a, target)], true)
                .unwrap();
            app.update();
            let mut state = SystemState::<TransformHelper>::new(app.world_mut());
            for (entity, pose) in [(chassis, target), (wheel, wheel_target)] {
                let gt = state
                    .get(app.world())
                    .unwrap()
                    .compute_global_transform(entity)
                    .unwrap();
                assert!((vec3_to_d(gt.translation()) - pose.translation).length() < 1e-5);
            }

            app.world_mut()
                .resource_mut::<PhysicsWorld>()
                .body_mut(a)
                .unwrap()
                .set_linvel(DVec3::X * 2.0, true);
            app.world_mut()
                .resource_mut::<super::super::PhysicsActive>()
                .0 = true;
            app.update();
            let physics = app.world().resource::<PhysicsWorld>();
            assert!(
                (physics.body(a).unwrap().position().translation - target.translation).length()
                    < 1e-8
            );
            assert!((physics.body(a).unwrap().linvel() - DVec3::X * 2.0).length() < 1e-8);

            app.world_mut()
                .resource_mut::<super::super::PhysicsActive>()
                .0 = false;
            app.update();
            app.world_mut()
                .get_mut::<Transform>(root)
                .unwrap()
                .translation
                .z += 5.0;
            app.update();
            assert!(
                (app.world()
                    .resource::<PhysicsWorld>()
                    .body(a)
                    .unwrap()
                    .position()
                    .translation
                    - target.translation)
                    .length()
                    < 1e-8
            );
            app.world_mut()
                .resource_mut::<super::super::PhysicsActive>()
                .0 = true;
            app.update();
            let physics = app.world().resource::<PhysicsWorld>();
            for (id, pose) in [(a, target), (b, wheel_target)] {
                assert!(
                    (physics.body(id).unwrap().position().translation
                        - pose.translation
                        - DVec3::Z * 5.0)
                        .length()
                        < 1e-5
                );
                assert!(physics.body(id).unwrap().linvel().length() < 1e-8);
            }
            assert_eq!(physics.body(extra).unwrap().linvel(), DVec3::Z);

            app.world_mut()
                .resource_mut::<super::super::PhysicsActive>()
                .0 = false;
            app.update();
            let newest = Pose::from_translation(DVec3::new(-8.0, 4.0, 2.0));
            app.world_mut()
                .resource_mut::<PhysicsWorld>()
                .set_body_poses(
                    &[
                        (a, newest),
                        (b, Pose::from_translation(newest.translation + DVec3::X)),
                    ],
                    true,
                )
                .unwrap();
            app.world_mut()
                .resource_mut::<super::super::PhysicsActive>()
                .0 = true;
            app.update();
            assert!(
                (app.world()
                    .resource::<PhysicsWorld>()
                    .body(a)
                    .unwrap()
                    .position()
                    .translation
                    - newest.translation)
                    .length()
                    < 1e-8
            );
        }
    }

    #[test]
    fn nested_wheel_uses_current_parent_pose_through_scaled_wrappers() {
        let mut world = World::new();
        let root = world
            .spawn(
                Transform::from_xyz(10.0, 2.0, -4.0)
                    .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
                    .with_scale(Vec3::splat(0.01)),
            )
            .id();
        let wheel = world.spawn(Transform::IDENTITY).id();
        let knuckle = world.spawn(Transform::IDENTITY).id();
        let chassis = world.spawn((Transform::IDENTITY, ChildOf(root))).id();
        let wrapper = world
            .spawn((
                Transform::from_xyz(20.0, 0.0, 0.0).with_rotation(Quat::from_rotation_z(0.2)),
                ChildOf(chassis),
            ))
            .id();
        world.entity_mut(knuckle).insert(ChildOf(wrapper));
        world.entity_mut(wheel).insert(ChildOf(knuckle));
        let mesh_local = Transform::from_rotation(Quat::from_rotation_z(0.5));
        let mesh = world.spawn((mesh_local, ChildOf(wheel))).id();
        let mut physics = PhysicsWorld::default();
        for entity in [wheel, knuckle, chassis] {
            let handle = physics.insert_body(BodyDesc::dynamic());
            physics.entity_to_body.insert(entity, handle);
        }
        world.insert_resource(physics);
        world.insert_resource(super::super::PhysicsActive(false));

        for angle in [0.3_f32, 0.7, -0.4] {
            let base_rotation = Quat::from_rotation_y(angle * 0.5);
            let steer_rotation = base_rotation * Quat::from_rotation_y(angle);
            let wheel_rotation = steer_rotation * Quat::from_rotation_x(angle * 4.0);
            let expected = [
                (chassis, Vec3::new(4.0 + angle, 2.0, 7.0), base_rotation),
                (knuckle, Vec3::new(5.0 + angle, 2.0, 7.0), steer_rotation),
                (wheel, Vec3::new(5.0 + angle, 2.0, 7.0), wheel_rotation),
            ];
            {
                let mut physics = world.resource_mut::<PhysicsWorld>();
                for (entity, translation, rotation) in expected {
                    let handle = physics.entity_to_body[&entity];
                    physics.body_mut(handle).unwrap().set_position(
                        Pose {
                            translation: vec3_to_d(translation),
                            rotation: quat_to_d(rotation),
                        },
                        true,
                    );
                }
            }
            world.run_system_once(writeback_transforms).unwrap();
            let mut state = SystemState::<TransformHelper>::new(&mut world);
            let helper = state.get(&world).expect("transform helper");
            for (entity, translation, rotation) in expected {
                let actual = helper
                    .compute_global_transform(entity)
                    .unwrap()
                    .compute_transform();
                assert!(actual.translation.abs_diff_eq(translation, 1e-4));
                assert!(actual.rotation.dot(rotation).abs() > 0.99999);
                assert!(actual.scale.abs_diff_eq(Vec3::splat(0.01), 1e-5));
            }
            let mesh_rotation = helper
                .compute_global_transform(mesh)
                .unwrap()
                .compute_transform()
                .rotation;
            assert!(
                mesh_rotation
                    .dot(wheel_rotation * mesh_local.rotation)
                    .abs()
                    > 0.99999
            );
        }
    }
}
