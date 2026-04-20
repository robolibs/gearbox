//! Vehicle data model.
//!
//! A [`VehicleSpec`] is a pure data description (chassis + wheels). A
//! [`VehicleState`] is what [`crate::Sim`] keeps internally once a spec has
//! been realized as a live rapier rigid body + vehicle controller.

pub mod builder;
pub mod chassis;
pub mod wheel;

use rapier3d::control::DynamicRayCastVehicleController;
use rapier3d::prelude::RigidBodyHandle;

use crate::control::ControlInput;

pub use builder::VehicleBuilder;
pub use chassis::ChassisSpec;
pub use wheel::WheelSpec;

/// Opaque identifier handed back by [`crate::Sim::spawn_vehicle`].
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct VehicleId(pub u32);

/// A declarative vehicle: chassis box + list of raycast wheels.
#[derive(Debug, Clone)]
pub struct VehicleSpec {
    pub name: String,
    pub chassis: ChassisSpec,
    pub wheels: Vec<WheelSpec>,
}

/// Live physics state for a spawned vehicle. Owned by [`crate::Sim`].
pub struct VehicleState {
    pub spec: VehicleSpec,
    pub body: RigidBodyHandle,
    pub controller: DynamicRayCastVehicleController,
    pub control: ControlInput,
}
