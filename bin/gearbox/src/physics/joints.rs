//! `UsdPhysicsJoint` → joints in `PhysicsWorld`.
//!
//! Revolute and prismatic joints move about the backend's joint X; the
//! authored axis rides in `JointKind`, so motors and limits always address
//! `AngX` / `LinX` whatever the USD axis token was.

use super::backend::{JointAxes, JointAxis, JointDesc, JointKind, MotorDesc, MotorModel, MotorTarget, Pose};
use super::convert::{quat_to_d, vec3_to_d};
use super::markers::{UsdDof, UsdDriveType, UsdJointDrive, UsdJointKind, UsdPhysicsJoint};
use bevy::prelude::*;
use glam::DVec3;

use super::backend::BodyKind;
use super::world::{AuthoredDrive, PhysicsWorld};

#[derive(Component)]
pub(crate) struct JointAttached;

pub fn convert_joints(
    mut commands: Commands,
    mut world: ResMut<PhysicsWorld>,
    joints: Query<(Entity, &UsdPhysicsJoint), Without<JointAttached>>,
) {
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
            desc.softness = joint.softness;
            let id = world.insert_joint(body0, body1, desc);
            world.entity_to_joint.insert(joint_entity, id);
            if let Some(drive) = authored_drive(joint) {
                world.authored_drives.insert(id, drive);
            }
            let axis = match joint.kind {
                UsdJointKind::Revolute => Some(JointAxis::AngX),
                UsdJointKind::Prismatic => Some(JointAxis::LinX),
                _ => None,
            };
            if let Some(axis) = axis.filter(|_| joint.friction.is_some() || joint.rolling.is_some()) {
                world.add_joint_friction(id, axis, joint.friction.unwrap_or(0.0), joint.rolling);
            }
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
    let mut desc = match j.kind {
        UsdJointKind::Revolute | UsdJointKind::Prismatic => {
            let (kind, free_axis, motor_axis): (JointKind, fn(UsdDof) -> bool, JointAxis) =
                if j.kind == UsdJointKind::Revolute {
                    (JointKind::Revolute { axis }, dof_is_angular, JointAxis::AngX)
                } else {
                    (JointKind::Prismatic { axis }, dof_is_linear, JointAxis::LinX)
                };
            let mut desc = JointDesc::new(kind, frame1, frame2);
            if let Some(range) = j.built_in_limit.and_then(|(lo, hi)| finite_range(lo, hi)) {
                desc.limits.push((motor_axis, range));
            }
            if let Some(drive) = j.drives.iter().find(|d| free_axis(d.dof)) {
                desc.motors.push(motor_desc(motor_axis, drive));
            }
            desc
        }
        UsdJointKind::Fixed => JointDesc::new(JointKind::Fixed, frame1, frame2),
        UsdJointKind::Spherical => JointDesc::new(JointKind::Spherical, frame1, frame2),
        UsdJointKind::Distance => {
            warn!("gearbox-physics: PhysicsDistanceJoint needs Molla's distance limits; skipping");
            return None;
        }
        UsdJointKind::Generic => generic_desc(j, frame1, frame2),
    };
    desc.contacts_enabled = j.collision_enabled;
    Some(desc)
}

/// A `PhysicsJoint` (D6): each axis of the joint frame is free, limited by
/// its `PhysicsLimitAPI:<axis>`, or locked when that limit's low passes its
/// high; a `PhysicsDriveAPI:<axis>` drives a free or limited axis.
fn generic_desc(j: &UsdPhysicsJoint, frame1: Pose, frame2: Pose) -> JointDesc {
    let mut locked = JointAxes::NONE;
    let mut limits = Vec::new();
    for limit in &j.limits {
        let Some(axis) = frame_axis(limit.dof) else {
            warn!("gearbox-physics: D6 limit on {:?} has no frame axis; ignored", limit.dof);
            continue;
        };
        if limit.low > limit.high {
            locked = locked.with(axis);
        } else if let Some(range) = finite_range(limit.low, limit.high) {
            limits.push((axis, range));
        }
    }
    let mut desc = JointDesc::new(JointKind::Generic { locked }, frame1, frame2);
    desc.limits = limits;
    desc.motors = j
        .drives
        .iter()
        .filter_map(|d| frame_axis(d.dof).filter(|axis| !locked.contains(*axis)).map(|axis| motor_desc(axis, d)))
        .collect();
    desc
}

/// The joint-frame axis a six-axis USD DOF token names.
fn frame_axis(dof: UsdDof) -> Option<JointAxis> {
    Some(match dof {
        UsdDof::TransX => JointAxis::LinX,
        UsdDof::TransY => JointAxis::LinY,
        UsdDof::TransZ => JointAxis::LinZ,
        UsdDof::RotX => JointAxis::AngX,
        UsdDof::RotY => JointAxis::AngY,
        UsdDof::RotZ => JointAxis::AngZ,
        UsdDof::Linear | UsdDof::Angular | UsdDof::Distance => return None,
    })
}

/// Molla takes finite limits only: an open side stands far out, and a
/// range open on both sides is no limit at all.
fn finite_range(low: f32, high: f32) -> Option<[f64; 2]> {
    const OPEN: f64 = 1.0e6;
    let (low, high) = (low as f64, high as f64);
    (low.is_finite() || high.is_finite()).then(|| [low.max(-OPEN), high.min(OPEN)])
}

/// The drive on a revolute or prismatic joint's free axis.
fn authored_drive(j: &UsdPhysicsJoint) -> Option<AuthoredDrive> {
    let free_axis: fn(UsdDof) -> bool = match j.kind {
        UsdJointKind::Revolute => dof_is_angular,
        UsdJointKind::Prismatic => dof_is_linear,
        _ => return None,
    };
    let d = j.drives.iter().find(|d| free_axis(d.dof))?;
    Some(AuthoredDrive {
        stiffness: d.stiffness as f64,
        damping: d.damping as f64,
        max_force: d.max_force.map(|f| f as f64).filter(|f| f.is_finite()),
        force: matches!(d.drive_type, UsdDriveType::Force),
        target_position: d.target_position.map(f64::from),
        target_velocity: d.target_velocity.map(f64::from),
    })
}

fn motor_desc(axis: JointAxis, d: &UsdJointDrive) -> MotorDesc {
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
        model: Some(match d.drive_type {
            UsdDriveType::Force => MotorModel::Force,
            UsdDriveType::Acceleration => MotorModel::Acceleration,
        }),
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

#[cfg(test)]
mod tests {
    use super::super::backend::{BodyDesc, Inertia, MassProps};
    use super::super::markers::UsdJointLimit;
    use super::*;

    #[test]
    fn a_fixed_joint_keeps_the_rotation_of_its_authored_frames() {
        let turn = Quat::from_rotation_x(2.64);
        let joint = UsdPhysicsJoint { kind: UsdJointKind::Fixed, local_rot0: turn, ..Default::default() };
        let desc = joint_desc(&joint).unwrap();
        assert!(desc.frame1.rotation.abs_diff_eq(quat_to_d(turn), 1e-9), "{:?}", desc.frame1);
        assert!(desc.frame2.rotation.abs_diff_eq(glam::DQuat::IDENTITY, 1e-9), "{:?}", desc.frame2);
    }

    #[test]
    fn a_joint_drive_runs_on_its_authored_model_whatever_its_frames() {
        for (kind, model) in [(UsdDriveType::Force, MotorModel::Force), (UsdDriveType::Acceleration, MotorModel::Acceleration)] {
            let drive = UsdJointDrive { dof: UsdDof::RotX, drive_type: kind, stiffness: 10.0, target_position: Some(0.0), ..Default::default() };
            let joint = UsdPhysicsJoint {
                kind: UsdJointKind::Revolute,
                local_rot0: Quat::from_rotation_x(0.3),
                drives: vec![drive],
                ..Default::default()
            };
            assert_eq!(joint_desc(&joint).unwrap().motors[0].model, Some(model));
        }
    }

    fn limit(dof: UsdDof, low: f32, high: f32) -> UsdJointLimit {
        UsdJointLimit { dof, low, high }
    }

    /// The translations locked, Z limited to ±0.5 rad, Y open below 0.25 rad.
    fn d6_hinge() -> UsdPhysicsJoint {
        UsdPhysicsJoint {
            kind: UsdJointKind::Generic,
            local_pos1: Vec3::new(-1.0, 0.0, 0.0),
            limits: vec![
                limit(UsdDof::TransX, 1.0, -1.0),
                limit(UsdDof::TransY, 1.0, -1.0),
                limit(UsdDof::TransZ, 1.0, -1.0),
                limit(UsdDof::RotX, 1.0, -1.0),
                limit(UsdDof::RotY, f32::NEG_INFINITY, 0.25),
                limit(UsdDof::RotZ, -0.5, 0.5),
            ],
            drives: vec![
                UsdJointDrive { dof: UsdDof::RotZ, damping: 2.0, target_velocity: Some(0.0), ..Default::default() },
                UsdJointDrive { dof: UsdDof::TransX, stiffness: 5.0, target_position: Some(0.0), ..Default::default() },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn a_d6_joint_locks_limits_and_drives_its_authored_axes() {
        let d6 = joint_desc(&d6_hinge()).unwrap();
        assert_eq!(d6.kind, JointKind::Generic { locked: JointAxes::LIN.with(JointAxis::AngX) });
        assert_eq!(d6.limits, vec![(JointAxis::AngY, [-1.0e6, 0.25]), (JointAxis::AngZ, [-0.5, 0.5])]);
        assert_eq!(d6.motors.iter().map(|m| m.axis).collect::<Vec<_>>(), vec![JointAxis::AngZ]);
    }

    #[test]
    fn a_joint_lets_its_bodies_collide_only_when_authored() {
        let mut pin = UsdPhysicsJoint { kind: UsdJointKind::Fixed, ..Default::default() };
        assert!(!joint_desc(&pin).unwrap().contacts_enabled);
        pin.collision_enabled = true;
        assert!(joint_desc(&pin).unwrap().contacts_enabled);
    }

    /// A 1 m arm on the D6 hinge falls under gravity to its −0.5 rad stop.
    #[test]
    fn a_d6_hinge_swings_down_to_its_limit() {
        let mut world = PhysicsWorld::default();
        let anchor = world.insert_body(BodyDesc::fixed());
        let mut arm = BodyDesc::dynamic().pose(Pose::from_translation(DVec3::X));
        arm.additional_mass = Some(MassProps {
            mass: 1.0,
            local_com: DVec3::ZERO,
            inertia: Inertia::Principal(DVec3::splat(0.01)),
        });
        let arm = world.insert_body(arm);
        world.insert_joint(anchor, arm, joint_desc(&d6_hinge()).unwrap());
        for _ in 0..480 {
            world.step();
        }
        let pose = world.body(arm).unwrap().position();
        let angle = 2.0 * pose.rotation.z.atan2(pose.rotation.w);
        assert!((angle + 0.5).abs() < 0.02, "{angle}");
        assert!(pose.translation.distance(DVec3::new(0.5_f64.cos(), -(0.5_f64.sin()), 0.0)) < 0.02, "{pose:?}");
    }
}
