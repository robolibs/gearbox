//! Vehicle data model.
//!
//! A [`VehicleSpec`] is a pure data description (chassis + wheels). A
//! [`VehicleState`] is what [`crate::Sim`] keeps internally once a spec has
//! been realized as a live rapier rigid body + vehicle controller.

pub mod builder;
pub mod chassis;
pub mod part;
pub mod wheel;

use rapier3d::control::DynamicRayCastVehicleController;
use rapier3d::prelude::RigidBodyHandle;

use crate::control::ControlInput;

pub use builder::VehicleBuilder;
pub use chassis::ChassisSpec;
pub use part::{PartKind, PartSpec};
pub use wheel::WheelSpec;

/// Opaque identifier handed back by [`crate::Sim::spawn_vehicle`].
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct VehicleId(pub u32);

/// How the vehicle steers under a `ControlInput::steer` command.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DriveMode {
    /// Ackermann steering — `steer` rotates the wheels flagged
    /// `steered: true` to a common turn-centre angle. Standard car /
    /// tractor behaviour.
    Ackermann,
    /// Differential drive — `steer` biases left-side vs right-side
    /// wheel *throttle* instead of a steering angle. Used for
    /// skid-steer / tracked robots (e.g. Husky). Wheels' `steered`
    /// flag is ignored; `driven` still applies to decide which
    /// wheels get engine force.
    Differential,
}

impl Default for DriveMode {
    fn default() -> Self { DriveMode::Ackermann }
}

/// A declarative vehicle: chassis box + list of raycast wheels + any
/// attached body parts (hitches, karosseries, tanks).
#[derive(Debug, Clone)]
pub struct VehicleSpec {
    pub name: String,
    pub chassis: ChassisSpec,
    pub wheels: Vec<WheelSpec>,
    /// Extra visual body parts. Parented to the chassis entity by the
    /// viz layer, so they ride the chassis pose automatically.
    pub parts: Vec<PartSpec>,
    /// How `ControlInput::steer` is interpreted.
    pub drive_mode: DriveMode,
}

/// Live physics state for a spawned vehicle. Owned by [`crate::Sim`].
pub struct VehicleState {
    pub spec: VehicleSpec,
    pub body: RigidBodyHandle,
    pub controller: DynamicRayCastVehicleController,
    pub control: ControlInput,
}
