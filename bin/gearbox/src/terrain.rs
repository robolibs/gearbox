//! Procedural ground: a heightfield mesh under an anti-repeat grass
//! material, the matching Rapier heightfield collider, and GPU-instanced
//! blade chunks that follow the camera. Replaces the flat startup ground
//! unless `GEARBOX_TERRAIN=flat`; a loaded USD terrain retires it.

use std::sync::{Arc, RwLock};

use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::{NoFrustumCulling, VisibilitySystems};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin, StandardMaterial};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Extent3d, TextureDimension, TextureFormat};
use bevy::shader::ShaderRef;
use bevy::transform::TransformSystems;
use rapier3d::math::Vector as DVec3;
use rapier3d::prelude::{ColliderBuilder, ColliderHandle};

use crate::grass::{DETAIL_FADE_END_M, DETAIL_FADE_START_M, GrassChunkDraw, GrassField, GrassParams};
use bevy::asset::RenderAssetUsages as Usages;
use crate::physics::PhysicsWorld;
use crate::world::{
    FlatGround, TerrainCollision, asset_path, fbm_world, remove_flat_ground, smooth_hill,
};

const SIZE_M: f32 = 800.0;
/// Height grid, collider and near surface tiles; `GEARBOX_TERRAIN_CELL_M`
/// overrides it, `GEARBOX_TERRAIN_COLLIDER_CELL_M` the collider alone.
const CELL_M: f32 = 1.0;
const TILE_M: f32 = 50.0;
/// Tiles farther than this from the camera drop to the coarse cell.
const TILE_FINE_RADIUS_M: f32 = 180.0;
const TILE_COARSE_CELL_M: f32 = 4.0;
const TILE_SKIRT_M: f32 = 2.0;
const TILE_REBUILDS_PER_FRAME: usize = 2;
const SPAWN_FLAT_RADIUS_M: f32 = 30.0;
const SPAWN_RELIEF_RADIUS_M: f32 = 70.0;
const SAFETY_FLOOR_Y_M: f64 = -40.0;
const SAFETY_FLOOR_HALF_EXTENT_M: f64 = 10_000.0;
const HORIZON_COLOR: Color = Color::srgb(0.46, 0.55, 0.30);

const CHUNK_M: f32 = 16.0;
/// Blades per square metre next to the camera; `GEARBOX_GRASS_DENSITY`
/// overrides it, 0 turns the grass off.
const GRASS_DENSITY_PER_M2: f32 = 6000.0;
/// Full density holds to the first distance and is gone at the second.
const GRASS_FADE_START_M: f32 = 16.0;
const GRASS_FADE_END_M: f32 = 60.0;
/// Blade template segments by chunk distance; centimetre blades are one
/// triangle at any range.
const GRASS_LODS: [(f32, u32); 1] = [(f32::INFINITY, 1)];
const BLADE_MAX_HEIGHT_M: f32 = 0.12;
/// Trample map resolution: 12.5 cm texels, a tractor tyre is four wide.
const TRAMPLE_TEXELS_PER_M: f32 = 8.0;

static HEIGHT_GRID: RwLock<Option<Arc<HeightGrid>>> = RwLock::new(None);

pub struct TerrainPlugin;

impl Plugin for TerrainPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "../assets/shaders/meadow_palette.wgsl");
        bevy::asset::embedded_asset!(app, "../assets/shaders/meadow_material.wgsl");
        app.add_plugins(MaterialPlugin::<MeadowMaterial>::default())
            .init_resource::<GrassChunks>()
            .add_systems(PostStartup, spawn_procedural_terrain)
            .add_systems(
                Update,
                (retire_for_usd_terrain, update_terrain_tiles).chain(),
            )
            .add_systems(
                PostUpdate,
                update_grass_chunks
                    .after(TransformSystems::Propagate)
                    .before(VisibilitySystems::VisibilityPropagate),
            );
    }
}

/// `GEARBOX_TERRAIN=flat|meadow`; the meadow is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerrainPreset {
    Flat,
    Meadow,
}

impl TerrainPreset {
    fn from_env() -> Self {
        match std::env::var("GEARBOX_TERRAIN").as_deref() {
            Ok("flat") => Self::Flat,
            _ => Self::Meadow,
        }
    }
}

/// Regular height samples over a square centred on the origin.
pub struct HeightGrid {
    min_x: f32,
    min_z: f32,
    cell: f32,
    cols: usize,
    rows: usize,
    heights: Vec<f32>,
}

impl HeightGrid {
    fn sample(size: f32, cell: f32, height: impl Fn(f32, f32) -> f32) -> Self {
        let cols = (size / cell).round() as usize + 1;
        let rows = cols;
        let min_x = -size * 0.5;
        let min_z = -size * 0.5;
        let mut heights = Vec::with_capacity(cols * rows);
        for i in 0..rows {
            let z = min_z + i as f32 * cell;
            for j in 0..cols {
                let x = min_x + j as f32 * cell;
                heights.push(height(x, z));
            }
        }
        Self { min_x, min_z, cell, cols, rows, heights }
    }

    fn at(&self, i: usize, j: usize) -> f32 {
        self.heights[i * self.cols + j]
    }

    /// Bilinear height, `None` outside the grid.
    pub fn height_at(&self, x: f32, z: f32) -> Option<f32> {
        let fx = (x - self.min_x) / self.cell;
        let fz = (z - self.min_z) / self.cell;
        if fx < 0.0 || fz < 0.0 {
            return None;
        }
        let j = (fx.floor() as usize).min(self.cols - 2);
        let i = (fz.floor() as usize).min(self.rows - 2);
        if fx > (self.cols - 1) as f32 || fz > (self.rows - 1) as f32 {
            return None;
        }
        let tx = (fx - j as f32).clamp(0.0, 1.0);
        let tz = (fz - i as f32).clamp(0.0, 1.0);
        let h00 = self.at(i, j);
        let h10 = self.at(i, j + 1);
        let h01 = self.at(i + 1, j);
        let h11 = self.at(i + 1, j + 1);
        let h0 = h00 + (h10 - h00) * tx;
        let h1 = h01 + (h11 - h01) * tx;
        Some(h0 + (h1 - h0) * tz)
    }

    fn normal_at(&self, x: f32, z: f32) -> Vec3 {
        let d = self.cell;
        let h = |x: f32, z: f32| self.height_at(x, z).unwrap_or(0.0);
        let dx = (h(x + d, z) - h(x - d, z)) / (2.0 * d);
        let dz = (h(x, z + d) - h(x, z - d)) / (2.0 * d);
        Vec3::new(-dx, 1.0, -dz).normalize()
    }

    fn normal_at_index(&self, i: usize, j: usize) -> Vec3 {
        let jl = j.saturating_sub(1);
        let jr = (j + 1).min(self.cols - 1);
        let iu = i.saturating_sub(1);
        let id = (i + 1).min(self.rows - 1);
        let dx = (self.at(i, jr) - self.at(i, jl)) / ((jr - jl) as f32 * self.cell);
        let dz = (self.at(id, j) - self.at(iu, j)) / ((id - iu) as f32 * self.cell);
        Vec3::new(-dx, 1.0, -dz).normalize()
    }

    fn half_size(&self) -> f32 {
        (self.cols - 1) as f32 * self.cell * 0.5
    }
}

/// Height of the procedural ground, `None` when none is active or outside it.
pub fn procedural_height_m(x: f32, z: f32) -> Option<f32> {
    let grid = HEIGHT_GRID.read().ok()?.clone()?;
    grid.height_at(x, z)
}

fn meadow_height_raw(x: f32, z: f32) -> f32 {
    let rolling = (fbm_world(x * 0.0045 + 3.1, z * 0.0045 - 7.7, 5) - 0.5) * 7.0;
    let ripples = (fbm_world(x * 0.035 - 11.0, z * 0.035 + 5.0, 3) - 0.5) * 0.5;
    let hills = smooth_hill(x, z, 140.0, 90.0, 70.0, 12.0)
        + smooth_hill(x, z, -170.0, 120.0, 90.0, 9.0)
        + smooth_hill(x, z, 60.0, -210.0, 110.0, 14.0)
        + smooth_hill(x, z, -230.0, -160.0, 80.0, 7.0)
        + smooth_hill(x, z, 280.0, -60.0, 60.0, -5.0)
        + smooth_hill(x, z, -40.0, 300.0, 120.0, 10.0);
    rolling + ripples + hills
}

/// The raw relief flattened to y=0 around the spawn point.
fn meadow_height(x: f32, z: f32) -> f32 {
    let origin = meadow_height_raw(0.0, 0.0);
    let distance = (x * x + z * z).sqrt();
    let t = ((distance - SPAWN_FLAT_RADIUS_M) / (SPAWN_RELIEF_RADIUS_M - SPAWN_FLAT_RADIUS_M))
        .clamp(0.0, 1.0);
    let fade = t * t * (3.0 - 2.0 * t);
    (meadow_height_raw(x, z) - origin) * fade
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
struct MeadowExtension {
    #[texture(100)]
    #[sampler(101)]
    grass_albedo: Handle<Image>,
    #[texture(102)]
    #[sampler(103)]
    dirt_albedo: Handle<Image>,
    #[texture(104, sample_type = "u_int")]
    trample: Handle<Image>,
    /// origin x, origin z, texels per metre, texel count of the trample map.
    #[uniform(105)]
    trample_params: Vec4,
}

impl MaterialExtension for MeadowExtension {
    fn fragment_shader() -> ShaderRef {
        "embedded://gearbox_sim/../assets/shaders/meadow_material.wgsl".into()
    }
}

type MeadowMaterial = ExtendedMaterial<StandardMaterial, MeadowExtension>;

/// The active procedural ground and everything that must go with it.
#[derive(Resource)]
pub struct ProceduralTerrain {
    entity: Entity,
    collider: ColliderHandle,
    safety_floor: ColliderHandle,
    grid: Arc<HeightGrid>,
    material: Handle<MeadowMaterial>,
    fine_cell: f32,
    tiles: HashMap<(i32, i32), (Entity, f32)>,
}

#[derive(Component)]
struct TerrainTile;

/// Every chunk owns its own blade mesh so the renderer never batches two
/// chunks into one draw.
#[derive(Resource, Default)]
struct GrassChunks {
    chunks: HashMap<(i32, i32), (Entity, usize)>,
    blades_per_chunk: f32,
    details: HashMap<(i32, i32), Entity>,
}

#[derive(Component)]
struct GrassChunk;

fn spawn_procedural_terrain(
    mut commands: Commands,
    mut materials: ResMut<Assets<MeadowMaterial>>,
    mut physics: ResMut<PhysicsWorld>,
    asset_server: Res<AssetServer>,
    flat: Option<Res<FlatGround>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    horizon: Query<(&Name, &MeshMaterial3d<StandardMaterial>)>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut chunks: ResMut<GrassChunks>,
) {
    let preset = TerrainPreset::from_env();
    if preset == TerrainPreset::Flat {
        info!("terrain: GEARBOX_TERRAIN=flat, keeping the flat ground");
        return;
    }
    let started = std::time::Instant::now();
    let trample_count = (SIZE_M * TRAMPLE_TEXELS_PER_M) as u32 + 1;
    let trample = images.add(Image::new_fill(
        Extent3d { width: trample_count, height: trample_count, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &[0u8; 4],
        TextureFormat::Rg16Uint,
        Usages::RENDER_WORLD,
    ));
    let cell = std::env::var("GEARBOX_TERRAIN_CELL_M")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| *value >= 0.5)
        .unwrap_or(CELL_M);
    let collider_cell = std::env::var("GEARBOX_TERRAIN_COLLIDER_CELL_M")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| *value >= 0.5)
        .unwrap_or(cell);
    let grid = Arc::new(HeightGrid::sample(SIZE_M, cell, meadow_height));
    let collider_grid = HeightGrid::sample(SIZE_M, collider_cell, meadow_height);
    let material = materials.add(ExtendedMaterial {
        base: StandardMaterial {
            perceptual_roughness: 0.95,
            metallic: 0.0,
            ..default()
        },
        extension: MeadowExtension {
            grass_albedo: asset_server.load(asset_path(
                "textures/terrain/Grass005/Grass005_1K-JPG_Color.jpg",
            )),
            dirt_albedo: asset_server.load(asset_path(
                "textures/terrain/Ground001/Ground001_1K-JPG_Color.jpg",
            )),
            trample: trample.clone(),
            trample_params: Vec4::new(
                -SIZE_M * 0.5,
                -SIZE_M * 0.5,
                TRAMPLE_TEXELS_PER_M,
                trample_count as f32,
            ),
        },
    });
    let entity = commands
        .spawn((Name::new("MeadowTerrain"), Transform::IDENTITY, Visibility::default()))
        .id();
    commands.spawn((
        Name::new("Distant meadow landscape"),
        ChildOf(entity),
        Transform::IDENTITY,
        Mesh3d(meshes.add(horizon_mesh())),
        MeshMaterial3d(material.clone()),
        bevy::light::NotShadowCaster,
    ));

    let collider = physics
        .colliders
        .insert(heightfield_collider(&collider_grid).friction(crate::world::ground_friction(1.4)).restitution(0.0).build());
    physics.entity_to_collider.insert(entity, collider);
    let safety_floor = physics.colliders.insert(
        ColliderBuilder::cuboid(SAFETY_FLOOR_HALF_EXTENT_M, 0.10, SAFETY_FLOOR_HALF_EXTENT_M)
            .translation(DVec3::new(0.0, SAFETY_FLOOR_Y_M, 0.0))
            .friction(1.2)
            .restitution(0.0)
            .build(),
    );
    if let Some(flat) = flat.as_deref().copied() {
        remove_flat_ground(&mut commands, physics.as_mut(), flat);
        commands.remove_resource::<FlatGround>();
    }
    if let Ok(mut slot) = HEIGHT_GRID.write() {
        *slot = Some(grid.clone());
    }
    // The planet sphere fills the horizon past the terrain edge.
    for (name, material) in &horizon {
        if name.as_str() == "Planet"
            && let Some(mut material) = standard.get_mut(&material.0)
        {
            material.base_color = HORIZON_COLOR;
        }
    }
    let density = grass_density_from_env();
    chunks.blades_per_chunk = CHUNK_M * CHUNK_M * density;
    if density > 0.0 {
        commands.insert_resource(GrassField {
            heightmap: images.add(heightmap_image(&grid)),
            trample,
            params: GrassParams {
                corner: Vec2::ZERO,
                origin: Vec2::new(grid.min_x, grid.min_z),
                texels_per_metre: 1.0 / grid.cell,
                chunk_size: CHUNK_M,
                fade_start: GRASS_FADE_START_M,
                fade_end: GRASS_FADE_END_M,
                blades_per_chunk: chunks.blades_per_chunk,
                texel_count: grid.cols as f32,
                trample_texels_per_metre: TRAMPLE_TEXELS_PER_M,
                trample_texel_count: trample_count as f32,
            },
        });
    }
    commands.insert_resource(ProceduralTerrain {
        entity,
        collider,
        safety_floor,
        grid,
        material,
        fine_cell: cell,
        tiles: HashMap::default(),
    });
    info!(
        "terrain: {preset:?} ground ready, {} m at {} m cells, in {:?}",
        SIZE_M,
        cell,
        started.elapsed()
    );
}

fn horizon_height(x: f32, z: f32) -> f32 {
    let edge = x.abs().max(z.abs());
    let t = ((edge - SIZE_M * 0.5) / 1_600.0).clamp(0.0, 1.0);
    let relief = (fbm_world(x * 0.00045 + 41.0, z * 0.00045 - 13.0, 4) - 0.5) * 220.0;
    meadow_height(x, z) + relief * t * t * (3.0 - 2.0 * t)
}

/// Concentric terrain rings extend the meadow to a 64 km visual backdrop.
fn horizon_mesh() -> Mesh {
    let radii = [398.0, 450.0, 600.0, 900.0, 1_400.0, 2_200.0, 3_500.0, 6_000.0, 10_000.0, 18_000.0, 32_000.0];
    let steps = 400usize;
    let ring_len = steps * 4;
    let mut positions = Vec::with_capacity(radii.len() * ring_len);
    let mut normals = Vec::with_capacity(positions.capacity());
    let mut uvs = Vec::with_capacity(positions.capacity());
    let mut indices = Vec::new();
    for (ring, radius) in radii.into_iter().enumerate() {
        for side in 0..4 {
            for step in 0..steps {
                let t = step as f32 / steps as f32 * 2.0 - 1.0;
                let (x, z) = match side {
                    0 => (-radius, t * radius),
                    1 => (t * radius, radius),
                    2 => (radius, -t * radius),
                    _ => (-t * radius, -radius),
                };
                let y = horizon_height(x, z) - if ring == 0 { 0.1 } else { 0.0 };
                let dx = horizon_height(x + 1.0, z) - horizon_height(x - 1.0, z);
                let dz = horizon_height(x, z + 1.0) - horizon_height(x, z - 1.0);
                positions.push([x, y, z]);
                normals.push(Vec3::new(-dx, 2.0, -dz).normalize().to_array());
                uvs.push([x, z]);
            }
        }
        if ring > 0 {
            for i in 0..ring_len {
                let next = (i + 1) % ring_len;
                let a = ((ring - 1) * ring_len + i) as u32;
                let b = (ring * ring_len + i) as u32;
                let c = ((ring - 1) * ring_len + next) as u32;
                let d = (ring * ring_len + next) as u32;
                indices.extend_from_slice(&[a, b, c, c, b, d]);
            }
        }
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(indices))
}

/// One surface tile at `cell` metres, with a skirt hanging below its rim so
/// a coarser neighbour cannot open a crack.
fn tile_mesh(grid: &HeightGrid, tx: i32, tz: i32, cell: f32) -> Mesh {
    let n = (TILE_M / cell).round() as usize;
    let min_x = tx as f32 * TILE_M;
    let min_z = tz as f32 * TILE_M;
    let count = (n + 1) * (n + 1);
    let mut positions = Vec::with_capacity(count + 4 * (n + 1));
    let mut normals = Vec::with_capacity(positions.capacity());
    let mut uvs = Vec::with_capacity(positions.capacity());
    for i in 0..=n {
        for j in 0..=n {
            let x = min_x + j as f32 * cell;
            let z = min_z + i as f32 * cell;
            positions.push([x, grid.height_at(x, z).unwrap_or(0.0), z]);
            normals.push(grid.normal_at(x, z).to_array());
            uvs.push([j as f32 / n as f32, i as f32 / n as f32]);
        }
    }
    let mut indices = Vec::with_capacity(n * n * 6 + n * 24);
    let at = |i: usize, j: usize| (i * (n + 1) + j) as u32;
    for i in 0..n {
        for j in 0..n {
            let (v00, v10, v01, v11) = (at(i, j), at(i, j + 1), at(i + 1, j), at(i + 1, j + 1));
            indices.extend_from_slice(&[v00, v01, v10, v10, v01, v11]);
        }
    }
    // Skirt: every rim vertex gets a twin `TILE_SKIRT_M` lower; quads join them.
    let rim: Vec<u32> = (0..=n)
        .map(|j| at(0, j))
        .chain((1..=n).map(|i| at(i, n)))
        .chain((0..n).rev().map(|j| at(n, j)))
        .chain((1..n).rev().map(|i| at(i, 0)))
        .collect();
    let base = positions.len() as u32;
    for &v in &rim {
        let p = positions[v as usize];
        positions.push([p[0], p[1] - TILE_SKIRT_M, p[2]]);
        normals.push(normals[v as usize]);
        uvs.push(uvs[v as usize]);
    }
    for k in 0..rim.len() {
        let a = rim[k];
        let b = rim[(k + 1) % rim.len()];
        let (a_low, b_low) = (base + k as u32, base + ((k + 1) % rim.len()) as u32);
        indices.extend_from_slice(&[a, a_low, b, b, a_low, b_low, a, b, a_low, b, b_low, a_low]);
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(indices))
}

/// Surface tiles near the camera are built at the fine cell, the rest at
/// the coarse one; a tile is rebuilt when its level changes.
fn update_terrain_tiles(
    mut commands: Commands,
    terrain: Option<ResMut<ProceduralTerrain>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut tiles: Query<&mut Mesh3d, With<TerrainTile>>,
) {
    let Some(mut terrain) = terrain else {
        return;
    };
    let Some(camera) = cameras.iter().next() else {
        return;
    };
    let eye = camera.translation();
    let eye_xz = Vec2::new(eye.x, eye.z);
    let half = terrain.grid.half_size();
    let span = (half / TILE_M).round() as i32;
    let terrain = terrain.as_mut();
    let mut rebuilt = 0usize;
    for tz in -span..span {
        for tx in -span..span {
            let min = Vec2::new(tx as f32 * TILE_M, tz as f32 * TILE_M);
            let nearest = eye_xz.clamp(min, min + Vec2::splat(TILE_M)).distance(eye_xz);
            let cell = if nearest <= TILE_FINE_RADIUS_M { terrain.fine_cell } else { TILE_COARSE_CELL_M };
            match terrain.tiles.get_mut(&(tx, tz)) {
                Some((entity, current)) if *current != cell => {
                    if rebuilt >= TILE_REBUILDS_PER_FRAME {
                        continue;
                    }
                    if let Ok(mut mesh) = tiles.get_mut(*entity) {
                        mesh.0 = meshes.add(tile_mesh(&terrain.grid, tx, tz, cell));
                        *current = cell;
                        rebuilt += 1;
                    }
                }
                Some(_) => {}
                None => {
                    let entity = commands
                        .spawn((
                            Name::new(format!("Terrain[{tx},{tz}]")),
                            TerrainTile,
                            ChildOf(terrain.entity),
                            Transform::IDENTITY,
                            Mesh3d(meshes.add(tile_mesh(&terrain.grid, tx, tz, cell))),
                            MeshMaterial3d(terrain.material.clone()),
                            bevy::light::NotShadowCaster,
                        ))
                        .id();
                    terrain.tiles.insert((tx, tz), (entity, cell));
                }
            }
        }
    }
}

/// Rows run along Z and columns along X, centred on the origin like the
/// grid; parry stores the matrix column-major.
fn heightfield_collider(grid: &HeightGrid) -> ColliderBuilder {
    let mut data = Vec::with_capacity(grid.rows * grid.cols);
    for j in 0..grid.cols {
        for i in 0..grid.rows {
            data.push(grid.at(i, j) as f64);
        }
    }
    let heights = rapier3d::parry::utils::Array2::new(grid.rows, grid.cols, data);
    let size_x = ((grid.cols - 1) as f32 * grid.cell) as f64;
    let size_z = ((grid.rows - 1) as f32 * grid.cell) as f64;
    ColliderBuilder::heightfield(heights, DVec3::new(size_x, 1.0, size_z))
}

/// A USD terrain that reaches its collider takes over the ground.
fn retire_for_usd_terrain(
    mut commands: Commands,
    usd_terrain: Option<Res<TerrainCollision>>,
    terrain: Option<Res<ProceduralTerrain>>,
    mut physics: ResMut<PhysicsWorld>,
    mut chunks: ResMut<GrassChunks>,
) {
    let (Some(_), Some(terrain)) = (usd_terrain, terrain) else {
        return;
    };
    commands.entity(terrain.entity).despawn();
    physics.entity_to_collider.remove(&terrain.entity);
    let physics = physics.as_mut();
    let colliders = &mut physics.colliders;
    let islands = &mut physics.islands;
    let bodies = &mut physics.bodies;
    colliders.remove(terrain.collider, islands, bodies, true);
    colliders.remove(terrain.safety_floor, islands, bodies, true);
    for (_, (entity, _)) in chunks.chunks.drain() {
        commands.entity(entity).despawn();
    }
    for (_, entity) in chunks.details.drain() {
        commands.entity(entity).despawn();
    }
    if let Ok(mut slot) = HEIGHT_GRID.write() {
        *slot = None;
    }
    commands.remove_resource::<GrassField>();
    commands.remove_resource::<ProceduralTerrain>();
    info!("terrain: USD terrain active, procedural ground retired");
}

/// Keeps chunk draws around the camera with conservative instance budgets.
fn update_grass_chunks(
    mut commands: Commands,
    terrain: Option<Res<ProceduralTerrain>>,
    field: Option<Res<GrassField>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    mut chunks: ResMut<GrassChunks>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut draws: Query<(&mut GrassChunkDraw, &mut Mesh3d, &mut Aabb)>,
) {
    let (Some(terrain), Some(_)) = (terrain, field) else {
        return;
    };
    let Some(camera) = cameras.iter().next() else {
        return;
    };
    let eye = camera.translation();
    let eye_xz = Vec2::new(eye.x, eye.z);
    let half = terrain.grid.half_size();

    let lo_x = ((eye.x - GRASS_FADE_END_M) / CHUNK_M).floor() as i32;
    let hi_x = ((eye.x + GRASS_FADE_END_M) / CHUNK_M).floor() as i32;
    let lo_z = ((eye.z - GRASS_FADE_END_M) / CHUNK_M).floor() as i32;
    let hi_z = ((eye.z + GRASS_FADE_END_M) / CHUNK_M).floor() as i32;
    let mut wanted: HashMap<(i32, i32), (u32, usize)> = HashMap::default();
    for cz in lo_z..=hi_z {
        for cx in lo_x..=hi_x {
            let centre = chunk_centre(cx, cz);
            if centre.x.abs() > half || centre.y.abs() > half {
                continue;
            }
            let nearest = chunk_nearest_distance(cx, cz, eye_xz);
            let instances = grass_instance_budget(chunks.blades_per_chunk, nearest);
            if instances == 0 {
                continue;
            }
            let lod = GRASS_LODS.iter().position(|(radius, _)| nearest <= *radius).unwrap_or(0);
            wanted.insert((cx, cz), (instances, lod));
        }
    }

    let stale: Vec<_> = chunks
        .chunks
        .iter()
        .filter(|(key, _)| !wanted.contains_key(*key))
        .map(|(key, (entity, _))| (*key, *entity))
        .collect();
    for (key, entity) in stale {
        commands.entity(entity).despawn();
        chunks.chunks.remove(&key);
    }

    for (&(cx, cz), &(instances, lod)) in &wanted {
        if let Some((entity, current_lod)) = chunks.chunks.get_mut(&(cx, cz)) {
            if let Ok((mut draw, mut mesh, mut aabb)) = draws.get_mut(*entity) {
                draw.instances = instances;
                if *current_lod != lod {
                    *current_lod = lod;
                    mesh.0 = meshes.add(blade_template(GRASS_LODS[lod].1));
                    *aabb = chunk_aabb(&terrain.grid, cx, cz);
                }
            }
            continue;
        }
        let corner = Vec3::new(cx as f32 * CHUNK_M, 0.0, cz as f32 * CHUNK_M);
        let entity = commands
            .spawn((
                Name::new(format!("Grass[{cx},{cz}]")),
                GrassChunk,
                NoFrustumCulling,
                Transform::from_translation(corner),
                GlobalTransform::from_translation(corner),
                Mesh3d(meshes.add(blade_template(GRASS_LODS[lod].1))),
                chunk_aabb(&terrain.grid, cx, cz),
                GrassChunkDraw {
                    corner: Vec2::new(corner.x, corner.z),
                    instances,
                    capacity: chunks.blades_per_chunk,
                    detail: false,
                },
            ))
            .id();
        chunks.chunks.insert((cx, cz), (entity, lod));
    }

    let capacity = CHUNK_M * CHUNK_M * 120.0;
    let mut detail_wanted: HashMap<(i32, i32), u32> = HashMap::default();
    for &(cx, cz) in wanted.keys() {
        let nearest = chunk_nearest_distance(cx, cz, eye_xz);
        let fade = ((DETAIL_FADE_END_M - nearest) / (DETAIL_FADE_END_M - DETAIL_FADE_START_M))
            .clamp(0.0, 1.0);
        let instances = (capacity * fade * fade).ceil() as u32;
        if instances > 0 {
            detail_wanted.insert((cx, cz), instances);
        }
    }
    chunks.details.retain(|key, entity| {
        if detail_wanted.contains_key(key) {
            true
        } else {
            commands.entity(*entity).despawn();
            false
        }
    });
    for ((cx, cz), instances) in detail_wanted {
        if let Some(entity) = chunks.details.get(&(cx, cz)) {
            if let Ok((mut draw, _, _)) = draws.get_mut(*entity) {
                draw.instances = instances;
            }
            continue;
        }
        let corner = Vec2::new(cx as f32 * CHUNK_M, cz as f32 * CHUNK_M);
        let translation = Vec3::new(corner.x, 0.0, corner.y);
        let mut bounds = chunk_aabb(&terrain.grid, cx, cz);
        bounds.half_extents += Vec3A::splat(0.2);
        let entity = commands
            .spawn((
                Name::new(format!("Meadow mixture[{cx},{cz}]")),
                GrassChunk,
                NoFrustumCulling,
                Transform::from_translation(translation),
                GlobalTransform::from_translation(translation),
                Mesh3d(meshes.add(meadow_detail_template())),
                bounds,
                GrassChunkDraw { corner, instances, capacity, detail: true },
            ))
            .id();
        chunks.details.insert((cx, cz), entity);
    }
}

/// Upper bound on blade survival at a distance, before individual fades.
fn grass_density_at(distance: f32) -> f32 {
    let fade = 1.0
        - ((distance - GRASS_FADE_START_M) / (GRASS_FADE_END_M - GRASS_FADE_START_M))
            .clamp(0.0, 1.0);
    fade * fade
}

fn grass_instance_budget(blades_per_chunk: f32, nearest_distance: f32) -> u32 {
    (blades_per_chunk * grass_density_at(nearest_distance)).ceil() as u32
}

fn grass_density_from_env() -> f32 {
    std::env::var("GEARBOX_GRASS_DENSITY")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .map(|value| value.max(0.0))
        .unwrap_or(GRASS_DENSITY_PER_M2)
}

/// Horizontal distance lower-bounds the camera's 3D distance to every blade.
fn chunk_nearest_distance(cx: i32, cz: i32, point: Vec2) -> f32 {
    let min = Vec2::new(cx as f32 * CHUNK_M, cz as f32 * CHUNK_M);
    let max = min + Vec2::splat(CHUNK_M);
    let closest = point.clamp(min, max);
    closest.distance(point)
}

/// Local-space bounds of a chunk draw: the entity sits at the chunk corner.
fn chunk_aabb(grid: &HeightGrid, cx: i32, cz: i32) -> Aabb {
    let min_x = cx as f32 * CHUNK_M;
    let min_z = cz as f32 * CHUNK_M;
    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    let samples = (CHUNK_M / grid.cell).ceil() as i32;
    for i in 0..=samples {
        for j in 0..=samples {
            let x = min_x + j as f32 * grid.cell;
            let z = min_z + i as f32 * grid.cell;
            if let Some(h) = grid.height_at(x, z) {
                lo = lo.min(h);
                hi = hi.max(h);
            }
        }
    }
    if !lo.is_finite() {
        lo = 0.0;
        hi = 0.0;
    }
    Aabb::from_min_max(
        Vec3::new(0.0, lo - 0.5, 0.0),
        Vec3::new(CHUNK_M, hi + BLADE_MAX_HEIGHT_M + 0.5, CHUNK_M),
    )
}

fn chunk_centre(cx: i32, cz: i32) -> Vec2 {
    Vec2::new((cx as f32 + 0.5) * CHUNK_M, (cz as f32 + 0.5) * CHUNK_M)
}

/// One blade as a strip: two vertices per segment and a single tip.
/// `position.x` is the side, `position.y` the height fraction.
fn blade_template(segments: u32) -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    for k in 0..=segments {
        let t = k as f32 / segments as f32;
        let sides: &[f32] = if k == segments { &[0.0] } else { &[-1.0, 1.0] };
        for side in sides {
            positions.push([*side, t, 0.0]);
            normals.push([0.0, 1.0, 0.0]);
            uvs.push([0.5 + 0.5 * side, t]);
        }
    }
    let mut indices = Vec::new();
    for k in 0..segments.saturating_sub(1) {
        let l = k * 2;
        indices.extend_from_slice(&[l, l + 2, l + 1, l + 1, l + 2, l + 3]);
    }
    let last = (segments - 1) * 2;
    indices.extend_from_slice(&[last, last + 2, last + 1]);
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(indices))
}


/// Six segmented leaves; position.z selects the leaf within a plant.
fn meadow_detail_template() -> Mesh {
    let segments = 5u32;
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for leaf in 0..6 {
        let start = positions.len() as u32;
        for step in 0..=segments {
            let t = step as f32 / segments as f32;
            if step == segments {
                positions.push([0.0, t, (leaf + 1) as f32]);
            } else {
                positions.push([-1.0, t, (leaf + 1) as f32]);
                positions.push([1.0, t, (leaf + 1) as f32]);
            }
        }
        for step in 0..segments - 1 {
            let i = start + step * 2;
            indices.extend_from_slice(&[i, i + 2, i + 1, i + 1, i + 2, i + 3]);
        }
        let i = start + (segments - 1) * 2;
        indices.extend_from_slice(&[i, i + 2, i + 1]);
    }
    let count = positions.len();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; count])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; count])
        .with_inserted_indices(Indices::U32(indices))
}

/// RGBA32F texels of (height, normal x, normal z, 0) for the grass shader.
fn heightmap_image(grid: &HeightGrid) -> Image {
    let mut texels: Vec<f32> = Vec::with_capacity(grid.cols * grid.rows * 4);
    for i in 0..grid.rows {
        for j in 0..grid.cols {
            let normal = grid.normal_at_index(i, j);
            texels.extend_from_slice(&[grid.at(i, j), normal.x, normal.z, 0.0]);
        }
    }
    Image::new(
        Extent3d { width: grid.cols as u32, height: grid.rows as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        bytemuck::cast_slice(&texels).to_vec(),
        TextureFormat::Rgba32Float,
        RenderAssetUsages::RENDER_WORLD,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rapier3d::prelude::Ray;

    fn bowl(x: f32, z: f32) -> f32 {
        0.002 * (x * x - z * z) + 0.05 * x
    }

    #[test]
    fn heightfield_collider_matches_height_grid() {
        let grid = HeightGrid::sample(64.0, 1.0, bowl);
        let collider = heightfield_collider(&grid).build();
        for (x, z) in [(10.3, -20.7), (-25.5, 3.2), (0.0, 0.0), (30.9, 30.1), (-31.0, -12.4)] {
            let expected = grid.height_at(x, z).unwrap();
            let ray = Ray::new(DVec3::new(x as f64, 100.0, z as f64), DVec3::new(0.0, -1.0, 0.0));
            let toi = collider
                .shape()
                .cast_local_ray(&ray, 1000.0, true)
                .expect("ray hits the heightfield");
            let hit = 100.0 - toi;
            assert!((hit - expected as f64).abs() < 0.05, "at ({x},{z}): collider {hit} vs grid {expected}");
        }
    }

    #[test]
    fn height_grid_interpolates_and_bounds() {
        let grid = HeightGrid::sample(8.0, 2.0, |x, z| x + 10.0 * z);
        assert!((grid.height_at(1.0, 1.0).unwrap() - 11.0).abs() < 1e-4);
        assert!((grid.height_at(-4.0, -4.0).unwrap() + 44.0).abs() < 1e-4);
        assert!(grid.height_at(4.0, 4.0).is_some());
        assert!(grid.height_at(4.5, 0.0).is_none());
        assert!(grid.height_at(0.0, -4.5).is_none());
    }

    #[test]
    fn grass_density_decreases_to_zero_at_the_outer_radius() {
        assert_eq!(grass_density_at(0.0), 1.0);
        assert_eq!(grass_density_at(GRASS_FADE_START_M), 1.0);
        assert_eq!(grass_density_at(GRASS_FADE_END_M), 0.0);
        assert_eq!(grass_density_at(GRASS_FADE_END_M + CHUNK_M), 0.0);
        let midpoint = (GRASS_FADE_START_M + GRASS_FADE_END_M) * 0.5;
        assert!((grass_density_at(midpoint) - 0.25).abs() < 1e-6);
        let mut previous = 1.0;
        for step in 0..=100 {
            let distance = GRASS_FADE_END_M * step as f32 / 100.0;
            let density = grass_density_at(distance);
            assert!(density <= previous);
            previous = density;
        }
    }

    #[test]
    fn grass_instance_budget_covers_every_blade_fade_radius() {
        let blades = 1024.0;
        for step in 0..=240 {
            let distance = GRASS_FADE_END_M * step as f32 / 240.0;
            let budget = grass_instance_budget(blades, distance);
            assert!(budget <= blades as u32);
            for index in budget..blades as u32 {
                let rank = index as f32 / blades;
                let blade_end = GRASS_FADE_END_M
                    + (GRASS_FADE_START_M - GRASS_FADE_END_M) * rank.sqrt();
                assert!(distance + 1e-5 >= blade_end);
            }
        }
        assert_eq!(grass_instance_budget(blades, 0.0), blades as u32);
        assert_eq!(grass_instance_budget(blades, GRASS_FADE_END_M), 0);
        assert_eq!(grass_instance_budget(0.0, 0.0), 0);
    }

    #[test]
    fn chunk_budget_distance_is_conservative_across_boundaries_and_heights() {
        for eye in [
            Vec3::new(-0.1, 0.2, -0.1),
            Vec3::new(0.1, 0.2, 0.1),
            Vec3::new(CHUNK_M - 0.1, 2.0, CHUNK_M),
            Vec3::new(CHUNK_M + 0.1, 2.0, CHUNK_M),
            Vec3::new(0.0, GRASS_FADE_END_M + 5.0, 0.0),
        ] {
            for cz in -2..=2 {
                for cx in -2..=2 {
                    let nearest = chunk_nearest_distance(cx, cz, Vec2::new(eye.x, eye.z));
                    for (x, z) in [(0.0, 0.0), (0.3, 0.7), (0.5, 0.5), (1.0, 1.0)] {
                        for height in [-8.0, 0.0, 15.0] {
                            let blade = Vec3::new(
                                (cx as f32 + x) * CHUNK_M,
                                height,
                                (cz as f32 + z) * CHUNK_M,
                            );
                            let distance = eye.distance(blade);
                            assert!(nearest <= distance + 1e-5);
                            assert!(
                                grass_instance_budget(1024.0, nearest)
                                    >= grass_instance_budget(1024.0, distance)
                            );
                        }
                    }
                }
            }
        }
    }
}
