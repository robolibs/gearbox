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
            ..default()
        },
        extension: TerrainExtension {
            globals: TerrainGlobals {
                radius: cfg.radius,
                height_amp: cfg.height_amp,
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
    let atmosphere = cfg.atmosphere.map(|rayleigh| {
        let shell = cfg.atmosphere_shell();
        let mut medium = ScatteringMedium::earth(256, 256);
        medium.terms[0].scattering = rayleigh;
        let medium = media.add(medium.with_density_multiplier(cfg.atmosphere_density()));
        let anchor = commands
            .spawn((
                Atmosphere {
                    inner_radius: cfg.radius,
                    outer_radius: cfg.radius + shell,
                    ground_albedo: Vec3::splat(0.3),
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
                        Visibility::Visible,
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
