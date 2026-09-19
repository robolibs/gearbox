//! Per-field materials, track textures, surface assignment, and vegetation streaming.

#[cfg(test)]
mod tests {
    use super::*;
    const START: f32 = 4.0;
    const END: f32 = 32.0;
    fn density(distance: f32) -> f32 {
        let radial = ((END - distance) / (END - START)).clamp(0.0, 1.0);
        let projected = START / distance.max(START);
        (radial * projected).powi(2)
    }
    fn budget(capacity: f32, distance: f32) -> u32 {
        instance_budget(capacity, distance, START, END, true)
    }
    fn chunk_nearest_distance(x: i32, z: i32, eye: Vec2) -> f32 {
        let min = Vec2::new(x as f32, z as f32) * CHUNK_M;
        FieldBounds {
            min,
            max: min + Vec2::splat(CHUNK_M),
        }
        .nearest_distance(eye)
    }
    #[test]
    fn grass_density_decreases_to_zero_at_the_outer_radius() {
        assert_eq!(density(0.0), 1.0);
        assert_eq!(density(START), 1.0);
        assert_eq!(density(END), 0.0);
        assert_eq!(density(END + CHUNK_M), 0.0);
        let midpoint = (START + END) * 0.5;
        assert!((density(midpoint) - 0.25 * (START / midpoint).powi(2)).abs() < 1e-6);
        let mut previous = 1.0;
        for step in 0..=100 {
            let distance = END * step as f32 / 100.0;
            let density = density(distance);
            assert!(density <= previous);
            previous = density;
        }
    }

    #[test]
    fn grass_instance_budget_covers_every_blade_fade_radius() {
        let blades = 1024.0;
        for step in 0..=240 {
            let distance = END * step as f32 / 240.0;
            let budget = budget(blades, distance);
            assert!(budget <= blades as u32);
            for index in budget..blades as u32 {
                let rank = index as f32 / blades;
                let blade_end = START * END / (START + (END - START) * rank.sqrt());
                assert!(distance + 1e-5 >= blade_end);
            }
        }
        assert_eq!(budget(blades, 0.0), blades as u32);
        assert_eq!(budget(blades, END), 0);
        assert_eq!(budget(0.0, 0.0), 0);
    }

    #[test]
    fn secondary_plants_keep_the_gentle_distance_budget() {
        assert_eq!(instance_budget(1024.0, 8.0, 8.0, 24.0, false), 1024);
        assert_eq!(instance_budget(1024.0, 16.0, 8.0, 24.0, false), 256);
        assert_eq!(instance_budget(1024.0, 24.0, 8.0, 24.0, false), 0);
        assert_eq!(instance_budget(1024.0, 16.0, 8.0, 24.0, true), 64);
    }

    #[test]
    fn chunk_budget_distance_is_conservative_across_boundaries_and_heights() {
        for eye in [
            Vec3::new(-0.1, 0.2, -0.1),
            Vec3::new(0.1, 0.2, 0.1),
            Vec3::new(CHUNK_M - 0.1, 2.0, CHUNK_M),
            Vec3::new(CHUNK_M + 0.1, 2.0, CHUNK_M),
            Vec3::new(0.0, END + 5.0, 0.0),
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
                            assert!(budget(1024.0, nearest) >= budget(1024.0, distance));
                        }
                    }
                }
            }
        }
    }
}

use super::geometry::{clip_mesh, mesh_bounds};
use super::layout::{FieldBounds, FieldLayout};
use super::profile::{FieldProfile, FieldProfiles, GroundSurface, VegetationLayer, WheelMapParams};
use super::render::{FieldGpu, RenderFields, VegetationChunk, VegetationParams};
use crate::heights::HeightGrid;
use crate::host::{CoverBackdrop, CoverHeights, CoverSurfaceMesh, CoverTerrain, HeightSource};
use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::{Aabb, Frustum};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::sync::Arc;

const CHUNK_M: f32 = 16.0;
const TRACK_TEXELS_PER_M: f32 = 8.0;

pub struct RuntimeField {
    pub entity: Entity,
    pub name: String,
    pub bounds: FieldBounds,
    pub profile: Arc<FieldProfile>,
    pub ground: Arc<dyn GroundSurface>,
}

#[derive(Resource)]
pub struct ActiveFields {
    pub terrain: Entity,
    pub space: u64,
    pub fields: Vec<RuntimeField>,
    pub background: Arc<dyn GroundSurface>,
}

#[derive(Resource, Default)]
pub struct VegetationChunks(HashMap<(Entity, usize, i32, i32), Entity>);

#[derive(Component)]
pub struct SurfaceParts(Vec<Entity>);

// Two channels for a wheel map, four where the cover prints tyre tread.
fn track_image(width: u32, height: u32, tread: bool) -> Image {
    let (texel, format): (&[u8], _) = if tread {
        (&[0u8; 8], TextureFormat::Rgba16Uint)
    } else {
        (&[0u8; 4], TextureFormat::Rg16Uint)
    };
    Image::new_fill(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        texel,
        format,
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn heightmap_image(grid: &HeightGrid) -> Image {
    let mut texels = Vec::<f32>::with_capacity(grid.cols * grid.rows * 4);
    for i in 0..grid.rows {
        for j in 0..grid.cols {
            let normal = grid.normal_at_index(i, j);
            texels.extend_from_slice(&[grid.at(i, j), normal.x, normal.z, 0.0]);
        }
    }
    Image::new(
        Extent3d {
            width: grid.cols as u32,
            height: grid.rows as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        bytemuck::cast_slice(&texels).to_vec(),
        TextureFormat::Rgba32Float,
        RenderAssetUsages::RENDER_WORLD,
    )
}

pub fn ensure_fields(world: &mut World) {
    let terrain = world
        .get_resource::<CoverTerrain>()
        .map(|terrain| (terrain.entity, terrain.grid.clone(), terrain.space));
    let Some((root, grid, space)) = terrain else {
        if world.remove_resource::<ActiveFields>().is_some() {
            world.resource_mut::<RenderFields>().0.clear();
            world.resource_mut::<VegetationChunks>().0.clear();
        }
        return;
    };
    if world
        .get_resource::<ActiveFields>()
        .is_some_and(|fields| fields.terrain == root)
    {
        // Retired fields are held only until the new ones have taken their tracks.
        let retired = &mut world.resource_mut::<RenderFields>().1;
        retired.frames_left = retired.frames_left.saturating_sub(1);
        if retired.frames_left == 0 && !retired.fields.is_empty() {
            retired.fields.clear();
        }
        return;
    }
    let same_space = world
        .get_resource::<ActiveFields>()
        .is_some_and(|fields| fields.space == space);
    let mut render = world.resource_mut::<RenderFields>();
    let retired: Vec<_> = render.0.drain().map(|(_, field)| field).collect();
    render.1 = super::render::RetiredFields {
        fields: if same_space { retired } else { Vec::new() },
        frames_left: 240,
    };
    world.resource_mut::<VegetationChunks>().0.clear();
    let domain = FieldBounds {
        min: Vec2::new(grid.min_x, grid.min_z),
        max: Vec2::new(grid.min_x, grid.min_z) + Vec2::splat(grid.half_size() * 2.0),
    };
    let layout = world.resource::<FieldLayout>().clone();
    layout
        .validate(world.resource::<FieldProfiles>())
        .expect("valid field layout");
    let profiles = world.resource::<FieldProfiles>().0.clone();
    let heightmap = world
        .resource_mut::<Assets<Image>>()
        .add(heightmap_image(&grid));
    let surface_geometry = super::profile::SurfaceGeometry {
        heightmap: heightmap.clone(),
        params: super::profile::SurfaceGeometryParams {
            origin: Vec2::new(grid.min_x, grid.min_z),
            texels_per_metre: 1.0 / grid.cell,
            texel_count: grid.cols as f32,
        },
    };
    let mut fields = Vec::new();
    for spec in layout.regions(domain) {
        let bounds = spec.bounds();
        let profile = profiles[&spec.profile].clone();
        let size = bounds.max - bounds.min;
        let width = (size.x * TRACK_TEXELS_PER_M).ceil() as u32 + 1;
        let height = (size.y * TRACK_TEXELS_PER_M).ceil() as u32 + 1;
        let trample = world
            .resource_mut::<Assets<Image>>()
            .add(track_image(width, height, profile.wheel_response.tread));
        let response = profile.wheel_response;
        let wheels = WheelMapParams {
            origin: bounds.min,
            texels_per_metre: TRACK_TEXELS_PER_M,
            width: width as f32,
            height: height as f32,
            recovery_seconds: response.recovery_seconds,
            bend: response.bend,
            darkening: response.darkening,
        };
        let ground = (profile.ground)(world, trample.clone(), wheels, surface_geometry.clone());
        let entity = world
            .spawn((
                Name::new(format!("Field {} ({})", spec.name, profile.name)),
                ChildOf(root),
                Transform::IDENTITY,
                Visibility::default(),
            ))
            .id();
        world.resource_mut::<RenderFields>().0.insert(
            entity,
            FieldGpu {
                heightmap: heightmap.clone(),
                trample,
                footprint_length: response.footprint_length,
                tread: response.tread,
                params: VegetationParams {
                    origin: Vec2::new(grid.min_x, grid.min_z),
                    texels_per_metre: 1.0 / grid.cell,
                    chunk_size: CHUNK_M,
                    texel_count: grid.cols as f32,
                    bounds: Vec4::new(bounds.min.x, bounds.min.y, bounds.max.x, bounds.max.y),
                    wheels,
                    ..default()
                },
            },
        );
        info!(
            "field {}: {} [{:?}..{:?}], independent {}x{} wheel map",
            spec.name, profile.name, bounds.min, bounds.max, width, height
        );
        fields.push(RuntimeField {
            entity,
            name: spec.name,
            bounds,
            profile,
            ground,
        });
    }
    let background_track = world.resource_mut::<Assets<Image>>().add(track_image(2, 2, false));
    let background = (profiles[&layout.default].ground)(
        world,
        background_track,
        WheelMapParams {
            origin: Vec2::splat(1_000_000.0),
            width: 2.0,
            height: 2.0,
            texels_per_metre: 1.0,
            recovery_seconds: 300.0,
            ..default()
        },
        surface_geometry,
    );
    world.insert_resource(ActiveFields {
        terrain: root,
        space,
        fields,
        background,
    });
    // Surfaces matched against the fields before these are matched again.
    let mut matched = world.query::<(Entity, &SurfaceParts)>();
    let stale: Vec<(Entity, Vec<Entity>)> = matched
        .iter(world)
        .map(|(entity, parts)| (entity, parts.0.clone()))
        .collect();
    for (entity, parts) in stale {
        for part in parts {
            if let Ok(part) = world.get_entity_mut(part) {
                part.despawn();
            }
        }
        world.entity_mut(entity).remove::<SurfaceParts>();
    }
}

pub fn assign_surfaces(
    mut commands: Commands,
    active: Option<Res<ActiveFields>>,
    mut meshes: ResMut<Assets<Mesh>>,
    sources: Query<
        (
            Entity,
            &Mesh3d,
            Option<&CoverBackdrop>,
            Option<&SurfaceParts>,
        ),
        (
            With<CoverSurfaceMesh>,
            Or<(Changed<Mesh3d>, Without<SurfaceParts>)>,
        ),
    >,
) {
    let Some(active) = active else {
        return;
    };
    for (entity, source, backdrop, previous) in &sources {
        if backdrop.is_some() {
            active.background.apply(&mut commands, entity);
            commands.entity(entity).insert(SurfaceParts(Vec::new()));
            continue;
        }
        let Some(mesh) = meshes.get(&source.0) else {
            continue;
        };
        let Some(bounds) = mesh_bounds(mesh) else {
            continue;
        };
        let mut parts = Vec::new();
        let mut new_meshes = Vec::new();
        for field in &active.fields {
            if !field.bounds.overlaps(bounds) {
                continue;
            }
            if field.bounds.contains(bounds.min) && field.bounds.contains(bounds.max) {
                new_meshes.push((field, None));
            } else if let Some(clipped) = clip_mesh(mesh, field.bounds) {
                new_meshes.push((field, Some(clipped)));
            }
        }
        if let Some(previous) = previous {
            for &part in &previous.0 {
                commands.entity(part).despawn();
            }
        }
        for (field, clipped) in new_meshes {
            let handle = clipped
                .map(|mesh| meshes.add(mesh))
                .unwrap_or_else(|| source.0.clone());
            let part = commands
                .spawn((
                    Name::new(format!("{} surface", field.name)),
                    ChildOf(entity),
                    Transform::IDENTITY,
                    Mesh3d(handle),
                    bevy::light::NotShadowCaster,
                ))
                .id();
            field.ground.apply(&mut commands, part);
            parts.push(part);
        }
        commands.entity(entity).insert(SurfaceParts(parts));
    }
}

pub fn instance_budget(
    capacity: f32,
    distance: f32,
    start: f32,
    end: f32,
    inverse_square: bool,
) -> u32 {
    let fade = ((end - distance) / (end - start)).clamp(0.0, 1.0);
    let projected = if inverse_square {
        start / distance.max(start)
    } else {
        1.0
    };
    (capacity * (fade * projected).powi(2)).ceil() as u32
}

/// Chunk side for a layer: detail levels near the camera stream in small
/// chunks so their density follows distance instead of the whole chunk.
fn chunk_m(layer: &VegetationLayer) -> f32 {
    (layer.lod_band[1] / 8.0).clamp(4.0, CHUNK_M)
}

fn farthest_distance(bounds: &FieldBounds, point: Vec2) -> f32 {
    let far = Vec2::new(
        (point.x - bounds.min.x).abs().max((point.x - bounds.max.x).abs()),
        (point.y - bounds.min.y).abs().max((point.y - bounds.max.y).abs()),
    );
    far.length()
}

/// Terrain height span of a chunk, with room for vegetation above it.
fn chunk_height_span(corner: Vec2, size: f32, ground: &dyn HeightSource) -> (f32, f32) {
    let samples = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0), (0.5, 0.5)]
        .map(|(u, v)| ground.height(corner.x + u * size, corner.y + v * size));
    let low = samples.iter().copied().fold(f32::INFINITY, f32::min);
    let high = samples.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    (low - 2.0, high + 2.0)
}

pub fn stream_vegetation(
    mut commands: Commands,
    active: Option<Res<ActiveFields>>,
    cameras: Query<(&GlobalTransform, &Frustum), With<Camera3d>>,
    mut chunks: ResMut<VegetationChunks>,
    mut meshes: ResMut<Assets<Mesh>>,
    assets: Res<AssetServer>,
    mut draws: Query<&mut VegetationChunk>,
    mut layer_meshes: Local<HashMap<(&'static str, usize), (Handle<Mesh>, Vec<std::ops::Range<u32>>)>>,
    mut heights: Local<HashMap<(i32, i32, u32), (f32, f32)>>,
    ground: Option<Res<CoverHeights>>,
) {
    let Some(active) = active else {
        return;
    };
    let Some(ground) = ground else {
        return;
    };
    let Some((camera, frustum)) = cameras.iter().next() else {
        return;
    };
    let eye = camera.translation().xz();
    let eye_y = camera.translation().y;
    let mut wanted: HashMap<_, _> = HashMap::default();
    for field in &active.fields {
        for (layer_index, layer) in field.profile.layers.iter().enumerate() {
            if layer.density <= 0.0 || field.bounds.nearest_distance(eye) >= layer.fade_end {
                continue;
            }
            let size = chunk_m(layer);
            let min = (eye - Vec2::splat(layer.fade_end)).max(field.bounds.min);
            let max = (eye + Vec2::splat(layer.fade_end)).min(field.bounds.max);
            for z in (min.y / size).floor() as i32..=(max.y / size).floor() as i32 {
                for x in (min.x / size).floor() as i32..=(max.x / size).floor() as i32 {
                    let corner = Vec2::new(x as f32, z as f32) * size;
                    let bounds = FieldBounds {
                        min: corner.max(field.bounds.min),
                        max: (corner + Vec2::splat(size)).min(field.bounds.max),
                    };
                    if !bounds.min.cmplt(bounds.max).all() {
                        continue;
                    }
                    let (low, high) = *heights
                        .entry((x, z, size.to_bits()))
                        .or_insert_with(|| chunk_height_span(corner, size, ground.0.as_ref()));
                    // Blades fade by distance to the eye, so height above them counts.
                    let lift = (eye_y - high).max(low - eye_y).max(0.0);
                    let nearest = bounds.nearest_distance(eye).hypot(lift);
                    let farthest = farthest_distance(&bounds, eye).hypot((eye_y - low).abs().max((eye_y - high).abs()));
                    // Beyond this detail level's band the next mesh takes over.
                    if nearest > layer.lod_band[1] + 2.0 || farthest < layer.lod_band[0] - 2.0 {
                        continue;
                    }
                    let capacity = size * size * layer.density;
                    let instances = instance_budget(
                        capacity,
                        nearest,
                        layer.fade_start,
                        layer.fade_end,
                        layer.inverse_square_thinning,
                    );
                    if instances == 0 {
                        continue;
                    }
                    // Chunks outside the view keep their slot but draw nothing.
                    let extent = Aabb::from_min_max(
                        Vec3::new(corner.x, low, corner.y),
                        Vec3::new(corner.x + size, high, corner.y + size),
                    );
                    let instances = if frustum.intersects_obb_identity(&extent) { instances } else { 0 };
                    let key = (field.entity, layer_index, x, z);
                    wanted.insert(key, (field, layer, size, capacity, instances));
                }
            }
        }
    }
    chunks.0.retain(|key, entity| {
        if wanted.contains_key(key) {
            true
        } else {
            commands.entity(*entity).despawn();
            false
        }
    });
    for (key, (field, layer, size, capacity, instances)) in wanted {
        if let Some(&entity) = chunks.0.get(&key) {
            if let Ok(mut draw) = draws.get_mut(entity) {
                draw.instances = instances;
            }
            continue;
        }
        let corner = Vec2::new(key.2 as f32, key.3 as f32) * size;
        // One mesh per layer, built once: chunks only place its instances.
        let (mesh, variants) = layer_meshes
            .entry((field.profile.name, key.1))
            .or_insert_with(|| {
                let mesh = (layer.template)();
                let variants = super::clumps::variant_ranges(&mesh);
                (meshes.add(mesh), variants)
            })
            .clone();
        let entity = commands
            .spawn((
                Name::new(format!(
                    "{} vegetation {} [{},{}]",
                    field.name, key.1, key.2, key.3
                )),
                ChildOf(field.entity),
                VegetationChunk {
                    corner,
                    size,
                    instances,
                    capacity,
                    field_id: field.entity,
                    shader: assets.load(layer.shader),
                    mesh,
                    fade_start: layer.fade_start,
                    fade_end: layer.fade_end,
                    inverse_square_thinning: layer.inverse_square_thinning,
                    albedo: layer.albedo.map(|path| assets.load(path)),
                    variants,
                },
            ))
            .id();
        chunks.0.insert(key, entity);
    }
}
