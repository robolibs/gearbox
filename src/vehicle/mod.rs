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
pub use part::{PartKind, PartShape, PartSpec};
pub use wheel::WheelSpec;

/// Opaque identifier handed back by [`crate::Sim::spawn_vehicle`].
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct VehicleId(pub u32);

/// How the vehicle is driven.
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
    /// Arcade drone — no wheels. Forces and torques are applied
    /// directly to the chassis rigid body:
    ///   - `throttle` → force along the drone's local forward axis,
    ///   - `steer`    → force along the drone's local right axis,
    ///   - `lift`     → vertical force (in addition to a constant
    ///                  `mass × g` hover force that cancels gravity),
    ///   - `yaw`      → torque around world +Y.
    /// Stabilising damping keeps the drone hover-steady.
    Drone,
    /// 4-wheel independent steering (4WIS / omni). Every wheel's
    /// steering angle is computed per-tick so the combined wheel
    /// velocities match:
    ///   - `throttle` → forward/back body velocity,
    ///   - `steer`    → lateral (strafe) body velocity,
    ///   - `yaw`      → rotational body rate around +Y.
    /// Used for gantry-style field robots like AGROINTELLI Robotti.
    Omni,
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
