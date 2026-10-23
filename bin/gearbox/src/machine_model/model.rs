//! The resolved machine records. Everything here is in SI units and in the
//! normalized world convention (right-handed, Z up, metres, radians); no
//! record holds a Bevy entity or a backend handle.

use std::collections::BTreeMap;

use bevy::math::{DQuat, DVec3};

use super::diagnostics::{Diagnostic, Severity};

/// Highest `gearbox:machine:schemaVersion` this runtime understands; the
/// `schema-v1-only` rollback build reads only legacy assets.
pub const SCHEMA_VERSION_CURRENT: i64 = if cfg!(feature = "schema-v1-only") { 1 } else { 2 };
/// Version tag of the serialized manifest.
pub const MANIFEST_VERSION: &str = "gearbox.robot_manifest.v1";
/// Convention every pose and vector of the manifest is expressed in.
pub const WORLD_CONVENTION: &str = "right_handed_z_up_si";
/// Default validity of a live velocity or effort command (ms).
pub const DEFAULT_COMMAND_VALIDITY_MS: u64 = 500;
/// Longest validity a live command may ask for (ms).
pub const MAX_COMMAND_VALIDITY_MS: u64 = 10_000;
/// `PhysxRigidBodyAPI` fallback body damping (1/s): strict profiles.
pub const SCHEMA_LINEAR_DAMPING: f64 = 0.0;
pub const SCHEMA_ANGULAR_DAMPING: f64 = 0.05;
/// Gearbox v1 body damping (1/s): legacy machines and bodies outside any machine.
pub const LEGACY_LINEAR_DAMPING: f64 = 0.1;
pub const LEGACY_ANGULAR_DAMPING: f64 = 0.5;

/// Where a value came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Provenance {
    /// Authored on the stage.
    Authored,
    /// The schema's fallback for an attribute nobody authored.
    SchemaDefault,
    /// Filled by the legacy (v1) decoder from conventions or names.
    LegacyDerived,
    /// The profile's documented default.
    ProfileDefault,
    /// Computed from other values (poses, graph, units).
    Derived,
}

impl Provenance {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Authored => "authored",
            Self::SchemaDefault => "schema_default",
            Self::LegacyDerived => "legacy_derived",
            Self::ProfileDefault => "profile_default",
            Self::Derived => "derived",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Valued<T> {
    pub value: T,
    pub source: Provenance,
}

impl<T> Valued<T> {
    pub fn new(value: T, source: Provenance) -> Self {
        Self { value, source }
    }
    pub fn authored(value: T) -> Self {
        Self::new(value, Provenance::Authored)
    }
    pub fn schema(value: T) -> Self {
        Self::new(value, Provenance::SchemaDefault)
    }
    pub fn derived(value: T) -> Self {
        Self::new(value, Provenance::Derived)
    }
    pub fn profile(value: T) -> Self {
        Self::new(value, Provenance::ProfileDefault)
    }
    pub fn legacy(value: T) -> Self {
        Self::new(value, Provenance::LegacyDerived)
    }
}

/// Which decoder read the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileKind {
    /// Bare UsdPhysics (optionally Isaac metadata): structure only, no
    /// guessed controllers or command interfaces.
    Standard,
    /// Gearbox metadata without `schemaVersion`: legacy v1 decoding.
    GearboxLegacy,
    /// `gearbox:machine:schemaVersion = 2`.
    GearboxV2,
}

impl ProfileKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::GearboxLegacy => "gearbox_v1_legacy",
            Self::GearboxV2 => "gearbox_v2",
        }
    }

    pub fn strict(self) -> bool {
        matches!(self, Self::GearboxV2 | Self::Standard)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RigidTransform {
    pub translation: DVec3,
    pub rotation: DQuat,
}

impl RigidTransform {
    pub const IDENTITY: Self = Self { translation: DVec3::ZERO, rotation: DQuat::IDENTITY };

    pub fn new(translation: DVec3, rotation: DQuat) -> Self {
        Self { translation, rotation }
    }

    pub fn inverse(&self) -> Self {
        let rotation = self.rotation.inverse();
        Self { translation: rotation * -self.translation, rotation }
    }

    pub fn mul(&self, rhs: &Self) -> Self {
        Self {
            translation: self.translation + self.rotation * rhs.translation,
            rotation: (self.rotation * rhs.rotation).normalize(),
        }
    }

    pub fn transform_point(&self, p: DVec3) -> DVec3 {
        self.translation + self.rotation * p
    }
}

impl Default for RigidTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpAxis {
    Y,
    Z,
}

/// Stage unit factors; length factor L (m per unit), mass factor M (kg per unit).
#[derive(Clone, Debug, PartialEq)]
pub struct StageUnits {
    pub meters_per_unit: Valued<f64>,
    pub kilograms_per_unit: Valued<f64>,
    pub up_axis: Valued<UpAxis>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyMode {
    Dynamic,
    Kinematic,
    /// `physics:rigidBodyEnabled = false`: a static body.
    Disabled,
}

impl BodyMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dynamic => "dynamic",
            Self::Kinematic => "kinematic",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MassProperties {
    /// Explicit mass (kg); None when the backend must compute it from colliders.
    pub mass: Option<Valued<f64>>,
    pub density: Option<Valued<f64>>,
    pub center_of_mass: Valued<DVec3>,
    /// Principal moments (kg·m²); None when not authored.
    pub diagonal_inertia: Option<Valued<DVec3>>,
    pub principal_axes: Valued<DQuat>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ColliderBinding {
    pub path: String,
    pub shape: String,
    pub approximation: Option<String>,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BodyDescription {
    pub path: String,
    pub name: String,
    pub mode: BodyMode,
    pub mass: MassProperties,
    /// World pose at load, normalized convention.
    pub initial_pose: RigidTransform,
    /// Uniform scale of the body prim's world transform.
    pub scale: f64,
    pub initial_linear_velocity: Valued<DVec3>,
    pub initial_angular_velocity: Valued<DVec3>,
    /// Velocity damping of the body in 1/s.
    pub linear_damping: Valued<f64>,
    pub angular_damping: Valued<f64>,
    pub colliders: Vec<ColliderBinding>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Endpoint {
    World,
    Body(usize),
    /// An authored target that does not resolve to a prim the physics can use.
    Invalid { target: String, reason: String },
}

impl Endpoint {
    pub fn body(&self) -> Option<usize> {
        match self {
            Self::Body(i) => Some(*i),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EndpointRecord {
    /// The authored relationship target, None when the relationship is empty.
    pub authored: Option<String>,
    pub endpoint: Endpoint,
    /// How the endpoint was reached when the target is not itself a body:
    /// `ancestor:<body path>` or `static:<prim path>`.
    pub via: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JointKind {
    Revolute,
    Prismatic,
    Fixed,
    Spherical,
    Distance,
    D6,
    Unknown(String),
}

impl JointKind {
    pub fn from_type_name(ty: &str) -> Option<Self> {
        Some(match ty {
            "PhysicsRevoluteJoint" => Self::Revolute,
            "PhysicsPrismaticJoint" => Self::Prismatic,
            "PhysicsFixedJoint" => Self::Fixed,
            "PhysicsSphericalJoint" => Self::Spherical,
            "PhysicsDistanceJoint" => Self::Distance,
            "PhysicsJoint" => Self::D6,
            other if other.starts_with("Physics") && other.ends_with("Joint") => Self::Unknown(other.to_string()),
            _ => return None,
        })
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Revolute => "revolute",
            Self::Prismatic => "prismatic",
            Self::Fixed => "fixed",
            Self::Spherical => "spherical",
            Self::Distance => "distance",
            Self::D6 => "d6",
            Self::Unknown(name) => name,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JointClass {
    /// Spanning-tree edge of the articulation.
    Tree,
    /// `excludeFromArticulation` loop closure: physical, never a TF parent.
    Closure,
    /// One endpoint is the world.
    WorldAnchor,
    /// `jointEnabled = false`: described, no physical connectivity.
    Disabled,
    /// An endpoint failed to resolve; never materialized.
    Invalid,
}

impl JointClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tree => "tree",
            Self::Closure => "closure",
            Self::WorldAnchor => "world_anchor",
            Self::Disabled => "disabled",
            Self::Invalid => "invalid",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub fn parse(token: &str) -> Option<Self> {
        Some(match token {
            "X" => Self::X,
            "Y" => Self::Y,
            "Z" => Self::Z,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::X => "X",
            Self::Y => "Y",
            Self::Z => "Z",
        }
    }

    pub fn vector(self) -> DVec3 {
        match self {
            Self::X => DVec3::X,
            Self::Y => DVec3::Y,
            Self::Z => DVec3::Z,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct JointDescription {
    pub path: String,
    pub name: String,
    pub kind: JointKind,
    pub enabled: bool,
    pub collision_enabled: bool,
    pub exclude_from_articulation: bool,
    pub body0: EndpointRecord,
    pub body1: EndpointRecord,
    /// Joint frame in body0's frame (or world), SI.
    pub local0: RigidTransform,
    /// Joint frame in body1's frame (or world), SI.
    pub local1: RigidTransform,
    pub axis: Option<Axis>,
    pub dofs: Vec<usize>,
    pub class: JointClass,
    pub break_force: Option<f64>,
    pub break_torque: Option<f64>,
}

impl JointDescription {
    pub fn bodies(&self) -> (Option<usize>, Option<usize>) {
        (self.body0.endpoint.body(), self.body1.endpoint.body())
    }

    pub fn is_physical(&self) -> bool {
        !matches!(self.class, JointClass::Disabled | JointClass::Invalid)
    }
}

/// The standard UsdPhysics DOF tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DofToken {
    Angular,
    Linear,
    TransX,
    TransY,
    TransZ,
    RotX,
    RotY,
    RotZ,
    Distance,
}

impl DofToken {
    pub fn parse(token: &str) -> Option<Self> {
        Some(match token {
            "angular" => Self::Angular,
            "linear" => Self::Linear,
            "transX" => Self::TransX,
            "transY" => Self::TransY,
            "transZ" => Self::TransZ,
            "rotX" => Self::RotX,
            "rotY" => Self::RotY,
            "rotZ" => Self::RotZ,
            "distance" => Self::Distance,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Angular => "angular",
            Self::Linear => "linear",
            Self::TransX => "transX",
            Self::TransY => "transY",
            Self::TransZ => "transZ",
            Self::RotX => "rotX",
            Self::RotY => "rotY",
            Self::RotZ => "rotZ",
            Self::Distance => "distance",
        }
    }

    pub fn rotational(self) -> bool {
        matches!(self, Self::Angular | Self::RotX | Self::RotY | Self::RotZ)
    }

    pub const D6: [DofToken; 6] = [Self::TransX, Self::TransY, Self::TransZ, Self::RotX, Self::RotY, Self::RotZ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Interface {
    Position,
    Velocity,
    Effort,
}

impl Interface {
    pub fn parse(token: &str) -> Option<Self> {
        Some(match token {
            "position" => Self::Position,
            "velocity" => Self::Velocity,
            "effort" => Self::Effort,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Position => "position",
            Self::Velocity => "velocity",
            Self::Effort => "effort",
        }
    }

    pub const ALL: [Interface; 3] = [Self::Position, Self::Velocity, Self::Effort];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefaultMode {
    Disabled,
    Position,
    Velocity,
    Effort,
}

impl DefaultMode {
    pub fn parse(token: &str) -> Option<Self> {
        Some(match token {
            "disabled" => Self::Disabled,
            "position" => Self::Position,
            "velocity" => Self::Velocity,
            "effort" => Self::Effort,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Position => "position",
            Self::Velocity => "velocity",
            Self::Effort => "effort",
        }
    }
}

/// A joint limit in SI; None on a side means unbounded there.
#[derive(Clone, Debug, PartialEq)]
pub struct Limit {
    pub lower: Option<f64>,
    pub upper: Option<f64>,
    /// D6 axis whose authored low exceeds its high.
    pub locked: bool,
    pub source: Provenance,
}

impl Limit {
    pub fn unbounded(source: Provenance) -> Self {
        Self { lower: None, upper: None, locked: false, source }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriveType {
    Force,
    Acceleration,
}

impl DriveType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Force => "force",
            Self::Acceleration => "acceleration",
        }
    }
}

/// An authored `PhysicsDriveAPI:<dof>` in SI: stiffness per rad or m,
/// damping per rad/s or m/s; acceleration drives act per unit of joint inertia.
#[derive(Clone, Debug, PartialEq)]
pub struct DriveDescription {
    pub drive_type: Valued<DriveType>,
    pub stiffness: Valued<f64>,
    pub damping: Valued<f64>,
    /// None: unbounded.
    pub max_force: Valued<Option<f64>>,
    pub target_position: Valued<f64>,
    pub target_velocity: Valued<f64>,
}

impl DriveDescription {
    pub fn kind(&self) -> PassiveKind {
        match (self.stiffness.value > 0.0, self.damping.value > 0.0) {
            (true, _) => PassiveKind::Spring,
            (false, true) => PassiveKind::Damper,
            (false, false) => PassiveKind::ZeroGain,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassiveKind {
    NoDrive,
    ZeroGain,
    Spring,
    Damper,
}

impl PassiveKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoDrive => "no_drive",
            Self::ZeroGain => "zero_gain",
            Self::Spring => "spring",
            Self::Damper => "damper",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DofDescription {
    pub joint: usize,
    pub token: DofToken,
    /// Public name: the joint name, with `:<token>` for multi-DOF joints.
    pub name: String,
    pub limit: Limit,
    pub drive: Option<DriveDescription>,
    pub passive: PassiveKind,
    /// Command interfaces an external owner may use; empty means none.
    pub interfaces: Valued<Vec<Interface>>,
    pub default_mode: Valued<DefaultMode>,
    pub initial_position: Valued<f64>,
    pub initial_velocity: Valued<f64>,
    /// Isaac `isaac:actuator` flag for this DOF, when authored.
    pub isaac_actuator: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameRole {
    Body,
    Base,
    Control,
    Site,
    Sensor,
    Coupling,
    Tool,
    Link,
}

impl FrameRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::Base => "base",
            Self::Control => "control",
            Self::Site => "site",
            Self::Sensor => "sensor",
            Self::Coupling => "coupling",
            Self::Tool => "tool",
            Self::Link => "link",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameOwner {
    Body(usize),
    Frame(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FrameDescription {
    pub path: String,
    pub name: String,
    pub owner: FrameOwner,
    /// Pose in the owner's frame, SI.
    pub transform: RigidTransform,
    pub role: FrameRole,
    pub publish: bool,
}

/// Bodies joined by non-excluded joints; one reduced-coordinate articulation.
#[derive(Clone, Debug, PartialEq)]
pub struct ArticulationGroup {
    pub markers: Vec<String>,
    pub bodies: Vec<usize>,
    pub joints: Vec<usize>,
    pub fixed_base: bool,
}

/// One edge of the reporting tree: `child` hangs from `parent` either through
/// a moving joint or statically.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeEdge {
    pub parent: usize,
    pub child: usize,
    pub joint: Option<usize>,
    /// The tree runs from body1 to body0 of the joint; coordinates keep the
    /// authored order.
    pub reversed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DofRef {
    pub joint: usize,
    pub dof: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ControllerBinding {
    pub instance: String,
    pub controller_type: String,
    pub enabled: bool,
    pub update_rate_hz: Valued<f64>,
    pub authority: String,
    /// Role → exact DOFs.
    pub roles: BTreeMap<String, Vec<DofRef>>,
    pub parameters: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapabilityStatus {
    SupportedExact,
    SupportedApproximate,
    Unsupported,
}

impl CapabilityStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SupportedExact => "supported_exact",
            Self::SupportedApproximate => "supported_approximate",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CapabilityResult {
    pub feature: String,
    pub subject: Option<String>,
    pub status: CapabilityStatus,
    pub source_property: Option<String>,
    pub reason: String,
    pub evidence: Vec<&'static str>,
}

/// The frame body-local commands are given in: +X forward, +Y left, +Z up.
#[derive(Clone, Debug, PartialEq)]
pub struct ControlFrame {
    pub frame: usize,
    pub source: Provenance,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MachineDescription {
    pub load_instance: String,
    pub root: String,
    pub machine_id: String,
    pub generation: u64,
    pub profile: ProfileKind,
    pub schema_version: Option<i64>,
    pub kind: Option<String>,
    pub isaac: bool,
    pub units: StageUnits,
    pub control_body: Option<usize>,
    pub base_frame: Option<usize>,
    pub control_frame: Option<ControlFrame>,
    pub bodies: Vec<BodyDescription>,
    pub joints: Vec<JointDescription>,
    pub dofs: Vec<DofDescription>,
    pub frames: Vec<FrameDescription>,
    pub tree: Vec<TreeEdge>,
    pub articulations: Vec<ArticulationGroup>,
    pub controllers: Vec<ControllerBinding>,
    /// Default command owner of each owned DOF, validated at activation.
    pub owners: BTreeMap<usize, DofOwner>,
    /// Isaac `robotLinks`/`robotJoints` reporting order, when authored.
    pub reporting: Option<ReportingOrder>,
    pub capabilities: Vec<CapabilityResult>,
    pub diagnostics: Vec<Diagnostic>,
    pub command_validity_ms: u64,
}

/// Declared reporting order; bodies and joints it leaves out stay in the model.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ReportingOrder {
    pub bodies: Vec<usize>,
    pub joints: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OwnerKind {
    Controller,
    Service,
    Device,
}

impl OwnerKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Controller => "controller",
            Self::Service => "service",
            Self::Device => "device",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DofOwner {
    pub owner: String,
    pub kind: OwnerKind,
    /// Device realizing the owner's commands on this DOF, when one sits there.
    pub device: Option<String>,
}

impl MachineDescription {
    pub fn accepted(&self) -> bool {
        !self.diagnostics.iter().any(|d| d.severity == Severity::Error)
    }

    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().filter(|d| d.severity == Severity::Error)
    }

    pub fn body_index(&self, path: &str) -> Option<usize> {
        self.bodies.iter().position(|b| b.path == path)
    }

    pub fn joint_index(&self, path: &str) -> Option<usize> {
        self.joints.iter().position(|j| j.path == path)
    }

    pub fn frame_index(&self, path: &str) -> Option<usize> {
        self.frames.iter().position(|f| f.path == path)
    }

    /// The DOF of `joint` named by `token`; a scalar joint's only DOF when None.
    pub fn dof(&self, joint: &str, token: Option<DofToken>) -> Option<DofRef> {
        let j = self.joint_index(joint)?;
        let dofs = &self.joints[j].dofs;
        let dof = match token {
            Some(token) => dofs.iter().copied().find(|d| self.dofs[*d].token == token)?,
            None if dofs.len() == 1 => dofs[0],
            None => return None,
        };
        Some(DofRef { joint: j, dof })
    }

    pub fn dof_by_name(&self, name: &str) -> Option<DofRef> {
        let dof = self.dofs.iter().position(|d| d.name == name)?;
        Some(DofRef { joint: self.dofs[dof].joint, dof })
    }

    /// Which body a frame finally rides on.
    pub fn frame_body(&self, frame: usize) -> Option<usize> {
        let mut current = frame;
        for _ in 0..self.frames.len() + 1 {
            match self.frames.get(current)?.owner {
                FrameOwner::Body(b) => return Some(b),
                FrameOwner::Frame(f) => current = f,
            }
        }
        None
    }

    /// A frame's pose in its riding body's frame.
    pub fn frame_in_body(&self, frame: usize) -> Option<(usize, RigidTransform)> {
        let mut pose = RigidTransform::IDENTITY;
        let mut current = frame;
        for _ in 0..self.frames.len() + 1 {
            let f = self.frames.get(current)?;
            pose = f.transform.mul(&pose);
            match f.owner {
                FrameOwner::Body(b) => return Some((b, pose)),
                FrameOwner::Frame(parent) => current = parent,
            }
        }
        None
    }
}
