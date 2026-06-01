//! Gearbox binding surface.

pub mod ffi;

#[cfg(feature = "python")]
pub mod python;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
