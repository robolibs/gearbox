//! Validated scene field layout and rectangular region partitioning.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_regions_cover_the_domain_without_overlap() {
        let layout: FieldLayout = serde_json::from_str(include_str!("layouts/mixed.json")).unwrap();
        let domain = FieldBounds {
            min: Vec2::splat(-400.0),
            max: Vec2::splat(400.0),
        };
        let regions = layout.regions(domain);
        let area: f32 = regions
            .iter()
            .map(|field| {
                let bounds = field.bounds();
                assert!(domain.contains(bounds.min) && domain.contains(bounds.max));
                let size = bounds.max - bounds.min;
                size.x * size.y
            })
            .sum();
        assert!((area - 640_000.0).abs() < 0.1);
        for (i, field) in regions.iter().enumerate() {
            for other in &regions[..i] {
                assert_ne!(field.name, other.name);
                assert!(!field.bounds().overlaps(other.bounds()));
            }
        }
    }

    #[test]
    fn default_layout_is_the_bundled_mixed_layout() {
        let domain = FieldBounds {
            min: Vec2::splat(-400.0),
            max: Vec2::splat(400.0),
        };
        let regions = FieldLayout::default().regions(domain);
        assert!(regions.iter().any(|field| field.profile == "grassland"));
        assert!(regions.iter().any(|field| field.profile == "harvested_wheat"));
    }

    // The shader reads the points by position out of the matrix columns, so the
    // packing is a contract between two files and not an implementation detail.
    #[test]
    fn a_way_packs_two_points_to_a_column() {
        let points: Vec<Vec2> = (0..8).map(|i| Vec2::new(i as f32, 10.0 + i as f32)).collect();
        let (matrix, shape) = Way::bend(&points, 2.5).packed();
        assert_eq!(shape.x, 8.0);
        assert_eq!(shape.y, 2.5);
        for (i, point) in points.iter().enumerate() {
            let column = matrix.col(i / 2);
            let packed = if i % 2 == 0 { column.xy() } else { column.zw() };
            assert_eq!(packed, *point);
        }
    }

    #[test]
    fn a_way_needs_two_points_and_keeps_at_most_eight() {
        assert_eq!(Way::bend(&[Vec2::ZERO], 1.0).points(), 0);
        assert_eq!(Way::straight().points(), 0);
        let many: Vec<Vec2> = (0..12).map(|i| Vec2::splat(i as f32)).collect();
        assert_eq!(Way::bend(&many, 1.0).points(), Way::MOST as u32);
    }

    #[test]
    fn a_layout_way_defaults_to_the_field_width() {
        let spec: FieldSpec = serde_json::from_str(
            r#"{"name":"lane","profile":"track","min":[0,0],"max":[10,100],
                "way":[[5,0],[5,100]]}"#,
        )
        .unwrap();
        assert_eq!(spec.way().packed().1.y, 5.0);
    }
}

use super::profile::FieldProfiles;
use bevy::prelude::*;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldBounds {
    pub min: Vec2,
    pub max: Vec2,
}

impl Default for FieldBounds {
    fn default() -> Self {
        Self { min: Vec2::ZERO, max: Vec2::ZERO }
    }
}

impl FieldBounds {
    pub fn contains(&self, point: Vec2) -> bool {
        point.cmpge(self.min).all() && point.cmple(self.max).all()
    }

    pub fn overlaps(&self, other: Self) -> bool {
        self.min.cmplt(other.max).all() && other.min.cmplt(self.max).all()
    }

    pub fn nearest_distance(&self, point: Vec2) -> f32 {
        point.distance(point.clamp(self.min, self.max))
    }
}

/// The line the wheels follow through a field, in world XZ. A field is always a
/// rectangle, so without this a way can only run straight down one; with it the
/// field is merely the corridor a track winds along inside.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Way {
    points: [Vec2; Way::MOST],
    count: u32,
    half_width: f32,
}

impl Way {
    pub const MOST: usize = 8;

    /// Nothing, for a field whose wear runs down its own long axis.
    pub fn straight() -> Self {
        Self::default()
    }

    /// How many points bend it; fewer than two is no way at all.
    pub fn points(&self) -> u32 {
        self.count
    }

    /// Fewer than two points is no way at all; beyond `MOST` the tail is cut.
    pub fn bend(points: &[Vec2], half_width: f32) -> Self {
        let mut way = Self { half_width, ..Self::default() };
        if points.len() < 2 {
            return way;
        }
        let taken = points.len().min(Self::MOST);
        way.points[..taken].copy_from_slice(&points[..taken]);
        way.count = taken as u32;
        way
    }

    /// For a shader: the points two to a column, and (how many, half the width).
    pub fn packed(&self) -> (Mat4, Vec4) {
        let pair = |i: usize| {
            let (a, b) = (self.points[i * 2], self.points[i * 2 + 1]);
            Vec4::new(a.x, a.y, b.x, b.y)
        };
        (
            Mat4::from_cols(pair(0), pair(1), pair(2), pair(3)),
            Vec4::new(self.count as f32, self.half_width, 0.0, 0.0),
        )
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldSpec {
    pub name: String,
    pub profile: String,
    pub min: [f32; 2],
    pub max: [f32; 2],
    /// How hard this field is worn, nought to one: a green lane barely marked,
    /// two bare ruts with grass between them, or bare across its width. Left
    /// out, the profile decides.
    #[serde(default)]
    pub wear: Option<f32>,
    /// World XZ points of the line the wheels follow, at most eight. Left out,
    /// the wear runs straight down the field's long axis.
    #[serde(default)]
    pub way: Vec<[f32; 2]>,
    /// How wide the worn corridor is, in metres. Left out, it is as wide as the
    /// field is across.
    #[serde(default)]
    pub way_width: Option<f32>,
}

impl FieldSpec {
    pub fn bounds(&self) -> FieldBounds {
        FieldBounds {
            min: Vec2::from_array(self.min),
            max: Vec2::from_array(self.max),
        }
    }

    pub fn way(&self) -> Way {
        let bounds = self.bounds();
        let span = bounds.max - bounds.min;
        let across = if span.x < span.y { span.x } else { span.y };
        let points: Vec<Vec2> = self.way.iter().copied().map(Vec2::from_array).collect();
        Way::bend(&points, self.way_width.unwrap_or(across) * 0.5)
    }
}

#[derive(Resource, Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldLayout {
    pub default: String,
    #[serde(default)]
    pub fields: Vec<FieldSpec>,
}

impl Default for FieldLayout {
    fn default() -> Self {
        // Grass everywhere, with a harvested-wheat stubble region cut into
        // it — the bundled `mixed.json` is the single source of truth for
        // this so the default layout can't drift from its own test fixture.
        serde_json::from_str(include_str!("layouts/mixed.json"))
            .expect("bundled default field layout must be valid")
    }
}

impl FieldLayout {
    pub fn from_env() -> Result<Self, String> {
        match std::env::var("GEARBOX_FIELD_LAYOUT") {
            Ok(path) => {
                let contents = std::fs::read_to_string(&path)
                    .map_err(|error| format!("field layout {path}: {error}"))?;
                serde_json::from_str(&contents)
                    .map_err(|error| format!("field layout {path}: {error}"))
            }
            Err(std::env::VarError::NotPresent) => Ok(Self::default()),
            Err(error) => Err(format!("GEARBOX_FIELD_LAYOUT: {error}")),
        }
    }

    pub fn validate(&self, profiles: &FieldProfiles) -> Result<(), String> {
        if !profiles.0.contains_key(&self.default) {
            return Err(format!("unknown default field profile: {}", self.default));
        }
        for (i, field) in self.fields.iter().enumerate() {
            let bounds = field.bounds();
            if field.name.trim().is_empty()
                || field.name.starts_with("__background/")
                || self.fields[..i].iter().any(|f| f.name == field.name)
            {
                return Err(format!("empty or duplicate field name: {}", field.name));
            }
            if !profiles.0.contains_key(&field.profile) {
                return Err(format!("unknown field profile: {}", field.profile));
            }
            if !bounds.min.is_finite()
                || !bounds.max.is_finite()
                || !bounds.min.cmplt(bounds.max).all()
            {
                return Err(format!("invalid bounds for {}", field.name));
            }
            if self.fields[..i]
                .iter()
                .any(|other| bounds.overlaps(other.bounds()))
            {
                return Err(format!("overlapping field: {}", field.name));
            }
        }
        Ok(())
    }

    /// The fields as they fall on `terrain`, cut to it, and the default cover
    /// over the rest of it. The terrain may lie anywhere over the layout.
    pub fn regions(&self, terrain: FieldBounds) -> Vec<FieldSpec> {
        let fields: Vec<FieldSpec> = self
            .fields
            .iter()
            .filter(|field| field.bounds().overlaps(terrain))
            .map(|field| FieldSpec {
                min: field.bounds().min.max(terrain.min).to_array(),
                max: field.bounds().max.min(terrain.max).to_array(),
                ..field.clone()
            })
            .collect();
        let mut xs = vec![terrain.min.x, terrain.max.x];
        let mut zs = vec![terrain.min.y, terrain.max.y];
        for field in &fields {
            xs.extend([field.min[0], field.max[0]]);
            zs.extend([field.min[1], field.max[1]]);
        }
        xs.sort_by(f32::total_cmp);
        zs.sort_by(f32::total_cmp);
        xs.dedup();
        zs.dedup();
        let mut regions = fields.clone();
        for z in zs.windows(2) {
            let mut start = None;
            for x in xs.windows(2) {
                let middle = Vec2::new((x[0] + x[1]) * 0.5, (z[0] + z[1]) * 0.5);
                let occupied = fields
                    .iter()
                    .any(|field| field.bounds().contains(middle));
                if occupied {
                    if let Some(left) = start.take() {
                        regions.push(self.background_region(left, x[0], z[0], z[1], regions.len()));
                    }
                } else {
                    start.get_or_insert(x[0]);
                }
            }
            if let Some(left) = start {
                regions.push(self.background_region(
                    left,
                    terrain.max.x,
                    z[0],
                    z[1],
                    regions.len(),
                ));
            }
        }
        regions
    }

    fn background_region(&self, x0: f32, x1: f32, z0: f32, z1: f32, index: usize) -> FieldSpec {
        FieldSpec {
            name: format!("__background/{index}"),
            profile: self.default.clone(),
            min: [x0, z0],
            max: [x1, z1],
            wear: None,
            way: Vec::new(),
            way_width: None,
        }
    }
}
