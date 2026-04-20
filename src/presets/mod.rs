//! Hand-rolled vehicle presets.
//!
//! These are small `fn () -> VehicleSpec` constructors — no inheritance,
//! just data. A later phase can add YAML/URDF loaders that produce the
//! same [`VehicleSpec`].

mod car;
mod tractor;

pub use car::car;
pub use tractor::tractor;
