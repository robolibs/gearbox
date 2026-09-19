//! Package registration and the terrain-independent surface contract.

use bevy::pbr::Material;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;
use std::sync::Arc;

pub trait GroundSurface: Send + Sync {
    fn apply(&self, commands: &mut Commands, entity: Entity);
}

pub struct MaterialSurface<M: Material>(pub Handle<M>);

impl<M: Material> GroundSurface for MaterialSurface<M> {
    fn apply(&self, commands: &mut Commands, entity: Entity) {
        commands
            .entity(entity)
            .insert(MeshMaterial3d(self.0.clone()));
    }
}

#[derive(Clone, Copy, Debug)]
pub struct WheelResponse {
    pub recovery_seconds: f32,
    pub bend: f32,
    pub darkening: f32,
    pub footprint_length: f32,
    /// Hard ground that prints tyre tread: its wheel map carries two more
    /// channels of tread coordinates, doubling its size.
    pub tread: bool,
}

#[derive(ShaderType, Reflect, Clone, Copy, Debug, Default)]
pub struct WheelMapParams {
    pub origin: Vec2,
    pub texels_per_metre: f32,
    pub width: f32,
    pub height: f32,
    pub recovery_seconds: f32,
    pub bend: f32,
    pub darkening: f32,
}

#[derive(Clone)]
pub struct VegetationLayer {
    pub shader: &'static str,
    pub template: fn() -> Mesh,
    pub density: f32,
    pub fade_start: f32,
    pub fade_end: f32,
    pub inverse_square_thinning: bool,
    /// Albedo of an asset clump layer; procedural layers have none.
    pub albedo: Option<&'static str>,
    /// Camera distance band this layer's mesh draws in; detail levels of
    /// one blade population split the distance between them.
    pub lod_band: [f32; 2],
    /// Nought to scatter this layer by its own reckoning. Otherwise the layer
    /// follows the grass patches of the ground it stands on, and this is the
    /// share of it that is still allowed out on the bare between them.
    pub follow_grass: f32,
}

impl VegetationLayer {
    /// The same layer, kept to the ground's grass patches but for `share` of it.
    pub fn following(mut self, share: f32) -> Self {
        self.follow_grass = share.max(0.001);
        self
    }
}

#[derive(ShaderType, Reflect, Clone, Copy, Debug, Default)]
pub struct SurfaceGeometryParams {
    pub origin: Vec2,
    pub texels_per_metre: f32,
    pub texel_count: f32,
}

#[derive(Clone)]
pub struct SurfaceGeometry {
    pub heightmap: Handle<Image>,
    pub params: SurfaceGeometryParams,
}

pub type GroundFactory =
    fn(&mut World, Handle<Image>, WheelMapParams, SurfaceGeometry) -> Arc<dyn GroundSurface>;

pub struct FieldProfile {
    pub name: &'static str,
    pub wheel_response: WheelResponse,
    pub layers: Vec<VegetationLayer>,
    pub ground: GroundFactory,
}

#[derive(Resource, Default)]
pub struct FieldProfiles(pub HashMap<String, Arc<FieldProfile>>);

impl FieldProfiles {
    pub fn register(&mut self, profile: FieldProfile) {
        assert!(
            !self.0.contains_key(profile.name),
            "duplicate field profile: {}",
            profile.name
        );
        self.0.insert(profile.name.to_owned(), Arc::new(profile));
    }
}
