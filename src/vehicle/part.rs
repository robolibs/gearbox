//! Vehicle body parts — karosseries (body panels), hitches, tanks.
//!
//! Currently visual-only. They're rendered as coloured boxes and
//! parented to the chassis, so they follow the chassis pose without
//! any extra sync work. A future physics layer can add per-part
//! collision to the Karosserie variant.

use datapod::{Point, Size};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartKind {
    /// Body panel / bumper / cabin.
    Karosserie,
    /// Hitch attachment point (visual marker).
    Hitch,
    /// Storage tank / harvest bin — typically sits on top of the chassis.
    Tank,
}

/// Visual mesh shape for a part.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PartShape {
    /// Axis-aligned cuboid sized by `size.x × size.y × size.z`.
    #[default]
    Box,
    /// Cylinder whose axis runs along chassis-local +Y.
    /// `size.x` = diameter (2 × radius), `size.y` = height, `size.z` ignored.
    /// Only supported for visual-only ([`PartKind::Hitch`]) parts today;
    /// non-Hitch cylinder parts will be drawn as cylinders but given a
    /// cuboid physics collider (a future refactor can add cylinder colliders).
    Cylinder,
}

#[derive(Debug, Clone)]
pub struct PartSpec {
    pub name: String,
    /// Position **relative to the chassis centre** in chassis-local space
    /// (same convention as [`crate::WheelSpec::chassis_connection`]).
    pub position: Point,
    /// Full-extent box dimensions `(x, y, z)`. For cylinder parts, see
    /// [`PartShape::Cylinder`] for the size-field interpretation.
    pub size: Size,
    /// Visual sRGB colour.
    pub color: [f32; 3],
    pub kind: PartKind,
    pub shape: PartShape,
}
