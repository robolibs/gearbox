//! The resolved machine model: one compiler from a composed stage to the
//! bodies, joints, DOFs, frames, controller bindings and owners every
//! runtime consumer reads, with diagnostics instead of panics on asset input.
//!
//! ```text
//! stage + selection -> scopes -> resolve -> normalize -> validate -> manifest
//! ```

pub mod audit;
pub mod bindings;
pub mod capabilities;
pub mod compile;
pub mod diagnostics;
pub mod frames;
pub mod graph;
pub mod isaac;
pub mod manifest;
pub mod model;
pub mod read;
pub mod scene;
pub mod units;
pub mod xform;

#[cfg(test)]
pub(crate) mod tests;

pub use compile::{CompileOptions, CompileResult, Selection, compile};

pub use model::*;

/// Open a USD file and compile it.
pub fn compile_file(path: &std::path::Path, options: &CompileOptions) -> Result<CompileResult, String> {
    let path = path.to_str().ok_or_else(|| "non-UTF-8 USD path".to_string())?;
    let stage = openusd::usd::Stage::open(path).map_err(|e| e.to_string())?;
    Ok(compile(&stage, options))
}
