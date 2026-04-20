//! Hierarchical LOD grid on the sphere surface.
//!
//! Seven fixed zoom levels are always in the scene, each with
//! `2·GRID_LINES_EACH_SIDE + 1` lat rings and the same number of lon
//! meridians. Each level's cell size is a fixed decade:
//! `1 m, 10 m, 100 m, 1 km, 10 km, 100 km, 1 000 km`.
//!
//! Meshes are built once as soon as the player vehicle exists, then
//! each frame we simply toggle `Visibility` based on the ratio
//! `cam_dist / step_k` — a level is only shown when its cells would be
//! between ~a half-view-width and ~a hundredth of a view-width in
//! size. The effect is that zooming in "refines" the grid and zooming
//! out "coarsens" it, with 2–3 levels overlapping at the transitions
//! so it feels continuous rather than stepped.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::math::{DQuat, DVec3};
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;
use big_space::prelude::BigSpatialBundle;

use super::camera::ChaseCamera;
use super::{GearboxSim, PlayerControlled, VehicleBody};

#[derive(Resource, Clone, Copy)]
pub struct GroundGrid {
    pub lat_color: Color,
    pub lon_color: Color,
}

impl Default for GroundGrid {
    fn default() -> Self {
        let blue = Color::srgba(0.10, 0.45, 1.00, 0.5);
        Self { lat_color: blue, lon_color: blue }
    }
}

/// One decade per level.
pub const GRID_STEPS_M: [f64; 7] = [
    1.0, 10.0, 100.0, 1_000.0, 10_000.0, 100_000.0, 1_000_000.0,
];
/// Lines per side, per level. Total per level = 2·N + 1.
pub const GRID_LINES_EACH_SIDE: i32 = 50;
/// Rings with `|ring| <= GRID_INNER_RINGS` keep full intensity;
/// beyond that they fade linearly out to full transparency at
/// `GRID_LINES_EACH_SIDE`.
pub const GRID_INNER_RINGS: i32 = 25;

/// Fade curve: Gaussian bell in log10(cam_dist / step) space.
/// Peak at `GAUSSIAN_PEAK_LOG_R` means the level's cells fit the view
/// best there. `GAUSSIAN_WIDTH` controls how fast the bell falls off:
/// narrower → sharper hand-off between levels; wider → more neighbours
/// visible. 0.85 gives ~25 % alpha one decade from peak — i.e. the two
/// LOD neighbours are visible but clearly secondary.
const GAUSSIAN_PEAK_LOG_R: f64 = 1.0;
const GAUSSIAN_WIDTH: f64 = 0.85;

/// Per-level material handles, so the fade system can tweak alpha in
/// one place instead of iterating every circle entity.
#[derive(Resource)]
pub struct GridMaterials {
    pub lat: Vec<Handle<StandardMaterial>>,
    pub lon: Vec<Handle<StandardMaterial>>,
}

#[derive(Component, Copy, Clone)]
pub struct LatCircle {
    pub level: u8,
    pub ring: i32,
}
#[derive(Component, Copy, Clone)]
pub struct LonCircle {
    pub level: u8,
    pub ring: i32,
}

fn lat_lon_unit_f64(lat_deg: f64, lon_deg: f64) -> DVec3 {
    let t = lat_deg.to_radians();
    let p = lon_deg.to_radians();
    DVec3::new(t.cos() * p.cos(), t.sin(), t.cos() * p.sin())
}

pub fn rotation_from_latlon_to_top(lat_deg: f64, lon_deg: f64) -> Quat {
    let from = lat_lon_unit_f64(lat_deg, lon_deg).as_vec3().normalize();
    if (from - Vec3::Y).length_squared() < 1e-12 {
        return Quat::IDENTITY;
    }
    Quat::from_rotation_arc(from, Vec3::Y)
}

fn rotation_f64(lat_deg: f64, lon_deg: f64) -> DQuat {
    let from = lat_lon_unit_f64(lat_deg, lon_deg).normalize();
    if (from - DVec3::Y).length_squared() < 1e-20 {
        return DQuat::IDENTITY;
    }
    DQuat::from_rotation_arc(from, DVec3::Y)
}

const SEGS: u32 = 4096;

fn empty_line_mesh() -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::LineStrip,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    let empty_pos: Vec<[f32; 3]> = Vec::new();
    let empty_col: Vec<[f32; 4]> = Vec::new();
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, empty_pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, empty_col);
    mesh
}

/// Spatial alpha for a ring with given distance from centre. 1.0
/// throughout the "inner" band, smoothstep-fades to 0 at the outer
/// edge. Baked into the mesh once, then multiplied against the
/// per-level Gaussian fade at render time by StandardMaterial's
/// automatic vertex-colour blend.
fn ring_spatial_alpha(ring: i32) -> f32 {
    let a = ring.abs() as f32;
    if a <= GRID_INNER_RINGS as f32 {
        1.0
    } else {
        let t = ((GRID_LINES_EACH_SIDE as f32 - a)
            / (GRID_LINES_EACH_SIDE - GRID_INNER_RINGS) as f32)
            .clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }
}

fn build_line_strip(
    us: &[DVec3],
    planet_rot: DQuat,
    r_circle: f64,
    sphere_centre: DVec3,
    anchor_world: DVec3,
    spatial_alpha: f32,
) -> Mesh {
    let positions: Vec<[f32; 3]> = us
        .iter()
        .map(|u| {
            let world = planet_rot * *u * r_circle + sphere_centre;
            let offset = world - anchor_world;
            [offset.x as f32, offset.y as f32, offset.z as f32]
        })
        .collect();
    let colors: Vec<[f32; 4]> =
        vec![[1.0, 1.0, 1.0, spatial_alpha]; positions.len()];

    let mut mesh = Mesh::new(
        PrimitiveTopology::LineStrip,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh
}

pub fn spawn_circle_meshes(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    big_space_root: Entity,
    cfg: &GroundGrid,
) {
    // One material per level so the fade system can write one alpha
    // per level instead of poking every entity. Starts fully
    // transparent — the fade system brings them in.
    let mut lat_mats: Vec<Handle<StandardMaterial>> = Vec::with_capacity(GRID_STEPS_M.len());
    let mut lon_mats: Vec<Handle<StandardMaterial>> = Vec::with_capacity(GRID_STEPS_M.len());
    for _ in 0..GRID_STEPS_M.len() {
        lat_mats.push(materials.add(StandardMaterial {
            base_color: cfg.lat_color.with_alpha(0.0),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            depth_bias: 1000.0,
            ..default()
        }));
        lon_mats.push(materials.add(StandardMaterial {
            base_color: cfg.lon_color.with_alpha(0.0),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            depth_bias: 1000.0,
            ..default()
        }));
    }

    for level_idx in 0..GRID_STEPS_M.len() {
        let level = level_idx as u8;
        let lat_mat = lat_mats[level_idx].clone();
        let lon_mat = lon_mats[level_idx].clone();
        for ring in -GRID_LINES_EACH_SIDE..=GRID_LINES_EACH_SIDE {
            let lat_mesh = meshes.add(empty_line_mesh());
            commands
                .spawn((
                    Name::new(format!("LatCircle[L{} r{:+}]", level, ring)),
                    BigSpatialBundle::default(),
                    Mesh3d(lat_mesh),
                    MeshMaterial3d(lat_mat.clone()),
                    NoFrustumCulling,
                    LatCircle { level, ring },
                ))
                .insert(ChildOf(big_space_root));

            let lon_mesh = meshes.add(empty_line_mesh());
            commands
                .spawn((
                    Name::new(format!("LonCircle[L{} r{:+}]", level, ring)),
                    BigSpatialBundle::default(),
                    Mesh3d(lon_mesh),
                    MeshMaterial3d(lon_mat.clone()),
                    NoFrustumCulling,
                    LonCircle { level, ring },
                ))
                .insert(ChildOf(big_space_root));
        }
    }

    commands.insert_resource(GridMaterials { lat: lat_mats, lon: lon_mats });
}

/// Build every ring ONCE, as soon as the player vehicle shows up.
pub fn build_grid_meshes(
    mut meshes: ResMut<Assets<Mesh>>,
    sim: Res<GearboxSim>,
    players: Query<&VehicleBody, With<PlayerControlled>>,
    mut lat_q: Query<
        (&LatCircle, &Mesh3d, &mut Transform),
        (Without<LonCircle>, Without<ChaseCamera>),
    >,
    mut lon_q: Query<
        (&LonCircle, &Mesh3d, &mut Transform),
        (Without<LatCircle>, Without<ChaseCamera>),
    >,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Ok(player) = players.single() else { return };

    let veh_pose = sim.0.vehicle_pose(player.id);
    let veh_world = DVec3::new(veh_pose.point.x, veh_pose.point.y, veh_pose.point.z);
    let sphere_centre = DVec3::new(0.0, -sim.0.planet.radius, 0.0);
    let planet_rot = rotation_f64(sim.0.planet.datum.latitude, sim.0.planet.datum.longitude);
    let inv_rot = planet_rot.inverse();
    let to_veh = veh_world - sphere_centre;
    let r_machine = to_veh.length().max(1.0);
    let r_circle = sim.0.planet.radius + 0.01;
    let dir = to_veh / r_machine;
    let unrotated = inv_rot * dir;
    let lat_rad = unrotated.y.clamp(-1.0, 1.0).asin();
    let lon_rad = unrotated.z.atan2(unrotated.x);
    let machine_pos_f32 = veh_world.as_vec3();
    let inv_r = 1.0 / sim.0.planet.radius;
    let inv_r_cos_lat = 1.0 / (sim.0.planet.radius * lat_rad.cos().abs().max(1e-9));

    // Lat max just shy of ±90° so degenerate pole-rings collapse to a
    // point rather than flipping through the pole.
    const LAT_LIMIT: f64 = std::f64::consts::FRAC_PI_2 - 1e-4;

    // --- Latitude parallels ---
    for (lat_c, mesh_h, mut tr) in lat_q.iter_mut() {
        tr.translation = machine_pos_f32;
        tr.rotation = Quat::IDENTITY;
        tr.scale = Vec3::ONE;

        let step = GRID_STEPS_M[lat_c.level as usize];
        let this_lat = lat_rad + lat_c.ring as f64 * step * inv_r;
        // If this ring walks past the pole, emit an empty mesh so it
        // draws nothing (still exists as an entity, just invisible).
        if this_lat.abs() > LAT_LIMIT {
            if let Some(mesh) = meshes.get_mut(&mesh_h.0) {
                *mesh = empty_line_mesh();
            }
            continue;
        }
        let cl = this_lat.cos();
        let sl = this_lat.sin();
        let mut us: Vec<DVec3> = Vec::with_capacity(SEGS as usize + 1);
        for i in 0..=SEGS {
            let phi = lon_rad + (i as f64 / SEGS as f64) * std::f64::consts::TAU;
            us.push(DVec3::new(cl * phi.cos(), sl, cl * phi.sin()));
        }
        let alpha = ring_spatial_alpha(lat_c.ring);
        let new_mesh = build_line_strip(&us, planet_rot, r_circle, sphere_centre, veh_world, alpha);
        if let Some(mesh) = meshes.get_mut(&mesh_h.0) {
            *mesh = new_mesh;
        }
    }

    // --- Longitude meridians ---
    for (lon_c, mesh_h, mut tr) in lon_q.iter_mut() {
        tr.translation = machine_pos_f32;
        tr.rotation = Quat::IDENTITY;
        tr.scale = Vec3::ONE;

        let step = GRID_STEPS_M[lon_c.level as usize];
        let this_lon = lon_rad + lon_c.ring as f64 * step * inv_r_cos_lat;
        let cn = this_lon.cos();
        let sn = this_lon.sin();
        let theta0 = std::f64::consts::FRAC_PI_2 - lat_rad;
        let mut us: Vec<DVec3> = Vec::with_capacity(SEGS as usize + 1);
        for i in 0..=SEGS {
            let theta = theta0 + (i as f64 / SEGS as f64) * std::f64::consts::TAU;
            let (st, ct) = theta.sin_cos();
            us.push(DVec3::new(st * cn, ct, st * sn));
        }
        let alpha = ring_spatial_alpha(lon_c.ring);
        let new_mesh = build_line_strip(&us, planet_rot, r_circle, sphere_centre, veh_world, alpha);
        if let Some(mesh) = meshes.get_mut(&mesh_h.0) {
            *mesh = new_mesh;
        }
    }

    *done = true;
}

/// Gaussian fade centred at `GAUSSIAN_PEAK_LOG_R`. Returns 1 at peak,
/// ~0.25 one decade away, ~0.002 two decades away.
fn level_fade(cam_dist: f64, step: f64) -> f32 {
    let log_r = (cam_dist / step).max(1e-12).log10();
    let x = (log_r - GAUSSIAN_PEAK_LOG_R) / GAUSSIAN_WIDTH;
    (-x * x).exp() as f32
}

/// Per-frame: compute each level's alpha from camera distance and
/// write it into that level's shared StandardMaterial. Entities don't
/// need to be touched at all — they already reference the right
/// handle. Just 14 colour writes a frame.
pub fn update_grid_alpha(
    mut materials: ResMut<Assets<StandardMaterial>>,
    grid_mats: Option<Res<GridMaterials>>,
    cfg: Res<GroundGrid>,
    cameras: Query<&ChaseCamera>,
) {
    let Some(grid_mats) = grid_mats else { return };
    let Ok(cam) = cameras.single() else { return };
    let cam_dist = cam.distance as f64;

    let base_lat = cfg.lat_color.alpha();
    let base_lon = cfg.lon_color.alpha();

    for (level_idx, step) in GRID_STEPS_M.iter().enumerate() {
        let fade = level_fade(cam_dist, *step);
        if let Some(mat) = materials.get_mut(&grid_mats.lat[level_idx]) {
            mat.base_color = cfg.lat_color.with_alpha(base_lat * fade);
        }
        if let Some(mat) = materials.get_mut(&grid_mats.lon[level_idx]) {
            mat.base_color = cfg.lon_color.with_alpha(base_lon * fade);
        }
    }
}
