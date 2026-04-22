//! gearbox-core — pure data for the gearbox simulator.
//!
//! Vehicle specs, presets, control input, power / container models,
//! and drive-mode enums. Compiles against `datapod` only — **no
//! rapier, no bevy** — so any tool that just needs to *describe* a
//! vehicle (URDF exporter, YAML loader, web viewer) can depend on
//! this crate alone.
//!
//! The live simulation (rigid bodies, wheel raycasts, drive
//! controllers) is in the sibling `gearbox-physics` crate.

pub mod control;
pub mod planet;
pub mod presets;
pub mod vehicle;

pub use control::ControlInput;
pub use planet::{Planet, EARTH_RADIUS_M};
pub use vehicle::{
    parts_lib, ChassisSpec, Container, DriveMode, MeshSource, PartKind, PartSpec, PowerKind,
    PowerSource, PowerSystem, VehicleBuilder, VehicleId, VehicleSpec, WheelSpec,
};

/// Re-export `datapod` so downstream consumers share the same spatial
/// types gearbox-core speaks.
pub use datapod;
