//! Validated scene field layout and rectangular region partitioning.

#[cfg(test)]
#[path = "layout/friction_tests.rs"]
mod friction_tests;

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
}

use super::profile::FieldProfiles;
use bevy::prelude::*;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldBounds {
    pub min: Vec2,
    pub max: Vec2,
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

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldSpec {
    pub name: String,
    pub profile: String,
    pub min: [f32; 2],
    pub max: [f32; 2],
    /// Dimensionless ground friction; absent values inherit the layout default.
    #[serde(default)]
    pub friction: Option<f64>,
}

impl FieldSpec {
    pub fn bounds(&self) -> FieldBounds {
        FieldBounds {
            min: Vec2::from_array(self.min),
            max: Vec2::from_array(self.max),
        }
    }
}

#[derive(Resource, Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldLayout {
    pub default: String,
    /// Dimensionless background friction; absent values use the ground collider.
    #[serde(default)]
    pub default_friction: Option<f64>,
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

    pub fn validate(&self, profiles: &FieldProfiles, terrain: FieldBounds) -> Result<(), String> {
        self.validate_bounds(terrain)?;
        if !profiles.0.contains_key(&self.default) {
            return Err(format!("unknown default field profile: {}", self.default));
        }
        for field in &self.fields {
            if !profiles.0.contains_key(&field.profile) {
                return Err(format!("unknown field profile: {}", field.profile));
            }
        }
        Ok(())
    }

    fn validate_bounds(&self, terrain: FieldBounds) -> Result<(), String> {
        if !terrain.min.is_finite()
            || !terrain.max.is_finite()
            || !terrain.min.cmplt(terrain.max).all()
        {
            return Err("invalid field terrain bounds".into());
        }
        for friction in self
            .default_friction
            .iter()
            .chain(self.fields.iter().filter_map(|field| field.friction.as_ref()))
        {
            if !friction.is_finite() || *friction < 0.0 {
                return Err("field friction must be finite and non-negative".into());
            }
        }
        for (i, field) in self.fields.iter().enumerate() {
            let bounds = field.bounds();
            if field.name.trim().is_empty()
                || field.name.starts_with("__background/")
                || self.fields[..i].iter().any(|f| f.name == field.name)
            {
                return Err(format!("empty or duplicate field name: {}", field.name));
            }
            if !bounds.min.is_finite()
                || !bounds.max.is_finite()
                || !bounds.min.cmplt(bounds.max).all()
                || !terrain.contains(bounds.min)
                || !terrain.contains(bounds.max)
            {
                return Err(format!(
                    "invalid or out-of-terrain bounds for {}",
                    field.name
                ));
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

    /// Row-major world-XZ friction samples, or None when no friction is authored.
    pub fn friction_samples(
        &self,
        grid: &super::HeightGrid,
        fallback: f64,
    ) -> Result<Option<Vec<f64>>, String> {
        if !fallback.is_finite()
            || fallback < 0.0
            || !grid.cell.is_finite()
            || grid.cell <= 0.0
            || grid.cols < 2
            || grid.rows < 2
        {
            return Err("invalid field friction grid or collider friction".into());
        }
        let domain = FieldBounds {
            min: Vec2::new(grid.min_x, grid.min_z),
            max: Vec2::new(
                grid.min_x + (grid.cols - 1) as f32 * grid.cell,
                grid.min_z + (grid.rows - 1) as f32 * grid.cell,
            ),
        };
        self.validate_bounds(domain)?;
        let count = grid.cols
            .checked_mul(grid.rows)
            .filter(|&n| n <= 16_777_216)
            .ok_or("field friction grid exceeds 16 million samples")?;
        if self.default_friction.is_none() && self.fields.iter().all(|field| field.friction.is_none()) {
            return Ok(None);
        }
        let default = self.default_friction.unwrap_or(fallback);
        let mut values = Vec::with_capacity(count);
        for row in 0..grid.rows {
            for col in 0..grid.cols {
                let point = Vec2::new(
                    grid.min_x + col as f32 * grid.cell,
                    grid.min_z + row as f32 * grid.cell,
                );
                let friction = self.fields
                    .iter()
                    .find(|field| {
                        point.cmpge(Vec2::from_array(field.min)).all()
                            && point.cmplt(Vec2::from_array(field.max)).all()
                    })
                    .and_then(|field| field.friction)
                    .unwrap_or(default);
                values.push(friction);
            }
        }
        Ok(Some(values))
    }

    pub fn regions(&self, terrain: FieldBounds) -> Vec<FieldSpec> {
        let mut xs = vec![terrain.min.x, terrain.max.x];
        let mut zs = vec![terrain.min.y, terrain.max.y];
        for field in &self.fields {
            xs.extend([field.min[0], field.max[0]]);
            zs.extend([field.min[1], field.max[1]]);
        }
        xs.sort_by(f32::total_cmp);
        zs.sort_by(f32::total_cmp);
        xs.dedup();
        zs.dedup();
        let mut regions = self.fields.clone();
        for z in zs.windows(2) {
            let mut start = None;
            for x in xs.windows(2) {
                let middle = Vec2::new((x[0] + x[1]) * 0.5, (z[0] + z[1]) * 0.5);
                let occupied = self
                    .fields
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
            friction: self.default_friction,
        }
    }
}
