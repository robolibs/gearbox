//! Vehicle body parts — karosseries (body panels), hitches, tanks.
//!
//! Currently visual-only. They're rendered as coloured boxes and
//! parented to the chassis, so they follow the chassis pose without
//! any extra sync work. A future physics layer can add per-part
//! collision to the Karosserie variant.

use datapod::{Point, Size};

use super::mesh::MeshSource;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartKind {
    /// Body panel / bumper / cabin.
    Karosserie,
    /// Hitch attachment point (visual marker).
    Hitch,
    /// Storage tank / harvest bin — typically sits on top of the chassis.
    Tank,
}

#[derive(Debug, Clone)]
pub struct PartSpec {
    pub name: String,
    /// Position **relative to the chassis centre** in chassis-local space
    /// (same convention as [`crate::WheelSpec::chassis_connection`]).
    pub position: Point,
    /// Full-extent box dimensions `(x, y, z)`. For cylinder parts, see
    /// [`MeshSource::Cylinder`] for the size-field interpretation.
    pub size: Size,
    /// Visual sRGB colour.
    pub color: [f32; 3],
    pub kind: PartKind,
    /// How the part should be rendered. Default is a cuboid — switch
    /// to `MeshSource::Cylinder` for visual kingpins, antennas, etc.
    pub mesh: MeshSource,
}
