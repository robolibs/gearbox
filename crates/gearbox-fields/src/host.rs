//! What a host gives the fields: ground heights, the terrain they cover,
//! the wind that blows over them, and which meshes they may dress.

use crate::heights::HeightGrid;
use bevy::prelude::*;
use std::sync::Arc;

/// Ground height anywhere, for plants that stand outside the covered grid.
pub trait HeightSource: Send + Sync {
    fn height(&self, x: f32, z: f32) -> f32;
}

#[derive(Resource, Clone)]
pub struct CoverHeights(pub Arc<dyn HeightSource>);

/// The terrain the fields cover: its root entity and its height samples.
#[derive(Resource, Clone)]
pub struct CoverTerrain {
    pub entity: Entity,
    pub grid: Arc<HeightGrid>,
}

/// Mesh geometry a field surface may be assigned to.
#[derive(Component)]
pub struct CoverSurfaceMesh;

/// A surface mesh that takes the distant backdrop material, not a field.
#[derive(Component)]
pub struct CoverBackdrop;

/// The wind every plant leans in.
#[derive(Resource, Clone, Copy, Debug)]
pub struct CoverWind {
    pub heading_deg: f32,
    pub speed_mps: f32,
    /// How far the push drops between gusts: 0 steady, 1 gusty.
    pub gustiness: f32,
}

impl Default for CoverWind {
    fn default() -> Self {
        Self { heading_deg: 36.87, speed_mps: 4.0, gustiness: 0.7 }
    }
}

impl CoverWind {
    /// Packed for the shaders as (downwind x, downwind z, speed, gustiness).
    pub fn vector(&self) -> Vec4 {
        let (sin, cos) = self.heading_deg.to_radians().sin_cos();
        Vec4::new(cos, sin, self.speed_mps.max(0.0), self.gustiness.clamp(0.0, 1.0))
    }
}

/// Scene roots whose descendant meshes take the bare soil material; the
/// host fills it (gearbox: its USD terrain roots).
#[derive(Resource, Default)]
pub struct CoverTerrainRoots(pub Vec<Entity>);
