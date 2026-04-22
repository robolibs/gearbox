//! Vehicle catalogue.
//!
//! Each file in this folder builds one [`crate::VehicleSpec`]. The folder
//! is intentionally a stable boundary — a future OpenUSD-backed loader
//! will be dropped in here as a sibling module (or an outright
//! replacement), keeping the rest of the codebase untouched.

mod drone;
mod husky;
mod oxbo_harvester;
mod robotti;
mod tractor;

pub use drone::drone;
pub use husky::husky;
pub use oxbo_harvester::oxbo_harvester;
pub use robotti::robotti;
pub use tractor::tractor;
