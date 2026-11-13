use std::collections::HashSet;

use serde::{Deserialize, Serialize};

pub const MAX_OCTREE_DEPTH: u8 = 21;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OctreeConfig {
    pub leaf_resolution_m: f64,
    pub max_depth: u8,
    pub free_max: f32,
    pub occupied_min: f32,
}

impl OctreeConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.max_depth > MAX_OCTREE_DEPTH {
            return Err("octree maximum depth exceeds 21".into());
        }
        if !self.leaf_resolution_m.is_finite()
            || self.leaf_resolution_m <= 0.0
            || !self.root_span_m().is_finite()
        {
            return Err("octree resolution and root span must be positive finite metres".into());
        }
        if !self.free_max.is_finite()
            || !self.occupied_min.is_finite()
            || self.free_max < 0.0
            || self.occupied_min > 1.0
            || self.free_max >= self.occupied_min
        {
            return Err("occupancy thresholds require 0 <= free_max < occupied_min <= 1".into());
        }
        Ok(())
    }

    pub fn root_span_m(&self) -> f64 {
        self.leaf_resolution_m * 2f64.powi(self.max_depth.into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoxelKey {
    pub tile: [i64; 3],
    pub depth: u8,
    pub morton: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridBounds {
    pub min: [i128; 3],
    pub size: u64,
}

impl VoxelKey {
    pub fn validate(&self, config: &OctreeConfig) -> Result<(), String> {
        config.validate()?;
        if self.depth > config.max_depth || self.morton >> (3 * self.depth as u32) != 0 {
            return Err("octree key has invalid depth or nonzero unused Morton bits".into());
        }
        Ok(())
    }

    pub fn parent(&self) -> Option<Self> {
        (self.depth > 0 && self.depth <= MAX_OCTREE_DEPTH).then(|| Self {
            tile: self.tile,
            depth: self.depth - 1,
            morton: self.morton >> 3,
        })
    }

    pub fn child(&self, child: u8, config: &OctreeConfig) -> Result<Self, String> {
        self.validate(config)?;
        if child >= 8 || self.depth == config.max_depth {
            return Err("octree child index/depth exceeds its bounds".into());
        }
        Ok(Self {
            tile: self.tile,
            depth: self.depth + 1,
            morton: (self.morton << 3) | u64::from(child),
        })
    }

    pub fn contains(&self, other: &Self) -> bool {
        self.depth <= other.depth
            && other.depth <= MAX_OCTREE_DEPTH
            && self.tile == other.tile
            && self.morton == other.morton >> (3 * (other.depth - self.depth) as u32)
    }

    /// Bounds in integer coordinates at the configured finest resolution.
    pub fn grid_bounds(&self, config: &OctreeConfig) -> Result<GridBounds, String> {
        self.validate(config)?;
        let root = 1i128 << config.max_depth;
        let size = 1u64 << (config.max_depth - self.depth);
        let mut local = [0i128; 3];
        for level in (0..self.depth).rev() {
            let child = self.morton >> (3 * level as u32) & 7;
            for (axis, value) in local.iter_mut().enumerate() {
                *value = (*value << 1) | ((child >> axis) & 1) as i128;
            }
        }
        Ok(GridBounds {
            min: std::array::from_fn(|axis| {
                self.tile[axis] as i128 * root + local[axis] * size as i128
            }),
            size,
        })
    }

    pub fn from_grid_cell(
        cell: [i128; 3],
        depth: u8,
        config: &OctreeConfig,
    ) -> Result<Self, String> {
        config.validate()?;
        if depth > config.max_depth {
            return Err("cell depth exceeds octree maximum".into());
        }
        let root = 1i128 << config.max_depth;
        let mut tile = [0i64; 3];
        let mut local = [0u64; 3];
        for axis in 0..3 {
            tile[axis] = i64::try_from(cell[axis].div_euclid(root))
                .map_err(|_| "octree tile coordinate overflow")?;
            local[axis] = (cell[axis].rem_euclid(root) >> (config.max_depth - depth)) as u64;
        }
        let mut morton = 0;
        for level in (0..depth).rev() {
            let child = (0..3).fold(0, |bits, axis| bits | ((local[axis] >> level) & 1) << axis);
            morton = (morton << 3) | child;
        }
        Ok(Self {
            tile,
            depth,
            morton,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OccupancyState {
    Unknown,
    Free,
    Uncertain,
    Occupied,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OccupancyValue {
    pub probability: Option<f32>,
    pub confidence: Option<f32>,
    pub class_id: Option<u32>,
}

impl OccupancyValue {
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("probability", self.probability),
            ("confidence", self.confidence),
        ] {
            if let Some(value) = value {
                if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                    return Err(format!("occupancy {name} must be finite in 0..1"));
                }
            }
        }
        Ok(())
    }

    pub fn state(&self, config: &OctreeConfig) -> Result<OccupancyState, String> {
        config.validate()?;
        self.validate()?;
        Ok(match self.probability {
            None => OccupancyState::Unknown,
            Some(p) if p <= config.free_max => OccupancyState::Free,
            Some(p) if p >= config.occupied_min => OccupancyState::Occupied,
            Some(_) => OccupancyState::Uncertain,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OccupancyLeaf {
    pub key: VoxelKey,
    pub value: OccupancyValue,
}

pub fn validate_leaf_set(config: &OctreeConfig, leaves: &[OccupancyLeaf]) -> Result<(), String> {
    config.validate()?;
    let mut keys = HashSet::with_capacity(leaves.len());
    for leaf in leaves {
        leaf.key.validate(config)?;
        leaf.value.validate()?;
        if !keys.insert(leaf.key) {
            return Err("duplicate occupancy leaf key".into());
        }
    }
    for leaf in leaves {
        let mut parent = leaf.key.parent();
        while let Some(key) = parent {
            if keys.contains(&key) {
                return Err("overlapping occupancy ancestor/descendant leaves".into());
            }
            parent = key.parent();
        }
    }
    Ok(())
}
