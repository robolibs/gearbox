//! Copy solved world poses into local transforms, ancestors before descendants.

use bevy::prelude::*;
use bevy::transform::helper::TransformHelper;

use super::convert::{quat_from_d, vec3_from_d};
use super::world::PhysicsWorld;

pub fn writeback_transforms(
    world: Res<PhysicsWorld>,
    parents: Query<&ChildOf>,
    mut transforms: ParamSet<(TransformHelper, Query<&mut Transform>)>,
) {
    let mut entities: Vec<_> = world.entity_to_body.keys().copied().collect();
    entities.sort_by_cached_key(|entity| parents.iter_ancestors(*entity).count());
    for entity in entities {
        let Some(rb) = world
            .entity_to_body
            .get(&entity)
            .and_then(|h| world.bodies.get(*h))
        else {
            continue;
        };
        let parent_world = if let Ok(parent) = parents.get(entity) {
            let Ok(transform) = transforms.p0().compute_global_transform(parent.parent()) else {
                continue;
            };
            transform
        } else {
            GlobalTransform::IDENTITY
        };
        let pose = rb.position();
        let translation = parent_world
            .affine()
            .inverse()
            .transform_point3(vec3_from_d(pose.translation));
        let rotation =
            parent_world.compute_transform().rotation.inverse() * quat_from_d(pose.rotation);
        if let Ok(mut transform) = transforms.p1().get_mut(entity) {
            transform.translation = translation;
            transform.rotation = rotation;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::convert::{quat_to_d, vec3_to_d};
    use super::*;
    use bevy::ecs::system::{RunSystemOnce, SystemState};
    use rapier3d::prelude::{Pose, RigidBodyBuilder};

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
            let handle = physics.bodies.insert(RigidBodyBuilder::dynamic().build());
            physics.entity_to_body.insert(entity, handle);
        }
        world.insert_resource(physics);

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
                    physics.bodies[handle].set_position(
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
            let helper = state.get(&world);
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
