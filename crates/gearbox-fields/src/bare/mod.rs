//! Bare ground: soil with nothing standing in it.
//!
//! One ground serves sand, dirt and ploughed earth alike; they differ in
//! their colour, how coarse their clods are and whether the wind has combed
//! ripples into them. The soil itself is made in the shader rather than read
//! from a photograph, and so are the stones, the crumbs of earth and the tufts
//! of grass standing in it. The weeds are not: those are scanned plants, drawn
//! from the packed clumps, because there are few enough of them in view to
//! afford it.

use std::sync::Arc;

use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::PrimitiveTopology;

use crate::profile::{
    FieldProfile, FieldProfiles, GroundFactory, GroundSurface, MaterialSurface, SurfaceGeometry,
    SurfaceGeometryParams, VegetationLayer, WheelMapParams, WheelResponse,
};

/// A stone, as a lump with `rings` bands of `around` faces, pushed in and out
/// so no two of its faces lie flat. The top and bottom rings close to a single
/// point, or the stone is a tube and the ground shows through the hole.
fn pebble() -> Mesh {
    lump_mesh(STONE, 0.0, 9, 5)
}

/// The same stone, marked in its second channel as one the sun and the sand
/// have bleached: no flint, and nothing darker than the ground it lies on.
fn bleached_pebble() -> Mesh {
    lump_mesh(STONE, 1.0, 9, 5)
}

/// A crumb of the ground's own earth, not a stone. The second channel says
/// which soil it was broken off, so the shader can colour it to match.
/// Coarse on purpose: a crumb is a centimetre across and there are thousands
/// of them, so it gets a fifth of a stone's triangles and none of them show.
fn turned_clod() -> Mesh {
    lump_mesh(CLOD, 0.0, 5, 3)
}

fn worn_clod() -> Mesh {
    lump_mesh(CLOD, 0.25, 5, 3)
}

fn blown_clod() -> Mesh {
    lump_mesh(CLOD, 0.5, 5, 3)
}

/// What the first channel of a lump's texture coordinate marks it as. A tuft
/// carries one; a stone's own coordinates run the whole way round it, so they
/// cannot be used to say.
const STONE: f32 = 0.0;
const CLOD: f32 = 0.35;

fn lump_mesh(kind: f32, mark: f32, around: u32, rings: u32) -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    // A hash, not a sine: one sine of `step` turns the same amount every face
    // and dents the stone in and out by turns, which carves it into a starfish.
    let dent = |ring: u32, step: u32| {
        let mut h = ring.wrapping_mul(0x9E37_79B9) ^ step.wrapping_mul(0x85EB_CA6B);
        h ^= h >> 15;
        h = h.wrapping_mul(0x2C1B_3C6D);
        h ^= h >> 12;
        0.76 + 0.24 * (h as f32 / u32::MAX as f32)
    };
    for ring in 0..=rings {
        let v = ring as f32 / rings as f32;
        let pole = ring == 0 || ring == rings;
        let lift = (v * std::f32::consts::PI).cos();
        let round = if pole { 0.0 } else { (v * std::f32::consts::PI).sin() };
        for step in 0..around {
            let u = step as f32 / around as f32 * std::f32::consts::TAU;
            // Every vertex of a closing ring takes the same radius, so they
            // land on one another and the cap has no seam.
            let radius = dent(ring, if pole { 0 } else { step });
            let point = Vec3::new(u.cos() * round * radius, lift * radius, u.sin() * round * radius);
            positions.push(point.to_array());
            normals.push(if pole {
                [0.0, lift.signum(), 0.0]
            } else {
                point.normalize().to_array()
            });
        }
    }
    for ring in 0..rings {
        for step in 0..around {
            let next = (step + 1) % around;
            let a = ring * around + step;
            let b = ring * around + next;
            let c = (ring + 1) * around + step;
            let d = (ring + 1) * around + next;
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals.clone())
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[kind, mark]; normals.len()])
        .with_inserted_indices(bevy::mesh::Indices::U32(indices))
}

/// A tuft of three leaves, each a strip two quads tall.
fn tuft() -> Mesh {
    leaf_mesh(TUFT, 3, 3)
}

const TUFT: f32 = 1.0;

fn leaf_mesh(kind: f32, leaves: u32, steps: u32) -> Mesh {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for leaf in 0..leaves {
        let start = positions.len() as u32;
        for step in 0..=steps {
            let t = step as f32 / steps as f32;
            positions.push([-1.0, t, leaf as f32]);
            positions.push([1.0, t, leaf as f32]);
        }
        for step in 0..steps {
            let row = start + step * 2;
            indices.extend_from_slice(&[row, row + 2, row + 1, row + 1, row + 2, row + 3]);
        }
    }
    let count = positions.len();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; count])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[kind, 0.0]; count])
        .with_inserted_indices(bevy::mesh::Indices::U32(indices))
}

/// What stands in a bare ground: stones, crumbs of its own earth, and the
/// tufts of grass that take it back. Sand carries no tufts; a worn track
/// carries all three.
fn standing(stones: f32, clods: f32, tufts: f32, weeds: f32, soil: fn() -> Mesh) -> Vec<VegetationLayer> {
    use super::clumps;
    let shader = "embedded://gearbox_fields/bare/shaders/vegetation.wgsl";
    let bleached = soil == blown_clod as fn() -> Mesh;
    let mut layers = vec![
        VegetationLayer {
            shader,
            template: if bleached { bleached_pebble } else { pebble },
            density: stones,
            fade_start: 6.0,
            fade_end: 42.0,
            inverse_square_thinning: true,
            follow_grass: 0.0,
            albedo: None,
            lod_band: [0.0, f32::MAX],
        },
        // Crumbs are smaller than stones and far more of them, so they are not
        // worth carrying anything like as far — but the fade has to be long, or
        // the ground ends in a ring of coarse texture with smooth beyond it.
        VegetationLayer {
            shader,
            template: soil,
            density: clods,
            fade_start: 6.0,
            fade_end: 26.0,
            inverse_square_thinning: true,
            follow_grass: 0.0,
            albedo: None,
            lod_band: [0.0, f32::MAX],
        },
    ];
    if tufts > 0.0 {
        layers.push(VegetationLayer {
            shader,
            template: tuft,
            density: tufts,
            fade_start: 6.0,
            fade_end: 34.0,
            inverse_square_thinning: true,
            follow_grass: 0.0,
            albedo: None,
            lod_band: [0.0, f32::MAX],
        });
    }
    // The weeds themselves are scanned plants, not shapes made in a shader:
    // there are only ever a few dozen of them in view, so they can carry the
    // triangles and the alpha maps the grass never could.
    if weeds > 0.0 {
        layers.push(clumps::flat_weeds(weeds * 0.5, 34.0).following(0.06));
        layers.push(clumps::dandelion(weeds * 0.3, 34.0).following(0.1));
        layers.push(clumps::nettle(weeds * 0.12, 30.0).following(0.03));
        layers.push(clumps::celandine(weeds * 0.2, 30.0).following(0.05));
    }
    layers
}

type BareMaterial = ExtendedMaterial<StandardMaterial, BareExtension>;

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
struct BareExtension {
    #[texture(105, sample_type = "u_int")]
    trample: Handle<Image>,
    #[uniform(106)]
    trample_params: WheelMapParams,
    #[texture(107, sample_type = "float", filterable = false)]
    heightmap: Handle<Image>,
    #[uniform(108)]
    geometry: SurfaceGeometryParams,
    #[uniform(109)]
    ground: BareGround,
}

/// What kind of bare ground this is.
#[derive(ShaderType, Reflect, Debug, Clone, Copy)]
pub struct BareGround {
    /// Multiplies the soil's own colour.
    pub tint: Vec4,
    /// Metres across the clods, how deep they sit, how much the wind has
    /// combed it into ripples, and how far apart those ripples run.
    pub grain: Vec4,
    /// The colour of the grass that grows through it, and how much of the
    /// ground it takes: nought is barren.
    pub grass: Vec4,
    /// The field's own rectangle, so a surface that wears unevenly knows which
    /// way it runs: a road's ruts follow its length.
    pub extent: Vec4,
    /// How the wheels have worn it: half the gauge between the ruts, half the
    /// width of one rut, how bare the rut is, and how bare the rest of it is.
    /// All nought and the surface wears evenly, as a field does.
    pub tread: Vec4,
}

/// The wear of a road: two ruts a tractor's gauge apart, bare where the wheels
/// run and progressively less so between them. `worn` dials the whole thing —
/// nought for a green lane barely driven, one for a road bare across its width.
fn tread_of(bounds: crate::layout::FieldBounds, worn: f32) -> (Vec4, Vec4) {
    if worn <= 0.0 {
        return (Vec4::ZERO, Vec4::ZERO);
    }
    let extent = Vec4::new(bounds.min.x, bounds.min.y, bounds.max.x, bounds.max.y);
    // Half a tractor's gauge, and a rut a little wider than the tyre that cut
    // it. Past halfway the ruts have spread far enough to meet in the middle.
    let tread = Vec4::new(0.9, 0.46, (0.55 + worn * 0.45).min(1.0), (worn - 0.35).max(0.0) / 0.65);
    (extent, tread)
}

impl MaterialExtension for BareExtension {
    fn fragment_shader() -> ShaderRef {
        "embedded://gearbox_fields/bare/shaders/material.wgsl".into()
    }
}

pub(super) struct BarePlugin;

impl Plugin for BarePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "shaders/material.wgsl");
        bevy::asset::embedded_asset!(app, "shaders/vegetation.wgsl");
        app.add_plugins(MaterialPlugin::<BareMaterial>::default());
        // A worn track recovers slowly and marks deeply; sand holds a wheel
        // mark just as long but hardly darkens where it has been pressed.
        let response = WheelResponse {
            recovery_seconds: 900.0,
            bend: 0.9,
            darkening: 0.3,
            footprint_length: 0.3,
            tread: true,
        };
        let mut profiles = app.world_mut().resource_mut::<FieldProfiles>();
        profiles.register(FieldProfile {
            name: "ploughed",
            wheel_response: response,
            layers: standing(30.0, 2400.0, 440.0, 3.0, turned_clod),
            ground: ploughed_ground,
            tread: Vec4::ZERO,
            soft_border: 0.8,
        });
        profiles.register(FieldProfile {
            name: "dirt",
            wheel_response: response,
            layers: standing(130.0, 1600.0, 1900.0, 6.0, worn_clod),
            ground: dirt_ground,
            tread: Vec4::ZERO,
            soft_border: 0.8,
        });
        for (name, ground, worn) in [
            ("green-lane", green_lane as GroundFactory, 0.3),
            ("track", worn_track as GroundFactory, 0.6),
            ("road", bare_road as GroundFactory, 1.0),
        ] {
            profiles.register(FieldProfile {
                name,
                wheel_response: response,
                layers: standing(90.0, 900.0, 1400.0, 5.0, worn_clod),
                ground,
                tread: tread_of(crate::layout::FieldBounds { min: Vec2::ZERO, max: Vec2::ZERO }, worn).1,
                soft_border: 0.8,
            });
        }
        profiles.register(FieldProfile {
            name: "sand",
            wheel_response: WheelResponse { darkening: 0.16, ..response },
            layers: standing(22.0, 120.0, 0.0, 0.8, blown_clod),
            ground: sand_ground,
            tread: Vec4::ZERO,
            soft_border: 0.8,
        });
    }
}

/// Turned earth: dark, coarse and clodded, with the plough's own combing.
fn ploughed_ground(
    world: &mut World,
    trample: Handle<Image>,
    trample_params: WheelMapParams,
    geometry: SurfaceGeometry,
    bounds: crate::layout::FieldBounds,
) -> Arc<dyn GroundSurface> {
    ground(
        world,
        trample,
        trample_params,
        geometry,
        bounds,
        BareGround {
            tint: Vec4::new(0.060, 0.034, 0.018, 1.0),
            grain: Vec4::new(0.34, 1.0, 1.0, 1.25),
            grass: Vec4::new(0.033, 0.068, 0.023, 0.55),
            extent: Vec4::ZERO,
            tread: Vec4::ZERO,
        },
        0.93,
    )
}

/// A worn track or yard: paler, packed flat, barely combed.
fn dirt_ground(
    world: &mut World,
    trample: Handle<Image>,
    trample_params: WheelMapParams,
    geometry: SurfaceGeometry,
    bounds: crate::layout::FieldBounds,
) -> Arc<dyn GroundSurface> {
    ground(
        world,
        trample,
        trample_params,
        geometry,
        bounds,
        BareGround {
            tint: Vec4::new(0.115, 0.070, 0.038, 1.0),
            grain: Vec4::new(0.3, 0.40, 0.0, 1.6),
            grass: Vec4::new(0.033, 0.068, 0.023, 0.9),
            extent: Vec4::ZERO,
            tread: Vec4::ZERO,
        },
        0.88,
    )
}

/// A way across a field, worn by whatever drives it. How hard is the whole
/// dial: a green lane barely marked, two bare ruts with grass still holding
/// between them, or a road worn bare from side to side.
fn green_lane(w: &mut World, t: Handle<Image>, p: WheelMapParams,
    g: SurfaceGeometry, b: crate::layout::FieldBounds) -> Arc<dyn GroundSurface> {
    track(w, t, p, g, b, 0.3)
}

fn worn_track(w: &mut World, t: Handle<Image>, p: WheelMapParams,
    g: SurfaceGeometry, b: crate::layout::FieldBounds) -> Arc<dyn GroundSurface> {
    track(w, t, p, g, b, 0.6)
}

fn bare_road(w: &mut World, t: Handle<Image>, p: WheelMapParams,
    g: SurfaceGeometry, b: crate::layout::FieldBounds) -> Arc<dyn GroundSurface> {
    track(w, t, p, g, b, 1.0)
}

fn track(
    world: &mut World,
    trample: Handle<Image>,
    trample_params: WheelMapParams,
    geometry: SurfaceGeometry,
    bounds: crate::layout::FieldBounds,
    worn: f32,
) -> Arc<dyn GroundSurface> {
    let (extent, tread) = tread_of(bounds, worn);
    ground(
        world,
        trample,
        trample_params,
        geometry,
        bounds,
        BareGround {
            tint: Vec4::new(0.108, 0.068, 0.038, 1.0),
            grain: Vec4::new(0.28, 0.42, 0.0, 1.6),
            grass: Vec4::new(0.033, 0.068, 0.023, 1.0),
            extent,
            tread,
        },
        0.9,
    )
}

/// Sand: pale, fine-grained, and combed into ripples by the wind.
fn sand_ground(
    world: &mut World,
    trample: Handle<Image>,
    trample_params: WheelMapParams,
    geometry: SurfaceGeometry,
    bounds: crate::layout::FieldBounds,
) -> Arc<dyn GroundSurface> {
    ground(
        world,
        trample,
        trample_params,
        geometry,
        bounds,
        BareGround {
            tint: Vec4::new(0.245, 0.182, 0.098, 1.0),
            grain: Vec4::new(0.16, 0.30, 1.0, 0.22),
            grass: Vec4::new(0.06, 0.08, 0.03, 0.0),
            extent: Vec4::ZERO,
            tread: Vec4::ZERO,
        },
        0.82,
    )
}

fn ground(
    world: &mut World,
    trample: Handle<Image>,
    trample_params: WheelMapParams,
    geometry: SurfaceGeometry,
    _bounds: crate::layout::FieldBounds,
    bare: BareGround,
    roughness: f32,
) -> Arc<dyn GroundSurface> {
    let extension = BareExtension {
        trample,
        trample_params,
        heightmap: geometry.heightmap,
        geometry: geometry.params,
        ground: bare,
    };
    let material = world
        .resource_mut::<Assets<BareMaterial>>()
        .add(ExtendedMaterial {
            base: StandardMaterial {
                perceptual_roughness: roughness,
                metallic: 0.0,
                ..default()
            },
            extension,
        });
    Arc::new(MaterialSurface(material))
}
