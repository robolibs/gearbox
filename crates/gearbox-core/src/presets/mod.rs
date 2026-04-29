//! Vehicle catalogue.
//!
//! Each file in this folder builds one [`crate::VehicleSpec`]. The folder
//! is intentionally a stable boundary — a future OpenUSD-backed loader
//! will be dropped in here as a sibling module (or an outright
//! replacement), keeping the rest of the codebase untouched.

mod drone;
mod husky;
mod oxbo_harvester;
pub mod registry;
mod robotti;
mod tractor_articulated;

pub use drone::drone;
pub use husky::husky;
pub use oxbo_harvester::oxbo_harvester;
pub use registry::{all_presets, PresetEntry};
pub use robotti::robotti;
pub use tractor_articulated::tractor_articulated;
