//! `UsdCollider` → colliders in `PhysicsWorld`.
//!
//! Mesh approximation fallback:
//!
//! | Authored      | Static body | Dynamic body          |
//! | ------------- | ----------- | --------------------- |
//! | None/default  | TriMesh     | ConvexHull (warn)     |
//! | ConvexHull    | ConvexHull  | ConvexHull            |
//! | ConvexDecomp  | Decomp      | Decomp                |
//! | MeshSimplify  | TriMesh     | ConvexHull (warn)     |
//!
//! `UsdGeomCylinder.axis` defaults to Z while backend cylinders run along
//! Y; `compute_local_pose` folds the Y→authored-axis rotation into the pose.

use super::backend::{ColliderDesc, ColliderId, CollisionGroups, Pose, Shape};
use super::markers::{
    UsdArticulationRoot, UsdCollider, UsdColliderShape, UsdCollisionApprox, UsdPhysicsMaterial,
    UsdRigidBody,
};
use bevy::mesh::Mesh3d;
use bevy::prelude::*;
use glam::DVec3;

use super::convert::{quat_to_d, vec3_to_d};
use super::world::PhysicsWorld;

#[derive(Component)]
pub(crate) struct ColliderAttached;

#[derive(Component, PartialEq)]
pub(crate) struct AppliedPhysicsMaterial {
    collider: ColliderId,
    material: Entity,
    friction: f64,
    restitution: Option<f64>,
}

#[cfg(test)]
mod material_tests;

pub fn convert_colliders(
    mut commands: Commands,
    mut world: ResMut<PhysicsWorld>,
    colliders: Query<
        (
            Entity,
            &UsdCollider,
            Option<&UsdRigidBody>,
            Option<&Mesh3d>,
            Option<&GlobalTransform>,
            Option<&bevy::ecs::hierarchy::Children>,
            Option<&ChildOf>,
        ),
        Without<ColliderAttached>,
    >,
    descendant_meshes: Query<&Mesh3d>,
    rigid_bodies: Query<(), With<UsdRigidBody>>,
    globals: Query<&GlobalTransform>,
    parents: Query<&ChildOf>,
    articulation_roots: Query<(), With<UsdArticulationRoot>>,
    meshes: Res<Assets<Mesh>>,
) {
    for (entity, col, rb_opt, mesh3d, gt, children, child_of) in &colliders {
        // Body-relative scale: scenes with `metersPerUnit != 1` push
        // a uniform scale onto every entity's GlobalTransform; we
        // need the SCALE relative to the parent body so vertices
        // aren't shrunk twice (once by the GlobalTransform chain at
        // render, once by us baking it into the hull).
        let parent_entity = find_rigid_body_ancestor(entity, child_of, &rigid_bodies, &parents);
        let parent_handle = match parent_entity {
            Some(body_entity) => {
                let Some(handle) = world.entity_to_body.get(&body_entity).copied() else {
                    // This collider belongs to a rigid body, but the body
                    // adapter has not inserted the Molla body yet. Do not
                    // attach it as a standalone world collider; that leaves
                    // the dynamic body collider-less and it falls through the
                    // terrain. Try again next frame while `ColliderAttached`
                    // is still absent.
                    continue;
                };
                Some(handle)
            }
            None => None,
        };
        let body_scale = parent_entity
            .and_then(|e| globals.get(e).ok())
            .map(|b| b.compute_transform().scale)
            .unwrap_or(Vec3::ONE);
        let mesh_world_scale = gt.map(|g| g.compute_transform().scale).unwrap_or(Vec3::ONE);
        let local_scale = Vec3::new(
            mesh_world_scale.x / body_scale.x,
            mesh_world_scale.y / body_scale.y,
            mesh_world_scale.z / body_scale.z,
        );
        let mesh3d = mesh3d.cloned().or_else(|| {
            children.and_then(|kids| {
                kids.iter()
                    .find_map(|child| descendant_meshes.get(child).ok().cloned())
            })
        });
        // entity_scale used for primitive shape baking (cube/cylinder
        // etc.) — those are world-space dimensions.
        let entity_scale = mesh_world_scale;
        let is_dynamic = rb_opt.is_some_and(|b| b.enabled && !b.kinematic);

        let shape = match &col.shape {
            // Primitive dimensions follow the prim's scale relative to its body;
            // a scaled cube is a box, not the unit cube it was authored as.
            UsdColliderShape::Cube { size } => {
                let half = local_scale.abs() * (*size * 0.5);
                Shape::Cuboid {
                    half_extents: DVec3::new(half.x as f64, half.y as f64, half.z as f64),
                }
            }
            UsdColliderShape::Sphere { radius } => Shape::Ball {
                radius: (*radius * local_scale.abs().max_element()) as f64,
            },
            UsdColliderShape::Capsule {
                radius,
                height,
                axis,
            } => {
                let half = axis.normalize_or_zero() * (*height * 0.5);
                let half = DVec3::new(half.x as f64, half.y as f64, half.z as f64);
                Shape::Capsule { a: -half, b: half, radius: *radius as f64 }
            }
            UsdColliderShape::Cylinder {
                radius,
                height,
                axis,
            } => {
                let unit_axis = axis.normalize_or(Vec3::Y);
                let abs_axis = unit_axis.abs();
                let (height_scale, radius_scale) =
                    if abs_axis.x > abs_axis.y && abs_axis.x > abs_axis.z {
                        (
                            entity_scale.x.abs(),
                            entity_scale.y.abs().max(entity_scale.z.abs()),
                        )
                    } else if abs_axis.z > abs_axis.y {
                        (
                            entity_scale.z.abs(),
                            entity_scale.x.abs().max(entity_scale.y.abs()),
                        )
                    } else {
                        (
                            entity_scale.y.abs(),
                            entity_scale.x.abs().max(entity_scale.z.abs()),
                        )
                    };
                Shape::Cylinder {
                    half_height: (*height * 0.5 * height_scale) as f64,
                    radius: (*radius * radius_scale) as f64,
                }
            }
            // UsdPhysics has no bounded plane; a thin slab stands in.
            UsdColliderShape::Plane => Shape::Cuboid {
                half_extents: DVec3::new(50.0, 0.001, 50.0),
            },
            UsdColliderShape::Mesh => {
                let Some(mesh3d) = mesh3d.as_ref() else {
                    continue;
                };
                let Some(mesh) = meshes.get(&mesh3d.0) else {
                    continue;
                };
                info!(
                    "gearbox-physics[mesh-collider]: ent={entity:?} local_scale={:?} approx={:?}",
                    local_scale, col.approximation
                );
                let Some((vertices, indices)) = extract_mesh(mesh, local_scale) else {
                    continue;
                };
                let Some(shape) = mesh_shape(vertices, indices, col.approximation, is_dynamic)
                else {
                    continue;
                };
                shape
            }
        };

        // The collider's body-local pose: the mesh's entity-to-body
        // translation/rotation, plus the Y→authored-axis remap for primitive
        // cylinders/capsules so the backend's Y long-axis matches what the
        // mesh xform expects.
        let mut desc = ColliderDesc::new(shape)
            .pose(compute_local_pose(parent_entity, &globals, gt, &col.shape))
            .entity(entity);
        desc.parent = parent_handle;
        // Colliders of one articulation never touch each other.
        desc.groups = find_articulation_root_ancestor(
            entity,
            child_of,
            &articulation_roots,
            &parents,
        )
        .map(|root| {
            let bit = articulation_group_bit(root);
            CollisionGroups { memberships: bit, filter: !bit }
        });

        if let Some(handle) = world.insert_collider(desc) {
            world.entity_to_collider.insert(entity, handle);
            commands.entity(entity).insert(ColliderAttached);
        }
    }
}

/// The shape a mesh collider gets; see the table at the top.
fn mesh_shape(
    vertices: Vec<DVec3>,
    indices: Option<Vec<[u32; 3]>>,
    approx: Option<UsdCollisionApprox>,
    is_dynamic: bool,
) -> Option<Shape> {
    match approx.unwrap_or(UsdCollisionApprox::None) {
        UsdCollisionApprox::ConvexDecomposition => {
            let Some(indices) = indices else {
                warn!("gearbox-physics: convex decomposition needs an indexed mesh; skipping");
                return None;
            };
            Some(Shape::ConvexDecomposition { vertices, indices })
        }
        approx @ (UsdCollisionApprox::None | UsdCollisionApprox::MeshSimplification) => {
            if is_dynamic {
                warn!(
                    "gearbox-physics: mesh collider on dynamic body approx={approx:?}; \
                     falling back to a convex hull (a dynamic trimesh has no volume)"
                );
                return Some(Shape::ConvexHull { points: vertices });
            }
            let indices = indices.unwrap_or_else(|| {
                (0..vertices.len() / 3)
                    .map(|i| [(i * 3) as u32, (i * 3 + 1) as u32, (i * 3 + 2) as u32])
                    .collect()
            });
            Some(Shape::TriMesh { vertices, indices })
        }
        UsdCollisionApprox::ConvexHull
        | UsdCollisionApprox::BoundingSphere
        | UsdCollisionApprox::BoundingCube => Some(Shape::ConvexHull { points: vertices }),
    }
}

fn extract_mesh(mesh: &Mesh, scale: Vec3) -> Option<(Vec<DVec3>, Option<Vec<[u32; 3]>>)> {
    let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION)?.as_float3()?;
    let sx = scale.x as f64;
    let sy = scale.y as f64;
    let sz = scale.z as f64;
    let vertices: Vec<DVec3> = positions
        .iter()
        .map(|p| DVec3::new(p[0] as f64 * sx, p[1] as f64 * sy, p[2] as f64 * sz))
        .collect();
    let indices: Option<Vec<[u32; 3]>> = mesh.indices().map(|i| {
        let raw: Vec<u32> = i.iter().map(|x| x as u32).collect();
        raw.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect()
    });
    Some((vertices, indices))
}

fn compute_local_pose(
    parent_entity: Option<Entity>,
    globals: &Query<&GlobalTransform>,
    gt: Option<&GlobalTransform>,
    shape: &UsdColliderShape,
) -> Pose {
    let (Some(parent_e), Some(mesh_gt)) = (parent_entity, gt) else {
        return Pose {
            translation: DVec3::ZERO,
            rotation: glam::DQuat::IDENTITY,
        };
    };
    let Ok(body_gt) = globals.get(parent_e) else {
        return Pose {
            translation: DVec3::ZERO,
            rotation: glam::DQuat::IDENTITY,
        };
    };
    let body_t = body_gt.compute_transform();
    let mesh_t = mesh_gt.compute_transform();
    let inv_body_rot = body_t.rotation.inverse();
    let world_delta = mesh_t.translation - body_t.translation;
    let local_delta = inv_body_rot * world_delta;
    let local_translation = Vec3::new(
        local_delta.x / body_t.scale.x,
        local_delta.y / body_t.scale.y,
        local_delta.z / body_t.scale.z,
    );
    let axis_remap = match shape {
        UsdColliderShape::Cylinder { axis, .. } | UsdColliderShape::Capsule { axis, .. } => {
            Quat::from_rotation_arc(Vec3::Y, axis.normalize_or(Vec3::Z))
        }
        _ => Quat::IDENTITY,
    };
    let local_rotation = inv_body_rot * mesh_t.rotation * axis_remap;
    Pose {
        translation: vec3_to_d(local_translation),
        rotation: quat_to_d(local_rotation),
    }
}

pub(crate) fn find_articulation_root_ancestor(
    start: Entity,
    own_parent: Option<&ChildOf>,
    articulation_roots: &Query<(), With<UsdArticulationRoot>>,
    parents: &Query<&ChildOf>,
) -> Option<Entity> {
    if articulation_roots.get(start).is_ok() {
        return Some(start);
    }
    let mut current = own_parent.map(|p| p.parent());
    while let Some(e) = current {
        if articulation_roots.get(e).is_ok() {
            return Some(e);
        }
        current = parents.get(e).ok().map(|p| p.parent());
    }
    None
}

/// Hash an articulation-root entity to one of the 32 group bits,
/// skipping bit 0 (reserved for "world / unfiltered").
fn articulation_group_bit(entity: Entity) -> u32 {
    let bit_index = (entity.to_bits() % 31) + 1;
    1 << bit_index
}

fn find_rigid_body_ancestor(
    start: Entity,
    own_parent: Option<&ChildOf>,
    rigid_bodies: &Query<(), With<UsdRigidBody>>,
    parents: &Query<&ChildOf>,
) -> Option<Entity> {
    if rigid_bodies.get(start).is_ok() {
        return Some(start);
    }
    let mut current = own_parent.map(|p| p.parent());
    while let Some(e) = current {
        if rigid_bodies.get(e).is_ok() {
            return Some(e);
        }
        current = parents.get(e).ok().map(|p| p.parent());
    }
    None
}

pub fn apply_physics_materials(
    mut commands: Commands,
    mut world: ResMut<PhysicsWorld>,
    colliders: Query<(Entity, &UsdCollider, Option<&AppliedPhysicsMaterial>), With<ColliderAttached>>,
    materials: Query<&UsdPhysicsMaterial>,
) {
    for (entity, col, applied) in &colliders {
        let Some(mat_e) = col.physics_material else {
            if applied.is_some() { commands.entity(entity).remove::<AppliedPhysicsMaterial>(); }
            continue;
        };
        let Ok(mat) = materials.get(mat_e) else {
            if applied.is_some() { commands.entity(entity).remove::<AppliedPhysicsMaterial>(); }
            continue;
        };
        let Some(handle) = world.entity_to_collider.get(&entity).copied() else {
            continue;
        };
        let friction_coef = mat.dynamic_friction.or(mat.static_friction).unwrap_or(0.5);
        let authored = AppliedPhysicsMaterial {
            collider: handle, material: mat_e, friction: f64::from(friction_coef),
            restitution: mat.restitution.map(f64::from),
        };
        if applied == Some(&authored) { continue; }
        if let Some(c) = world.collider_mut(handle) {
            c.set_friction(friction_coef as f64);
            if let Some(r) = mat.restitution {
                c.set_restitution(r as f64);
            }
            commands.entity(entity).insert(authored);
        }
    }
}

/// Feed every `PhysicsFilteredPairsAPI` into the world's contact filter
/// once both bodies exist. Idempotent: the set dedups.
pub fn apply_collision_filters(
    filters: Query<(Entity, &super::markers::UsdCollisionFilter)>,
    mut world: ResMut<PhysicsWorld>,
) {
    let mut pairs = Vec::new();
    for (entity, filter) in &filters {
        let Some(a) = world.entity_to_body.get(&entity).copied() else {
            continue;
        };
        for other in &filter.filtered {
            if let Some(b) = world.entity_to_body.get(other).copied() {
                pairs.push((a, b));
            }
        }
    }
    for (a, b) in pairs {
        world.filter_pair(a, b);
    }
}
