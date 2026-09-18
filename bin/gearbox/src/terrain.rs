//! Procedural terrain geometry, sampled heights, mesh LOD, and collision.

use std::sync::{Arc, RwLock};

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use gearbox_fields::HeightGrid;
use rapier3d::math::Vector as DVec3;
use rapier3d::prelude::{ColliderBuilder, ColliderHandle};

use crate::physics::PhysicsWorld;
use crate::world::{FlatGround, TerrainCollision, fbm_world, remove_flat_ground, smooth_hill};

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
// Measured against the meadow backdrop it borders, from 60 km in clear air.
const HORIZON_COLOR: Color = Color::linear_rgb(0.058, 0.108, 0.015);
/// Peak-to-trough relief of the distant land around the meadow.
pub(crate) const HORIZON_RELIEF_M: f32 = 220.0;

static HEIGHT_GRID: RwLock<Option<Arc<HeightGrid>>> = RwLock::new(None);

pub struct TerrainPlugin;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TerrainUpdates;

impl Plugin for TerrainPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, spawn_procedural_terrain)
            .add_systems(
                Update,
                (retire_for_usd_terrain, update_terrain_tiles)
                    .chain()
                    .in_set(TerrainUpdates),
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

/// The active procedural ground and everything that must go with it.
#[derive(Resource)]
pub struct ProceduralTerrain {
    pub(crate) entity: Entity,
    collider: ColliderHandle,
    safety_floor: ColliderHandle,
    pub(crate) grid: Arc<HeightGrid>,
    fine_cell: f32,
    tiles: HashMap<(i32, i32), (Entity, f32)>,
}

impl ProceduralTerrain {
    /// Height on the rendered tile's current LOD triangles.
    pub(crate) fn surface_height_m(&self, x: f32, z: f32) -> Option<f32> {
        let tx = (x / TILE_M).floor() as i32;
        let tz = (z / TILE_M).floor() as i32;
        let (_, cell) = self.tiles.get(&(tx, tz))?;
        let n = tile_segments(*cell);
        let step = TILE_M / n as f32;
        let fx = (x - tx as f32 * TILE_M) / step;
        let fz = (z - tz as f32 * TILE_M) / step;
        let j = (fx.floor() as usize).min(n - 1);
        let i = (fz.floor() as usize).min(n - 1);
        let u = (fx - j as f32).clamp(0.0, 1.0);
        let v = (fz - i as f32).clamp(0.0, 1.0);
        let x0 = tx as f32 * TILE_M + j as f32 * step;
        let z0 = tz as f32 * TILE_M + i as f32 * step;
        let h00 = self.grid.height_at(x0, z0)?;
        let h10 = self.grid.height_at(x0 + step, z0)?;
        let h01 = self.grid.height_at(x0, z0 + step)?;
        let h11 = self.grid.height_at(x0 + step, z0 + step)?;
        Some(if u + v <= 1.0 {
            h00 + (h10 - h00) * u + (h01 - h00) * v
        } else {
            h11 + (h01 - h11) * (1.0 - u) + (h10 - h11) * (1.0 - v)
        })
    }
}

#[derive(Component)]
struct TerrainTile;

/// Mesh geometry available for an independently assigned field surface.
#[derive(Component)]
pub(crate) struct TerrainSurfaceMesh;

#[derive(Component)]
pub(crate) struct TerrainBackdrop;

fn spawn_procedural_terrain(
    mut commands: Commands,
    mut physics: ResMut<PhysicsWorld>,
    flat: Option<Res<FlatGround>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    horizon: Query<(&Name, &MeshMaterial3d<StandardMaterial>)>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let preset = TerrainPreset::from_env();
    if preset == TerrainPreset::Flat {
        info!("terrain: GEARBOX_TERRAIN=flat, keeping the flat ground");
        return;
    }
    let started = std::time::Instant::now();
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
    let entity = commands
        .spawn((
            Name::new("MeadowTerrain"),
            Transform::IDENTITY,
            Visibility::default(),
        ))
        .id();
    commands.spawn((
        Name::new("Distant meadow landscape"),
        TerrainBackdrop,
        gearbox_fields::CoverBackdrop,
        ChildOf(entity),
        Transform::IDENTITY,
        Mesh3d(meshes.add(horizon_mesh(&grid))),
        TerrainSurfaceMesh,
        gearbox_fields::CoverSurfaceMesh,
        bevy::light::NotShadowCaster,
    ));

    let collider = physics.colliders.insert(
        heightfield_collider(&collider_grid)
            .friction(crate::world::ground_friction(1.4))
            .restitution(0.0)
            .build(),
    );
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
    commands.insert_resource(ProceduralTerrain {
        entity,
        collider,
        safety_floor,
        grid,
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
    let relief = (fbm_world(x * 0.00045 + 41.0, z * 0.00045 - 13.0, 4) - 0.5) * HORIZON_RELIEF_M;
    meadow_height(x, z) + relief * t * t * (3.0 - 2.0 * t)
}

/// Concentric terrain rings extend the meadow to a 64 km visual backdrop.
fn horizon_mesh(grid: &HeightGrid) -> Mesh {
    let radii = [
        400.0, 404.0, 412.0, 424.0, 440.0, 464.0, 496.0, 540.0, 600.0, 700.0, 850.0, 1_050.0,
        1_300.0, 1_600.0, 2_000.0, 2_500.0, 3_200.0, 4_000.0, 5_000.0, 6_400.0, 8_000.0, 10_000.0,
        12_800.0, 16_000.0, 20_000.0, 25_600.0, 32_000.0,
    ];
    let steps = (SIZE_M / TILE_M) as usize * tile_segments(grid.cell);
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
                let (y, normal) = if ring == 0 {
                    (
                        grid.height_at(x, z)
                            .expect("horizon rim inside height grid"),
                        grid.normal_at(x, z),
                    )
                } else {
                    let dx = horizon_height(x + 1.0, z) - horizon_height(x - 1.0, z);
                    let dz = horizon_height(x, z + 1.0) - horizon_height(x, z - 1.0);
                    (horizon_height(x, z), Vec3::new(-dx, 2.0, -dz).normalize())
                };
                positions.push([x, y, z]);
                normals.push(normal.to_array());
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
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

fn tile_segments(cell: f32) -> usize {
    (TILE_M / cell).ceil().max(1.0) as usize
}

/// One surface tile with spacing at most `cell`, and a skirt below its rim so
/// a coarser neighbour cannot open a crack.
fn tile_mesh(grid: &HeightGrid, tx: i32, tz: i32, cell: f32) -> Mesh {
    let n = tile_segments(cell);
    let min_x = tx as f32 * TILE_M;
    let min_z = tz as f32 * TILE_M;
    let count = (n + 1) * (n + 1);
    let mut positions = Vec::with_capacity(count + 4 * (n + 1));
    let mut normals = Vec::with_capacity(positions.capacity());
    let mut uvs = Vec::with_capacity(positions.capacity());
    for i in 0..=n {
        for j in 0..=n {
            let x = min_x + TILE_M * (j as f32 / n as f32);
            let z = min_z + TILE_M * (i as f32 / n as f32);
            positions.push([
                x,
                grid.height_at(x, z)
                    .expect("terrain tile inside height grid"),
                z,
            ]);
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
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
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
            let nearest = eye_xz
                .clamp(min, min + Vec2::splat(TILE_M))
                .distance(eye_xz);
            let boundary = tx == -span || tz == -span || tx == span - 1 || tz == span - 1;
            let cell = if boundary || nearest <= TILE_FINE_RADIUS_M {
                terrain.fine_cell
            } else {
                TILE_COARSE_CELL_M
            };
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
                            TerrainSurfaceMesh,
                            gearbox_fields::CoverSurfaceMesh,
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
    if let Ok(mut slot) = HEIGHT_GRID.write() {
        *slot = None;
    }
    commands.remove_resource::<ProceduralTerrain>();
    info!("terrain: USD terrain active, procedural ground retired");
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
        for (x, z) in [
            (10.3, -20.7),
            (-25.5, 3.2),
            (0.0, 0.0),
            (30.9, 30.1),
            (-31.0, -12.4),
        ] {
            let expected = grid.height_at(x, z).unwrap();
            let ray = Ray::new(
                DVec3::new(x as f64, 100.0, z as f64),
                DVec3::new(0.0, -1.0, 0.0),
            );
            let toi = collider
                .shape()
                .cast_local_ray(&ray, 1000.0, true)
                .expect("ray hits the heightfield");
            let hit = 100.0 - toi;
            assert!(
                (hit - expected as f64).abs() < 0.05,
                "at ({x},{z}): collider {hit} vs grid {expected}"
            );
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
    fn irregular_cell_size_keeps_exact_grid_extent() {
        let grid = HeightGrid::sample(100.0, 3.0, |x, z| x + z);
        assert!((grid.half_size() - 50.0).abs() < 1e-5);
        for (x, z) in [(-50.0, -50.0), (50.0, 50.0), (-50.0, 50.0)] {
            assert!((grid.height_at(x, z).unwrap() - x - z).abs() < 1e-4);
        }
    }

    #[test]
    fn height_grid_boundary_normals_preserve_a_plane() {
        let grid = HeightGrid::sample(100.0, 1.0, |x, z| 20.0 + 0.1 * x + 0.2 * z);
        let expected = Vec3::new(-0.1, 1.0, -0.2).normalize();
        for (x, z) in [(-50.0, -50.0), (50.0, 50.0), (50.0, 0.0), (0.0, -50.0)] {
            assert!(grid.normal_at(x, z).distance(expected) < 1e-5);
        }
    }

    #[test]
    fn coarse_tiles_stop_at_their_shared_boundary() {
        let grid = HeightGrid::sample(100.0, 1.0, |x, z| 20.0 + 0.1 * x + 0.2 * z);
        for cell in [1.0, 3.0, TILE_COARSE_CELL_M] {
            let n = tile_segments(cell);
            for tx in [-1, 0] {
                let mesh = tile_mesh(&grid, tx, 0, cell);
                let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) =
                    mesh.attribute(Mesh::ATTRIBUTE_POSITION)
                else {
                    panic!("position attribute");
                };
                for p in &positions[..(n + 1) * (n + 1)] {
                    assert!(p[0] >= tx as f32 * TILE_M && p[0] <= (tx + 1) as f32 * TILE_M);
                    assert!(p[2] >= 0.0 && p[2] <= TILE_M);
                    assert!((p[1] - (20.0 + 0.1 * p[0] + 0.2 * p[2])).abs() < 1e-4);
                }
                assert_eq!(positions[n][0], (tx + 1) as f32 * TILE_M);
                assert_eq!(positions[n * (n + 1)][2], TILE_M);
            }
        }
    }

    #[test]
    fn horizon_rim_matches_local_ground_without_overlap() {
        let grid = HeightGrid::sample(SIZE_M, 1.0, |x, z| 10.0 + x * 0.01 + z * 0.02);
        let mesh = horizon_mesh(&grid);
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("position attribute");
        };
        let rim_len = (SIZE_M / TILE_M) as usize * tile_segments(grid.cell) * 4;
        for p in &positions[..rim_len] {
            assert_eq!(p[0].abs().max(p[2].abs()), SIZE_M * 0.5);
            assert!((p[1] - grid.height_at(p[0], p[2]).unwrap()).abs() < 1e-5);
        }
        for p in positions {
            assert!(p[0].abs().max(p[2].abs()) >= SIZE_M * 0.5);
        }
    }
}

/// Ground height for the field cover: the procedural grid, else USD terrain.
struct GroundHeights;

impl gearbox_fields::HeightSource for GroundHeights {
    fn height(&self, x: f32, z: f32) -> f32 {
        crate::world::terrain_height_m(x, z)
    }
}

/// Hands the active ground and the USD terrain roots to the field cover.
pub fn publish_cover_terrain(
    mut commands: Commands,
    terrain: Option<Res<ProceduralTerrain>>,
    roots: Query<(Entity, &Name), With<usd_bevy::UsdSceneRoot>>,
    children: Query<&Children>,
    mut cover_roots: ResMut<gearbox_fields::CoverTerrainRoots>,
    mut published: Local<Option<Entity>>,
) {
    match terrain.as_deref() {
        Some(terrain) if *published != Some(terrain.entity) => {
            commands.insert_resource(gearbox_fields::CoverTerrain {
                entity: terrain.entity,
                grid: terrain.grid.clone(),
            });
            commands.insert_resource(gearbox_fields::CoverHeights(Arc::new(GroundHeights)));
            *published = Some(terrain.entity);
        }
        None if published.is_some() => {
            commands.remove_resource::<gearbox_fields::CoverTerrain>();
            *published = None;
        }
        _ => {}
    }
    let found: Vec<Entity> = roots
        .iter()
        .filter(|(root, name)| {
            crate::world::is_usd_terrain_root_name(name.as_str())
                && crate::world::is_usd_terrain_scene_instantiated(*root, &children)
        })
        .map(|(root, _)| root)
        .collect();
    if found != cover_roots.0 {
        cover_roots.0 = found;
    }
}
