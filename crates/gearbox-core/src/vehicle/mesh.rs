//! Mesh source — the abstraction the viz layer uses to pick how a
//! chassis or part should be rendered.
//!
//! Keeping this behind an enum (rather than hard-coding
//! `Cuboid::new(...)` in `viz/spawn.rs`) is what makes it a one-line
//! change to add external asset formats later. When USD support lands:
//!
//! ```ignore
//! pub enum MeshSource {
//!     Box,
//!     Cylinder,
//!     Asset(std::path::PathBuf),   // glTF / OBJ
//!     Usd(UsdReference),           // OpenUSD prim
//! }
//! ```
//!
//! Presets just replace `mesh: MeshSource::Box` with
//! `mesh: MeshSource::Asset("robotti.gltf")` and nothing else needs to
//! change. The viz layer is the only place that dispatches on the
//! variant.

/// How a given sized volume should be rendered.
///
/// The numeric extents come from the owning part / chassis (`Size`);
/// this enum just says how to turn those extents into geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MeshSource {
    /// Axis-aligned cuboid, `size.x × size.y × size.z`.
    #[default]
    Box,
    /// Cylinder with its axis along chassis-local +Y.
    /// `size.x` = diameter (2 × radius), `size.y` = height.
    /// `size.z` is ignored.
    ///
    /// Only supported for visual-only parts today; non-Hitch cylinder
    /// parts currently fall back to a cuboid collider (a future
    /// refactor can add cylinder colliders too).
    Cylinder,
}
