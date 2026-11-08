use super::*;
use crate::physics::MollaBackend;
use crate::physics::backend::{BodyDesc, ShapeView};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;

fn rings(axis: usize, radius: f64, half: f64, segments: usize) -> Vec<DVec3> {
    let mut points = Vec::new();
    for end in [-half, half] {
        for k in 0..segments {
            let angle = std::f64::consts::TAU * k as f64 / segments as f64;
            let (u, v) = (radius * angle.cos(), radius * angle.sin());
            points.push(match axis {
                0 => DVec3::new(end, u, v),
                1 => DVec3::new(u, end, v),
                _ => DVec3::new(u, v, end),
            });
        }
    }
    points
}

/// Points as they come back from a stored f32 mesh.
fn stored(points: &[DVec3]) -> Vec<DVec3> {
    points.iter().map(|p| p.as_vec3().as_dvec3()).collect()
}

#[test]
fn a_cylinder_hull_gives_its_axis_radius_and_half_height() {
    for (axis, unit) in [(0, Vec3::X), (1, Vec3::Y), (2, Vec3::Z)] {
        let (found, radius, half) =
            hull_cylinder(&stored(&rings(axis, 0.826, 0.29, 48))).expect("a cylinder hull");
        assert_eq!(found, unit);
        assert!(
            (radius - 0.826).abs() < 1e-6 && (half - 0.29).abs() < 1e-6,
            "{radius} {half}"
        );
    }
}

#[test]
fn other_hulls_are_not_cylinders() {
    let corners: Vec<DVec3> = (0..8)
        .map(|i| {
            DVec3::new(
                if i & 1 == 0 { -1.0 } else { 1.0 },
                if i & 2 == 0 { -1.0 } else { 1.0 },
                if i & 4 == 0 { -1.0 } else { 1.0 },
            )
        })
        .collect();
    assert!(hull_cylinder(&corners).is_none(), "a box");
    let shifted: Vec<DVec3> = rings(0, 0.5, 0.2, 48)
        .into_iter()
        .map(|p| p + DVec3::Y * 0.3)
        .collect();
    assert!(hull_cylinder(&shifted).is_none(), "off its origin");
    let mut cone = rings(2, 0.5, 0.2, 48);
    for p in cone.iter_mut().filter(|p| p.z > 0.0) {
        p.x *= 0.5;
        p.y *= 0.5;
    }
    assert!(hull_cylinder(&cone).is_none(), "unequal rings");
    assert!(
        hull_cylinder(&rings(0, 0.5, 0.2, 8)).is_none(),
        "too few points a ring"
    );
}

#[test]
fn a_cylinder_shaped_hull_mesh_builds_an_exact_cylinder_along_its_axis() {
    let mut app = App::new();
    app.insert_resource(PhysicsWorld::with_backend(
        Box::new(MollaBackend::default()),
    ));
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    let positions: Vec<[f32; 3]> = rings(0, 0.6439, 0.24, 48)
        .iter()
        .map(|p| p.as_vec3().to_array())
        .collect();
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    let mut meshes = Assets::<Mesh>::default();
    let handle = meshes.add(mesh);
    app.insert_resource(meshes);

    let body = app
        .world_mut()
        .spawn((
            UsdRigidBody {
                enabled: true,
                ..Default::default()
            },
            Transform::default(),
            GlobalTransform::default(),
        ))
        .id();
    let id = app
        .world_mut()
        .resource_mut::<PhysicsWorld>()
        .insert_body(BodyDesc::dynamic().entity(body));
    app.world_mut()
        .resource_mut::<PhysicsWorld>()
        .entity_to_body
        .insert(body, id);
    let collider = app
        .world_mut()
        .spawn((
            UsdCollider {
                shape: UsdColliderShape::Mesh,
                enabled: true,
                approximation: Some(UsdCollisionApprox::ConvexHull),
                physics_material: None,
                simulation_owner: None,
            },
            Mesh3d(handle),
            Transform::default(),
            GlobalTransform::default(),
            ChildOf(body),
        ))
        .id();

    let mut schedule = bevy::ecs::schedule::Schedule::default();
    schedule.add_systems(convert_colliders);
    schedule.run(app.world_mut());

    let world = app.world().resource::<PhysicsWorld>();
    let built = world
        .collider(world.entity_to_collider[&collider])
        .expect("collider built");
    let ShapeView::Cylinder {
        half_height,
        radius,
    } = built.shape()
    else {
        panic!("expected an exact cylinder, got {:?}", built.shape());
    };
    assert!(
        (radius - 0.6439).abs() < 1e-5 && (half_height - 0.24).abs() < 1e-5,
        "{radius} {half_height}"
    );
    let axis = built
        .position_wrt_parent()
        .expect("a body-local pose")
        .rotation
        * glam::DVec3::Y;
    assert!(
        axis.x.abs() > 0.999,
        "the cylinder runs along X, got {axis:?}"
    );
}
