//! Reusable [`PartSpec`] constructors for common vehicle features.
//!
//! Presets can compose these helpers instead of hand-rolling every
//! struct literal. Keep additions here purely declarative — just
//! shapes + sensible defaults; any preset-specific tuning still lives
//! in the preset file.
//!
//! Coordinate convention matches [`PartSpec::position`]: chassis-local
//! `(x, y, z)` with `+Z` forward, `+Y` up, `+X` right.

use datapod::{Point, Size};

use super::{MeshSource, PartKind, PartSpec};

/// A plain coloured cuboid — the workhorse for cabs, beams, panels.
///
/// `kind` chooses physics collider behaviour:
///   - [`PartKind::Karosserie`] / [`PartKind::Tank`] → solid bodywork
///     collider, visible.
///   - [`PartKind::Hitch`] → visual only (no collider).
pub fn cuboid(
    name: impl Into<String>,
    position: Point,
    size: Size,
    color: [f32; 3],
    kind: PartKind,
) -> PartSpec {
    PartSpec {
        name: name.into(),
        position,
        size,
        color,
        kind,
        mesh: MeshSource::Box,
    }
}

/// A vertical cylinder with its axis along chassis-local +Y — the
/// archetypal king-pin strut / antenna mast / sensor pole.
///
/// `diameter` is `size.x`; `height` is `size.y`. `size.z` is ignored
/// by the renderer, but we set it to the diameter too so cylinder
/// pickers and colliders end up with a sensible bounding box.
pub fn vertical_cylinder(
    name: impl Into<String>,
    position: Point,
    diameter: f64,
    height: f64,
    color: [f32; 3],
    kind: PartKind,
) -> PartSpec {
    PartSpec {
        name: name.into(),
        position,
        size: Size::new(diameter, height, diameter),
        color,
        kind,
        mesh: MeshSource::Cylinder,
    }
}

/// Cabin / bodywork box sitting on top of the chassis. `centre_z` is
/// chassis-local Z of the cab's centre; height spans upward from the
/// chassis top.
///
/// Returns a single Karosserie part named `"cab"`.
pub fn cab(
    centre_z: f64,
    width: f64,
    height: f64,
    depth: f64,
    chassis_top: f64,
    color: [f32; 3],
) -> PartSpec {
    cuboid(
        "cab",
        Point::new(0.0, chassis_top + height * 0.5, centre_z),
        Size::new(width, height, depth),
        color,
        PartKind::Karosserie,
    )
}

/// Thin dark roof cap above a cab. Placed just above `cab_top_y` with
/// a slight lateral / longitudinal overhang. Typical `thickness` 0.08
/// m, `overhang` 0.08 m.
pub fn cab_roof(
    centre_z: f64,
    cab_width: f64,
    cab_depth: f64,
    cab_top_y: f64,
    thickness: f64,
    overhang: f64,
    color: [f32; 3],
) -> PartSpec {
    cuboid(
        "cab_roof",
        Point::new(0.0, cab_top_y + thickness * 0.5, centre_z),
        Size::new(cab_width + overhang, thickness, cab_depth + overhang),
        color,
        PartKind::Karosserie,
    )
}

/// Small visual-only hitch marker cube — tow point, drawbar nub,
/// indicator lamp.
pub fn hitch_marker(
    name: impl Into<String>,
    position: Point,
    edge: f64,
    color: [f32; 3],
) -> PartSpec {
    cuboid(
        name,
        position,
        Size::new(edge, edge, edge),
        color,
        PartKind::Hitch,
    )
}

/// Thin top plate that rides on the chassis roof — the "sensor mount"
/// slab used by Husky and similar. Spans most of the chassis top.
pub fn top_plate(
    chassis_width: f64,
    chassis_depth: f64,
    chassis_top: f64,
    width_frac: f64,
    depth_frac: f64,
    thickness: f64,
    color: [f32; 3],
) -> PartSpec {
    cuboid(
        "top_plate",
        Point::new(0.0, chassis_top + thickness * 0.5, 0.0),
        Size::new(
            chassis_width * width_frac,
            thickness,
            chassis_depth * depth_frac,
        ),
        color,
        PartKind::Karosserie,
    )
}
