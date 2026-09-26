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
    /// Drawn with alpha-to-coverage and a fragment shader that may discard;
    /// otherwise opaque, without either.
    pub cutout: bool,
    /// Grows only where a way wears the ground: chunks no way reaches issue none.
    pub way_only: bool,
    /// Sown on the GPU into blade records and drawn from them, instead of
    /// drawn per chunk from `template`.
    pub blade: Option<BladeLayer>,
    /// Compute shader that sieves each chunk's candidates before it is drawn,
    /// so the vertex shader runs only for those that may show.
    pub sieve: Option<&'static str>,
}

/// One kind of GPU-sown blade: a compute shader that culls and shapes each
/// blade into a record, and a draw shader that bends a strip from it.
#[derive(Debug)]
pub struct BladeKind {
    pub cull: &'static str,
    pub draw: &'static str,
    /// The strip every record bends, at the finest detail level.
    pub template: fn() -> Mesh,
    /// Words (u32) per record.
    pub stride: u32,
    /// Records one view holds.
    pub capacity: u32,
}

/// A detail level of a GPU-sown blade population.
#[derive(Clone, Copy, Debug)]
pub struct BladeLayer {
    pub kind: &'static BladeKind,
    /// Blades rooted at each instance.
    pub twins: u32,
    /// Segments a blade has at this detail level.
    pub segments: u32,
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
    /// Registers a profile, its vegetation distances scaled by
    /// `GEARBOX_VEGETATION_RANGE` (1 when unset).
    pub fn register(&mut self, mut profile: FieldProfile) {
        assert!(
            !self.0.contains_key(profile.name),
            "duplicate field profile: {}",
            profile.name
        );
        let range = vegetation_range();
        for layer in &mut profile.layers {
            layer.scale_range(range);
        }
        self.0.insert(profile.name.to_owned(), Arc::new(profile));
    }
}

/// `GEARBOX_VEGETATION_RANGE`: how far vegetation reaches, as a share of the
/// authored distances.
pub fn vegetation_range() -> f32 {
    std::env::var("GEARBOX_VEGETATION_RANGE")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(1.0)
}

impl VegetationLayer {
    /// Every distance of the layer multiplied by `range`: the fade and the
    /// inner and outer edges of its detail band.
    pub fn scale_range(&mut self, range: f32) {
        self.fade_start *= range;
        self.fade_end *= range;
        self.lod_band = self.lod_band.map(|edge| if edge >= f32::MAX / 2.0 { edge } else { edge * range });
    }
}
