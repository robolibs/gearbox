//! gearbox — modular Rust vehicle / robot simulator.
//!
//! The library is fully renderer-agnostic: it depends on `rapier3d` for
//! physics and `datapod` for spatial types, nothing else. The interactive
//! editor lives in the `gearbox` binary (see `src/bin/gearbox/`) and pulls
//! Bevy + bevy_egui behind the `editor` cargo feature.

pub mod control;
pub mod convert;
pub mod planet;
pub mod presets;
pub mod sensor;
pub mod sim;
pub mod vehicle;
pub mod world;

pub use control::ControlInput;
pub use planet::{Planet, EARTH_RADIUS_M};
pub use sim::Sim;
pub use vehicle::{
    ChassisSpec, DriveMode, PartKind, PartShape, PartSpec, VehicleId, VehicleSpec, VehicleState,
    WheelSpec,
};

/// Re-export rapier so downstream consumers don't need to pin the version.
pub use rapier3d;
/// Re-export datapod so downstream consumers can use the same spatial types
/// gearbox's public API speaks.
pub use datapod;
