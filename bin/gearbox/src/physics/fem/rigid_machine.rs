use super::machine::FemMachineLayout;
use crate::physics::markers::{
    UsdDof, UsdDriveType, UsdJointKind, UsdMass, UsdPhysicsJoint, UsdRigidBody,
};
use bevy::prelude::{Entity, GlobalTransform, World};
use molla_core::{BodyId, Error, JointId, Result, WorldId};
use molla_math::{Mat3, Quat, Transform, Vec3};
use molla_sim::{BodyParams, Control, JointTargetMode, Model, ModelBuilder, State, eval_fk};
use std::collections::HashMap;

pub(crate) struct LoopConstraint {
    pub entity: Entity,
    pub parent: BodyId,
    pub child: BodyId,
    pub joint: UsdPhysicsJoint,
}

/// Authored rest articulation and retained loop constraints, before GPU ownership transfer.
pub(crate) struct FemRigidMachine {
    pub model: Model,
    pub state: State,
    pub control: Control,
    pub bodies: HashMap<Entity, BodyId>,
    pub joints: HashMap<Entity, JointId>,
    pub loops: Vec<LoopConstraint>,
    pub origin: Vec3,
}

fn invalid(message: &str) -> Error {
    Error::Build(format!("FEM machine: {message}"))
}
fn vector(v: bevy::prelude::Vec3) -> Vec3 {
    Vec3::new(v.x as f64, v.y as f64, v.z as f64)
}
fn rotation(q: bevy::prelude::Quat) -> Result<Quat> {
    if !q.is_finite() || (q.length_squared() - 1.0).abs() > 1e-4 {
        return Err(invalid("non-unit or nonfinite rotation"));
    }
    Ok(Quat::from_xyzw(q.x as f64, q.y as f64, q.z as f64, q.w as f64).normalize())
}
fn pose(world: &World, entity: Entity) -> Result<Transform> {
    let global = world
        .get::<GlobalTransform>(entity)
        .ok_or_else(|| invalid("missing body transform"))?;
    let (scale, q, p) = global.to_scale_rotation_translation();
    if !scale.is_finite()
        || (scale - bevy::prelude::Vec3::ONE).abs().max_element() > 1e-5
        || !p.is_finite()
    {
        return Err(invalid("body transform is scaled or nonfinite"));
    }
    Ok(Transform {
        position: vector(p),
        rotation: rotation(q)?,
    })
}
fn frames(joint: &UsdPhysicsJoint) -> Result<(Transform, Transform)> {
    if !joint.local_pos0.is_finite() || !joint.local_pos1.is_finite() {
        return Err(invalid("nonfinite joint anchor"));
    }
    Ok((
        Transform {
            position: vector(joint.local_pos0),
            rotation: rotation(joint.local_rot0)?,
        },
        Transform {
            position: vector(joint.local_pos1),
            rotation: rotation(joint.local_rot1)?,
        },
    ))
}

fn drive_axis(joint: &UsdPhysicsJoint, dof: UsdDof) -> bool {
    use bevy::prelude::Vec3 as Axis;
    match (joint.kind, dof) {
        (UsdJointKind::Revolute, UsdDof::Angular) | (UsdJointKind::Prismatic, UsdDof::Linear) => {
            true
        }
        (UsdJointKind::Revolute, UsdDof::RotX) | (UsdJointKind::Prismatic, UsdDof::TransX) => {
            joint.axis == Axis::X
        }
        (UsdJointKind::Revolute, UsdDof::RotY) | (UsdJointKind::Prismatic, UsdDof::TransY) => {
            joint.axis == Axis::Y
        }
        (UsdJointKind::Revolute, UsdDof::RotZ) | (UsdJointKind::Prismatic, UsdDof::TransZ) => {
            joint.axis == Axis::Z
        }
        _ => false,
    }
}

impl FemRigidMachine {
    pub(crate) fn prepare(world: &World, layout: &FemMachineLayout) -> Result<Self> {
        let origin = pose(world, layout.chassis)?.position;
        let mut builder = ModelBuilder::new();
        builder.begin_articulation();
        builder.set_world_gravity(WorldId(0), -Vec3::Y * 9.81);
        let mut bodies = HashMap::new();
        let mut reference = Vec::new();
        for &entity in &layout.bodies {
            let body = world
                .get::<UsdRigidBody>(entity)
                .ok_or_else(|| invalid("body disappeared"))?;
            if !body.enabled
                || body.kinematic
                || body.velocity != bevy::prelude::Vec3::ZERO
                || body.angular_velocity != bevy::prelude::Vec3::ZERO
            {
                return Err(invalid(
                    "preparation requires enabled dynamic bodies at rest",
                ));
            }
            let mass = world
                .get::<UsdMass>(entity)
                .ok_or_else(|| invalid("missing authored mass properties"))?;
            let m = mass
                .mass
                .ok_or_else(|| invalid("missing explicit body mass"))?;
            let diagonal = mass
                .diagonal_inertia
                .ok_or_else(|| invalid("missing explicit body inertia"))?;
            let com = mass
                .center_of_mass
                .ok_or_else(|| invalid("missing explicit body COM"))?;
            if !m.is_finite()
                || m <= 0.0
                || !diagonal.is_finite()
                || diagonal.min_element() <= 0.0
                || !com.is_finite()
            {
                return Err(invalid("invalid mass, inertia or COM"));
            }
            let axes = Mat3::from_quat(rotation(mass.principal_axes.unwrap_or_default())?);
            let mut initial = pose(world, entity)?;
            initial.position -= origin;
            let id = builder.add_body(
                BodyParams::new()
                    .with_mass(m as f64)
                    .with_inertia(axes * Mat3::from_diagonal(vector(diagonal)) * axes.transpose())
                    .with_com(vector(com))
                    .with_pose(initial),
            );
            bodies.insert(entity, id);
            reference.push((id, initial));
        }
        let chassis = bodies[&layout.chassis];
        builder.add_joint_free(BodyId::NONE, chassis, reference[0].1, Transform::IDENTITY)?;
        let mut joints = HashMap::new();
        let mut drives = Vec::new();
        for &entity in &layout.tree_joints {
            let joint = world
                .get::<UsdPhysicsJoint>(entity)
                .ok_or_else(|| invalid("joint disappeared"))?;
            if !joint.joint_enabled
                || joint.exclude_from_articulation
                || !joint.limits.is_empty()
                || joint.cone_limit.is_some()
                || joint.distance_limit.is_some()
                || [joint.break_force, joint.break_torque]
                    .into_iter()
                    .flatten()
                    .any(|v| v.is_nan() || v < f32::MAX)
            {
                return Err(invalid(
                    "unsupported joint limits, breakability or tree membership",
                ));
            }
            let parent = *bodies
                .get(&joint.body0.ok_or_else(|| invalid("missing parent"))?)
                .ok_or_else(|| invalid("foreign parent"))?;
            let child = *bodies
                .get(&joint.body1.ok_or_else(|| invalid("missing child"))?)
                .ok_or_else(|| invalid("foreign child"))?;
            let (a, b) = frames(joint)?;
            let id = match joint.kind {
                UsdJointKind::Fixed => builder.add_joint_fixed(parent, child, a, b)?,
                UsdJointKind::Revolute => {
                    builder.add_joint_revolute(parent, child, a, b, vector(joint.axis))?
                }
                UsdJointKind::Prismatic => {
                    builder.add_joint_prismatic(parent, child, a, b, vector(joint.axis))?
                }
                UsdJointKind::Spherical => builder.add_joint_ball(parent, child, a, b)?,
                _ => return Err(invalid("unsupported tree joint type")),
            };
            let scalar = matches!(joint.kind, UsdJointKind::Revolute | UsdJointKind::Prismatic);
            if let Some((lower, upper)) = joint.built_in_limit {
                if !scalar || lower.is_nan() || upper.is_nan() || lower > upper {
                    return Err(invalid("invalid scalar joint bounds"));
                }
                builder.set_joint_limits(id, &[lower as f64], &[upper as f64])?;
            }
            if joint.drives.len() > 1 || (!scalar && !joint.drives.is_empty()) {
                return Err(invalid("unsupported multi-axis drive"));
            }
            if let Some(drive) = joint.drives.first() {
                if !drive_axis(joint, drive.dof)
                    || drive.drive_type != UsdDriveType::Force
                    || !drive.stiffness.is_finite()
                    || drive.stiffness < 0.0
                    || !drive.damping.is_finite()
                    || drive.damping < 0.0
                    || drive.max_force.is_some_and(|v| v.is_nan() || v < 0.0)
                    || [drive.target_position, drive.target_velocity]
                        .into_iter()
                        .flatten()
                        .any(|v| !v.is_finite())
                {
                    return Err(invalid("unsupported or invalid drive"));
                }
                drives.push((id, drive.clone()));
            }
            joints.insert(entity, id);
        }
        let model = builder.build()?;
        let mut state = model.state()?;
        eval_fk(&model, &mut state)?;
        for (body, expected) in reference {
            let actual = state.body_q.host()?[body.index()];
            if (actual.position - expected.position).length() > 1e-4
                || actual.rotation.dot(expected.rotation).abs() < 1.0 - 1e-8
            {
                return Err(invalid(
                    "joint frames do not reproduce the authored rest pose",
                ));
            }
        }
        let mut control = model.control();
        for (id, drive) in drives {
            let dof = model.joint_dof_offset.host()?[id.index()] as usize;
            control.joint_target_mode.host_mut()?[dof] = JointTargetMode::PositionVelocity as u32;
            control.joint_target_pos.host_mut()?[dof] = drive.target_position.unwrap_or(0.0) as f64;
            control.joint_target_vel.host_mut()?[dof] = drive.target_velocity.unwrap_or(0.0) as f64;
            control.joint_target_kp.host_mut()?[dof] = drive.stiffness as f64;
            control.joint_target_kd.host_mut()?[dof] = drive.damping as f64;
            control.joint_motor_max_force.host_mut()?[dof] =
                drive.max_force.map_or(f64::INFINITY, |f| f as f64);
        }
        let mut loops = Vec::new();
        for &entity in &layout.loop_joints {
            let joint = world
                .get::<UsdPhysicsJoint>(entity)
                .ok_or_else(|| invalid("loop joint disappeared"))?
                .clone();
            if joint.kind != UsdJointKind::Spherical
                || !joint.exclude_from_articulation
                || !joint.joint_enabled
                || !joint.drives.is_empty()
                || !joint.limits.is_empty()
                || joint.cone_limit.is_some()
                || joint.distance_limit.is_some()
                || [joint.break_force, joint.break_torque]
                    .into_iter()
                    .flatten()
                    .any(|v| v.is_nan() || v < f32::MAX)
            {
                return Err(invalid("unsupported loop joint"));
            }
            let parent = *bodies
                .get(&joint.body0.ok_or_else(|| invalid("missing loop parent"))?)
                .ok_or_else(|| invalid("foreign loop parent"))?;
            let child = *bodies
                .get(&joint.body1.ok_or_else(|| invalid("missing loop child"))?)
                .ok_or_else(|| invalid("foreign loop child"))?;
            let (a, b) = frames(&joint)?;
            let poses = state.body_q.host()?;
            let error = poses[parent.index()].transform_point(a.position)
                - poses[child.index()].transform_point(b.position);
            if error.length() > 1e-4 {
                return Err(invalid(
                    "loop anchors do not meet in the authored rest pose",
                ));
            }
            loops.push(LoopConstraint {
                entity,
                parent,
                child,
                joint,
            });
        }
        Ok(Self {
            model,
            state,
            control,
            bodies,
            joints,
            loops,
            origin,
        })
    }
}
