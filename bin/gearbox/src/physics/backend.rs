//! The physics engine, as gearbox sees it.
//!
//! Everything outside `physics::molla` talks to Molla through
//! [`PhysicsBackend`] and the small accessor traits [`Body`], [`Collider`]
//! and [`Joint`]. Values are SI, f64, glam; handles are opaque ids Molla
//! hands out.

use bevy::prelude::Entity;
pub use glam::{DMat3, DQuat, DVec3};

/// A rigid transform: rotate, then translate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub translation: DVec3,
    pub rotation: DQuat,
}

impl Pose {
    pub const IDENTITY: Self = Self {
        translation: DVec3::ZERO,
        rotation: DQuat::IDENTITY,
    };

    pub fn new(translation: DVec3, rotation: DQuat) -> Self {
        Self {
            translation,
            rotation,
        }
    }

    pub fn from_translation(translation: DVec3) -> Self {
        Self {
            translation,
            rotation: DQuat::IDENTITY,
        }
    }

    pub fn inverse(&self) -> Self {
        let rotation = self.rotation.inverse();
        Self {
            translation: rotation * -self.translation,
            rotation,
        }
    }

    pub fn transform_point(&self, point: DVec3) -> DVec3 {
        self.rotation * point + self.translation
    }

    pub fn transform_vector(&self, vector: DVec3) -> DVec3 {
        self.rotation * vector
    }

    pub fn inverse_transform_point(&self, point: DVec3) -> DVec3 {
        self.rotation.inverse() * (point - self.translation)
    }
}

impl Default for Pose {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl std::ops::Mul for Pose {
    type Output = Pose;
    fn mul(self, rhs: Pose) -> Pose {
        Pose {
            translation: self.rotation * rhs.translation + self.translation,
            rotation: self.rotation * rhs.rotation,
        }
    }
}

impl std::ops::Mul<DVec3> for Pose {
    type Output = DVec3;
    fn mul(self, rhs: DVec3) -> DVec3 {
        self.transform_point(rhs)
    }
}

macro_rules! id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub u64);
    };
}
id!(BodyId);
id!(ColliderId);
id!(JointId);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyKind {
    Dynamic,
    Fixed,
    Kinematic,
}

/// Mass, centre of mass in the body frame, and inertia about it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MassProps {
    pub local_com: DVec3,
    pub mass: f64,
    pub inertia: Inertia,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Inertia {
    /// Principal moments, axes aligned with the body frame.
    Principal(DVec3),
    /// Full tensor in the body frame.
    Tensor(DMat3),
}

#[derive(Clone, Debug)]
pub struct BodyDesc {
    pub kind: BodyKind,
    pub pose: Pose,
    pub linvel: DVec3,
    pub angvel: DVec3,
    pub linear_damping: f64,
    pub angular_damping: f64,
    pub sleeping: bool,
    /// Added on top of what the colliders contribute.
    pub additional_mass: Option<MassProps>,
    pub entity: Option<Entity>,
}

impl BodyDesc {
    pub fn new(kind: BodyKind) -> Self {
        Self {
            kind,
            pose: Pose::IDENTITY,
            linvel: DVec3::ZERO,
            angvel: DVec3::ZERO,
            linear_damping: 0.0,
            angular_damping: 0.0,
            sleeping: false,
            additional_mass: None,
            entity: None,
        }
    }

    pub fn dynamic() -> Self {
        Self::new(BodyKind::Dynamic)
    }

    pub fn fixed() -> Self {
        Self::new(BodyKind::Fixed)
    }

    pub fn pose(mut self, pose: Pose) -> Self {
        self.pose = pose;
        self
    }

    pub fn entity(mut self, entity: Entity) -> Self {
        self.entity = Some(entity);
        self
    }
}

/// Collision geometry, in the collider's own frame. Cylinders and
/// capsules-by-axis run along Y.
#[derive(Clone, Debug)]
pub enum Shape {
    Cuboid {
        half_extents: DVec3,
    },
    Ball {
        radius: f64,
    },
    Capsule {
        a: DVec3,
        b: DVec3,
        radius: f64,
    },
    Cylinder {
        half_height: f64,
        radius: f64,
    },
    /// A cylinder shrunk by `border_radius` and rounded back out by it.
    RoundCylinder {
        half_height: f64,
        radius: f64,
        border_radius: f64,
    },
    ConvexHull {
        points: Vec<DVec3>,
    },
    ConvexDecomposition {
        vertices: Vec<DVec3>,
        indices: Vec<[u32; 3]>,
    },
    TriMesh {
        vertices: Vec<DVec3>,
        indices: Vec<[u32; 3]>,
    },
    /// `heights` is row-major, `rows × cols`, spanning `scale.x × scale.z`
    /// centred on the collider origin; `scale.y` multiplies the heights.
    Heightfield {
        rows: usize,
        cols: usize,
        heights: Vec<f64>,
        scale: DVec3,
    },
}

/// What a collider's shape looks like from outside, for gizmos and
/// geometry queries. Meshes report their bounds only.
#[derive(Clone, Debug)]
pub enum ShapeView {
    Cuboid {
        half_extents: DVec3,
    },
    Ball {
        radius: f64,
    },
    Capsule {
        a: DVec3,
        b: DVec3,
        radius: f64,
    },
    Cylinder {
        half_height: f64,
        radius: f64,
    },
    RoundCylinder {
        half_height: f64,
        radius: f64,
        border_radius: f64,
    },
    ConvexPolyhedron {
        points: Vec<DVec3>,
        edges: Vec<[u32; 2]>,
    },
    Other,
}

/// Axis-aligned bounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub mins: DVec3,
    pub maxs: DVec3,
}

impl Aabb {
    pub fn half_extents(&self) -> DVec3 {
        (self.maxs - self.mins) * 0.5
    }

    pub fn center(&self) -> DVec3 {
        (self.maxs + self.mins) * 0.5
    }
}

/// Two colliders touch when each one's `memberships` meets the other's
/// `filter`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CollisionGroups {
    pub memberships: u32,
    pub filter: u32,
}

impl CollisionGroups {
    pub const ALL: Self = Self {
        memberships: u32::MAX,
        filter: u32::MAX,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CombineRule {
    Average,
    Min,
    Multiply,
    Max,
}

#[derive(Clone, Debug)]
pub struct ColliderDesc {
    pub shape: Shape,
    /// In the parent body's frame, or the world's when there is no parent.
    pub pose: Pose,
    pub parent: Option<BodyId>,
    pub friction: Option<f64>,
    pub restitution: Option<f64>,
    pub friction_combine: Option<CombineRule>,
    pub density: Option<f64>,
    pub groups: Option<CollisionGroups>,
    pub entity: Option<Entity>,
}

impl ColliderDesc {
    pub fn new(shape: Shape) -> Self {
        Self {
            shape,
            pose: Pose::IDENTITY,
            parent: None,
            friction: None,
            restitution: None,
            friction_combine: None,
            density: None,
            groups: None,
            entity: None,
        }
    }

    pub fn pose(mut self, pose: Pose) -> Self {
        self.pose = pose;
        self
    }

    pub fn translation(mut self, translation: DVec3) -> Self {
        self.pose.translation = translation;
        self
    }

    pub fn parent(mut self, parent: BodyId) -> Self {
        self.parent = Some(parent);
        self
    }

    pub fn friction(mut self, friction: f64) -> Self {
        self.friction = Some(friction);
        self
    }

    pub fn restitution(mut self, restitution: f64) -> Self {
        self.restitution = Some(restitution);
        self
    }

    pub fn density(mut self, density: f64) -> Self {
        self.density = Some(density);
        self
    }

    pub fn entity(mut self, entity: Entity) -> Self {
        self.entity = Some(entity);
        self
    }
}

/// One degree of freedom of a joint, in the joint's first frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JointAxis {
    LinX,
    LinY,
    LinZ,
    AngX,
    AngY,
    AngZ,
}

impl JointAxis {
    pub const ALL: [JointAxis; 6] = [
        JointAxis::LinX,
        JointAxis::LinY,
        JointAxis::LinZ,
        JointAxis::AngX,
        JointAxis::AngY,
        JointAxis::AngZ,
    ];

    pub const fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// A set of [`JointAxis`], as bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct JointAxes(pub u8);

impl JointAxes {
    pub const NONE: Self = Self(0);
    pub const LIN: Self =
        Self(JointAxis::LinX.bit() | JointAxis::LinY.bit() | JointAxis::LinZ.bit());
    pub const ANG: Self =
        Self(JointAxis::AngX.bit() | JointAxis::AngY.bit() | JointAxis::AngZ.bit());
    pub const ALL: Self = Self(Self::LIN.0 | Self::ANG.0);

    pub const fn of(axis: JointAxis) -> Self {
        Self(axis.bit())
    }

    pub const fn with(self, axis: JointAxis) -> Self {
        Self(self.0 | axis.bit())
    }

    pub const fn without(self, axis: JointAxis) -> Self {
        Self(self.0 & !axis.bit())
    }

    pub const fn contains(self, axis: JointAxis) -> bool {
        self.0 & axis.bit() != 0
    }
}

impl std::ops::BitOr for JointAxes {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// How a motor turns its spring into effort.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotorModel {
    /// Stiffness and damping are accelerations per unit error.
    Acceleration,
    /// Stiffness and damping are forces per unit error.
    Force,
}

/// A joint motor as authored or as it stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motor {
    pub target_position: f64,
    pub target_velocity: f64,
    pub stiffness: f64,
    pub damping: f64,
    pub max_force: f64,
    pub model: MotorModel,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MotorTarget {
    Position {
        target: f64,
        stiffness: f64,
        damping: f64,
    },
    Velocity {
        target: f64,
        damping: f64,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct MotorDesc {
    pub axis: JointAxis,
    pub target: Option<MotorTarget>,
    pub max_force: Option<f64>,
    pub model: Option<MotorModel>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum JointKind {
    /// One free rotation about `axis` of the first frame.
    Revolute {
        axis: DVec3,
    },
    /// One free translation along `axis` of the first frame.
    Prismatic {
        axis: DVec3,
    },
    Fixed,
    Spherical,
    /// Exactly the `locked` axes are held; the joint's X is frame X.
    Generic {
        locked: JointAxes,
    },
}

#[derive(Clone, Debug)]
pub struct JointDesc {
    pub kind: JointKind,
    /// Joint frame in the first body's frame.
    pub frame1: Pose,
    /// Joint frame in the second body's frame.
    pub frame2: Pose,
    pub limits: Vec<(JointAxis, [f64; 2])>,
    pub motors: Vec<MotorDesc>,
    /// Constraint spring: natural frequency (Hz) and damping ratio.
    pub softness: Option<(f64, f64)>,
    /// Whether the two jointed bodies still collide with each other.
    pub contacts_enabled: bool,
    /// Close an articulation loop without adding a reduced-coordinate edge.
    pub loop_closure: bool,
}

impl JointDesc {
    pub fn new(kind: JointKind, frame1: Pose, frame2: Pose) -> Self {
        Self {
            kind,
            frame1,
            frame2,
            limits: Vec::new(),
            motors: Vec::new(),
            softness: None,
            contacts_enabled: false,
            loop_closure: false,
        }
    }
}

/// One contact point of a manifold.
#[derive(Clone, Copy, Debug)]
pub struct ContactPoint {
    /// Normal impulse the last step applied here, N·s.
    pub impulse: f64,
    /// Friction coefficient the solver used here; 0 when not `solved`.
    pub friction: f64,
    /// Whether the last step's solver acted on this point.
    pub solved: bool,
    /// World position, on the first collider.
    pub point: DVec3,
    /// Signed distance; negative when penetrating.
    pub dist: f64,
}

/// The contacts two colliders share along one normal.
#[derive(Clone, Debug)]
pub struct ContactManifold {
    pub collider1: ColliderId,
    pub collider2: ColliderId,
    /// World normal, pointing from `collider1` to `collider2`.
    pub normal: DVec3,
    /// Whether the solver is acting on any point of it.
    pub active: bool,
    pub points: Vec<ContactPoint>,
}

/// Read access to one rigid body.
pub trait Body {
    fn position(&self) -> Pose;
    fn translation(&self) -> DVec3 {
        self.position().translation
    }
    fn rotation(&self) -> DQuat {
        self.position().rotation
    }
    fn linvel(&self) -> DVec3;
    fn angvel(&self) -> DVec3;
    fn mass(&self) -> f64;
    /// Centre of mass, world.
    fn center_of_mass(&self) -> DVec3;
    /// Centre of mass, body frame.
    fn local_center_of_mass(&self) -> DVec3;
    /// Principal moments of inertia about the centre of mass.
    fn principal_inertia(&self) -> DVec3;
    /// Inertia tensor about the centre of mass, in the body frame.
    fn inertia_tensor(&self) -> DMat3;
    fn kind(&self) -> BodyKind;
    fn is_dynamic(&self) -> bool {
        self.kind() == BodyKind::Dynamic
    }
    fn is_fixed(&self) -> bool {
        self.kind() == BodyKind::Fixed
    }
    fn is_enabled(&self) -> bool;
    fn is_sleeping(&self) -> bool;
    fn colliders(&self) -> Vec<ColliderId>;
    fn entity(&self) -> Option<Entity>;
    /// Velocity of the body's material at a world point.
    fn velocity_at_point(&self, point: DVec3) -> DVec3 {
        self.linvel() + self.angvel().cross(point - self.center_of_mass())
    }
}

/// Write access to one rigid body.
pub trait BodyMut: Body {
    fn set_position(&mut self, pose: Pose, wake: bool);
    fn set_translation(&mut self, translation: DVec3, wake: bool) {
        let rotation = self.rotation();
        self.set_position(
            Pose {
                translation,
                rotation,
            },
            wake,
        );
    }
    fn set_rotation(&mut self, rotation: DQuat, wake: bool) {
        let translation = self.translation();
        self.set_position(
            Pose {
                translation,
                rotation,
            },
            wake,
        );
    }
    fn set_linvel(&mut self, linvel: DVec3, wake: bool);
    fn set_angvel(&mut self, angvel: DVec3, wake: bool);
    fn set_kind(&mut self, kind: BodyKind, wake: bool);
    fn set_enabled(&mut self, enabled: bool);
    /// Continuous collision detection, where the engine has it.
    fn enable_ccd(&mut self, enabled: bool);
    fn wake_up(&mut self, strong: bool);
    fn sleep(&mut self);
    fn set_linear_damping(&mut self, damping: f64);
    fn set_angular_damping(&mut self, damping: f64);
    /// Replace what the body carries on top of its colliders' mass.
    fn set_additional_mass(&mut self, props: MassProps, wake: bool);
    fn add_force(&mut self, force: DVec3, wake: bool);
    fn add_torque(&mut self, torque: DVec3, wake: bool);
    fn reset_forces(&mut self, wake: bool);
    fn apply_impulse(&mut self, impulse: DVec3, wake: bool);
    fn apply_torque_impulse(&mut self, impulse: DVec3, wake: bool);
    fn apply_impulse_at_point(&mut self, impulse: DVec3, point: DVec3, wake: bool);
}

/// Read access to one collider.
pub trait Collider {
    /// World pose.
    fn position(&self) -> Pose;
    /// Pose in the parent body's frame, when it has a parent.
    fn position_wrt_parent(&self) -> Option<Pose>;
    fn parent(&self) -> Option<BodyId>;
    fn shape(&self) -> ShapeView;
    /// Bounds in the collider's own frame.
    fn local_aabb(&self) -> Aabb;
    /// Bounds in the world.
    fn aabb(&self) -> Aabb;
    /// Bounds of the shape placed at `pose`.
    fn aabb_at(&self, pose: Pose) -> Aabb;
    fn is_sensor(&self) -> bool;
    fn groups(&self) -> CollisionGroups;
    fn friction(&self) -> f64;
    fn restitution(&self) -> f64;
    fn mass(&self) -> f64;
    fn is_enabled(&self) -> bool;
    fn entity(&self) -> Option<Entity>;
}

/// Write access to one collider.
pub trait ColliderMut: Collider {
    fn set_shape(&mut self, shape: Shape);
    fn set_position_wrt_parent(&mut self, pose: Pose);
    fn set_position(&mut self, pose: Pose);
    fn set_friction(&mut self, friction: f64);
    fn set_restitution(&mut self, restitution: f64);
    fn set_friction_combine_rule(&mut self, rule: CombineRule);
    fn set_density(&mut self, density: f64);
    fn set_mass(&mut self, mass: f64);
    fn set_groups(&mut self, groups: CollisionGroups);
    fn set_enabled(&mut self, enabled: bool);
}

/// Read access to one joint.
pub trait Joint {
    /// Current coordinate in motor-target space, or `None` when unavailable.
    fn motor_position(&self, _axis: JointAxis) -> Option<f64> { None }
    fn frame1(&self) -> Pose;
    fn frame2(&self) -> Pose;
    fn locked_axes(&self) -> JointAxes;
    fn limits(&self, axis: JointAxis) -> Option<[f64; 2]>;
    fn motor(&self, axis: JointAxis) -> Option<Motor>;
    fn contacts_enabled(&self) -> bool;
    fn is_enabled(&self) -> bool;
}

/// Write access to one joint.
pub trait JointMut: Joint {
    fn set_frame1(&mut self, frame: Pose);
    fn set_frame2(&mut self, frame: Pose);
    fn set_limits(&mut self, axis: JointAxis, limits: [f64; 2]);
    fn set_motor_model(&mut self, axis: JointAxis, model: MotorModel);
    fn set_motor_velocity(&mut self, axis: JointAxis, target: f64, damping: f64);
    fn set_motor_position(&mut self, axis: JointAxis, target: f64, stiffness: f64, damping: f64);
    fn set_motor(
        &mut self,
        axis: JointAxis,
        target_position: f64,
        target_velocity: f64,
        stiffness: f64,
        damping: f64,
    );
    fn set_motor_max_force(&mut self, axis: JointAxis, max_force: f64);
    fn set_softness(&mut self, natural_frequency: f64, damping_ratio: f64);
    fn set_contacts_enabled(&mut self, enabled: bool);
}

/// The solver settings gearbox turns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SolverSettings {
    pub dt: f64,
    pub solver_iterations: usize,
    pub internal_iterations: usize,
}

/// Decides which body pairs the step must not let touch.
pub type PairExcluded<'a> = &'a (dyn Fn(BodyId, BodyId) -> bool + Send + Sync);

#[derive(Clone, Debug)]
pub struct TrackForceDesc {
    pub carrier: BodyId,
    pub sprocket: BodyId,
    pub joint: JointId,
    pub contact_colliders: Vec<ColliderId>,
    pub local_axle: DVec3,
    pub local_forward: DVec3,
    pub pitch_radius: f64,
    pub longitudinal_friction: f64,
    pub lateral_friction: f64,
    pub slip_damping: f64,
    pub max_torque: f64,
    pub max_power: f64,
    pub speed_gain: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct TrackForceOutput {
    pub angular_velocity: f64,
    pub travel: f64,
    pub motor_torque: f64,
    pub longitudinal_force: f64,
    pub normal_load: f64,
    pub contacts: usize,
}

// ── Actuator devices ────────────────────────────────────────────────────

/// A propeller, belt or connector the backend steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeviceId(pub u64);

/// What a motor or belt is told to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DeviceCommand {
    /// Reach a position (rad or m).
    Position(f64),
    /// Run at a velocity (rad/s or m/s).
    Velocity(f64),
    /// Apply a force or torque (N or N·m).
    Force(f64),
}

/// Speed limit, acceleration, PID gains on the position error and soft
/// position limits of a motor controller.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeviceLimits {
    pub max_velocity: f64,
    pub acceleration: Option<f64>,
    pub pid: [f64; 3],
    pub position_limits: Option<[f64; 2]>,
}

impl Default for DeviceLimits {
    fn default() -> Self {
        Self {
            max_velocity: 10.0,
            acceleration: None,
            pid: [10.0, 0.0, 0.0],
            position_limits: None,
        }
    }
}

/// A motor or belt setting besides its command.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DeviceSetting {
    /// Speed position control runs at.
    Speed(f64),
    Acceleration(Option<f64>),
    Pid([f64; 3]),
    /// Force or torque available to a motor.
    MaxForce(f64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotorOutput {
    pub position: f64,
    pub velocity: f64,
    pub command: DeviceCommand,
    pub commanded_velocity: f64,
    /// Force or torque the motor applied over the last step.
    pub force: f64,
    pub brake_damping: f64,
    pub brake_force: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PropellerDesc {
    pub body: BodyId,
    /// Shaft frame in the body frame: +X the shaft, origin the centre of thrust.
    pub frame: Pose,
    /// `[t1, t2]` of `T = t1·|ω|·ω − t2·|ω|·V`.
    pub thrust: [f64; 2],
    /// `[q1, q2]` of `Q = q1·|ω|·ω − q2·|ω|·V`; the body takes `−Q`.
    pub torque: [f64; 2],
    pub limits: DeviceLimits,
    pub max_torque: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PropellerOutput {
    pub omega: f64,
    pub thrust: f64,
    pub torque: f64,
    pub advance: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BeltDesc {
    /// Colliders forming the running surface.
    pub colliders: Vec<ColliderId>,
    /// Running direction in each collider's frame.
    pub direction: DVec3,
    pub limits: DeviceLimits,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BeltOutput {
    pub position: f64,
    pub velocity: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ConnectorKind {
    #[default]
    Symmetric,
    Active,
    Passive,
}

/// A docking face (Webots `Connector`); `body` `None` fixes it to the world.
#[derive(Clone, Debug, PartialEq)]
pub struct ConnectorDesc {
    pub body: Option<BodyId>,
    /// Docking frame in the body frame (world for none); +X out of the face.
    pub frame: Pose,
    pub model: String,
    pub kind: ConnectorKind,
    pub auto_lock: bool,
    pub unilateral_lock: bool,
    pub unilateral_unlock: bool,
    pub distance_tolerance: f64,
    pub axis_tolerance: f64,
    pub rotation_tolerance: f64,
    pub rotations: u32,
    pub snap: bool,
    pub tensile_strength: Option<f64>,
    pub shear_strength: Option<f64>,
    /// Natural frequency (Hz) of the link.
    pub stiffness: f64,
}

impl Default for ConnectorDesc {
    fn default() -> Self {
        Self {
            body: None,
            frame: Pose::IDENTITY,
            model: String::new(),
            kind: ConnectorKind::Symmetric,
            auto_lock: false,
            unilateral_lock: true,
            unilateral_unlock: true,
            distance_tolerance: 0.01,
            axis_tolerance: 0.2,
            rotation_tolerance: 0.2,
            rotations: 4,
            snap: true,
            tensile_strength: None,
            shear_strength: None,
            stiffness: 30.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ConnectorOutput {
    pub presence: Option<DeviceId>,
    pub locked: bool,
    pub linked: Option<DeviceId>,
    pub tensile: f64,
    pub shear: f64,
}

/// What a copter (multirotor) controller holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CopterCommand {
    /// Rotors stopped.
    Off,
    /// Heading-frame velocity (forward, left, up; m/s) and yaw rate (rad/s).
    Velocity { forward: f64, left: f64, up: f64, yaw_rate: f64 },
    /// Roll (right side down), pitch (nose down), yaw rate and climb rate.
    Attitude { roll: f64, pitch: f64, yaw_rate: f64, climb: f64 },
}

/// Limits and loop gains of a copter controller.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CopterLimits {
    pub max_tilt: f64,
    pub max_speed: f64,
    pub max_climb: f64,
    pub max_yaw_rate: f64,
    pub velocity_gain: f64,
    /// Time constant (s) of the wind and payload disturbance observer.
    pub disturbance_time: f64,
    pub attitude_frequency: f64,
    pub yaw_gain: f64,
}

impl Default for CopterLimits {
    fn default() -> Self {
        Self {
            max_tilt: 0.5,
            max_speed: 10.0,
            max_climb: 3.0,
            max_yaw_rate: 1.5,
            velocity_gain: 1.5,
            disturbance_time: 0.5,
            attitude_frequency: 6.0,
            yaw_gain: 4.0,
        }
    }
}

/// A copter controller flying `body` on `propellers`.
#[derive(Clone, Debug, PartialEq)]
pub struct CopterDesc {
    pub body: BodyId,
    /// Flight frame in the body frame: +X forward, +Y left, +Z up.
    pub frame: Pose,
    pub propellers: Vec<DeviceId>,
    pub limits: CopterLimits,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CopterOutput {
    pub command: CopterCommand,
    pub roll: f64,
    pub pitch: f64,
    /// Heading-frame velocity: forward, left, up.
    pub velocity: [f64; 3],
    pub yaw_rate: f64,
    pub thrust: f64,
    pub saturated: bool,
}

/// Aerodynamic drag on a body: `CdA` along each axis of a frame whose origin
/// is the centre of pressure.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DragDesc {
    pub body: BodyId,
    pub frame: Pose,
    /// Drag coefficient times area along the frame's X, Y and Z (m²).
    pub area: DVec3,
    /// Air density (kg/m³).
    pub density: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DragOutput {
    /// Airspeed of the centre of pressure in the drag frame (m/s).
    pub airspeed: DVec3,
    /// Drag force in world axes (N), averaged over the last step.
    pub force: DVec3,
    /// Wind at the body (m/s, world).
    pub wind: DVec3,
}

#[derive(Clone, Copy, Debug)]
pub struct WheelForceDesc {
    pub body: BodyId,
    pub joint: JointId,
    pub local_hub: DVec3,
    pub forward: DVec3,
    pub radius: f64,
    pub supported_mass: f64,
    pub tyre: Option<PressureTyreDesc>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PressureTyreDesc {
    pub width: f64,
    pub pressure_pa: f64,
    pub min_pressure_pa: f64,
    pub max_pressure_pa: f64,
    pub pressure_rate_pa_s: f64,
    pub carcass_stiffness: f64,
    pub tread_stiffness: f64,
    pub damping_ratio: f64,
    pub hysteresis_fraction: f64,
}

impl PressureTyreDesc {
    /// Uncalibrated reference properties; pressures are gauge Pa.
    pub fn reference(width: f64) -> Self {
        Self {
            width,
            pressure_pa: 180_000.0,
            min_pressure_pa: 50_000.0,
            max_pressure_pa: 400_000.0,
            pressure_rate_pa_s: 100_000.0,
            carcass_stiffness: 400_000.0,
            tread_stiffness: 2_600_000.0,
            damping_ratio: 0.7,
            hysteresis_fraction: 0.1,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PressureTyreOutput {
    pub hub: DVec3,
    pub forward: DVec3,
    pub ground: Option<TyreGroundPlane>,
    pub radius: f64,
    pub width: f64,
    pub pressure_pa: f64,
    pub target_pressure_pa: f64,
    pub min_pressure_pa: f64,
    pub max_pressure_pa: f64,
    pub loaded_radius: f64,
    pub deflection: f64,
    pub patch_length: f64,
    pub patch_width: f64,
    pub patch_area: f64,
    /// Last-step mean rolling-resistance moment in world axes (N m).
    pub rolling_moment: DVec3,
}

#[derive(Clone, Copy, Debug)]
pub struct TyreGroundPlane {
    pub point: DVec3,
    pub normal: DVec3,
}

#[derive(Clone, Debug)]
pub struct TerrainFrictionGrid {
    pub origin: [f64; 2],
    pub cell_size: [f64; 2],
    pub cols: usize,
    pub rows: usize,
    pub values: Vec<f64>,
}

/// Last-step mean tyre loads and normal-impulse-weighted contact/slip readout.
#[derive(Clone, Copy, Debug, Default)]
pub struct WheelForceOutput {
    pub in_contact: bool,
    /// A sleeping tyre's retained reaction, not a newly applied wrench.
    pub held_support: bool,
    pub normal: DVec3,
    pub contact_point: DVec3,
    pub normal_force: f64,
    pub grip_force: f64,
    /// Last-step mean resultant tyre force in world axes (N).
    pub force: DVec3,
    /// Last-step mean aligning moment in world axes (N m), excluding lever arm.
    pub aligning_moment: DVec3,
    pub slip_ratio: f64,
    pub slip_angle: f64,
    pub pressure: Option<PressureTyreOutput>,
}

/// Molla, as gearbox drives it.
pub trait PhysicsBackend: Send + Sync {
    /// Short name, for logs.
    fn name(&self) -> &'static str;

    /// Runs `f` on the Molla runtime scene, holding its lock only for the
    /// call.
    fn with_molla_scene(&self, f: &mut dyn FnMut(&molla_sim::runtime::RigidScene));
    /// Molla handle of a body, for lookups inside `with_molla_scene`.
    fn molla_body_handle(&self, body: BodyId) -> Option<molla_sim::runtime::BodyHandle>;

    fn configure_track(&mut self, desc: TrackForceDesc) -> Result<(), String>;
    fn set_track_speed(&mut self, sprocket: BodyId, speed: f64) -> Result<(), String>;
    fn track_output(&self, sprocket: BodyId) -> Option<TrackForceOutput>;
    fn remove_track(&mut self, sprocket: BodyId);

    /// A motor on a revolute or prismatic joint, holding its position.
    fn insert_motor(&mut self, joint: JointId, limits: DeviceLimits, max_force: f64) -> Result<(), String>;
    fn remove_motor(&mut self, joint: JointId);
    fn command_motor(&mut self, joint: JointId, command: DeviceCommand) -> Result<(), String>;
    fn configure_motor(&mut self, joint: JointId, setting: DeviceSetting) -> Result<(), String>;
    /// Brake damping on a revolute or prismatic joint; zero releases it.
    fn set_joint_brake(&mut self, joint: JointId, damping: f64) -> Result<(), String>;
    /// The motor or brake of `joint` over the last step.
    fn motor_output(&self, joint: JointId) -> Option<MotorOutput>;
    fn insert_propeller(&mut self, desc: PropellerDesc) -> Result<DeviceId, String>;
    fn set_propeller_speed(&mut self, id: DeviceId, omega: f64) -> Result<(), String>;
    fn propeller_output(&self, id: DeviceId) -> Option<PropellerOutput>;
    fn insert_belt(&mut self, desc: BeltDesc) -> Result<DeviceId, String>;
    fn command_belt(&mut self, id: DeviceId, command: DeviceCommand) -> Result<(), String>;
    fn configure_belt(&mut self, id: DeviceId, setting: DeviceSetting) -> Result<(), String>;
    fn belt_output(&self, id: DeviceId) -> Option<BeltOutput>;
    fn insert_connector(&mut self, desc: ConnectorDesc) -> Result<DeviceId, String>;
    /// Latches or unlatches a connector.
    fn lock_connector(&mut self, id: DeviceId, lock: bool) -> Result<(), String>;
    fn connector_output(&self, id: DeviceId) -> Option<ConnectorOutput>;
    /// A copter (multirotor) controller over propellers already inserted.
    fn insert_copter(&mut self, desc: CopterDesc) -> Result<DeviceId, String>;
    fn command_copter(&mut self, id: DeviceId, command: CopterCommand) -> Result<(), String>;
    fn copter_output(&self, id: DeviceId) -> Option<CopterOutput>;
    /// Aerodynamic drag on a body.
    fn insert_drag(&mut self, desc: DragDesc) -> Result<DeviceId, String>;
    fn drag_output(&self, id: DeviceId) -> Option<DragOutput>;
    /// The wind everywhere (m/s, world).
    fn set_wind(&mut self, wind: DVec3);
    /// The wind one body flies in; `None` returns it to the world's.
    fn set_body_wind(&mut self, body: BodyId, wind: Option<DVec3>);
    /// Removes a propeller, belt, connector, copter or drag element.
    fn remove_device(&mut self, id: DeviceId);

    fn configure_wheel(&mut self, desc: WheelForceDesc) -> Result<(), String>;
    fn register_wheel_ground(
        &mut self,
        collider: ColliderId,
        friction: Option<TerrainFrictionGrid>,
    ) -> Result<(), String>;
    fn wheel_output(&self, body: BodyId) -> Option<WheelForceOutput>;
    fn set_wheel_pressures(&mut self, targets: &[(BodyId, f64)]) -> Result<(), String>;
    fn wheel_drive_sign(&self, joint: JointId) -> f64;

    fn gravity(&self) -> DVec3;
    fn set_gravity(&mut self, gravity: DVec3);
    fn settings(&self) -> SolverSettings;
    fn set_settings(&mut self, settings: SolverSettings);
    /// Advance by `settings().dt`, skipping contacts of excluded pairs.
    fn step(&mut self, excluded: PairExcluded<'_>);

    /// Bodies isolated internally during the latest step attempt.
    fn quarantined_bodies(&self) -> Vec<BodyId>;

    fn insert_body(&mut self, desc: BodyDesc) -> BodyId;
    /// Takes the body's colliders and joints with it.
    fn remove_body(&mut self, id: BodyId);
    fn body(&self, id: BodyId) -> Option<&dyn Body>;
    fn body_mut(&mut self, id: BodyId) -> Option<&mut dyn BodyMut>;
    fn bodies(&self) -> Vec<BodyId>;
    /// Set a validated group of world poses without sequential subtree transforms.
    fn set_body_poses(
        &mut self,
        poses: &[(BodyId, Pose)],
        reset_velocity: bool,
    ) -> Result<(), String>;
    fn contains_body(&self, id: BodyId) -> bool {
        self.body(id).is_some()
    }
    /// Rebuild a body's mass from its colliders, after their shapes or
    /// densities changed.
    fn recompute_mass(&mut self, id: BodyId);
    /// Move every collider to where its parent body now is, without a step.
    fn sync_collider_positions(&mut self);

    /// `None` when the shape cannot be built (degenerate mesh).
    fn insert_collider(&mut self, desc: ColliderDesc) -> Option<ColliderId>;
    fn remove_collider(&mut self, id: ColliderId, wake: bool);
    fn collider(&self, id: ColliderId) -> Option<&dyn Collider>;
    fn collider_mut(&mut self, id: ColliderId) -> Option<&mut dyn ColliderMut>;
    fn colliders(&self) -> Vec<ColliderId>;

    fn insert_joint(&mut self, body1: BodyId, body2: BodyId, desc: JointDesc) -> JointId;
    fn remove_joint(&mut self, id: JointId);
    fn joint(&self, id: JointId) -> Option<&dyn Joint>;
    /// `wake` wakes the jointed bodies.
    fn joint_mut(&mut self, id: JointId, wake: bool) -> Option<&mut dyn JointMut>;
    fn joint_bodies(&self, id: JointId) -> Option<(BodyId, BodyId)>;
    fn joints(&self) -> Vec<JointId>;
    /// Whether the engine solves this joint in reduced coordinates.
    fn joint_is_reduced(&self, id: JointId) -> bool;
    /// The joint between two bodies, in either order.
    fn joint_between(&self, a: BodyId, b: BodyId) -> Option<JointId> {
        self.joints().into_iter().find(|id| {
            self.joint_bodies(*id)
                .is_some_and(|(x, y)| (x == a && y == b) || (x == b && y == a))
        })
    }

    /// Every joint between two bodies, in either order.
    fn joints_between(&self, a: BodyId, b: BodyId) -> Vec<JointId> {
        self.joints()
            .into_iter()
            .filter(|id| {
                self.joint_bodies(*id)
                    .is_some_and(|(x, y)| (x == a && y == b) || (x == b && y == a))
            })
            .collect()
    }

    /// Nearest hit of a ray within `max_distance`, as its collider and
    /// the distance along `direction` (which need not be unit length).
    fn cast_ray(
        &self,
        origin: DVec3,
        direction: DVec3,
        max_distance: f64,
    ) -> Option<(ColliderId, f64)> {
        self.cast_ray_filtered(origin, direction, max_distance, &|_| true)
    }

    /// [`Self::cast_ray`] against the colliders `include` accepts.
    fn cast_ray_filtered(
        &self,
        origin: DVec3,
        direction: DVec3,
        max_distance: f64,
        include: &dyn Fn(ColliderId) -> bool,
    ) -> Option<(ColliderId, f64)>;

    /// Manifolds of the last step that involve `collider`.
    fn contacts_with(&self, collider: ColliderId) -> Vec<ContactManifold>;
    /// Every manifold of the last step.
    fn contacts(&self) -> Vec<ContactManifold>;
}

impl<'a> dyn PhysicsBackend + 'a {
    /// `f` on the Molla runtime scene, under its lock.
    pub fn molla<R>(&self, f: impl FnOnce(&molla_sim::runtime::RigidScene) -> R) -> R {
        let (mut f, mut out) = (Some(f), None);
        self.with_molla_scene(&mut |scene| out = f.take().map(|f| f(scene)));
        out.expect("with_molla_scene runs its closure once")
    }
}
