//! Rapier (f64) behind [`PhysicsBackend`]. The only module that names
//! `rapier3d`; everything else sees ids, poses and the accessor traits.
//!
//! Joints live in one id space: rapier keeps reduced-coordinate
//! (multibody) and constraint (impulse) joints in separate sets, so a
//! [`JointId`] carries which set it came from in its top bit.

use bevy::prelude::Entity;
use rapier3d::parry::shape::TypedShape;
use rapier3d::prelude as r;

use super::backend::*;

pub struct RapierBackend {
    gravity: DVec3,
    integration_parameters: r::IntegrationParameters,
    physics_pipeline: r::PhysicsPipeline,
    islands: r::IslandManager,
    broad_phase: r::BroadPhaseBvh,
    narrow_phase: r::NarrowPhase,
    bodies: r::RigidBodySet,
    colliders: r::ColliderSet,
    impulse_joints: r::ImpulseJointSet,
    multibody_joints: r::MultibodyJointSet,
    ccd_solver: r::CCDSolver,
}

impl Default for RapierBackend {
    fn default() -> Self {
        Self {
            gravity: DVec3::new(0.0, -9.81, 0.0),
            integration_parameters: r::IntegrationParameters::default(),
            physics_pipeline: r::PhysicsPipeline::new(),
            islands: r::IslandManager::new(),
            broad_phase: r::BroadPhaseBvh::new(),
            narrow_phase: r::NarrowPhase::new(),
            bodies: r::RigidBodySet::new(),
            colliders: r::ColliderSet::new(),
            impulse_joints: r::ImpulseJointSet::new(),
            multibody_joints: r::MultibodyJointSet::new(),
            ccd_solver: r::CCDSolver::new(),
        }
    }
}

const REDUCED_BIT: u64 = 1 << 63;

fn pack(parts: (u32, u32)) -> u64 {
    ((parts.1 as u64) << 32) | parts.0 as u64
}

fn unpack(id: u64) -> (u32, u32) {
    (id as u32, (id >> 32) as u32)
}

fn body_id(h: r::RigidBodyHandle) -> BodyId {
    BodyId(pack(h.into_raw_parts()))
}

fn body_handle(id: BodyId) -> r::RigidBodyHandle {
    let (index, generation) = unpack(id.0);
    r::RigidBodyHandle::from_raw_parts(index, generation)
}

fn collider_id(h: r::ColliderHandle) -> ColliderId {
    ColliderId(pack(h.into_raw_parts()))
}

fn collider_handle(id: ColliderId) -> r::ColliderHandle {
    let (index, generation) = unpack(id.0);
    r::ColliderHandle::from_raw_parts(index, generation)
}

enum JointHandle {
    Impulse(r::ImpulseJointHandle),
    Reduced(r::MultibodyJointHandle),
}

fn impulse_id(h: r::ImpulseJointHandle) -> JointId {
    JointId(pack(h.into_raw_parts()))
}

fn reduced_id(h: r::MultibodyJointHandle) -> JointId {
    JointId(pack(h.into_raw_parts()) | REDUCED_BIT)
}

fn joint_handle(id: JointId) -> JointHandle {
    let (index, generation) = unpack(id.0 & !REDUCED_BIT);
    if id.0 & REDUCED_BIT != 0 {
        JointHandle::Reduced(r::MultibodyJointHandle::from_raw_parts(index, generation))
    } else {
        JointHandle::Impulse(r::ImpulseJointHandle::from_raw_parts(index, generation))
    }
}

fn to_pose(p: &r::Pose) -> Pose {
    Pose { translation: p.translation, rotation: p.rotation }
}

fn from_pose(p: Pose) -> r::Pose {
    r::Pose { translation: p.translation, rotation: p.rotation }
}

fn entity_of(user_data: u128) -> Option<Entity> {
    (user_data != 0).then(|| Entity::from_bits(user_data as u64))
}

fn user_data_of(entity: Option<Entity>) -> u128 {
    entity.map_or(0, |e| e.to_bits() as u128)
}

fn to_kind(t: r::RigidBodyType) -> BodyKind {
    match t {
        r::RigidBodyType::Dynamic => BodyKind::Dynamic,
        r::RigidBodyType::Fixed => BodyKind::Fixed,
        _ => BodyKind::Kinematic,
    }
}

fn from_kind(kind: BodyKind) -> r::RigidBodyType {
    match kind {
        BodyKind::Dynamic => r::RigidBodyType::Dynamic,
        BodyKind::Fixed => r::RigidBodyType::Fixed,
        BodyKind::Kinematic => r::RigidBodyType::KinematicPositionBased,
    }
}

fn from_mass(props: MassProps) -> r::MassProperties {
    match props.inertia {
        Inertia::Principal(i) => r::MassProperties::new(props.local_com, props.mass, i),
        Inertia::Tensor(m) => {
            r::MassProperties::with_inertia_matrix(props.local_com, props.mass, m)
        }
    }
}

fn from_axis(axis: JointAxis) -> r::JointAxis {
    match axis {
        JointAxis::LinX => r::JointAxis::LinX,
        JointAxis::LinY => r::JointAxis::LinY,
        JointAxis::LinZ => r::JointAxis::LinZ,
        JointAxis::AngX => r::JointAxis::AngX,
        JointAxis::AngY => r::JointAxis::AngY,
        JointAxis::AngZ => r::JointAxis::AngZ,
    }
}

fn from_axes(axes: JointAxes) -> r::JointAxesMask {
    let mut mask = r::JointAxesMask::empty();
    for axis in JointAxis::ALL {
        if axes.contains(axis) {
            mask |= r::JointAxesMask::from(from_axis(axis));
        }
    }
    mask
}

fn to_axes(mask: r::JointAxesMask) -> JointAxes {
    let mut axes = JointAxes::NONE;
    for axis in JointAxis::ALL {
        if mask.contains(r::JointAxesMask::from(from_axis(axis))) {
            axes = axes.with(axis);
        }
    }
    axes
}

fn from_model(model: MotorModel) -> r::MotorModel {
    match model {
        MotorModel::Acceleration => r::MotorModel::AccelerationBased,
        MotorModel::Force => r::MotorModel::ForceBased,
    }
}

fn from_rule(rule: CombineRule) -> r::CoefficientCombineRule {
    match rule {
        CombineRule::Average => r::CoefficientCombineRule::Average,
        CombineRule::Min => r::CoefficientCombineRule::Min,
        CombineRule::Multiply => r::CoefficientCombineRule::Multiply,
        CombineRule::Max => r::CoefficientCombineRule::Max,
    }
}

fn from_groups(groups: CollisionGroups) -> r::InteractionGroups {
    r::InteractionGroups::new(
        r::Group::from_bits_truncate(groups.memberships),
        r::Group::from_bits_truncate(groups.filter),
        r::InteractionTestMode::And,
    )
}

fn shared_shape(shape: Shape) -> Option<r::SharedShape> {
    Some(match shape {
        Shape::Cuboid { half_extents: h } => r::SharedShape::cuboid(h.x, h.y, h.z),
        Shape::Ball { radius } => r::SharedShape::ball(radius),
        Shape::Capsule { a, b, radius } => r::SharedShape::capsule(a, b, radius),
        Shape::Cylinder { half_height, radius } => r::SharedShape::cylinder(half_height, radius),
        Shape::RoundCylinder { half_height, radius, border_radius } => {
            r::SharedShape::round_cylinder(half_height, radius, border_radius)
        }
        Shape::ConvexHull { points } => r::SharedShape::convex_hull(&points)?,
        Shape::ConvexDecomposition { vertices, indices } => {
            r::SharedShape::convex_decomposition(&vertices, &indices)
        }
        Shape::TriMesh { vertices, indices } => r::SharedShape::trimesh(vertices, indices).ok()?,
        Shape::Heightfield { rows, cols, heights, scale } => {
            // parry stores the matrix column-major.
            let mut data = Vec::with_capacity(rows * cols);
            for col in 0..cols {
                for row in 0..rows {
                    data.push(heights[row * cols + col]);
                }
            }
            r::SharedShape::heightfield(
                rapier3d::parry::utils::Array2::new(rows, cols, data),
                scale,
            )
        }
    })
}

impl Body for r::RigidBody {
    fn position(&self) -> Pose {
        to_pose(r::RigidBody::position(self))
    }
    fn linvel(&self) -> DVec3 {
        r::RigidBody::linvel(self)
    }
    fn angvel(&self) -> DVec3 {
        r::RigidBody::angvel(self)
    }
    fn mass(&self) -> f64 {
        r::RigidBody::mass(self)
    }
    fn center_of_mass(&self) -> DVec3 {
        r::RigidBody::center_of_mass(self)
    }
    fn local_center_of_mass(&self) -> DVec3 {
        r::RigidBody::local_center_of_mass(self)
    }
    fn principal_inertia(&self) -> DVec3 {
        self.mass_properties().local_mprops.principal_inertia()
    }
    fn inertia_tensor(&self) -> DMat3 {
        self.mass_properties().local_mprops.reconstruct_inertia_matrix()
    }
    fn kind(&self) -> BodyKind {
        to_kind(self.body_type())
    }
    fn is_enabled(&self) -> bool {
        r::RigidBody::is_enabled(self)
    }
    fn is_sleeping(&self) -> bool {
        r::RigidBody::is_sleeping(self)
    }
    fn colliders(&self) -> Vec<ColliderId> {
        r::RigidBody::colliders(self).iter().copied().map(collider_id).collect()
    }
    fn entity(&self) -> Option<Entity> {
        entity_of(self.user_data)
    }
}

impl BodyMut for r::RigidBody {
    fn set_position(&mut self, pose: Pose, wake: bool) {
        r::RigidBody::set_position(self, from_pose(pose), wake);
    }
    fn set_linvel(&mut self, linvel: DVec3, wake: bool) {
        r::RigidBody::set_linvel(self, linvel, wake);
    }
    fn set_angvel(&mut self, angvel: DVec3, wake: bool) {
        r::RigidBody::set_angvel(self, angvel, wake);
    }
    fn set_kind(&mut self, kind: BodyKind, wake: bool) {
        self.set_body_type(from_kind(kind), wake);
    }
    fn set_enabled(&mut self, enabled: bool) {
        r::RigidBody::set_enabled(self, enabled);
    }
    fn enable_ccd(&mut self, enabled: bool) {
        r::RigidBody::enable_ccd(self, enabled);
    }
    fn wake_up(&mut self, strong: bool) {
        r::RigidBody::wake_up(self, strong);
    }
    fn sleep(&mut self) {
        r::RigidBody::sleep(self);
    }
    fn set_linear_damping(&mut self, damping: f64) {
        r::RigidBody::set_linear_damping(self, damping);
    }
    fn set_angular_damping(&mut self, damping: f64) {
        r::RigidBody::set_angular_damping(self, damping);
    }
    fn set_additional_mass(&mut self, props: MassProps, wake: bool) {
        self.set_additional_mass_properties(from_mass(props), wake);
    }
    fn add_force(&mut self, force: DVec3, wake: bool) {
        r::RigidBody::add_force(self, force, wake);
    }
    fn add_torque(&mut self, torque: DVec3, wake: bool) {
        r::RigidBody::add_torque(self, torque, wake);
    }
    fn reset_forces(&mut self, wake: bool) {
        r::RigidBody::reset_forces(self, wake);
        r::RigidBody::reset_torques(self, wake);
    }
    fn apply_impulse(&mut self, impulse: DVec3, wake: bool) {
        r::RigidBody::apply_impulse(self, impulse, wake);
    }
    fn apply_torque_impulse(&mut self, impulse: DVec3, wake: bool) {
        r::RigidBody::apply_torque_impulse(self, impulse, wake);
    }
    fn apply_impulse_at_point(&mut self, impulse: DVec3, point: DVec3, wake: bool) {
        r::RigidBody::apply_impulse_at_point(self, impulse, point, wake);
    }
}

impl Collider for r::Collider {
    fn position(&self) -> Pose {
        to_pose(r::Collider::position(self))
    }
    fn position_wrt_parent(&self) -> Option<Pose> {
        r::Collider::position_wrt_parent(self).map(to_pose)
    }
    fn parent(&self) -> Option<BodyId> {
        r::Collider::parent(self).map(body_id)
    }
    fn shape(&self) -> ShapeView {
        match r::Collider::shape(self).as_typed_shape() {
            TypedShape::Cuboid(c) => ShapeView::Cuboid { half_extents: c.half_extents },
            TypedShape::Ball(b) => ShapeView::Ball { radius: b.radius },
            TypedShape::Capsule(c) => ShapeView::Capsule {
                a: c.segment.a,
                b: c.segment.b,
                radius: c.radius,
            },
            TypedShape::Cylinder(c) => ShapeView::Cylinder {
                half_height: c.half_height,
                radius: c.radius,
            },
            TypedShape::RoundCylinder(c) => ShapeView::RoundCylinder {
                half_height: c.inner_shape.half_height,
                radius: c.inner_shape.radius,
                border_radius: c.border_radius,
            },
            TypedShape::ConvexPolyhedron(p) => ShapeView::ConvexPolyhedron {
                points: p.points().to_vec(),
                edges: p.edges().iter().map(|e| e.vertices).collect(),
            },
            _ => ShapeView::Other,
        }
    }
    fn local_aabb(&self) -> Aabb {
        let aabb = r::Collider::shape(self).compute_local_aabb();
        Aabb { mins: aabb.mins, maxs: aabb.maxs }
    }
    fn aabb(&self) -> Aabb {
        let aabb = self.compute_aabb();
        Aabb { mins: aabb.mins, maxs: aabb.maxs }
    }
    fn aabb_at(&self, pose: Pose) -> Aabb {
        let aabb = r::Collider::shape(self).compute_aabb(&from_pose(pose));
        Aabb { mins: aabb.mins, maxs: aabb.maxs }
    }
    fn is_sensor(&self) -> bool {
        r::Collider::is_sensor(self)
    }
    fn groups(&self) -> CollisionGroups {
        let groups = self.collision_groups();
        CollisionGroups {
            memberships: groups.memberships.bits(),
            filter: groups.filter.bits(),
        }
    }
    fn friction(&self) -> f64 {
        r::Collider::friction(self)
    }
    fn restitution(&self) -> f64 {
        r::Collider::restitution(self)
    }
    fn mass(&self) -> f64 {
        r::Collider::mass(self)
    }
    fn is_enabled(&self) -> bool {
        r::Collider::is_enabled(self)
    }
    fn entity(&self) -> Option<Entity> {
        entity_of(self.user_data)
    }
}

impl ColliderMut for r::Collider {
    fn set_shape(&mut self, shape: Shape) {
        if let Some(shape) = shared_shape(shape) {
            r::Collider::set_shape(self, shape);
        }
    }
    fn set_position_wrt_parent(&mut self, pose: Pose) {
        r::Collider::set_position_wrt_parent(self, from_pose(pose));
    }
    fn set_position(&mut self, pose: Pose) {
        r::Collider::set_position(self, from_pose(pose));
    }
    fn set_friction(&mut self, friction: f64) {
        r::Collider::set_friction(self, friction);
    }
    fn set_restitution(&mut self, restitution: f64) {
        r::Collider::set_restitution(self, restitution);
    }
    fn set_friction_combine_rule(&mut self, rule: CombineRule) {
        r::Collider::set_friction_combine_rule(self, from_rule(rule));
    }
    fn set_density(&mut self, density: f64) {
        r::Collider::set_density(self, density);
    }
    fn set_mass(&mut self, mass: f64) {
        r::Collider::set_mass(self, mass);
    }
    fn set_groups(&mut self, groups: CollisionGroups) {
        self.set_collision_groups(from_groups(groups));
    }
    fn set_enabled(&mut self, enabled: bool) {
        r::Collider::set_enabled(self, enabled);
    }
}

impl Joint for r::GenericJoint {
    fn frame1(&self) -> Pose {
        to_pose(&self.local_frame1)
    }
    fn frame2(&self) -> Pose {
        to_pose(&self.local_frame2)
    }
    fn locked_axes(&self) -> JointAxes {
        to_axes(self.locked_axes)
    }
    fn limits(&self, axis: JointAxis) -> Option<[f64; 2]> {
        r::GenericJoint::limits(self, from_axis(axis)).map(|l| [l.min, l.max])
    }
    fn motor(&self, axis: JointAxis) -> Option<Motor> {
        r::GenericJoint::motor(self, from_axis(axis)).map(|m| Motor {
            target_position: m.target_pos,
            target_velocity: m.target_vel,
            stiffness: m.stiffness,
            damping: m.damping,
            max_force: m.max_force,
            model: match m.model {
                r::MotorModel::ForceBased => MotorModel::Force,
                _ => MotorModel::Acceleration,
            },
        })
    }
    fn contacts_enabled(&self) -> bool {
        r::GenericJoint::contacts_enabled(self)
    }
    fn is_enabled(&self) -> bool {
        r::GenericJoint::is_enabled(self)
    }
}

impl JointMut for r::GenericJoint {
    fn set_frame1(&mut self, frame: Pose) {
        self.local_frame1 = from_pose(frame);
    }
    fn set_frame2(&mut self, frame: Pose) {
        self.local_frame2 = from_pose(frame);
    }
    fn set_limits(&mut self, axis: JointAxis, limits: [f64; 2]) {
        r::GenericJoint::set_limits(self, from_axis(axis), limits);
    }
    fn set_motor_model(&mut self, axis: JointAxis, model: MotorModel) {
        r::GenericJoint::set_motor_model(self, from_axis(axis), from_model(model));
    }
    fn set_motor_velocity(&mut self, axis: JointAxis, target: f64, damping: f64) {
        r::GenericJoint::set_motor_velocity(self, from_axis(axis), target, damping);
    }
    fn set_motor_position(&mut self, axis: JointAxis, target: f64, stiffness: f64, damping: f64) {
        r::GenericJoint::set_motor_position(self, from_axis(axis), target, stiffness, damping);
    }
    fn set_motor(
        &mut self,
        axis: JointAxis,
        target_position: f64,
        target_velocity: f64,
        stiffness: f64,
        damping: f64,
    ) {
        r::GenericJoint::set_motor(
            self,
            from_axis(axis),
            target_position,
            target_velocity,
            stiffness,
            damping,
        );
    }
    fn set_motor_max_force(&mut self, axis: JointAxis, max_force: f64) {
        r::GenericJoint::set_motor_max_force(self, from_axis(axis), max_force);
    }
    fn set_softness(&mut self, natural_frequency: f64, damping_ratio: f64) {
        self.softness = r::SpringCoefficients::new(natural_frequency, damping_ratio);
    }
    fn set_contacts_enabled(&mut self, enabled: bool) {
        r::GenericJoint::set_contacts_enabled(self, enabled);
    }
}

/// The rapier joint a [`JointDesc`] asks for. Revolute and prismatic
/// joints whose two frames share a basis use rapier's typed builders —
/// its reduced-coordinate solver indexes out of bounds on the generic
/// path — and the rest a generic joint with the axis remapped onto X.
fn build_joint(desc: &JointDesc) -> r::GenericJoint {
    let mut joint: r::GenericJoint = match desc.kind {
        JointKind::Revolute { axis } | JointKind::Prismatic { axis } => {
            let revolute = matches!(desc.kind, JointKind::Revolute { .. });
            let same_basis = desc.frame1.rotation.abs_diff_eq(desc.frame2.rotation, 1e-4);
            if same_basis {
                let world_axis = (desc.frame1.rotation * axis).normalize();
                if revolute {
                    r::RevoluteJointBuilder::new(world_axis)
                        .local_anchor1(desc.frame1.translation)
                        .local_anchor2(desc.frame2.translation)
                        .build()
                        .into()
                } else {
                    r::PrismaticJointBuilder::new(world_axis)
                        .local_anchor1(desc.frame1.translation)
                        .local_anchor2(desc.frame2.translation)
                        .build()
                        .into()
                }
            } else {
                let remap = DQuat::from_rotation_arc(DVec3::X, axis);
                let locked = if revolute {
                    r::JointAxesMask::LOCKED_REVOLUTE_AXES
                } else {
                    r::JointAxesMask::LOCKED_PRISMATIC_AXES
                };
                r::GenericJointBuilder::new(locked)
                    .local_frame1(r::Pose {
                        rotation: desc.frame1.rotation * remap,
                        translation: desc.frame1.translation,
                    })
                    .local_frame2(r::Pose {
                        rotation: desc.frame2.rotation * remap,
                        translation: desc.frame2.translation,
                    })
                    .build()
            }
        }
        JointKind::Fixed => r::FixedJointBuilder::new()
            .local_frame1(from_pose(desc.frame1))
            .local_frame2(from_pose(desc.frame2))
            .build()
            .into(),
        JointKind::Spherical => r::SphericalJointBuilder::new()
            .local_anchor1(desc.frame1.translation)
            .local_anchor2(desc.frame2.translation)
            .build()
            .into(),
        JointKind::Generic { locked } => r::GenericJointBuilder::new(from_axes(locked))
            .local_frame1(from_pose(desc.frame1))
            .local_frame2(from_pose(desc.frame2))
            .build(),
    };
    for (axis, limits) in &desc.limits {
        joint.set_limits(from_axis(*axis), *limits);
    }
    for motor in &desc.motors {
        let axis = from_axis(motor.axis);
        if let Some(model) = motor.model {
            joint.set_motor_model(axis, from_model(model));
        }
        match motor.target {
            Some(MotorTarget::Position { target, stiffness, damping }) => {
                joint.set_motor_position(axis, target, stiffness, damping);
            }
            Some(MotorTarget::Velocity { target, damping }) => {
                joint.set_motor_velocity(axis, target, damping);
            }
            None => {}
        }
        if let Some(max_force) = motor.max_force {
            joint.set_motor_max_force(axis, max_force);
        }
    }
    if let Some((frequency, damping)) = desc.softness {
        joint.softness = r::SpringCoefficients::new(frequency, damping);
    }
    joint.set_contacts_enabled(desc.contacts_enabled);
    joint
}

struct PairHooks<'a>(PairExcluded<'a>);

impl r::PhysicsHooks for PairHooks<'_> {
    fn filter_contact_pair(&self, context: &r::PairFilterContext) -> Option<r::SolverFlags> {
        match (context.rigid_body1, context.rigid_body2) {
            (Some(a), Some(b)) if (self.0)(body_id(a), body_id(b)) => None,
            _ => Some(r::SolverFlags::COMPUTE_IMPULSES),
        }
    }
}

impl RapierBackend {
    fn manifolds_of(&self, pair: &r::ContactPair) -> Vec<ContactManifold> {
        let active = pair.has_any_active_contact();
        let frame1 = self.colliders.get(pair.collider1).map(|c| *c.position());
        pair.manifolds
            .iter()
            .map(|manifold| {
                let friction_of = |index: usize| {
                    manifold
                        .data
                        .solver_contacts
                        .iter()
                        .find(|c| c.contact_id[0] as usize == index)
                        .map(|c| c.friction)
                };
                ContactManifold {
                    collider1: collider_id(pair.collider1),
                    collider2: collider_id(pair.collider2),
                    normal: manifold.data.normal,
                    active,
                    points: manifold
                        .points
                        .iter()
                        .enumerate()
                        .map(|(index, point)| {
                            let friction = friction_of(index);
                            ContactPoint {
                                impulse: point.data.impulse,
                                friction: friction.unwrap_or(0.0),
                                solved: friction.is_some(),
                                point: frame1.map_or(point.local_p1, |f| f * point.local_p1),
                                dist: point.dist,
                            }
                        })
                        .collect(),
                }
            })
            .collect()
    }
}

impl PhysicsBackend for RapierBackend {
    fn name(&self) -> &'static str {
        "rapier"
    }

    fn gravity(&self) -> DVec3 {
        self.gravity
    }

    fn set_gravity(&mut self, gravity: DVec3) {
        self.gravity = gravity;
    }

    fn settings(&self) -> SolverSettings {
        SolverSettings {
            dt: self.integration_parameters.dt,
            solver_iterations: self.integration_parameters.num_solver_iterations,
            internal_iterations: self.integration_parameters.num_internal_pgs_iterations,
        }
    }

    fn set_settings(&mut self, settings: SolverSettings) {
        self.integration_parameters.dt = settings.dt;
        self.integration_parameters.num_solver_iterations = settings.solver_iterations;
        self.integration_parameters.num_internal_pgs_iterations = settings.internal_iterations;
    }

    fn step(&mut self, excluded: PairExcluded<'_>) {
        self.physics_pipeline.step(
            self.gravity,
            &self.integration_parameters,
            &mut self.islands,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd_solver,
            &PairHooks(excluded),
            &(),
        );
    }

    fn insert_body(&mut self, desc: BodyDesc) -> BodyId {
        let mut builder = r::RigidBodyBuilder::new(from_kind(desc.kind))
            .pose(from_pose(desc.pose))
            .linvel(desc.linvel)
            .angvel(desc.angvel)
            .linear_damping(desc.linear_damping)
            .angular_damping(desc.angular_damping)
            .user_data(user_data_of(desc.entity));
        if desc.sleeping {
            builder = builder.sleeping(true);
        }
        if let Some(props) = desc.additional_mass {
            builder = builder.additional_mass_properties(from_mass(props));
        }
        body_id(self.bodies.insert(builder.build()))
    }

    fn remove_body(&mut self, id: BodyId) {
        let _ = self.bodies.remove(
            body_handle(id),
            &mut self.islands,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            true,
        );
    }

    fn body(&self, id: BodyId) -> Option<&dyn Body> {
        self.bodies.get(body_handle(id)).map(|b| b as &dyn Body)
    }

    fn body_mut(&mut self, id: BodyId) -> Option<&mut dyn BodyMut> {
        self.bodies.get_mut(body_handle(id)).map(|b| b as &mut dyn BodyMut)
    }

    fn bodies(&self) -> Vec<BodyId> {
        self.bodies.iter().map(|(h, _)| body_id(h)).collect()
    }

    fn recompute_mass(&mut self, id: BodyId) {
        if let Some(body) = self.bodies.get_mut(body_handle(id)) {
            body.recompute_mass_properties_from_colliders(&self.colliders);
        }
    }

    fn sync_collider_positions(&mut self) {
        self.bodies
            .propagate_modified_body_positions_to_colliders(&mut self.colliders);
    }

    fn insert_collider(&mut self, desc: ColliderDesc) -> Option<ColliderId> {
        let mut builder = r::ColliderBuilder::new(shared_shape(desc.shape)?)
            .position(from_pose(desc.pose))
            .user_data(user_data_of(desc.entity))
            .active_hooks(r::ActiveHooks::FILTER_CONTACT_PAIRS);
        if let Some(groups) = desc.groups {
            builder = builder.collision_groups(from_groups(groups));
        }
        if let Some(friction) = desc.friction {
            builder = builder.friction(friction);
        }
        if let Some(restitution) = desc.restitution {
            builder = builder.restitution(restitution);
        }
        if let Some(rule) = desc.friction_combine {
            builder = builder.friction_combine_rule(from_rule(rule));
        }
        if let Some(density) = desc.density {
            builder = builder.density(density);
        }
        let handle = match desc.parent {
            Some(parent) => self.colliders.insert_with_parent(
                builder.build(),
                body_handle(parent),
                &mut self.bodies,
            ),
            None => self.colliders.insert(builder.build()),
        };
        Some(collider_id(handle))
    }

    fn remove_collider(&mut self, id: ColliderId, wake: bool) {
        self.colliders
            .remove(collider_handle(id), &mut self.islands, &mut self.bodies, wake);
    }

    fn collider(&self, id: ColliderId) -> Option<&dyn Collider> {
        self.colliders.get(collider_handle(id)).map(|c| c as &dyn Collider)
    }

    fn collider_mut(&mut self, id: ColliderId) -> Option<&mut dyn ColliderMut> {
        self.colliders
            .get_mut(collider_handle(id))
            .map(|c| c as &mut dyn ColliderMut)
    }

    fn colliders(&self) -> Vec<ColliderId> {
        self.colliders.iter().map(|(h, _)| collider_id(h)).collect()
    }

    fn insert_joint(&mut self, body1: BodyId, body2: BodyId, desc: JointDesc) -> JointId {
        let joint = build_joint(&desc);
        let (body1, body2) = (body_handle(body1), body_handle(body2));
        if desc.reduced {
            if let Some(handle) = self.multibody_joints.insert(body1, body2, joint, true) {
                return reduced_id(handle);
            }
            bevy::log::warn!("rapier: reduced-coordinate insert failed (loop?); using a constraint joint");
        }
        impulse_id(self.impulse_joints.insert(body1, body2, joint, true))
    }

    fn remove_joint(&mut self, id: JointId) {
        match joint_handle(id) {
            JointHandle::Impulse(h) => {
                self.impulse_joints.remove(h, true);
            }
            JointHandle::Reduced(h) => self.multibody_joints.remove(h, true),
        }
    }

    fn joint(&self, id: JointId) -> Option<&dyn Joint> {
        match joint_handle(id) {
            JointHandle::Impulse(h) => self.impulse_joints.get(h).map(|j| &j.data as &dyn Joint),
            JointHandle::Reduced(h) => {
                let (multibody, link) = self.multibody_joints.get(h)?;
                multibody.link(link).map(|l| &l.joint.data as &dyn Joint)
            }
        }
    }

    fn joint_mut(&mut self, id: JointId, wake: bool) -> Option<&mut dyn JointMut> {
        match joint_handle(id) {
            JointHandle::Impulse(h) => self
                .impulse_joints
                .get_mut(h, wake)
                .map(|j| &mut j.data as &mut dyn JointMut),
            JointHandle::Reduced(h) => {
                let (multibody, link) = self.multibody_joints.get_mut(h)?;
                multibody
                    .link_mut(link)
                    .map(|l| &mut l.joint.data as &mut dyn JointMut)
            }
        }
    }

    fn joint_bodies(&self, id: JointId) -> Option<(BodyId, BodyId)> {
        match joint_handle(id) {
            JointHandle::Impulse(h) => self
                .impulse_joints
                .get(h)
                .map(|j| (body_id(j.body1), body_id(j.body2))),
            JointHandle::Reduced(h) => {
                let (multibody, link) = self.multibody_joints.get(h)?;
                let link = multibody.link(link)?;
                let parent = multibody.link(link.parent_id()?)?;
                Some((body_id(parent.rigid_body_handle()), body_id(link.rigid_body_handle())))
            }
        }
    }

    fn joints(&self) -> Vec<JointId> {
        self.multibody_joints
            .iter()
            .map(|(h, ..)| reduced_id(h))
            .chain(self.impulse_joints.iter().map(|(h, _)| impulse_id(h)))
            .collect()
    }

    fn joint_is_reduced(&self, id: JointId) -> bool {
        id.0 & REDUCED_BIT != 0
    }

    fn joint_between(&self, a: BodyId, b: BodyId) -> Option<JointId> {
        let (ha, hb) = (body_handle(a), body_handle(b));
        if let Some((handle, ..)) = self.multibody_joints.joint_between(ha, hb) {
            return Some(reduced_id(handle));
        }
        self.impulse_joints.iter().find_map(|(handle, joint)| {
            ((joint.body1 == ha && joint.body2 == hb) || (joint.body1 == hb && joint.body2 == ha))
                .then(|| impulse_id(handle))
        })
    }

    fn joints_between(&self, a: BodyId, b: BodyId) -> Vec<JointId> {
        let (ha, hb) = (body_handle(a), body_handle(b));
        let reduced = self
            .multibody_joints
            .joint_between(ha, hb)
            .map(|(handle, ..)| reduced_id(handle));
        let impulse = self.impulse_joints.iter().filter_map(|(handle, joint)| {
            ((joint.body1 == ha && joint.body2 == hb) || (joint.body1 == hb && joint.body2 == ha))
                .then(|| impulse_id(handle))
        });
        reduced.into_iter().chain(impulse).collect()
    }

    // Every collider is tested: gearbox casts rays in tests and tools, not
    // per frame.
    fn cast_ray_filtered(
        &self,
        origin: DVec3,
        direction: DVec3,
        max_distance: f64,
        include: &dyn Fn(ColliderId) -> bool,
    ) -> Option<(ColliderId, f64)> {
        let length = direction.length();
        if length <= 0.0 {
            return None;
        }
        let ray = r::Ray::new(origin, direction / length);
        self.colliders
            .iter()
            .filter(|(handle, collider)| collider.is_enabled() && include(collider_id(*handle)))
            .filter_map(|(handle, collider)| {
                let toi =
                    collider
                        .shape()
                        .cast_ray(collider.position(), &ray, max_distance, true)?;
                Some((collider_id(handle), toi))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }

    fn contacts_with(&self, collider: ColliderId) -> Vec<ContactManifold> {
        self.narrow_phase
            .contact_pairs_with(collider_handle(collider))
            .flat_map(|pair| self.manifolds_of(pair))
            .collect()
    }

    fn contacts(&self) -> Vec<ContactManifold> {
        self.narrow_phase
            .contact_pairs()
            .flat_map(|pair| self.manifolds_of(pair))
            .collect()
    }
}
