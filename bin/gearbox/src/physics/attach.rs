//! From usd_bevy's physics components to the adapter's markers. usd_bevy
//! projects every UsdPhysics opinion in authored units with prim paths for
//! relationships; this step converts units into SI and Bevy axes, resolves
//! paths to entities through the instance's prim map, and leaves the marker
//! components `bodies`, `colliders` and `joints` consume.

use std::collections::HashMap;
use std::f32::consts::PI;

use bevy::ecs::entity::Entity;
use bevy::ecs::hierarchy::Children;
use bevy::ecs::world::World;
use bevy::math::{Quat, Vec3};
use bevy::mesh::Mesh3d;
use openusd::sdf::Value;
use openusd::usd::Stage;
use usd_bevy::route::physics as up;

use super::markers::*;

/// Stage-level conversion factors, read once per stage and threaded through
/// every conversion.
#[derive(Debug, Clone, Copy)]
pub struct StageMeta {
    /// Linear unit conversion: scene units × this = metres.
    pub meters_per_unit: f32,
    /// Mass unit conversion: scene units × this = kilograms.
    pub kilograms_per_unit: f32,
    /// Rotation that takes USD-native vectors into Bevy world space
    /// (identity for Y-up stages; -π/2 about X for Z-up).
    pub basis_rotation: Quat,
}

impl Default for StageMeta {
    fn default() -> Self {
        Self {
            meters_per_unit: 1.0,
            kilograms_per_unit: 1.0,
            basis_rotation: Quat::IDENTITY,
        }
    }
}

/// Collected from the stage's metadata before the walk.
pub fn read_stage_meta(stage: &Stage) -> StageMeta {
    let scalar = |key: &str| match stage.stage_metadata(key).ok().flatten() {
        Some(Value::Double(d)) => Some(d as f32),
        Some(Value::Float(f)) => Some(f),
        Some(Value::Int(i)) => Some(i as f32),
        Some(Value::Int64(i)) => Some(i as f32),
        _ => None,
    };
    let up_axis = match stage.stage_metadata("upAxis").ok().flatten() {
        Some(Value::Token(t)) => Some(t.as_str().to_string()),
        Some(Value::String(s)) => Some(s),
        _ => None,
    };
    // Match the projection's default: an unauthored metersPerUnit is USD's
    // centimetres.
    let meters_per_unit = std::env::var("BEVY_OPENUSD_METERS_PER_UNIT")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
        .or(scalar("metersPerUnit"))
        .unwrap_or(0.01);
    let kilograms_per_unit = scalar("kilogramsPerUnit").unwrap_or(1.0);
    let basis_rotation = match up_axis.as_deref() {
        Some("Z") => Quat::from_rotation_x(-core::f32::consts::FRAC_PI_2),
        _ => Quat::IDENTITY,
    };
    StageMeta {
        meters_per_unit,
        kilograms_per_unit,
        basis_rotation,
    }
}

/// Relationship targets still to resolve once every prim entity is known.
#[derive(Default)]
pub struct PendingPhysics {
    /// Joint entities awaiting body0/body1 path → Entity resolution.
    pub joints: Vec<(Entity, Option<String>, Option<String>)>,
    /// (group entity, member prim paths, filtered group prim paths)
    pub collision_groups: Vec<(Entity, Vec<String>, Vec<String>)>,
    /// (filter entity, filtered prim paths)
    pub filtered_pairs: Vec<(Entity, Vec<String>)>,
    /// (collider entity, bound material prim path)
    pub colliders_with_material: Vec<(Entity, String)>,
    /// ArticulationRoot entities — populated with joint subtree post-pass.
    pub articulation_roots: Vec<Entity>,
}

/// Convert whatever usd_bevy put on `entity` into the adapter's markers.
pub fn attach_physics_to_entity(
    world: &mut World,
    entity: Entity,
    pending: &mut PendingPhysics,
    meta: &StageMeta,
) {
    if let Some(scene) = world.get::<up::UsdPhysicsScene>(entity).cloned() {
        let dir = scene
            .gravity_direction
            .map(|d| meta.basis_rotation * d)
            .unwrap_or(Vec3::NEG_Y);
        // The spec says scene units per second squared, but authored scenes
        // write Earth gravity in m/s² whatever their metersPerUnit is.
        world.entity_mut(entity).insert(UsdPhysicsScene {
            gravity_direction: dir.normalize_or_zero(),
            gravity_magnitude: scene.gravity_magnitude.unwrap_or(9.81),
        });
    }

    if let Some(body) = world.get::<up::UsdRigidBody>(entity).cloned() {
        world.entity_mut(entity).insert(UsdRigidBody {
            kinematic: body.kinematic,
            enabled: body.enabled,
            starts_asleep: body.starts_asleep,
            velocity: body
                .velocity
                .map(|v| v * meta.meters_per_unit)
                .unwrap_or(Vec3::ZERO),
            angular_velocity: body
                .angular_velocity
                .map(|v| v * (PI / 180.0))
                .unwrap_or(Vec3::ZERO),
            simulation_owner: body.simulation_owner,
        });
    }

    if let Some(mass) = world.get::<up::UsdMass>(entity).copied() {
        world.entity_mut(entity).insert(UsdMass {
            mass: mass.mass.map(|m| m * meta.kilograms_per_unit),
            density: mass
                .density
                .map(|d| d * meta.kilograms_per_unit / meta.meters_per_unit.powi(3)),
            center_of_mass: mass.center_of_mass.map(|c| c * meta.meters_per_unit),
            diagonal_inertia: mass
                .diagonal_inertia
                .map(|i| i * meta.kilograms_per_unit * meta.meters_per_unit.powi(2)),
            principal_axes: mass.principal_axes,
        });
    }

    if let Some(collider) = world.get::<up::UsdCollider>(entity).cloned() {
        let shape = match collider.shape {
            up::UsdColliderShape::Cube { size } => UsdColliderShape::Cube { size },
            up::UsdColliderShape::Sphere { radius } => UsdColliderShape::Sphere { radius },
            up::UsdColliderShape::Capsule {
                radius,
                height,
                axis,
            } => UsdColliderShape::Capsule {
                radius,
                height,
                axis: axis_vector(&axis),
            },
            up::UsdColliderShape::Cylinder {
                radius,
                height,
                axis,
            }
            | up::UsdColliderShape::Cone {
                radius,
                height,
                axis,
            } => UsdColliderShape::Cylinder {
                radius,
                height,
                axis: axis_vector(&axis),
            },
            up::UsdColliderShape::Mesh => UsdColliderShape::Mesh,
            up::UsdColliderShape::Plane => UsdColliderShape::Plane,
            up::UsdColliderShape::Other => {
                if has_mesh_descendant(world, entity) {
                    UsdColliderShape::Mesh
                } else {
                    UsdColliderShape::Cube { size: 1.0 }
                }
            }
        };
        if let Some(material) = &collider.material {
            pending
                .colliders_with_material
                .push((entity, material.clone()));
        }
        world.entity_mut(entity).insert(UsdCollider {
            shape,
            enabled: collider.enabled,
            approximation: collider.approximation.as_deref().map(approximation),
            physics_material: None,
            simulation_owner: collider.simulation_owner,
        });
    }

    if let Some(material) = world.get::<up::UsdPhysicsMaterial>(entity).cloned() {
        world.entity_mut(entity).insert(UsdPhysicsMaterial {
            static_friction: material.static_friction,
            dynamic_friction: material.dynamic_friction,
            restitution: material.restitution,
            density: material
                .density
                .map(|d| d * meta.kilograms_per_unit / meta.meters_per_unit.powi(3)),
        });
    }

    if world.get::<up::UsdArticulationRoot>(entity).is_some() {
        world
            .entity_mut(entity)
            .insert(UsdArticulationRoot::default());
        pending.articulation_roots.push(entity);
    }

    if let Some(filter) = world.get::<up::UsdCollisionFilter>(entity).cloned() {
        world
            .entity_mut(entity)
            .insert(UsdCollisionFilter::default());
        pending.filtered_pairs.push((entity, filter.filtered));
    }

    if let Some(group) = world.get::<up::UsdCollisionGroup>(entity).cloned() {
        world.entity_mut(entity).insert(UsdCollisionGroup {
            members: Vec::new(),
            filtered: Vec::new(),
            merge_group: group.merge_group,
            invert_filtered_groups: group.invert_filtered_groups,
        });
        pending
            .collision_groups
            .push((entity, group.members, group.filtered_groups));
    }

    if let Some(joint) = world.get::<up::UsdJoint>(entity).cloned() {
        let kind = match joint.kind.as_str() {
            "fixed" => UsdJointKind::Fixed,
            "revolute" => UsdJointKind::Revolute,
            "prismatic" => UsdJointKind::Prismatic,
            "spherical" => UsdJointKind::Spherical,
            "distance" => UsdJointKind::Distance,
            _ => UsdJointKind::Generic,
        };
        let built_in_limit = match (kind, joint.lower, joint.upper) {
            (UsdJointKind::Revolute, Some(lo), Some(hi)) => {
                Some((lo.to_radians(), hi.to_radians()))
            }
            (UsdJointKind::Prismatic, Some(lo), Some(hi)) => {
                Some((lo * meta.meters_per_unit, hi * meta.meters_per_unit))
            }
            _ => None,
        };
        let cone_limit = match (joint.cone_angle0, joint.cone_angle1) {
            (Some(a), Some(b)) => Some((a.to_radians(), b.to_radians())),
            _ => None,
        };
        let distance_limit = match (joint.min_distance, joint.max_distance) {
            (Some(lo), Some(hi)) => Some((lo * meta.meters_per_unit, hi * meta.meters_per_unit)),
            _ => None,
        };
        let limits = world
            .get::<up::UsdLimits>(entity)
            .map(|l| l.0.iter().map(|l| convert_limit(l, meta)).collect())
            .unwrap_or_default();
        let drives = world
            .get::<up::UsdDrives>(entity)
            .map(|d| d.0.iter().map(|d| convert_drive(d, meta)).collect())
            .unwrap_or_default();
        world.entity_mut(entity).insert(UsdPhysicsJoint {
            kind,
            body0: None,
            body1: None,
            local_pos0: joint.local_pos0 * meta.meters_per_unit,
            local_rot0: joint.local_rot0,
            local_pos1: joint.local_pos1 * meta.meters_per_unit,
            local_rot1: joint.local_rot1,
            axis: axis_vector(joint.axis.as_deref().unwrap_or("X")),
            joint_enabled: joint.enabled,
            collision_enabled: joint.collision_enabled,
            exclude_from_articulation: joint.exclude_from_articulation,
            break_force: joint.break_force,
            break_torque: joint.break_torque,
            built_in_limit,
            cone_limit,
            distance_limit,
            limits,
            drives,
        });
        pending.joints.push((entity, joint.body0, joint.body1));
    }
}

/// Substitute every prim path with its entity: joint bodies, collision
/// groups, filtered pairs and collider material bindings.
pub fn resolve_pending_physics(
    world: &mut World,
    pending: &PendingPhysics,
    prim_paths: &HashMap<String, Entity>,
) {
    for (joint_entity, body0_path, body1_path) in &pending.joints {
        let body0 = body0_path.as_ref().and_then(|p| prim_paths.get(p).copied());
        let body1 = body1_path.as_ref().and_then(|p| prim_paths.get(p).copied());
        if let Ok(mut entity_mut) = world.get_entity_mut(*joint_entity)
            && let Some(mut joint) = entity_mut.get_mut::<UsdPhysicsJoint>()
        {
            joint.body0 = body0;
            joint.body1 = body1;
        }
    }

    for (group_entity, members, filtered) in &pending.collision_groups {
        let members: Vec<Entity> = members
            .iter()
            .filter_map(|p| prim_paths.get(p).copied())
            .collect();
        let filtered: Vec<Entity> = filtered
            .iter()
            .filter_map(|p| prim_paths.get(p).copied())
            .collect();
        if let Ok(mut entity_mut) = world.get_entity_mut(*group_entity)
            && let Some(mut group) = entity_mut.get_mut::<UsdCollisionGroup>()
        {
            group.members = members;
            group.filtered = filtered;
        }
    }

    for (filter_entity, paths) in &pending.filtered_pairs {
        let resolved: Vec<Entity> = paths
            .iter()
            .filter_map(|p| prim_paths.get(p).copied())
            .collect();
        if let Ok(mut entity_mut) = world.get_entity_mut(*filter_entity)
            && let Some(mut filt) = entity_mut.get_mut::<UsdCollisionFilter>()
        {
            filt.filtered = resolved;
        }
    }

    for (collider_entity, mat_path) in &pending.colliders_with_material {
        let mat_entity = prim_paths.get(mat_path).copied();
        if let Ok(mut entity_mut) = world.get_entity_mut(*collider_entity)
            && let Some(mut col) = entity_mut.get_mut::<UsdCollider>()
        {
            col.physics_material = mat_entity;
        }
    }
}

/// For each articulation root, collect every enabled joint below it that is
/// not excluded from the articulation.
pub fn populate_articulation_joints(world: &mut World, articulation_roots: &[Entity]) {
    for root in articulation_roots {
        let mut joints = Vec::new();
        collect_joints_recursive(world, *root, &mut joints);
        if let Ok(mut entity_mut) = world.get_entity_mut(*root)
            && let Some(mut ar) = entity_mut.get_mut::<UsdArticulationRoot>()
        {
            ar.joints = joints;
        }
    }
}

fn collect_joints_recursive(world: &World, entity: Entity, out: &mut Vec<Entity>) {
    if let Some(joint) = world.get::<UsdPhysicsJoint>(entity)
        && !joint.exclude_from_articulation
        && joint.joint_enabled
    {
        out.push(entity);
    }
    if let Some(children) = world.get::<Children>(entity) {
        let child_entities: Vec<Entity> = children.iter().copied().collect();
        for child in child_entities {
            collect_joints_recursive(world, child, out);
        }
    }
}

fn has_mesh_descendant(world: &World, entity: Entity) -> bool {
    let Some(children) = world.get::<Children>(entity) else {
        return false;
    };
    let children: Vec<Entity> = children.iter().copied().collect();
    children
        .into_iter()
        .any(|child| world.get::<Mesh3d>(child).is_some() || has_mesh_descendant(world, child))
}

fn axis_vector(token: &str) -> Vec3 {
    match token {
        "X" => Vec3::X,
        "Y" => Vec3::Y,
        _ => Vec3::Z,
    }
}

fn approximation(token: &str) -> UsdCollisionApprox {
    match token {
        "convexHull" => UsdCollisionApprox::ConvexHull,
        "convexDecomposition" => UsdCollisionApprox::ConvexDecomposition,
        "boundingSphere" => UsdCollisionApprox::BoundingSphere,
        "boundingCube" => UsdCollisionApprox::BoundingCube,
        "meshSimplification" => UsdCollisionApprox::MeshSimplification,
        _ => UsdCollisionApprox::None,
    }
}

fn dof(token: &str) -> Option<UsdDof> {
    Some(match token {
        "transX" => UsdDof::TransX,
        "transY" => UsdDof::TransY,
        "transZ" => UsdDof::TransZ,
        "rotX" => UsdDof::RotX,
        "rotY" => UsdDof::RotY,
        "rotZ" => UsdDof::RotZ,
        "linear" => UsdDof::Linear,
        "angular" => UsdDof::Angular,
        "distance" => UsdDof::Distance,
        _ => return None,
    })
}

fn dof_is_rotational(d: UsdDof) -> bool {
    matches!(
        d,
        UsdDof::RotX | UsdDof::RotY | UsdDof::RotZ | UsdDof::Angular
    )
}

fn convert_limit(l: &up::UsdLimit, meta: &StageMeta) -> UsdJointLimit {
    let dof = dof(&l.dof).unwrap_or_default();
    let (low, high) = (l.low.unwrap_or(0.0), l.high.unwrap_or(0.0));
    let (low, high) = if dof_is_rotational(dof) {
        (low.to_radians(), high.to_radians())
    } else {
        (low * meta.meters_per_unit, high * meta.meters_per_unit)
    };
    UsdJointLimit { dof, low, high }
}

fn convert_drive(d: &up::UsdDrive, meta: &StageMeta) -> UsdJointDrive {
    let dof = dof(&d.dof).unwrap_or_default();
    let rot = dof_is_rotational(dof);
    let pos_scale = if rot {
        PI / 180.0
    } else {
        meta.meters_per_unit
    };
    let dyn_scale = if rot {
        180.0 / PI
    } else {
        1.0 / meta.meters_per_unit
    };
    UsdJointDrive {
        dof,
        drive_type: match d.drive_type.as_deref() {
            Some("acceleration") => UsdDriveType::Acceleration,
            _ => UsdDriveType::Force,
        },
        target_position: d.target_position.map(|v| v * pos_scale),
        target_velocity: d.target_velocity.map(|v| v * pos_scale),
        damping: d.damping.unwrap_or(0.0) * dyn_scale,
        stiffness: d.stiffness.unwrap_or(0.0) * dyn_scale,
        max_force: d.max_force,
    }
}
