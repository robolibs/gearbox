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
    /// The tyre rolling here, as `TyreTread::packed`. A field's wheel map is
    /// stamped by whatever drives over it, and until a machine declares its own
    /// tyre this is the one it is taken to have.
    pub bar: Vec4,
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

/// Builds a field's ground. The bounds are the field's own rectangle: a
/// surface that wears unevenly — a road with ruts down it — needs to know
/// where the field runs before it can say which part of it is worn.
pub type GroundFactory =
    fn(&mut World, Handle<Image>, WheelMapParams, SurfaceGeometry, Placed) -> Arc<dyn GroundSurface>;

/// Where a field sits and what it is up against: everything a ground needs to
/// know that its profile cannot say, because it differs field by field.
#[derive(Clone, Copy, Debug, Default)]
pub struct Placed {
    /// The field's own rectangle.
    pub bounds: super::layout::FieldBounds,
    /// What lies across each side, as that neighbour's own surface colour, and
    /// how far the two blend. Order is west, east, south, north; a side with no
    /// soft neighbour blends nought.
    pub tint: [Vec4; 4],
    pub reach: [f32; 4],
    /// How hard this whole field is worn, nought to one, when the layout says
    /// so and bends no line through it. `None` leaves it to the profile.
    pub wear: Option<f32>,
    /// The lines the wheels follow through it, each worn its own amount, when
    /// the layout bends any.
    pub way: super::layout::Way,
}

impl Placed {
    /// Whether the layout wears this field at all, rather than leaving it to
    /// whatever its profile is.
    pub fn worn(&self) -> bool {
        self.wear.is_some() || self.way.points() >= 2
    }

    /// What the shaders read wear from, for however many lines cross it.
    pub fn tread(&self) -> Vec4 {
        self.way.tread(self.wear)
    }

    /// The tyre that prints each line crossing it. A field worn down its own
    /// long axis with no line bent through it has no tyre of its own to name,
    /// and takes the default one.
    pub fn bars(&self) -> (Vec4, Vec4) {
        self.way.bars()
    }
}

pub struct FieldProfile {
    pub name: &'static str,
    pub wheel_response: WheelResponse,
    pub layers: Vec<VegetationLayer>,
    pub ground: GroundFactory,
    /// How this profile's surface is worn by wheels, as `BareGround::tread`:
    /// zero for a surface that wears evenly, which is most of them.
    pub tread: Vec4,
    /// What this profile's ground looks like from a distance, as a linear
    /// colour. A neighbouring field blends its own ground towards this across
    /// a soft border, so two surfaces meet in a wash rather than on a line.
    pub surface_tint: Vec4,
    /// How far what grows in this field carries past its own edge, in metres.
    /// A hedge line or a yard wall is a hard border and gets nought; a meadow
    /// running into a track is a soft one, and the two interleave over this
    /// distance instead of meeting along a ruled line.
    pub soft_border: f32,
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
