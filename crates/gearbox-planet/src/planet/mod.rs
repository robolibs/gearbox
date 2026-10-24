//! CDLOD cube-sphere planet terrain.

pub mod bake;
pub mod cube_sphere;
pub mod material;
pub mod mesh;
pub mod quadtree;

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::light::{atmosphere::ScatteringMedium, Atmosphere};
use bevy::mesh::MeshTag;
use bevy::prelude::*;
use bevy::math::DVec3;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::storage::ShaderBuffer;
use big_space::prelude::*;

use crate::config::*;
use material::{
    TerrainExtension, TerrainGlobals, TerrainMaterial, TerrainNodeGpu, WaterExtension,
    WaterMaterial,
};
use quadtree::{PlanetRenderer, SlotMap, TerrainStats, TileCache};

/// The big space the planet and the camera share. Upstream makes its own;
/// here it is whatever grid gearbox already keeps its sites in, and the host
/// inserts this to say which.
#[derive(Resource, Clone, Copy)]
pub struct RootGrid(pub Entity);

/// Planet entities in spawn order (index 0 is the primary world).
#[derive(Resource, Default)]
pub struct Planets(pub Vec<Entity>);

/// Where the camera is, in the root grid's frame, when the host works it out
/// itself. A camera nested in a child grid cannot be read against the root
/// grid's own cells — the cell sizes differ — so the host that owns those
/// grids hands the answer over rather than letting the selector guess.
#[derive(Resource, Clone, Copy, Debug)]
pub struct CameraAt {
    pub position: bevy::math::DVec3,
    /// Which way it looks, in that same frame. The frustum needs this as much
    /// as the position does: given the place but not the heading, the frustum
    /// sits right and points wrong, and tiles are culled in wedges.
    pub rotation: Quat,
}

/// A zero-density `ScatteringMedium`: swapped into a planet's atmosphere while
/// the camera is underwater there (the sky pass would otherwise pin the view
/// to the surface and wash a blue haze over the underwater fog). Swapping the
/// medium handle is free; toggling `AtmosphereSettings` on the camera would
/// recompile every mesh pipeline.
#[derive(Resource, Clone)]
pub struct VacuumMedium(pub Handle<ScatteringMedium>);

/// Cell size of the root grid. Rendering magnitudes near the camera are
/// roughly one cell plus the planet radius, so ~10 km keeps f32 precision at
/// millimetres while cell crossings stay rare.
pub const CELL_EDGE: f32 = 10_000.0;
pub const CELL_HYSTERESIS: f32 = 1_000.0;

/// How the camera sits relative to one planet. All positions are derived from
/// f64 grid positions, so this stays exact across interplanetary distances.
pub struct PlanetView {
    pub entity: Entity,
    /// Camera position expressed in the planet's local frame (spin undone).
    pub local_cam: Vec3,
    /// World-space unit vector from the planet's center to the camera.
    pub up: Vec3,
    pub radius: f32,
    /// Distance from the planet center to the camera.
    pub dist: f32,
}

impl PlanetView {
    /// Height above sea level (negative when submerged).
    pub fn altitude(&self) -> f32 {
        self.dist - self.radius
    }
}

/// Mie phase function of Martian dust per colour channel, 1024 columns from
/// back- to forward-scattering (see `textures/README.md`).
const DUST_PHASE_KTX2: &[u8] = include_bytes!("../../textures/mars_mie_phase.ktx2");

/// The dust phase as a curve. Bevy resolves a phase texture only when that
/// image finishes loading, so a medium made after it would never resolve;
/// decoded here, every medium gets it at once.
fn dust_phase() -> bevy::light::atmosphere::PhaseFunction {
    use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
    use bevy::math::curve::{Interval, SampleAutoCurve};
    let image = Image::from_buffer(
        DUST_PHASE_KTX2,
        ImageType::Extension("ktx2"),
        CompressedImageFormats::NONE,
        false,
        ImageSampler::Default,
        RenderAssetUsages::MAIN_WORLD,
    )
    .expect("embedded Mars dust phase decodes");
    let samples: Vec<LinearRgba> = (0..image.texture_descriptor.size.width)
        .map(|x| image.get_color_at_1d(x).expect("phase column").to_linear())
        .collect();
    let curve = SampleAutoCurve::new(Interval::new(-1.0, 1.0).expect("cos θ"), samples)
        .expect("phase samples");
    bevy::light::atmosphere::PhaseFunction::from_chromatic_curve(curve)
}

/// Martian air at true scale (Schneegans et al. 2024), its dust thickened
/// from the preset's optical depth to `dust_tau`. Only the dust changes: the
/// carbon dioxide is the same on a clear day and in a storm.
pub fn martian_air(dust_tau: f32) -> ScatteringMedium {
    let mut medium = ScatteringMedium::mars(256, 256, Handle::default());
    medium.terms[1].phase = dust_phase();
    set_dust(&mut medium, MARS_PRESET_DUST_TAU, dust_tau);
    medium
}

/// Rescales a Martian medium's dust from optical depth `from` to `to`.
pub fn set_dust(medium: &mut ScatteringMedium, from: f32, to: f32) {
    let thicken = to.max(0.0) / from.max(1e-6);
    if let Some(dust) = medium.terms.get_mut(1) {
        dust.scattering *= thicken;
        dust.absorption *= thicken;
    }
}

/// Relates the camera to one planet, in that planet's local frame.
pub fn planet_view(
    grid: &Grid,
    cam_pos: DVec3,
    entity: Entity,
    cfg: &PlanetConfig,
    cell: &CellCoord,
    tf: &Transform,
) -> PlanetView {
    let center = grid.grid_position_double(cell, tf);
    // The difference is small wherever it matters (near a planet), so f32 is
    // exact there; far away it only feeds coarse LOD/nearest tests.
    let delta = (cam_pos - center).as_vec3();
    PlanetView {
        entity,
        local_cam: tf.rotation.inverse() * delta,
        up: delta.normalize_or_zero(),
        radius: cfg.radius,
        dist: delta.length(),
    }
}

/// Picks the planet whose *surface* is closest to the camera.
pub fn nearest_planet<'a>(
    grid: &Grid,
    cam_pos: DVec3,
    planets: impl Iterator<Item = (Entity, &'a PlanetConfig, &'a CellCoord, &'a Transform)>,
) -> Option<PlanetView> {
    planets
        .map(|(entity, cfg, cell, tf)| planet_view(grid, cam_pos, entity, cfg, cell, tf))
        .min_by(|a, b| a.altitude().abs().total_cmp(&b.altitude().abs()))
}

/// Spawns one planet: config + renderer state + its two pools of tile
/// entities (children, so they inherit the planet's transform).
#[allow(clippy::too_many_arguments)]
pub fn spawn_planet(
    commands: &mut Commands,
    root_grid: Entity,
    cfg: PlanetConfig,
    // Position in the root grid, in metres from the big space's origin.
    position: DVec3,
    grid: &Handle<Mesh>,
    images: &mut Assets<Image>,
    buffers: &mut Assets<ShaderBuffer>,
    materials: &mut Assets<TerrainMaterial>,
    water_materials: &mut Assets<WaterMaterial>,
    media: &mut Assets<ScatteringMedium>,
) -> Entity {
    // Heightmap atlas: storage-written by the bake compute pass, sampled by the
    // terrain vertex/fragment shaders.
    let mut atlas = Image::new_fill(
        Extent3d {
            width: TILE_TEXELS,
            height: TILE_TEXELS,
            depth_or_array_layers: cfg.atlas_layers,
        },
        TextureDimension::D2,
        &0.0f32.to_le_bytes(),
        TextureFormat::R32Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    atlas.texture_descriptor.usage =
        TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING;
    let atlas = images.add(atlas);

    let node_buffer = buffers.add(ShaderBuffer::from(
        vec![TerrainNodeGpu::default(); MAX_VISIBLE].as_slice(),
    ));

    let material = materials.add(TerrainMaterial {
        base: StandardMaterial {
            perceptual_roughness: 0.95,
            metallic: 0.0,
            fog_enabled: cfg.surface_kind == 0,
            ..default()
        },
        extension: TerrainExtension {
            globals: TerrainGlobals {
                radius: cfg.radius,
                height_amp: cfg.height_amp,
                surface_kind: cfg.surface_kind,
                ..default()
            },
            heightmaps: atlas.clone(),
            nodes: node_buffer.clone(),
        },
    });

    let water_material = water_materials.add(WaterMaterial {
        base: StandardMaterial {
            alpha_mode: AlphaMode::Blend,
            double_sided: true,
            // Above water: back faces culled so a near-limb ray can't hit the
            // ocean sphere twice (two same-sort-key translucent layers blend
            // in arbitrary, frame-unstable order = tile flicker). The
            // underwater system flips this to Front when submerged.
            cull_mode: Some(bevy::render::render_resource::Face::Back),
            perceptual_roughness: 0.12,
            metallic: 0.0,
            ..default()
        },
        extension: WaterExtension {
            globals: TerrainGlobals {
                radius: cfg.radius,
                height_amp: cfg.height_amp,
                surface_kind: cfg.surface_kind,
                ..default()
            },
            heightmaps: atlas.clone(),
            nodes: node_buffer.clone(),
        },
    });

    // Tile pools are CHILDREN of the planet entity: their identity local
    // transforms inherit the planet's placement/rotation, so every shader stays
    // in planet-local space and `world_from_local` does the placement.
    //
    // NoFrustumCulling: the entities share one flat grid mesh whose real
    // world-space extent only exists after vertex-shader displacement, and the
    // GPU-preprocessing cull path uses mesh bounds (not per-entity Aabb), so
    // engine culling would cull by the wrong box. Culling is done instead in
    // quadtree selection (node bounding sphere vs camera frustum).
    let reach = cfg.radius + cfg.height_amp * 2.0;
    let big_aabb = move || {
        bevy::camera::primitives::Aabb::from_min_max(Vec3::splat(-reach), Vec3::splat(reach))
    };

    // High-precision placement: cell + small in-cell offset. The planet is a
    // spatial child of the root grid; its tile pools are ordinary
    // low-precision children of the planet.
    let (cell, offset) = Grid::new(CELL_EDGE, CELL_HYSTERESIS).translation_to_grid(position);

    // Atmosphere: a SEPARATE non-spinning anchor at the same position. The
    // atmosphere shader mixes atmosphere-space positions with world-space ray
    // directions, so its entity must never rotate — and a spherically
    // symmetric atmosphere doesn't care about the planet's spin anyway.
    // Bevy picks whichever Atmosphere entity is nearest the camera, so one
    // per planet gives correct per-world skies for free.
    let atmosphere = cfg.atmosphere.map(|air| {
        let shell = cfg.atmosphere_shell();
        let (medium, ground_albedo) = match air {
            Air::Earthlike => (
                ScatteringMedium::earth(256, 256).with_density_multiplier(cfg.atmosphere_density()),
                0.3,
            ),
            Air::Martian { dust_tau } => (martian_air(dust_tau), MARS_GROUND_ALBEDO),
        };
        let medium = media.add(medium);
        let anchor = commands
            .spawn((
                Atmosphere {
                    inner_radius: cfg.radius,
                    outer_radius: cfg.radius + shell,
                    ground_albedo: Vec3::splat(ground_albedo),
                    medium: medium.clone(),
                },
                cell,
                Transform::from_translation(offset),
                ChildOf(root_grid),
            ))
            .id();
        (anchor, medium)
    });
    let planet = commands
        .spawn((
            cfg.clone(),
            material::TerrainCoverage::default(),
            cell,
            Transform::from_translation(offset),
            Visibility::Visible,
            ChildOf(root_grid),
        ))
        .id();

    let mut terrain_pool = Vec::with_capacity(MAX_VISIBLE);
    let mut water_pool = Vec::with_capacity(MAX_VISIBLE);
    commands.entity(planet).with_children(|tiles| {
        for slot in 0..MAX_VISIBLE {
            terrain_pool.push(
                tiles
                    .spawn((
                        Mesh3d(grid.clone()),
                        MeshMaterial3d(material.clone()),
                        MeshTag(slot as u32),
                        Transform::IDENTITY,
                        Visibility::Hidden,
                        bevy::camera::visibility::NoFrustumCulling,
                        big_aabb(),
                    ))
                    .id(),
            );
        }
        // Water tiles mirror the terrain pool slot-for-slot: same grid mesh and
        // per-node records, translucent sea-level surface material. Permanently
        // Visible — unused slots have degenerate (scale 0) records instead, so
        // the retained transparent phase never sees item churn (churn = flicker).
        for slot in 0..MAX_VISIBLE {
            water_pool.push(
                tiles
                    .spawn((
                        Mesh3d(grid.clone()),
                        MeshMaterial3d(water_material.clone()),
                        MeshTag(slot as u32),
                        Transform::IDENTITY,
                        // The ground is one flat surface at sea level, so a sea
                        // drawn at that level is a sphere laid exactly over the
                        // whole planet — which is what covered it, not the
                        // terrain. Nothing is below water, so nothing is sea.
                        Visibility::Hidden,
                        bevy::camera::visibility::NoFrustumCulling,
                        bevy::light::NotShadowCaster,
                        big_aabb(),
                    ))
                    .id(),
            );
        }
    });

    commands.entity(planet).insert(PlanetRenderer {
        cache: TileCache::new(cfg.atlas_layers),
        slots: SlotMap::default(),
        stats: TerrainStats::default(),
        material,
        water_material,
        node_buffer,
        atlas,
        terrain_pool,
        water_pool,
        last_flags: None,
        active: true,
        atmosphere,
    });
    planet
}

#[cfg(test)]
mod martian_air_tests {
    use super::*;

    /// Bevy's Mars dust profile (`ScatteringMedium::mars`, Fig. 8 of
    /// Schneegans et al.), over height in metres.
    fn dust_density(h: f64) -> f64 {
        0.75 * (1.0 - (h / 4_000.0).exp()).exp() + 0.25 * (1.0 - (h / 20_000.0).exp()).exp()
    }

    #[test]
    fn the_preset_dust_has_the_optical_depth_it_is_scaled_from() {
        let column: f64 = (0..120_000).map(|h| dust_density(h as f64 + 0.5)).sum();
        let medium = ScatteringMedium::mars(16, 16, Handle::default());
        let dust = &medium.terms[1];
        let extinction = (dust.scattering.y + dust.absorption.y) as f64;
        let tau = column * extinction;
        assert!((tau - MARS_PRESET_DUST_TAU as f64).abs() < 0.002, "preset dust tau {tau}");
    }

    #[test]
    fn thickening_the_dust_leaves_the_carbon_dioxide_alone() {
        let preset = ScatteringMedium::mars(16, 16, Handle::default());
        let storm = martian_air(4.0 * MARS_PRESET_DUST_TAU);
        assert_eq!(storm.terms[0].scattering, preset.terms[0].scattering);
        assert!((storm.terms[1].scattering - preset.terms[1].scattering * 4.0).length() < 1e-9);
        assert!((storm.terms[1].absorption - preset.terms[1].absorption * 4.0).length() < 1e-9);
        let (forward, back) = (storm.terms[1].phase.sample(1.0).unwrap(), storm.terms[1].phase.sample(-1.0).unwrap());
        assert!(forward.green > 20.0 * back.green, "the forward peak is resolved");
        assert!(forward.blue > forward.red, "blue scatters most strongly forward: the blue aureole of a Martian sunset");
        let mut calm = storm;
        set_dust(&mut calm, 4.0 * MARS_PRESET_DUST_TAU, MARS_PRESET_DUST_TAU);
        assert!((calm.terms[1].scattering - preset.terms[1].scattering).length() < 1e-9);
    }
}
