use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::frames::bounded_text;
use super::{AgeClock, AttributeSemantic, CaptureStamp, CloudSchema, OctreeConfig};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorSpace {
    Srgb,
    Linear,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaletteEntry {
    pub value: u32,
    pub rgba: [u8; 4],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ColorMode {
    Constant {
        rgba: [u8; 4],
    },
    Attribute {
        name: String,
        space: ColorSpace,
    },
    Scalar {
        name: String,
        colormap: u32,
        range: [f64; 2],
        missing: [u8; 4],
    },
    Category {
        name: String,
        entries: Vec<PaletteEntry>,
        missing: [u8; 4],
    },
    Axis {
        axis: u8,
        colormap: u32,
        range: [f64; 2],
    },
    Range {
        origin_m: [f64; 3],
        colormap: u32,
        range: [f64; 2],
    },
    Age {
        clock: AgeClock,
        colormap: u32,
        range: [f64; 2],
    },
    Occupancy {
        occupied: [u8; 4],
        free: [u8; 4],
        uncertain: [u8; 4],
        unknown: [u8; 4],
        mixed: [u8; 4],
    },
    OccupancyProbability {
        colormap: u32,
        unknown: [u8; 4],
    },
    OctreeDepth {
        colormap: u32,
    },
}

fn scalar_range(colormap: u32, range: [f64; 2]) -> Result<(), String> {
    if colormap == crate::wire::colormap::NONE
        || crate::wire::colormap::parse(crate::wire::colormap::name(colormap)) != Some(colormap)
        || !range[0].is_finite()
        || !range[1].is_finite()
        || range[0] > range[1]
    {
        return Err("scalar colors require a known colormap and finite ordered range".into());
    }
    Ok(())
}

impl ColorMode {
    pub fn validate_intrinsic(&self) -> Result<(), String> {
        match self {
            Self::Attribute { name, .. } => bounded_text(name, 63, false)?,
            Self::Scalar {
                name,
                colormap,
                range,
                ..
            } => {
                bounded_text(name, 63, false)?;
                scalar_range(*colormap, *range)?;
            }
            Self::Category { name, entries, .. } => {
                bounded_text(name, 63, false)?;
                if entries.len() > 64
                    || entries
                        .iter()
                        .map(|e| e.value)
                        .collect::<HashSet<_>>()
                        .len()
                        != entries.len()
                {
                    return Err("category palette exceeds 64 unique entries".into());
                }
            }
            Self::Axis {
                axis,
                colormap,
                range,
            } => {
                if *axis > 2 {
                    return Err("color axis must be 0..2".into());
                }
                scalar_range(*colormap, *range)?;
            }
            Self::Range {
                origin_m,
                colormap,
                range,
            } => {
                if origin_m.iter().any(|v| !v.is_finite()) {
                    return Err("range origin must be finite".into());
                }
                scalar_range(*colormap, *range)?;
            }
            Self::Age {
                colormap, range, ..
            } => scalar_range(*colormap, *range)?,
            Self::OccupancyProbability { colormap, .. } | Self::OctreeDepth { colormap } => {
                scalar_range(*colormap, [0.0, 1.0])?
            }
            _ => {}
        }
        Ok(())
    }

    pub fn validate(
        &self,
        schema: Option<&CloudSchema>,
        octree: Option<&OctreeConfig>,
        has_capture: bool,
    ) -> Result<(), String> {
        self.validate_intrinsic()?;
        let field = |name: &str| -> Result<_, String> {
            bounded_text(name, 63, false)?;
            schema
                .and_then(|s| s.attributes.iter().find(|a| a.name == name))
                .ok_or_else(|| format!("unknown color attribute {name}"))
        };
        match self {
            Self::Constant { .. } => {}
            Self::Attribute { name, .. } => {
                if field(name)?.semantic != AttributeSemantic::Color {
                    return Err("RGBA view requires a color attribute".into());
                }
            }
            Self::Scalar {
                name,
                colormap,
                range,
                ..
            } => {
                if field(name)?.components != 1 {
                    return Err("scalar view requires one component".into());
                }
                scalar_range(*colormap, *range)?;
            }
            Self::Category { name, entries, .. } => {
                if field(name)?.semantic != AttributeSemantic::Category {
                    return Err("category view requires a category attribute".into());
                }
                if entries.len() > 64 {
                    return Err("category palette exceeds 64 entries".into());
                }
                let mut keys = HashSet::new();
                for entry in entries {
                    if !keys.insert(entry.value) {
                        return Err("duplicate category palette entry".into());
                    }
                }
            }
            Self::Axis {
                axis,
                colormap,
                range,
            } => {
                if schema.is_none() || *axis > 2 {
                    return Err("axis view requires cloud positions and axis 0..2".into());
                }
                scalar_range(*colormap, *range)?;
            }
            Self::Range {
                origin_m,
                colormap,
                range,
            } => {
                if schema.is_none() || origin_m.iter().any(|v| !v.is_finite()) {
                    return Err("range view requires cloud positions and a finite origin".into());
                }
                scalar_range(*colormap, *range)?;
            }
            Self::Age {
                colormap, range, ..
            } => {
                if !has_capture {
                    return Err("age view requires a capture timestamp".into());
                }
                scalar_range(*colormap, *range)?;
            }
            Self::Occupancy { .. } => {
                if octree.is_none() {
                    return Err("occupancy colors require an octree".into());
                }
            }
            Self::OccupancyProbability { colormap, .. } | Self::OctreeDepth { colormap } => {
                if octree.is_none() {
                    return Err("occupancy probability/depth requires an octree".into());
                }
                scalar_range(*colormap, [0.0, 1.0])?;
            }
        }
        Ok(())
    }

    pub fn validate_capture(&self, capture: Option<&CaptureStamp>) -> Result<(), String> {
        if let Self::Age { clock, .. } = self {
            let capture = capture.ok_or("age view requires a capture timestamp")?;
            capture.validate()?;
            if *clock == AgeClock::Source && capture.source_time_ns.is_none() {
                return Err("source-age view requires a named source clock".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "unit", rename_all = "snake_case", deny_unknown_fields)]
pub enum PointSize {
    Metres { radius: f32 },
    Pixels { radius: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoxelStyle {
    Solid,
    Surface,
    Wire,
}

/// A box in the dataset frame, optionally turned by `rotation_wxyz` (the box
/// axes in that frame; bounds are along them). A very large box with one thin
/// axis is a cross-section; one bound far away makes it a half-space plane.
/// `invert` keeps what lies outside instead of inside.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipBounds {
    pub min_m: [f64; 3],
    pub max_m: [f64; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_wxyz: Option<[f64; 4]>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub invert: bool,
}

impl Default for ClipBounds {
    fn default() -> Self {
        Self {
            min_m: [-0.5; 3],
            max_m: [0.5; 3],
            rotation_wxyz: None,
            invert: false,
        }
    }
}

impl ClipBounds {
    pub fn validate(&self) -> Result<(), String> {
        if (0..3).any(|axis| {
            !self.min_m[axis].is_finite()
                || !self.max_m[axis].is_finite()
                || self.min_m[axis] >= self.max_m[axis]
                || !(self.max_m[axis] - self.min_m[axis]).is_finite()
        }) {
            return Err("clip bounds must have finite, positive extents".into());
        }
        if let Some(rotation) = self.rotation_wxyz {
            super::frames::validate_quaternion(rotation)?;
        }
        Ok(())
    }

    /// Whether the box is axis-aligned and keeps its inside.
    pub fn plain(&self) -> bool {
        self.rotation_wxyz.is_none_or(|q| q == [1.0, 0.0, 0.0, 0.0]) && !self.invert
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OctreeDisplay {
    pub show_occupied: bool,
    pub show_free: bool,
    pub show_unknown: bool,
    pub show_uncertain: bool,
    pub depth_limit: Option<u8>,
    pub roi: Option<ClipBounds>,
    pub unknown_cell_budget: u32,
    pub style: VoxelStyle,
}

impl Default for OctreeDisplay {
    fn default() -> Self {
        Self {
            show_occupied: true,
            show_free: false,
            show_unknown: false,
            show_uncertain: true,
            depth_limit: None,
            roi: None,
            unknown_cell_budget: 100_000,
            style: VoxelStyle::Solid,
        }
    }
}

impl OctreeDisplay {
    pub fn validate_intrinsic(&self) -> Result<(), String> {
        if self
            .depth_limit
            .is_some_and(|depth| depth > super::MAX_OCTREE_DEPTH)
        {
            return Err("display depth exceeds octree maximum".into());
        }
        if let Some(bounds) = self.roi {
            bounds.validate()?;
            if !bounds.plain() {
                return Err("an octree ROI is an axis-aligned box".into());
            }
        }
        if self.unknown_cell_budget == 0
            || self.unknown_cell_budget > 1_000_000
            || (self.show_unknown && self.roi.is_none())
        {
            return Err(
                "unknown-space inspection requires a finite ROI and a 1..1000000 cell budget"
                    .into(),
            );
        }
        Ok(())
    }

    pub fn validate(&self, config: &OctreeConfig) -> Result<(), String> {
        self.validate_intrinsic()?;
        config.validate()?;
        if self
            .depth_limit
            .is_some_and(|depth| depth > config.max_depth)
        {
            return Err("display depth exceeds octree maximum".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Presentation {
    pub color: ColorMode,
    pub opacity: f32,
    pub glow: f32,
    pub blur: f32,
    pub size: PointSize,
    pub radius_attribute: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<ClipBounds>,
    pub octree: Option<OctreeDisplay>,
    /// Sparse arrows along supplied surfel or mesh normals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normals: Option<NormalArrows>,
    /// Slices sampled through a volume.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<super::VolumeView>,
    /// Eye-Dome Lighting strength for opaque cloud points, 0..8; 0 draws them unshaded.
    #[serde(default, skip_serializing_if = "unshaded")]
    pub eye_dome: f32,
}

fn unshaded(strength: &f32) -> bool {
    *strength == 0.0
}

/// One arrow per `every` records that carry a normal; records without one get none.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalArrows {
    pub every: u32,
    pub length_m: f32,
    pub rgba: [u8; 4],
}

impl NormalArrows {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=65_536).contains(&self.every) || !self.length_m.is_finite() || self.length_m <= 0.0 || self.length_m > 100.0 {
            return Err("normal arrows need 1..65536 spacing and a 0..100 m length".into());
        }
        Ok(())
    }
}

impl Default for Presentation {
    fn default() -> Self {
        Self {
            color: ColorMode::Constant {
                rgba: [220, 220, 220, 255],
            },
            opacity: 1.0,
            glow: 0.0,
            blur: 0.0,
            size: PointSize::Pixels { radius: 3.0 },
            radius_attribute: None,
            clip: None,
            octree: None,
            normals: None,
            volume: None,
            eye_dome: 0.0,
        }
    }
}

impl Presentation {
    pub fn for_octree() -> Self {
        Self {
            color: ColorMode::Occupancy {
                occupied: [230, 130, 35, 255],
                free: [65, 190, 115, 64],
                uncertain: [220, 190, 90, 180],
                unknown: [150, 150, 150, 80],
                mixed: [190, 120, 195, 160],
            },
            octree: Some(OctreeDisplay::default()),
            ..Self::default()
        }
    }

    pub fn validate_intrinsic(&self) -> Result<(), String> {
        for (name, value) in [("opacity", self.opacity), ("blur", self.blur)] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(format!("{name} must be finite in 0..1"));
            }
        }
        if !self.glow.is_finite() || self.glow < 0.0 {
            return Err("glow must be finite and nonnegative".into());
        }
        if !self.eye_dome.is_finite() || !(0.0..=8.0).contains(&self.eye_dome) {
            return Err("eye_dome must be finite in 0..8".into());
        }
        let radius = match self.size {
            PointSize::Metres { radius } | PointSize::Pixels { radius } => radius,
        };
        if !radius.is_finite() || radius <= 0.0 {
            return Err("point radius must be positive and finite".into());
        }
        if let Some(name) = &self.radius_attribute {
            bounded_text(name, 63, false)?;
        }
        if let Some(bounds) = self.clip {
            bounds.validate()?;
        }
        if let Some(display) = &self.octree {
            display.validate_intrinsic()?;
        }
        if let Some(arrows) = &self.normals {
            arrows.validate()?;
        }
        if let Some(view) = &self.volume {
            view.validate_intrinsic()?;
        }
        self.color.validate_intrinsic()
    }

    pub fn validate(
        &self,
        schema: Option<&CloudSchema>,
        octree: Option<&OctreeConfig>,
        has_capture: bool,
    ) -> Result<(), String> {
        self.validate_intrinsic()?;
        if let Some(name) = &self.radius_attribute {
            if !schema.is_some_and(|s| {
                s.attributes
                    .iter()
                    .any(|a| a.name == *name && a.semantic == AttributeSemantic::Radius)
            }) {
                return Err("radius source must name a metric radius attribute".into());
            }
        }
        match (&self.octree, octree) {
            (Some(display), Some(config)) => display.validate(config)?,
            (None, None) => {}
            _ => return Err("octree display settings must match the dataset kind".into()),
        }
        self.color.validate(schema, octree, has_capture)
    }
}
