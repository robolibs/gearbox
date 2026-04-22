//! Live, physics-backed vehicle state.
//!
//! Wraps a declarative [`gearbox_core::VehicleSpec`] with the engine-
//! owned handles ([`crate::vehicle_physics::PhysicsHandles`]) and the
//! vehicle's current [`gearbox_core::ControlInput`]. Owned by
//! [`crate::Sim`].
//!
//! The handles are `pub(crate)` — external consumers see only `spec`
//! and `control`. Drive controllers reach into the physics through the
//! proxies in [`crate::vehicle_physics`], constructed per-tick inside
//! [`crate::Sim::step`].

use gearbox_core::{ControlInput, VehicleSpec};

use crate::vehicle_physics::PhysicsHandles;

pub struct VehicleState {
    pub spec: VehicleSpec,
    pub control: ControlInput,
    pub(crate) handles: PhysicsHandles,
}
