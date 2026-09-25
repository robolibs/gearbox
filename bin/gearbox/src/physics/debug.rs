//! Gizmo-based collider debug renderer.
//! Iterates `PhysicsWorld.colliders` and draws each shape's
//! wireframe via `bevy::gizmos`. Driven by `ColliderDebugEnabled`.

use bevy::math::primitives::{Cuboid, Cylinder, Sphere};
use bevy::prelude::*;
use super::backend::ShapeView;

use super::convert::{quat_from_d, vec3_from_d};
use super::world::PhysicsWorld;

#[derive(Resource, Clone, Copy, Debug)]
pub struct ColliderDebugEnabled(pub bool);

impl Default for ColliderDebugEnabled {
    fn default() -> Self {
        Self(false)
    }
}

const DEBUG_COLOR: Color = Color::srgb(0.0, 0.9, 0.6);

pub fn report_transform_alignment(
    time: Res<Time>,
    world: Res<PhysicsWorld>,
    globals: Query<(&GlobalTransform, &usd_bevy::UsdPrimRef)>,
    meshes: Query<(Entity, &GlobalTransform), With<Mesh3d>>,
    parents: Query<&ChildOf>,
    mut mesh_bindings: Local<std::collections::HashMap<Entity, Quat>>,
    mut next: Local<f32>,
) {
    if time.elapsed_secs() < *next {
        return;
    }
    *next = time.elapsed_secs() + 1.0;
    let mut count = 0;
    let mut position_error = 0.0_f32;
    let mut angle_error = 0.0_f32;
    let mut wheel_speed = 0.0_f64;
    let mut wheels = std::collections::HashMap::new();
    for (entity, handle) in &world.entity_to_body {
        let (Ok((global, prim)), Some(body)) = (globals.get(*entity), world.body(*handle))
        else {
            continue;
        };
        let actual = global.compute_transform();
        let expected = body.position();
        let delta = (actual.rotation.inverse() * quat_from_d(expected.rotation)).normalize();
        let angle = 2.0
            * Vec3::new(delta.x, delta.y, delta.z)
                .length()
                .atan2(delta.w.abs());
        position_error = position_error.max(
            actual
                .translation
                .distance(vec3_from_d(expected.translation)),
        );
        angle_error = angle_error.max(angle.to_degrees());
        if prim.path.contains("wheel") {
            wheel_speed = wheel_speed.max(body.angvel().length());
            wheels.insert(*entity, actual.rotation);
        }
        count += 1;
    }
    let mut mesh_count = 0;
    let mut mesh_drift = 0.0_f32;
    mesh_bindings.retain(|entity, _| meshes.contains(*entity));
    for (entity, transform) in &meshes {
        let Some(wheel_rotation) = std::iter::once(entity)
            .chain(parents.iter_ancestors(entity))
            .find_map(|parent| wheels.get(&parent))
        else {
            continue;
        };
        let relative =
            (wheel_rotation.inverse() * transform.compute_transform().rotation).normalize();
        let bind = mesh_bindings.entry(entity).or_insert(relative);
        let delta = (bind.inverse() * relative).normalize();
        let angle = 2.0
            * Vec3::new(delta.x, delta.y, delta.z)
                .length()
                .atan2(delta.w.abs());
        mesh_drift = mesh_drift.max(angle.to_degrees());
        mesh_count += 1;
    }
    if count > 0 {
        info!(
            "gearbox-tf alignment: bodies={count} max_position_m={position_error:.7} max_angle_deg={angle_error:.6} max_wheel_rad_s={wheel_speed:.3} wheel_meshes={mesh_count} mesh_relative_drift_deg={mesh_drift:.6}"
        );
    }
}

pub fn draw_collider_gizmos(
    enabled: Res<ColliderDebugEnabled>,
    world: Res<PhysicsWorld>,
    mut gizmos: Gizmos,
) {
    if !enabled.0 {
        return;
    }
    for id in world.colliders() {
        let Some(collider) = world.collider(id) else {
            continue;
        };
        let pose = collider.position();
        let translation = vec3_from_d(pose.translation);
        let rotation = quat_from_d(pose.rotation);
        let iso = Isometry3d::new(translation, rotation);

        match collider.shape() {
            ShapeView::Cuboid { half_extents: h } => {
                gizmos.primitive_3d(
                    &Cuboid::new(h.x as f32 * 2.0, h.y as f32 * 2.0, h.z as f32 * 2.0),
                    iso,
                    DEBUG_COLOR,
                );
            }
            ShapeView::Ball { radius } => {
                gizmos
                    .primitive_3d(&Sphere::new(radius as f32), iso, DEBUG_COLOR)
                    .resolution(32);
            }
            ShapeView::Cylinder { half_height, radius } => {
                gizmos
                    .primitive_3d(
                        &Cylinder::new(radius as f32, half_height as f32 * 2.0),
                        iso,
                        DEBUG_COLOR,
                    )
                    .resolution(32);
            }
            ShapeView::RoundCylinder { half_height, radius, border_radius } => {
                gizmos
                    .primitive_3d(
                        &Cylinder::new(
                            (radius + border_radius) as f32,
                            (half_height + border_radius) as f32 * 2.0,
                        ),
                        iso,
                        DEBUG_COLOR,
                    )
                    .resolution(32);
            }
            ShapeView::Capsule { a, b, .. } => {
                gizmos.line(
                    translation + rotation * vec3_from_d(a),
                    translation + rotation * vec3_from_d(b),
                    DEBUG_COLOR,
                );
            }
            ShapeView::ConvexPolyhedron { points, edges } => {
                for edge in edges {
                    let a = vec3_from_d(points[edge[0] as usize]);
                    let b = vec3_from_d(points[edge[1] as usize]);
                    gizmos.line(
                        translation + rotation * a,
                        translation + rotation * b,
                        DEBUG_COLOR,
                    );
                }
            }
            // Trimeshes / heightfields fall through to AABB outline.
            ShapeView::Other => {
                let h = collider.local_aabb().half_extents();
                gizmos.primitive_3d(
                    &Cuboid::new(h.x as f32 * 2.0, h.y as f32 * 2.0, h.z as f32 * 2.0),
                    iso,
                    DEBUG_COLOR.with_alpha(0.4),
                );
            }
        }
    }
}
