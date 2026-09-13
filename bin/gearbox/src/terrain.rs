//! Procedural ground: a heightfield mesh under an anti-repeat grass
//! material, the matching Rapier heightfield collider, and blade chunks that
//! follow the camera. Replaces the flat startup ground unless
//! `GEARBOX_TERRAIN=flat`; a loaded USD terrain retires it.

use std::sync::{Arc, RwLock};

use bevy::asset::RenderAssetUsages;
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin, StandardMaterial};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use rapier3d::math::Vector as DVec3;
use rapier3d::prelude::{ColliderBuilder, ColliderHandle};

use crate::physics::PhysicsWorld;
use crate::world::{
    FlatGround, TerrainCollision, asset_path, fbm_world, remove_flat_ground, smooth_hill,
};

const SIZE_M: f32 = 800.0;
const CELL_M: f32 = 1.0;
const SPAWN_FLAT_RADIUS_M: f32 = 30.0;
const SPAWN_RELIEF_RADIUS_M: f32 = 70.0;
const SAFETY_FLOOR_Y_M: f64 = -40.0;
const SAFETY_FLOOR_HALF_EXTENT_M: f64 = 10_000.0;

const CHUNK_M: f32 = 16.0;
/// Outer radius of each detail ring and its blades per square metre.
const GRASS_RINGS: [(f32, f32); 3] = [(48.0, 10.0), (96.0, 5.0), (150.0, 2.0)];
const GRASS_CHUNK_BUILDS_PER_FRAME: usize = 12;
/// Below this surface-normal Y the ground reads as bare dirt: no blades.
const DIRT_SLOPE_NORMAL_Y: f32 = 0.86;
const HORIZON_COLOR: Color = Color::srgb(0.46, 0.55, 0.30);

static HEIGHT_GRID: RwLock<Option<Arc<HeightGrid>>> = RwLock::new(None);

pub struct TerrainPlugin;

impl Plugin for TerrainPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "../assets/shaders/meadow_material.wgsl");
        bevy::asset::embedded_asset!(app, "../assets/shaders/grass_material.wgsl");
        app.add_plugins((
            MaterialPlugin::<MeadowMaterial>::default(),
            MaterialPlugin::<GrassMaterial>::default(),
        ))
        .init_resource::<GrassChunks>()
        .add_systems(PostStartup, spawn_procedural_terrain)
        .add_systems(Update, (retire_for_usd_terrain, update_grass_chunks).chain());
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
}

impl MaterialExtension for MeadowExtension {
    fn fragment_shader() -> ShaderRef {
        "embedded://gearbox_sim/../assets/shaders/meadow_material.wgsl".into()
    }
}

type MeadowMaterial = ExtendedMaterial<StandardMaterial, MeadowExtension>;

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
struct GrassExtension {
    /// x: sway amplitude (m), y: speed, z/w: wind direction.
    #[uniform(100)]
    wind: Vec4,
}

impl MaterialExtension for GrassExtension {
    fn vertex_shader() -> ShaderRef {
        "embedded://gearbox_sim/../assets/shaders/grass_material.wgsl".into()
    }
}

type GrassMaterial = ExtendedMaterial<StandardMaterial, GrassExtension>;

/// The active procedural ground and everything that must go with it.
#[derive(Resource)]
pub struct ProceduralTerrain {
    entity: Entity,
    collider: ColliderHandle,
    safety_floor: ColliderHandle,
    grid: Arc<HeightGrid>,
}

#[derive(Resource, Default)]
struct GrassChunks {
    material: Option<Handle<GrassMaterial>>,
    chunks: HashMap<(i32, i32), (Entity, usize)>,
}

#[derive(Component)]
struct GrassChunk;

fn spawn_procedural_terrain(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<MeadowMaterial>>,
    mut physics: ResMut<PhysicsWorld>,
    asset_server: Res<AssetServer>,
    flat: Option<Res<FlatGround>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    horizon: Query<(&Name, &MeshMaterial3d<StandardMaterial>)>,
) {
    let preset = TerrainPreset::from_env();
    if preset == TerrainPreset::Flat {
        info!("terrain: GEARBOX_TERRAIN=flat, keeping the flat ground");
        return;
    }
    let started = std::time::Instant::now();
    let grid = Arc::new(HeightGrid::sample(SIZE_M, CELL_M, meadow_height));
    let mesh = meshes.add(terrain_mesh(&grid));
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
        },
    });
    let entity = commands
        .spawn((
            Name::new("MeadowTerrain"),
            Transform::IDENTITY,
            Mesh3d(mesh),
            MeshMaterial3d(material),
        ))
        .id();

    let collider = physics
        .colliders
        .insert(heightfield_collider(&grid).friction(1.4).restitution(0.0).build());
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
    commands.insert_resource(ProceduralTerrain { entity, collider, safety_floor, grid });
    info!(
        "terrain: {preset:?} ground ready, {} m at {} m cells, in {:?}",
        SIZE_M,
        CELL_M,
        started.elapsed()
    );
}

fn terrain_mesh(grid: &HeightGrid) -> Mesh {
    let count = grid.cols * grid.rows;
    let mut positions = Vec::with_capacity(count);
    let mut normals = Vec::with_capacity(count);
    let mut uvs = Vec::with_capacity(count);
    let size = (grid.cols - 1) as f32 * grid.cell;
    for i in 0..grid.rows {
        for j in 0..grid.cols {
            let x = grid.min_x + j as f32 * grid.cell;
            let z = grid.min_z + i as f32 * grid.cell;
            positions.push([x, grid.at(i, j), z]);
            normals.push(grid.normal_at_index(i, j).to_array());
            uvs.push([(x - grid.min_x) / size, (z - grid.min_z) / size]);
        }
    }
    let mut indices = Vec::with_capacity((grid.cols - 1) * (grid.rows - 1) * 6);
    for i in 0..grid.rows - 1 {
        for j in 0..grid.cols - 1 {
            let v00 = (i * grid.cols + j) as u32;
            let v10 = v00 + 1;
            let v01 = v00 + grid.cols as u32;
            let v11 = v01 + 1;
            indices.extend_from_slice(&[v00, v01, v10, v10, v01, v11]);
        }
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(indices))
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
    if let Ok(mut slot) = HEIGHT_GRID.write() {
        *slot = None;
    }
    commands.remove_resource::<ProceduralTerrain>();
    info!("terrain: USD terrain active, procedural ground retired");
}

fn update_grass_chunks(
    mut commands: Commands,
    terrain: Option<Res<ProceduralTerrain>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    mut chunks: ResMut<GrassChunks>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GrassMaterial>>,
) {
    let Some(terrain) = terrain else {
        return;
    };
    let Some(camera) = cameras.iter().next() else {
        return;
    };
    let eye = camera.translation();
    let (max_radius, _) = GRASS_RINGS[GRASS_RINGS.len() - 1];
    let half = terrain.grid.half_size();
    let material = chunks
        .material
        .get_or_insert_with(|| {
            materials.add(ExtendedMaterial {
                base: StandardMaterial {
                    base_color: Color::WHITE,
                    perceptual_roughness: 0.9,
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                },
                extension: GrassExtension { wind: Vec4::new(0.12, 1.6, 0.8, 0.6) },
            })
        })
        .clone();

    let lo_x = ((eye.x - max_radius) / CHUNK_M).floor() as i32;
    let hi_x = ((eye.x + max_radius) / CHUNK_M).floor() as i32;
    let lo_z = ((eye.z - max_radius) / CHUNK_M).floor() as i32;
    let hi_z = ((eye.z + max_radius) / CHUNK_M).floor() as i32;
    let mut wanted: HashMap<(i32, i32), usize> = HashMap::default();
    for cz in lo_z..=hi_z {
        for cx in lo_x..=hi_x {
            let centre = Vec2::new((cx as f32 + 0.5) * CHUNK_M, (cz as f32 + 0.5) * CHUNK_M);
            if centre.x.abs() > half || centre.y.abs() > half {
                continue;
            }
            let distance = centre.distance(Vec2::new(eye.x, eye.z));
            if let Some(ring) = GRASS_RINGS.iter().position(|(radius, _)| distance <= *radius) {
                wanted.insert((cx, cz), ring);
            }
        }
    }

    let stale: Vec<_> = chunks
        .chunks
        .iter()
        .filter(|(key, (_, ring))| wanted.get(*key) != Some(ring))
        .map(|(key, (entity, _))| (*key, *entity))
        .collect();
    for (key, entity) in stale {
        commands.entity(entity).despawn();
        chunks.chunks.remove(&key);
    }

    let mut budget = GRASS_CHUNK_BUILDS_PER_FRAME;
    let mut missing: Vec<_> = wanted
        .iter()
        .filter(|(key, _)| !chunks.chunks.contains_key(*key))
        .map(|(key, ring)| (*key, *ring))
        .collect();
    missing.sort_by_key(|((cx, cz), _)| {
        let centre = Vec2::new((*cx as f32 + 0.5) * CHUNK_M, (*cz as f32 + 0.5) * CHUNK_M);
        centre.distance_squared(Vec2::new(eye.x, eye.z)) as i64
    });
    for ((cx, cz), ring) in missing {
        if budget == 0 {
            break;
        }
        budget -= 1;
        let Some(mesh) = grass_chunk_mesh(&terrain.grid, cx, cz, ring) else {
            continue;
        };
        let entity = commands
            .spawn((
                Name::new(format!("Grass[{cx},{cz}]")),
                GrassChunk,
                Transform::IDENTITY,
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(material.clone()),
                NotShadowCaster,
            ))
            .id();
        chunks.chunks.insert((cx, cz), (entity, ring));
    }
}

struct Rng(u32);

impl Rng {
    fn seeded(cx: i32, cz: i32, ring: usize) -> Self {
        let mut seed = (cx as u32).wrapping_mul(73_856_093)
            ^ (cz as u32).wrapping_mul(19_349_663)
            ^ (ring as u32 + 1).wrapping_mul(83_492_791);
        if seed == 0 {
            seed = 0x9E37_79B9;
        }
        Self(seed)
    }

    fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next()
    }
}

/// Blades of one chunk merged into a single mesh; `None` when nothing grows.
fn grass_chunk_mesh(grid: &HeightGrid, cx: i32, cz: i32, ring: usize) -> Option<Mesh> {
    let (_, density) = GRASS_RINGS[ring];
    let blades = (CHUNK_M * CHUNK_M * density) as usize;
    let mut rng = Rng::seeded(cx, cz, ring);
    let origin = Vec2::new(cx as f32 * CHUNK_M, cz as f32 * CHUNK_M);
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(blades * 7);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(blades * 7);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(blades * 7);
    let mut colors: Vec<[f32; 4]> = Vec::with_capacity(blades * 7);
    let mut indices: Vec<u32> = Vec::with_capacity(blades * 15);
    for _ in 0..blades {
        let x = origin.x + rng.range(0.0, CHUNK_M);
        let z = origin.y + rng.range(0.0, CHUNK_M);
        let Some(y) = grid.height_at(x, z) else {
            continue;
        };
        let normal = grid.normal_at(x, z);
        if normal.y < DIRT_SLOPE_NORMAL_Y {
            continue;
        }
        let yaw = rng.range(0.0, std::f32::consts::TAU);
        let height = rng.range(0.25, 0.7);
        let width = rng.range(0.025, 0.055);
        let lean = Vec3::new(rng.range(-0.15, 0.15), 0.0, rng.range(-0.15, 0.15));
        let right = Vec3::new(yaw.cos(), 0.0, yaw.sin());
        let tone = rng.range(0.75, 1.2);
        let dry = if rng.next() < 0.18 { rng.range(0.3, 1.0) } else { 0.0 };
        let root = Vec3::new(0.10, 0.22, 0.04) * tone;
        let tip = Vec3::new(0.36, 0.62, 0.14).lerp(Vec3::new(0.55, 0.52, 0.16), dry) * tone;
        let base = positions.len() as u32;
        for k in 0..=3u32 {
            let t = k as f32 / 3.0;
            let centre = Vec3::new(x, y, z) + Vec3::Y * (height * t) + lean * (height * t * t);
            let half_width = width * 0.5 * (1.0 - t * 0.85);
            let color = root.lerp(tip, t);
            let samples = if k == 3 { 1 } else { 2 };
            for s in 0..samples {
                let side = if samples == 1 { 0.0 } else if s == 0 { -1.0 } else { 1.0 };
                let p = centre + right * (half_width * side);
                positions.push(p.to_array());
                normals.push(normal.to_array());
                uvs.push([0.5 + 0.5 * side, t]);
                colors.push([color.x, color.y, color.z, 1.0]);
            }
        }
        for k in 0..2u32 {
            let l = base + k * 2;
            let r = l + 1;
            let l2 = l + 2;
            let r2 = l + 3;
            indices.extend_from_slice(&[l, l2, r, r, l2, r2]);
        }
        indices.extend_from_slice(&[base + 4, base + 6, base + 5]);
    }
    if positions.is_empty() {
        return None;
    }
    Some(
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
            .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
            .with_inserted_indices(Indices::U32(indices)),
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
}
