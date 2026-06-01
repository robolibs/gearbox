//! gearbox-physics — rapier3d-backed simulation layer.
//!
//! Owns the rapier world ([`Sim`]), the static scene helpers
//! ([`world`]), and the pluggable per-vehicle [`drive`] controllers.
//! Builds `VehicleState` (live, rapier-bound) on top of
//! [`gearbox_core::VehicleSpec`] (declarative, engine-agnostic).
//!
//! Downstream consumers who want the *pure* data (spec, presets,
//! control input) should depend on `gearbox-core` directly; this
//! crate re-exports those types so a single `gearbox-physics`
//! dependency is enough for simulation-level work.

pub mod convert;
pub mod drive;
pub mod sensor;
pub mod sim;
pub mod vehicle;
pub mod vehicle_physics;
pub mod world;

pub use sim::Sim;
pub use vehicle::VehicleState;

// Re-export every type the library/binary previously pulled from
// `gearbox::*` — downstream code can continue to speak one name for
// "the simulator" without juggling both crates.
pub use gearbox_core::{
    ChassisSpec, Container, ControlInput, DriveMode, MeshSource, PartKind, PartSpec, PowerKind,
    PowerSource, PowerSystem, VehicleBuilder, VehicleId, VehicleSpec, WheelSpec, control,
    parts_lib, planet, presets, vehicle as vehicle_core,
};

pub use datapod;
pub use rapier3d;
