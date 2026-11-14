//! Identities, setpoints and the effective actuator state of the joint
//! command contract. Values are SI: rad or m, rad/s or m/s, N·m or N.

use std::fmt;

use crate::machine_model::{DofToken, DriveType, Interface};

/// One DOF of one generation of one machine: the only key commands bind to.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DofKey {
    pub machine: String,
    pub generation: u64,
    pub joint: String,
    pub dof: DofToken,
}

impl fmt::Display for DofKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}:{}:{}", self.machine, self.generation, self.joint, self.dof.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OwnerClass {
    /// A drive controller of the machine.
    Controller,
    /// A service controller (hitch, PTO, valve, joint position/velocity, brake, trailer steer).
    Service,
    /// An authored motor device with no other owner.
    Device,
    /// An external client holding a server-issued lease.
    Direct,
    /// The trusted deterministic replay driver.
    TestDriver,
}

impl OwnerClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Controller => "controller",
            Self::Service => "service",
            Self::Device => "device",
            Self::Direct => "direct",
            Self::TestDriver => "test_driver",
        }
    }
}

/// Who writes a DOF. Internal owners are named by their machine and instance;
/// a direct owner's name is issued by the server with its epoch.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OwnerId {
    pub class: OwnerClass,
    pub machine: String,
    pub name: String,
}

impl OwnerId {
    pub fn new(class: OwnerClass, machine: &str, name: &str) -> Self {
        Self { class, machine: machine.to_string(), name: name.to_string() }
    }
}

impl fmt::Display for OwnerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}/{}", self.class.as_str(), self.machine, self.name)
    }
}

/// What an owner asks of one DOF.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Setpoint {
    /// Joint position, with an optional feed-forward velocity.
    Position { q: f64, qdot: Option<f64> },
    /// Joint velocity; not a wheel's ground speed.
    Velocity { qdot: f64 },
    /// Generalized force on the joint coordinate.
    Effort { force: f64 },
    /// Hold the coordinate measured once when the hold begins.
    Hold,
    /// No active actuator effort; passive springs and limits stay.
    Free,
    /// An internal controller's own servo: explicit gains and model.
    /// Never accepted from an external owner.
    Servo { q: Option<f64>, qdot: f64, stiffness: f64, damping: f64, drive: DriveType },
}

impl Setpoint {
    pub fn interface(&self) -> Option<Interface> {
        match self {
            Self::Position { .. } => Some(Interface::Position),
            Self::Velocity { .. } => Some(Interface::Velocity),
            Self::Effort { .. } => Some(Interface::Effort),
            Self::Hold => Some(Interface::Position),
            Self::Free | Self::Servo { .. } => None,
        }
    }

    pub fn is_finite(&self) -> bool {
        match *self {
            Self::Position { q, qdot } => q.is_finite() && qdot.is_none_or(f64::is_finite),
            Self::Velocity { qdot } => qdot.is_finite(),
            Self::Effort { force } => force.is_finite(),
            Self::Hold | Self::Free => true,
            Self::Servo { q, qdot, stiffness, damping, .. } => {
                q.is_none_or(f64::is_finite) && qdot.is_finite() && stiffness.is_finite() && damping.is_finite() && stiffness >= 0.0 && damping >= 0.0
            }
        }
    }
}

/// How long an accepted command stays in force.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Validity {
    /// Until replaced: internal owners re-evaluated every due tick, and
    /// position latching under a declared policy.
    Latched,
    /// Simulation ticks from acceptance: deterministic replay.
    Ticks(u64),
    /// Server monotonic wall time from acceptance: live external commands.
    WallMs(u64),
}

/// The resolved write of one DOF for one tick, in SI with the drive's gains.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effective {
    /// Motor off.
    Free,
    /// Position and velocity targets through spring and damper gains.
    Servo { q: Option<f64>, qdot: f64, stiffness: f64, damping: f64, drive: DriveType },
    /// Generalized force.
    Effort { force: f64 },
}

/// One DOF's resolved actuator state: the mode and the cap it is held to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActuatorState {
    pub effective: Effective,
    /// Most force or torque the actuator applies; None is no bound.
    pub cap: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rejection {
    UnknownDof(String),
    StaleGeneration { dof: String, have: u64, given: u64 },
    NotOwner { dof: String, owner: String, holder: String },
    StaleEpoch { given: u64, current: u64 },
    OutOfOrder { given: u64, last: u64 },
    InterfaceNotAllowed { dof: String, interface: String },
    ServoFromExternal { dof: String },
    NonFinite { dof: String },
    UnboundedLiveCommand { dof: String },
    DuplicateDof { dof: String },
    Revoked,
    Conflict { dof: String, owners: Vec<String> },
}

impl fmt::Display for Rejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownDof(d) => write!(f, "unknown DOF {d}"),
            Self::StaleGeneration { dof, have, given } => write!(f, "{dof}: generation {given} is stale, the machine is at {have}"),
            Self::NotOwner { dof, owner, holder } => write!(f, "{dof}: {owner} does not own it, {holder} does"),
            Self::StaleEpoch { given, current } => write!(f, "epoch {given} is stale, the lease is at {current}"),
            Self::OutOfOrder { given, last } => write!(f, "sequence {given} is not after {last}"),
            Self::InterfaceNotAllowed { dof, interface } => write!(f, "{dof}: interface {interface} is not allowed"),
            Self::ServoFromExternal { dof } => write!(f, "{dof}: servo setpoints are internal"),
            Self::NonFinite { dof } => write!(f, "{dof}: non-finite target"),
            Self::UnboundedLiveCommand { dof } => write!(f, "{dof}: live velocity and effort commands need a finite validity"),
            Self::DuplicateDof { dof } => write!(f, "{dof}: named twice in one batch"),
            Self::Revoked => write!(f, "lease revoked"),
            Self::Conflict { dof, owners } => write!(f, "{dof}: owned by {}", owners.join(" and ")),
        }
    }
}
