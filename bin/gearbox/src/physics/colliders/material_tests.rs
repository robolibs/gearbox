use super::*;
use crate::physics::MollaBackend;

fn apply(app: &mut App) {
    let mut schedule = bevy::ecs::schedule::Schedule::default();
    schedule.add_systems(apply_physics_materials);
    schedule.run(app.world_mut());
}

fn friction(app: &App, id: ColliderId) -> f64 {
    app.world()
        .resource::<PhysicsWorld>()
        .collider(id)
        .unwrap()
        .friction()
}

#[test]
fn material_sync_preserves_runtime_override_until_authored_input_changes() {
    let mut app = App::new();
    app.insert_resource(PhysicsWorld::with_backend(
        Box::new(MollaBackend::default()),
    ));
    let material = app
        .world_mut()
        .spawn(UsdPhysicsMaterial {
            dynamic_friction: Some(0.9),
            restitution: Some(0.2),
            ..Default::default()
        })
        .id();
    let entity = app
        .world_mut()
        .spawn((
            ColliderAttached,
            UsdCollider {
                shape: UsdColliderShape::Sphere { radius: 0.5 },
                enabled: true,
                approximation: None,
                physics_material: Some(material),
                simulation_owner: None,
            },
        ))
        .id();
    let mut physics = app.world_mut().resource_mut::<PhysicsWorld>();
    let id = physics
        .insert_collider(ColliderDesc::new(Shape::Ball { radius: 0.5 }))
        .unwrap();
    physics.entity_to_collider.insert(entity, id);
    drop(physics);
    apply(&mut app);
    assert_eq!(friction(&app, id), f64::from(0.9_f32));
    app.world_mut()
        .resource_mut::<PhysicsWorld>()
        .collider_mut(id)
        .unwrap()
        .set_friction(1.1);
    apply(&mut app);
    assert_eq!(friction(&app, id), 1.1);
    app.world_mut()
        .get_mut::<UsdPhysicsMaterial>(material)
        .unwrap()
        .dynamic_friction = Some(0.7);
    apply(&mut app);
    assert_eq!(friction(&app, id), f64::from(0.7_f32));
    app.world_mut()
        .resource_mut::<PhysicsWorld>()
        .collider_mut(id)
        .unwrap()
        .set_friction(1.2);
    app.world_mut()
        .get_mut::<UsdCollider>(entity)
        .unwrap()
        .physics_material = None;
    apply(&mut app);
    app.world_mut()
        .get_mut::<UsdCollider>(entity)
        .unwrap()
        .physics_material = Some(material);
    apply(&mut app);
    assert_eq!(friction(&app, id), f64::from(0.7_f32));
    let replacement = app
        .world_mut()
        .resource_mut::<PhysicsWorld>()
        .insert_collider(ColliderDesc::new(Shape::Ball { radius: 0.5 }))
        .unwrap();
    app.world_mut()
        .resource_mut::<PhysicsWorld>()
        .entity_to_collider
        .insert(entity, replacement);
    apply(&mut app);
    assert_eq!(friction(&app, replacement), f64::from(0.7_f32));
    assert_eq!(
        app.world()
            .resource::<PhysicsWorld>()
            .collider(replacement)
            .unwrap()
            .restitution(),
        f64::from(0.2_f32)
    );
}

#[test]
fn a_collider_on_no_body_stands_where_its_prim_does() {
    let mut app = App::new();
    app.insert_resource(PhysicsWorld::with_backend(Box::new(MollaBackend::default())));
    app.init_resource::<Assets<Mesh>>();
    let turn = Quat::from_rotation_y(0.7);
    let parent = app.world_mut().spawn(Transform::from_xyz(10.0, 0.0, -4.0).with_rotation(turn)).id();
    let entity = app
        .world_mut()
        .spawn((
            Transform::from_xyz(2.0, 1.5, 0.0).with_scale(Vec3::new(2.0, 1.0, 0.5)),
            ChildOf(parent),
            UsdCollider { shape: UsdColliderShape::Cube { size: 1.0 }, enabled: true, ..Default::default() },
        ))
        .id();
    let mut schedule = bevy::ecs::schedule::Schedule::default();
    schedule.add_systems(convert_colliders);
    schedule.run(app.world_mut());
    let physics = app.world().resource::<PhysicsWorld>();
    let collider = physics.collider(physics.entity_to_collider[&entity]).unwrap();
    let pose = collider.position();
    let expected = Vec3::new(10.0, 0.0, -4.0) + turn * Vec3::new(2.0, 1.5, 0.0);
    assert!((pose.translation - vec3_to_d(expected)).length() < 1e-5, "{:?}", pose.translation);
    assert!(pose.rotation.angle_between(quat_to_d(turn)) < 1e-3, "{:?} against {:?}", pose.rotation, turn);
    let super::super::backend::ShapeView::Cuboid { half_extents } = collider.shape() else { panic!("a cube is a box") };
    assert!((half_extents - DVec3::new(1.0, 0.5, 0.25)).length() < 1e-6);
}
