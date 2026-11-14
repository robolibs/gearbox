//! `UsdPhysicsJoint` → joints in `PhysicsWorld`.
//!
//! Revolute and prismatic joints move about the backend's joint X; the
//! authored axis rides in `JointKind`, so motors and limits always address
//! `AngX` / `LinX` whatever the USD axis token was.

use super::backend::{BodyDesc, BodyId, JointAxes, JointAxis, JointDesc, JointKind, MotorDesc, MotorModel, MotorTarget, Pose};
use super::convert::{quat_to_d, vec3_to_d};
use super::markers::{
    EndpointMark, JointEndpoints, JointNotBuilt, ModelMember, UsdDof, UsdDriveType, UsdJointDrive, UsdJointKind, UsdPhysicsJoint,
    UsdRigidBody, WorldAnchorBody,
};
use bevy::prelude::*;
use glam::DVec3;

use super::world::{AuthoredDrive, PhysicsWorld};

#[derive(Component)]
pub(crate) struct JointAttached;

/// One side of a joint once its target is known.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Side {
    /// A body, with the target prim's frame in that body's frame.
    Body(BodyId, Pose),
    /// The world, at this frame (Bevy world, before the joint's local frame).
    World(Pose),
    /// The target body is not in the physics world yet.
    Pending,
}

fn rigid_pose(g: &GlobalTransform) -> Pose {
    let t = g.compute_transform();
    Pose::new(vec3_to_d(t.translation), quat_to_d(t.rotation))
}

/// Where `mark` attaches: the target body, the nearest body above a target
/// that is not one, or a world-fixed frame when no body is above it. An
/// unresolved target is an error the caller reports; it never pins a body.
fn resolve_side(
    mark: &EndpointMark,
    stage_root: Option<Entity>,
    world: &PhysicsWorld,
    rigid: &Query<(), With<UsdRigidBody>>,
    parents: &Query<&ChildOf>,
    globals: &Query<&GlobalTransform>,
) -> Result<Side, String> {
    let pose_of = |e: Entity| globals.get(e).map(rigid_pose).unwrap_or(Pose::IDENTITY);
    let target = match mark {
        EndpointMark::Unresolved(path) => return Err(format!("target {path} is not in the projected stage")),
        EndpointMark::Absent => return Ok(Side::World(stage_root.map_or(Pose::IDENTITY, pose_of))),
        EndpointMark::Resolved(e) => *e,
    };
    let mut current = target;
    loop {
        if let Some(handle) = world.entity_to_body.get(&current).copied() {
            let offset = if current == target { Pose::IDENTITY } else { pose_of(current).inverse() * pose_of(target) };
            return Ok(Side::Body(handle, offset));
        }
        if rigid.contains(current) {
            return Ok(Side::Pending);
        }
        match parents.get(current) {
            Ok(parent) => current = parent.parent(),
            Err(_) => return Ok(Side::World(pose_of(target))),
        }
    }
}

fn mark_of(entity: Option<Entity>) -> EndpointMark {
    entity.map_or(EndpointMark::Absent, EndpointMark::Resolved)
}

pub fn convert_joints(
    mut commands: Commands,
    mut world: ResMut<PhysicsWorld>,
    joints: Query<(Entity, &UsdPhysicsJoint, Option<&JointEndpoints>, Option<&ModelMember>), (Without<JointAttached>, Without<JointNotBuilt>)>,
    rigid: Query<(), With<UsdRigidBody>>,
    parents: Query<&ChildOf>,
    globals: Query<&GlobalTransform>,
    sites: Query<&crate::globe::Site>,
) {
    for (joint_entity, joint, endpoints, member) in &joints {
        let joint = with_schema_targets(joint);
        let joint = joint.as_ref();
        if !joint.joint_enabled {
            commands.entity(joint_entity).insert(JointAttached);
            continue;
        }
        let path = endpoints.map(|e| e.path.as_str()).unwrap_or("<joint>");
        if member.is_some_and(|m| m.rejected) {
            commands.entity(joint_entity).insert(JointNotBuilt { reason: "machine rejected".into() });
            continue;
        }
        let (mark0, mark1, stage_root) = match endpoints {
            Some(e) => (e.body0.clone(), e.body1.clone(), e.stage_root),
            None => (mark_of(joint.body0), mark_of(joint.body1), None),
        };
        let sides = (
            resolve_side(&mark0, stage_root, &world, &rigid, &parents, &globals),
            resolve_side(&mark1, stage_root, &world, &rigid, &parents, &globals),
        );
        let (side0, side1) = match sides {
            (Err(reason), _) | (_, Err(reason)) => {
                error!("gearbox-physics: joint {path} not built: {reason}");
                commands.entity(joint_entity).insert(JointNotBuilt { reason });
                continue;
            }
            (Ok(Side::Pending), _) | (_, Ok(Side::Pending)) => continue,
            (Ok(a), Ok(b)) => (a, b),
        };
        let scale = |mark: &EndpointMark| match mark {
            EndpointMark::Resolved(e) => globals.get(*e).map_or(1.0, |g| g.compute_transform().scale.x as f64),
            _ => 1.0,
        };
        let local0 = Pose::new(vec3_to_d(joint.local_pos0) * scale(&mark0), quat_to_d(joint.local_rot0));
        let local1 = Pose::new(vec3_to_d(joint.local_pos1) * scale(&mark1), quat_to_d(joint.local_rot1));
        let region = gearbox_globe::physics_offset(crate::globe::site_of(joint_entity, &parents, &sites));
        let joint_pose = globals.get(joint_entity).map(rigid_pose).unwrap_or(Pose::IDENTITY);
        let mut anchor = |world: &mut PhysicsWorld, frame: Pose| {
            let mut pose = frame;
            pose.translation.x += region.x;
            let body = world.insert_body(BodyDesc::fixed().pose(pose));
            let local = joint_pose.inverse() * frame;
            let transform = Transform::from_translation(super::convert::vec3_from_d(local.translation))
                .with_rotation(super::convert::quat_from_d(local.rotation));
            let entity = commands
                .spawn((Name::new("world_anchor"), WorldAnchorBody { joint: joint_entity }, transform, ChildOf(joint_entity)))
                .id();
            world.entity_to_body.insert(entity, body);
            world.anchor_bodies.insert(joint_entity, (entity, body));
            body
        };
        let (body0, frame0, body1, frame1) = match (side0, side1) {
            (Side::Body(b0, o0), Side::Body(b1, o1)) => (b0, o0 * local0, b1, o1 * local1),
            (Side::World(w), Side::Body(b1, o1)) => (anchor(&mut world, w * local0), Pose::IDENTITY, b1, o1 * local1),
            (Side::Body(b0, o0), Side::World(w)) => (b0, o0 * local0, anchor(&mut world, w * local1), Pose::IDENTITY),
            _ => {
                let reason = "neither endpoint is a body".to_string();
                error!("gearbox-physics: joint {path} not built: {reason}");
                commands.entity(joint_entity).insert(JointNotBuilt { reason });
                continue;
            }
        };

        if std::env::var_os("GEARBOX_PHYSICS_LOG").is_some() {
            info!(
                "gearbox-physics: joint {path} {:?} {body0:?}->{body1:?} frame0 {frame0:?} frame1 {frame1:?} axis {:?} limit {:?} excl {} drives {}",
                joint.kind,
                joint.axis,
                joint.built_in_limit,
                joint.exclude_from_articulation,
                joint.drives.len()
            );
        }

        if let Some(mut desc) = joint_desc(joint) {
            desc.frame1 = frame0;
            desc.frame2 = frame1;
            desc.loop_closure = joint.exclude_from_articulation;
            desc.softness = joint.softness;
            let id = world.insert_joint(body0, body1, desc);
            world.entity_to_joint.insert(joint_entity, id);
            world.joint_entities.insert(id, joint_entity);
            if let Some(drive) = authored_drive(joint) {
                world.authored_drives.insert(id, drive);
            }
            if joint.kind == UsdJointKind::Generic {
                for d in &joint.drives {
                    if let Some(axis) = frame_axis(d.dof) {
                        world.authored_axis_drives.insert((id, axis), drive_of(d));
                    }
                }
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

/// Every UsdPhysics joint kind has a backend path.
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
        UsdJointKind::Spherical => {
            let mut desc = JointDesc::new(JointKind::Spherical { axis }, frame1, frame2);
            if let Some((angle0, angle1)) = j.cone_limit {
                desc.cone_limits = [angle0, angle1].map(|a| (a >= 0.0).then(|| (a as f64).min(std::f64::consts::PI)));
            }
            desc
        }
        UsdJointKind::Distance => {
            let mut desc = JointDesc::new(JointKind::Distance, frame1, frame2);
            if let Some((min, max)) = j.distance_limit {
                desc.distance_limits = [min, max].map(|d| (d >= 0.0).then_some(d as f64));
            }
            desc
        }
        UsdJointKind::Generic => generic_desc(j, frame1, frame2),
    };
    desc.contacts_enabled = j.collision_enabled;
    desc.break_limits = [j.break_force, j.break_torque]
        .map(|limit| limit.filter(|v| *v >= 0.0 && *v < f32::MAX).map(f64::from));
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

/// A drive with neither target authored takes the UsdPhysics fallback,
/// position 0: a stiff drive springs to 0, a damper holds the joint still.
fn with_schema_targets(joint: &UsdPhysicsJoint) -> std::borrow::Cow<'_, UsdPhysicsJoint> {
    if joint.drives.iter().all(|d| d.target_position.is_some() || d.target_velocity.is_some()) {
        return std::borrow::Cow::Borrowed(joint);
    }
    let mut joint = joint.clone();
    for d in joint.drives.iter_mut().filter(|d| d.target_position.is_none() && d.target_velocity.is_none()) {
        d.target_position = Some(0.0);
    }
    std::borrow::Cow::Owned(joint)
}

fn authored_drive(j: &UsdPhysicsJoint) -> Option<AuthoredDrive> {
    let free_axis: fn(UsdDof) -> bool = match j.kind {
        UsdJointKind::Revolute => dof_is_angular,
        UsdJointKind::Prismatic => dof_is_linear,
        _ => return None,
    };
    j.drives.iter().find(|d| free_axis(d.dof)).map(drive_of)
}

fn drive_of(d: &UsdJointDrive) -> AuthoredDrive {
    AuthoredDrive {
        stiffness: d.stiffness as f64,
        damping: d.damping as f64,
        max_force: d.max_force.map(|f| f as f64).filter(|f| f.is_finite()),
        force: matches!(d.drive_type, UsdDriveType::Force),
        target_position: d.target_position.map(f64::from),
        target_velocity: d.target_velocity.map(f64::from),
    }
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

    #[test]
    fn ball_distance_and_break_limits_carry_their_authored_sides() {
        let ball = UsdPhysicsJoint { kind: UsdJointKind::Spherical, axis: Vec3::Y, cone_limit: Some((0.25, -1.0)), ..Default::default() };
        let ball = joint_desc(&ball).unwrap();
        assert_eq!(ball.kind, JointKind::Spherical { axis: DVec3::Y });
        assert_eq!(ball.cone_limits, [Some(0.25), None]);

        let rope = UsdPhysicsJoint { kind: UsdJointKind::Distance, distance_limit: Some((-1.0, 2.0)), ..Default::default() };
        assert_eq!(joint_desc(&rope).unwrap().distance_limits, [None, Some(2.0)]);

        let pin = UsdPhysicsJoint { kind: UsdJointKind::Fixed, break_force: Some(100.0), break_torque: Some(f32::MAX), ..Default::default() };
        assert_eq!(joint_desc(&pin).unwrap().break_limits, [Some(100.0), None]);
    }

    fn body_entity(app: &mut App, at: Vec3) -> Entity {
        app.world_mut()
            .spawn((
                UsdRigidBody { enabled: true, ..Default::default() },
                super::super::markers::UsdMass {
                    mass: Some(1.0),
                    diagonal_inertia: Some(Vec3::splat(0.01)),
                    ..Default::default()
                },
                Transform::from_translation(at),
                GlobalTransform::from_translation(at),
            ))
            .id()
    }

    fn joint_app() -> App {
        let mut app = App::new();
        app.insert_resource(PhysicsWorld::default())
            .add_systems(Update, (super::super::bodies::convert_rigid_bodies, convert_joints).chain());
        app
    }

    fn hinge(body0: EndpointMark, body1: EndpointMark, stage_root: Entity, pos0: Vec3, pos1: Vec3) -> (UsdPhysicsJoint, JointEndpoints) {
        let joint = UsdPhysicsJoint {
            kind: UsdJointKind::Revolute,
            axis: Vec3::Z,
            joint_enabled: true,
            local_pos0: pos0,
            local_pos1: pos1,
            local_rot0: Quat::IDENTITY,
            local_rot1: Quat::IDENTITY,
            ..Default::default()
        };
        (joint, JointEndpoints { path: "/hinge".into(), body0, body1, stage_root: Some(stage_root) })
    }

    fn step(app: &mut App, steps: usize) {
        app.update();
        for _ in 0..steps {
            app.world_mut().resource_mut::<PhysicsWorld>().step();
        }
    }

    /// A pendulum hinged to the world swings about its hinge: the world
    /// side keeps its DOF instead of fixing the bob.
    #[test]
    fn a_world_revolute_joint_keeps_its_degree_of_freedom() {
        for world_first in [true, false] {
            let mut app = joint_app();
            let root = app.world_mut().spawn(GlobalTransform::IDENTITY).id();
            let bob = body_entity(&mut app, Vec3::new(1.0, 0.0, 0.0));
            let (joint, ends) = if world_first {
                hinge(EndpointMark::Absent, EndpointMark::Resolved(bob), root, Vec3::ZERO, Vec3::new(-1.0, 0.0, 0.0))
            } else {
                hinge(EndpointMark::Resolved(bob), EndpointMark::Absent, root, Vec3::new(-1.0, 0.0, 0.0), Vec3::ZERO)
            };
            let joint_entity = app.world_mut().spawn((joint, ends, GlobalTransform::IDENTITY)).id();
            step(&mut app, 240);
            let world = app.world().resource::<PhysicsWorld>();
            let handle = world.entity_to_body[&bob];
            let body = world.body(handle).unwrap();
            assert!(body.is_dynamic(), "the bob stays dynamic");
            let p = body.position().translation;
            assert!(p.y < -0.5, "the bob swung down about the hinge: {p:?}");
            assert!((p.length() - 1.0).abs() < 0.02, "the bob stays 1 m from the world hinge: {p:?}");
            let (_, anchor) = world.anchor_bodies[&joint_entity];
            assert!(world.body(anchor).unwrap().is_fixed());
            assert_eq!(world.joint_entities[&world.entity_to_joint[&joint_entity]], joint_entity);
        }
    }

    /// A joint whose target does not exist is not built, and the body it
    /// names on its other side is not pinned to the world.
    #[test]
    fn an_unresolved_target_never_pins_the_other_body() {
        let mut app = joint_app();
        let root = app.world_mut().spawn(GlobalTransform::IDENTITY).id();
        let bob = body_entity(&mut app, Vec3::ZERO);
        let (joint, ends) = hinge(EndpointMark::Resolved(bob), EndpointMark::Unresolved("/typo".into()), root, Vec3::ZERO, Vec3::ZERO);
        let joint_entity = app.world_mut().spawn((joint, ends, GlobalTransform::IDENTITY)).id();
        step(&mut app, 120);
        assert!(app.world().get::<JointNotBuilt>(joint_entity).is_some());
        let world = app.world().resource::<PhysicsWorld>();
        let body = world.body(world.entity_to_body[&bob]).unwrap();
        assert!(body.is_dynamic() && body.position().translation.y < -1.0, "falls freely: {:?}", body.position());
        assert!(world.entity_to_joint.get(&joint_entity).is_none());
    }

    /// A joint frame authored in a scaled body's frame sits at the scaled
    /// offset, as the manifest has it (S12): the anchors coincide and nothing moves.
    #[test]
    fn a_joint_frame_follows_its_body_scale() {
        let mut app = joint_app();
        let root = app.world_mut().spawn(GlobalTransform::IDENTITY).id();
        let base = body_entity(&mut app, Vec3::ZERO);
        let big = body_entity(&mut app, Vec3::X);
        let scaled = Transform::from_translation(Vec3::X).with_scale(Vec3::splat(2.0));
        app.world_mut().entity_mut(big).insert((scaled, GlobalTransform::from(scaled)));
        let (joint, ends) = hinge(EndpointMark::Resolved(base), EndpointMark::Resolved(big), root, Vec3::new(0.5, 0.0, 0.0), Vec3::new(-0.25, 0.0, 0.0));
        let joint_entity = app.world_mut().spawn((joint, ends, GlobalTransform::IDENTITY)).id();
        step(&mut app, 1);
        let world = app.world().resource::<PhysicsWorld>();
        let frame = world.joint(world.entity_to_joint[&joint_entity]).unwrap().frame2();
        assert!((frame.translation - glam::DVec3::new(-0.5, 0.0, 0.0)).length() < 1e-6, "{frame:?}");
    }

    /// Two joints between the same two bodies are two physics joints, each
    /// reachable from its own prim.
    #[test]
    fn parallel_joints_keep_their_own_identity() {
        let mut app = joint_app();
        let root = app.world_mut().spawn(GlobalTransform::IDENTITY).id();
        let a = body_entity(&mut app, Vec3::ZERO);
        let b = body_entity(&mut app, Vec3::X);
        let (j1, e1) = hinge(EndpointMark::Resolved(a), EndpointMark::Resolved(b), root, Vec3::new(0.5, 0.0, 0.0), Vec3::new(-0.5, 0.0, 0.0));
        let (mut j2, e2) = hinge(EndpointMark::Resolved(a), EndpointMark::Resolved(b), root, Vec3::new(0.5, 0.0, 0.0), Vec3::new(-0.5, 0.0, 0.0));
        j2.exclude_from_articulation = true;
        let first = app.world_mut().spawn((j1, e1, GlobalTransform::IDENTITY)).id();
        let second = app.world_mut().spawn((j2, e2, GlobalTransform::IDENTITY)).id();
        step(&mut app, 1);
        let world = app.world().resource::<PhysicsWorld>();
        let (i1, i2) = (world.entity_to_joint[&first], world.entity_to_joint[&second]);
        assert_ne!(i1, i2);
        assert_eq!((world.joint_entities[&i1], world.joint_entities[&i2]), (first, second));
        assert_eq!(world.joints_between(world.entity_to_body[&a], world.entity_to_body[&b]).len(), 2);
    }

    fn pendulum(desc: JointDesc, start: DVec3) -> (PhysicsWorld, BodyId, BodyId) {
        let mut world = PhysicsWorld::default();
        let anchor = world.insert_body(BodyDesc::fixed());
        let mut bob = BodyDesc::dynamic().pose(Pose::from_translation(start));
        bob.additional_mass = Some(MassProps { mass: 1.0, local_com: DVec3::ZERO, inertia: Inertia::Principal(DVec3::splat(0.01)) });
        let bob = world.insert_body(bob);
        world.insert_joint(anchor, bob, desc);
        (world, anchor, bob)
    }

    /// Characterization: a distance joint's maximum holds a falling bob.
    #[test]
    fn a_distance_joint_holds_its_maximum() {
        let mut desc = JointDesc::new(JointKind::Distance, Pose::IDENTITY, Pose::IDENTITY);
        desc.distance_limits = [None, Some(1.0)];
        let (mut world, _, bob) = pendulum(desc, DVec3::new(0.5, 0.0, 0.0));
        for _ in 0..480 {
            world.step();
        }
        let d = world.body(bob).unwrap().position().translation.length();
        assert!(d < 1.05 && d > 0.9, "distance {d}");
    }

    /// Characterization: a spherical joint's cone limit stops the swing near
    /// its authored half angle.
    #[test]
    fn a_spherical_cone_limit_stops_the_swing() {
        let mut desc = JointDesc::new(JointKind::Spherical { axis: DVec3::NEG_Y }, Pose::IDENTITY, Pose::from_translation(DVec3::Y));
        desc.cone_limits = [Some(0.5), Some(0.5)];
        let (mut world, _, bob) = pendulum(desc, DVec3::new(0.0, -1.0, 0.0));
        world.body_mut(bob).unwrap().set_linvel(DVec3::new(6.0, 0.0, 0.0), true);
        let mut widest = 0.0f64;
        for _ in 0..240 {
            world.step();
            let p = world.body(bob).unwrap().position().translation;
            widest = widest.max(p.x.atan2(-p.y).abs());
        }
        assert!(widest < 0.5 + 0.15, "swing reached {widest} rad against a 0.5 rad cone");
    }

    /// Characterization: a fixed joint overloaded past its break force lets go.
    #[test]
    fn a_break_force_releases_an_overloaded_joint() {
        let mut desc = JointDesc::new(JointKind::Fixed, Pose::IDENTITY, Pose::from_translation(DVec3::Y));
        desc.break_limits = [Some(2.0), None];
        let (mut world, _, bob) = pendulum(desc, DVec3::new(0.0, -1.0, 0.0));
        for _ in 0..240 {
            world.step();
        }
        let y = world.body(bob).unwrap().position().translation.y;
        assert!(y < -1.5, "a 9.8 N load breaks a 2 N joint and the bob falls: y {y}");
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
