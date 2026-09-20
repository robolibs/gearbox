//! `UsdPhysicsJoint` → joints in `PhysicsWorld`.
//!
//! Revolute and prismatic joints move about the backend's joint X; the
//! authored axis rides in `JointKind`, so motors and limits always address
//! `AngX` / `LinX` whatever the USD axis token was.

use super::backend::{JointAxis, JointDesc, JointKind, MotorDesc, MotorModel, MotorTarget, Pose};
use super::convert::{quat_to_d, vec3_to_d};
use super::markers::{UsdArticulationRoot, UsdDof, UsdJointDrive, UsdJointKind, UsdPhysicsJoint};
use bevy::prelude::*;
use glam::DVec3;

use super::backend::BodyKind;
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
    // Reduced-coordinate joints are opt-in (`GEARBOX_MULTIBODY=1`): rapier
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
            if let Some(target) = joint.body1.or(joint.body0)
                && let Some(handle) = world.entity_to_body.get(&target).copied()
                && let Some(b) = world.body_mut(handle)
            {
                b.set_kind(BodyKind::Fixed, false);
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

        if let Some(mut desc) = joint_desc(joint) {
            desc.loop_closure = joint.exclude_from_articulation;
            desc.reduced = multibody
                && articulated(body0_e)
                && articulated(body1_e)
                && !joint.exclude_from_articulation;
            world.insert_joint(body0, body1, desc);
        }
        commands.entity(joint_entity).insert(JointAttached);
    }
}

/// `None` for the joint kinds no backend path exists for yet.
fn joint_desc(j: &UsdPhysicsJoint) -> Option<JointDesc> {
    let frame1 = Pose::new(vec3_to_d(j.local_pos0), quat_to_d(j.local_rot0));
    let frame2 = Pose::new(vec3_to_d(j.local_pos1), quat_to_d(j.local_rot1));
    let axis = if j.axis.y.abs() > 0.9 {
        DVec3::Y
    } else if j.axis.z.abs() > 0.9 {
        DVec3::Z
    } else {
        DVec3::X
    };
    let (kind, free_axis): (JointKind, fn(UsdDof) -> bool) = match j.kind {
        UsdJointKind::Revolute => (JointKind::Revolute { axis }, dof_is_angular),
        UsdJointKind::Prismatic => (JointKind::Prismatic { axis }, dof_is_linear),
        // Anchors only: fixed and spherical joints ignore the authored
        // frame rotations.
        UsdJointKind::Fixed => {
            return Some(JointDesc::new(
                JointKind::Fixed,
                Pose::from_translation(frame1.translation),
                Pose::from_translation(frame2.translation),
            ));
        }
        UsdJointKind::Spherical => {
            return Some(JointDesc::new(JointKind::Spherical, frame1, frame2));
        }
        UsdJointKind::Distance => {
            warn!("gearbox-physics: PhysicsDistanceJoint not yet supported; skipping");
            return None;
        }
        UsdJointKind::Generic => {
            warn!(
                "gearbox-physics: generic D6 joint not yet implemented; skipping ({} limits, {} drives)",
                j.limits.len(),
                j.drives.len()
            );
            return None;
        }
    };
    let motor_axis = match kind {
        JointKind::Prismatic { .. } => JointAxis::LinX,
        _ => JointAxis::AngX,
    };
    let mut desc = JointDesc::new(kind, frame1, frame2);
    if let Some((lo, hi)) = j.built_in_limit {
        desc.limits.push((motor_axis, [lo as f64, hi as f64]));
    }
    if let Some(drive) = j.drives.iter().find(|d| free_axis(d.dof)) {
        // Joints whose frames share a basis have always run force-based
        // drives; the differing-basis path kept the backend's default.
        let same_basis = frame1.rotation.abs_diff_eq(frame2.rotation, 1e-4);
        desc.motors.push(motor_desc(motor_axis, drive, same_basis));
    }
    Some(desc)
}

fn motor_desc(axis: JointAxis, d: &UsdJointDrive, force_based: bool) -> MotorDesc {
    let target = if let Some(target) = d.target_position {
        Some(MotorTarget::Position {
            target: target as f64,
            stiffness: d.stiffness as f64,
            damping: d.damping as f64,
        })
    } else {
        d.target_velocity.map(|velocity| MotorTarget::Velocity {
            target: velocity as f64,
            damping: d.damping as f64,
        })
    };
    MotorDesc {
        axis,
        target,
        max_force: d.max_force.map(|f| f as f64),
        model: force_based.then_some(MotorModel::Force),
    }
}

fn dof_is_angular(dof: UsdDof) -> bool {
    matches!(
        dof,
        UsdDof::Angular | UsdDof::RotX | UsdDof::RotY | UsdDof::RotZ
    )
}

fn dof_is_linear(dof: UsdDof) -> bool {
    matches!(
        dof,
        UsdDof::Linear | UsdDof::TransX | UsdDof::TransY | UsdDof::TransZ
    )
}
