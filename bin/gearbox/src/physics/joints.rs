//! `UsdPhysicsJoint` → entries in `PhysicsWorld.{multibody,impulse}_joints`.
//! Bevy ECS adapter; all joint construction lives in `super::rapier::joints`.

use super::markers::{UsdArticulationRoot, UsdDof, UsdDriveType, UsdJointKind, UsdPhysicsJoint};
use super::rapier::joints::build_and_insert_joint;
use super::reader::{Dof, JointKind, ReadDrive, ReadJoint, ReadLimit};
use bevy::prelude::*;
use rapier3d::prelude::*;

use super::world::PhysicsWorld;

#[derive(Component)]
pub(crate) struct JointAttached;

pub fn convert_joints(
    mut commands: Commands,
    mut world: ResMut<PhysicsWorld>,
    joints: Query<(Entity, &UsdPhysicsJoint), Without<JointAttached>>,
    articulation_roots: Query<(), With<UsdArticulationRoot>>,
    parents: Query<&ChildOf>,
) {
    if joints.is_empty() {
        return;
    }
    // Featherstone multibodies are opt-in (`GEARBOX_MULTIBODY=1`): rapier
    // 0.32 indexes past its Jacobian table when a multibody is assembled
    // over several frames, and has no two-axis joint there. With it on, a
    // joint joins one only when its own bodies sit under an articulation
    // root, so one articulated machine cannot convert everyone else's.
    let multibody = std::env::var_os("GEARBOX_MULTIBODY").is_some_and(|v| v == "1");
    let articulated = |e: Entity| {
        crate::physics::colliders::find_articulation_root_ancestor(
            e,
            parents.get(e).ok(),
            &articulation_roots,
            &parents,
        )
        .is_some()
    };

    for (joint_entity, joint) in &joints {
        if !joint.joint_enabled {
            commands.entity(joint_entity).insert(JointAttached);
            continue;
        }
        let (Some(body0_e), Some(body1_e)) = (joint.body0, joint.body1) else {
            // World-anchored joint — pin the referenced body to fixed.
            if let Some(target) = joint.body1.or(joint.body0) {
                if let Some(handle) = world.entity_to_body.get(&target).copied() {
                    if let Some(b) = world.bodies.get_mut(handle) {
                        b.set_body_type(RigidBodyType::Fixed, false);
                    }
                }
            }
            commands.entity(joint_entity).insert(JointAttached);
            continue;
        };
        let (Some(body0), Some(body1)) = (
            world.entity_to_body.get(&body0_e).copied(),
            world.entity_to_body.get(&body1_e).copied(),
        ) else {
            // Body entities not yet materialised; try again next frame.
            continue;
        };

        let use_multibody = multibody
            && articulated(body0_e)
            && articulated(body1_e)
            && !joint.exclude_from_articulation;
        let read_joint = bridge_to_read_joint(joint);
        if std::env::var_os("GEARBOX_PHYSICS_LOG").is_some() {
            info!(
                "gearbox-physics: joint {joint_entity:?} {:?} {body0_e:?}->{body1_e:?} pos0 {:?} rot0 {:?} pos1 {:?} rot1 {:?} axis {:?} limit {:?} excl {} drives {}",
                joint.kind,
                joint.local_pos0,
                joint.local_rot0,
                joint.local_pos1,
                joint.local_rot1,
                joint.axis,
                joint.built_in_limit,
                joint.exclude_from_articulation,
                joint.drives.len()
            );
        }

        let world_mut = world.as_mut();
        if let Ok(_) = build_and_insert_joint(
            &mut world_mut.multibody_joints,
            &mut world_mut.impulse_joints,
            &read_joint,
            body0,
            body1,
            use_multibody,
        ) {
            commands.entity(joint_entity).insert(JointAttached);
        }
    }
}

/// Bridge a Bevy-component `UsdPhysicsJoint` (Vec3/Quat fields) to
/// the upstream `super::reader::ReadJoint` ([f32; 3] / [f32; 4]
/// fields) the `usd_rapier` builder expects.
fn bridge_to_read_joint(j: &UsdPhysicsJoint) -> ReadJoint {
    let (lower, upper) = match j.built_in_limit {
        Some((lo, hi)) => (Some(lo), Some(hi)),
        None => (None, None),
    };
    let axis_token = if j.axis.x.abs() > 0.9 {
        Some("X".into())
    } else if j.axis.y.abs() > 0.9 {
        Some("Y".into())
    } else if j.axis.z.abs() > 0.9 {
        Some("Z".into())
    } else {
        None
    };
    ReadJoint {
        path: String::new(),
        kind: kind_to_openusd(j.kind),
        body0: None,
        body1: None,
        local_pos0: [j.local_pos0.x, j.local_pos0.y, j.local_pos0.z],
        // USD authors quat as (w, x, y, z); Bevy Quat is (x, y, z, w).
        local_rot0: [
            j.local_rot0.w,
            j.local_rot0.x,
            j.local_rot0.y,
            j.local_rot0.z,
        ],
        local_pos1: [j.local_pos1.x, j.local_pos1.y, j.local_pos1.z],
        local_rot1: [
            j.local_rot1.w,
            j.local_rot1.x,
            j.local_rot1.y,
            j.local_rot1.z,
        ],
        axis: axis_token,
        lower_limit: lower,
        upper_limit: upper,
        collision_enabled: j.collision_enabled,
        joint_enabled: j.joint_enabled,
        exclude_from_articulation: j.exclude_from_articulation,
        break_force: j.break_force,
        break_torque: j.break_torque,
        min_distance: j.distance_limit.map(|(lo, _)| lo),
        max_distance: j.distance_limit.map(|(_, hi)| hi),
        cone_angle_0: j.cone_limit.map(|(a, _)| a),
        cone_angle_1: j.cone_limit.map(|(_, b)| b),
        limits: j
            .limits
            .iter()
            .map(|l| ReadLimit {
                dof: dof_to_openusd(l.dof),
                low: l.low,
                high: l.high,
            })
            .collect(),
        drives: j
            .drives
            .iter()
            .map(|d| ReadDrive {
                dof: dof_to_openusd(d.dof),
                drive_type: drive_type_to_openusd(d.drive_type),
                target_position: d.target_position,
                target_velocity: d.target_velocity,
                stiffness: d.stiffness,
                damping: d.damping,
                max_force: d.max_force,
            })
            .collect(),
    }
}

fn kind_to_openusd(k: UsdJointKind) -> JointKind {
    match k {
        UsdJointKind::Fixed => JointKind::Fixed,
        UsdJointKind::Revolute => JointKind::Revolute,
        UsdJointKind::Prismatic => JointKind::Prismatic,
        UsdJointKind::Spherical => JointKind::Spherical,
        UsdJointKind::Distance => JointKind::Distance,
        UsdJointKind::Generic => JointKind::Generic,
    }
}

fn dof_to_openusd(d: UsdDof) -> Dof {
    match d {
        UsdDof::TransX => Dof::TransX,
        UsdDof::TransY => Dof::TransY,
        UsdDof::TransZ => Dof::TransZ,
        UsdDof::RotX => Dof::RotX,
        UsdDof::RotY => Dof::RotY,
        UsdDof::RotZ => Dof::RotZ,
        UsdDof::Linear => Dof::Linear,
        UsdDof::Angular => Dof::Angular,
        UsdDof::Distance => Dof::Distance,
    }
}

fn drive_type_to_openusd(t: UsdDriveType) -> super::reader::DriveType {
    match t {
        UsdDriveType::Acceleration => super::reader::DriveType::Acceleration,
        UsdDriveType::Force => super::reader::DriveType::Force,
    }
}
