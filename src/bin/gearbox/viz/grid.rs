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
use super::GearboxSim;

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

const SEGS: u32 = 512;

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

/// World-space position of the chase camera's *focus* (the point it
/// is looking at). We anchor the grid here rather than at the eye so
/// that orbiting the camera — which only changes yaw/elevation, not
/// `cam.focus` — doesn't drag the grid with it.
fn chase_camera_focus_world(cam: &ChaseCamera) -> DVec3 {
    DVec3::new(cam.focus.x as f64, cam.focus.y as f64, cam.focus.z as f64)
}

/// Rebuild grid circles every frame, centred on the current camera
/// eye. Heavy optimisations keep this cheap:
///   1. Invisible levels (Gaussian fade near 0) are skipped entirely.
///   2. A single cos/sin parameter table is computed once per rebuild
///      and shared across every ring on both axes.
///   3. The planet rotation is baked into three pre-scaled DVec3 column
///      vectors so per-vertex math is just two scale-and-add ops.
pub fn build_grid_meshes(
    mut meshes: ResMut<Assets<Mesh>>,
    sim: Res<GearboxSim>,
    cameras: Query<&ChaseCamera>,
    mut lat_q: Query<
        (&LatCircle, &Mesh3d, &mut Transform),
        (Without<LonCircle>, Without<ChaseCamera>),
    >,
    mut lon_q: Query<
        (&LonCircle, &Mesh3d, &mut Transform),
        (Without<LatCircle>, Without<ChaseCamera>),
    >,
    mut trig_table: Local<Vec<(f64, f64)>>,
) {
    let Ok(cam) = cameras.single() else { return };
    let cam_world = chase_camera_focus_world(cam);
    let cam_dist = cam.distance as f64;

    let sphere_centre = DVec3::new(0.0, -sim.0.planet.radius, 0.0);
    let planet_rot = rotation_f64(sim.0.planet.datum.latitude, sim.0.planet.datum.longitude);
    let inv_rot = planet_rot.inverse();
    let to_cam = cam_world - sphere_centre;
    let r_cam = to_cam.length().max(1.0);
    let r_circle = sim.0.planet.radius + 0.01;
    let dir = to_cam / r_cam;
    let unrotated = inv_rot * dir;
    let lat_rad = unrotated.y.clamp(-1.0, 1.0).asin();
    let lon_rad = unrotated.z.atan2(unrotated.x);
    let cam_pos_f32 = cam_world.as_vec3();
    let inv_r = 1.0 / sim.0.planet.radius;
    let inv_r_cos_lat = 1.0 / (sim.0.planet.radius * lat_rad.cos().abs().max(1e-9));

    // Precompute the three columns of (planet_rot * r_circle) so the
    // inner loop reduces to `cos_phi * col_x + sin_phi * col_z + y_term`.
    let col_x = planet_rot * DVec3::X * r_circle;
    let col_y = planet_rot * DVec3::Y * r_circle;
    let col_z = planet_rot * DVec3::Z * r_circle;

    // Per-rebuild constant offset: shifts everything into anchor-relative
    // coordinates (f32 is fine near anchor, huge otherwise).
    let delta = sphere_centre - cam_world;

    // (cos φ, sin φ) table. Use the phase offset `lon_rad` as a base for
    // lat rings; lon meridians apply their own phase (theta0). For this
    // reason we store `(cos(phase + step_i), sin(phase + step_i))` without
    // a baked phase — phase is handled per-ring.
    let segs = SEGS as usize;
    trig_table.clear();
    trig_table.reserve(segs + 1);
    let inv_segs = 1.0 / segs as f64;
    for i in 0..=segs {
        let t = i as f64 * inv_segs * std::f64::consts::TAU;
        trig_table.push(t.sin_cos()); // (sin t, cos t)
    }
    // Borrow as slice so closures don't re-borrow the Local.
    let trig: &[(f64, f64)] = &trig_table[..];

    // For phase ψ and ring angle t: cos(ψ+t) = cos ψ cos t − sin ψ sin t,
    // sin(ψ+t) = sin ψ cos t + cos ψ sin t.  Do the phase mix here.
    let (sin_lon, cos_lon) = lon_rad.sin_cos();
    // Meridian sweep starts at theta0 = π/2 − lat so vertex 0 is on the
    // machine's latitude; we fold that into a (sin, cos) phase too.
    let theta0 = std::f64::consts::FRAC_PI_2 - lat_rad;
    let (sin_t0, cos_t0) = theta0.sin_cos();

    const LAT_LIMIT: f64 = std::f64::consts::FRAC_PI_2 - 1e-4;

    // Small helper: 10× faster than `build_line_strip` because it
    // doesn't re-allocate — we reuse caller-owned buffers.
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(segs + 1);
    let mut colors: Vec<[f32; 4]> = Vec::with_capacity(segs + 1);

    // --- Latitude parallels ---
    for (lat_c, mesh_h, mut tr) in lat_q.iter_mut() {
        tr.translation = cam_pos_f32;
        tr.rotation = Quat::IDENTITY;
        tr.scale = Vec3::ONE;

        let step = GRID_STEPS_M[lat_c.level as usize];
        // Skip levels that are essentially invisible at this zoom.
        if level_fade(cam_dist, step) < 0.005 {
            continue;
        }

        // World-fixed grid: snap the camera's lat to a multiple of the
        // level's lat step, then stack rings outward from that snap.
        // Between snaps the grid is stationary in world — which is
        // what the robot and the globe are already doing, so nothing
        // slides relative to anything else.
        let step_lat = step * inv_r;
        let snapped_lat = (lat_rad / step_lat).round() * step_lat;
        let this_lat = snapped_lat + lat_c.ring as f64 * step_lat;
        if this_lat.abs() > LAT_LIMIT {
            if let Some(mesh) = meshes.get_mut(&mesh_h.0) {
                *mesh = empty_line_mesh();
            }
            continue;
        }
        let cl = this_lat.cos();
        let sl = this_lat.sin();
        let alpha = ring_spatial_alpha(lat_c.ring);

        positions.clear();
        colors.clear();
        // vertex = cl * cos(lon_rad + t) * col_x
        //        + sl * col_y
        //        + cl * sin(lon_rad + t) * col_z
        //        + delta
        // Using the sum-angle identity with precomputed (sin_lon, cos_lon).
        let y_term = col_y * sl + delta;
        let sx = col_x * cl;
        let sz = col_z * cl;
        for &(sin_t, cos_t) in trig {
            let c_phi = cos_lon * cos_t - sin_lon * sin_t;
            let s_phi = sin_lon * cos_t + cos_lon * sin_t;
            let v = sx * c_phi + sz * s_phi + y_term;
            positions.push([v.x as f32, v.y as f32, v.z as f32]);
            colors.push([1.0, 1.0, 1.0, alpha]);
        }
        if let Some(mesh) = meshes.get_mut(&mesh_h.0) {
            *mesh = make_line_mesh(&positions, &colors);
        }
    }

    // --- Longitude meridians ---
    for (lon_c, mesh_h, mut tr) in lon_q.iter_mut() {
        tr.translation = cam_pos_f32;
        tr.rotation = Quat::IDENTITY;
        tr.scale = Vec3::ONE;

        let step = GRID_STEPS_M[lon_c.level as usize];
        if level_fade(cam_dist, step) < 0.005 {
            continue;
        }

        // Same world-fixed snap for meridians.
        let step_lon = step * inv_r_cos_lat;
        let snapped_lon = (lon_rad / step_lon).round() * step_lon;
        let this_lon = snapped_lon + lon_c.ring as f64 * step_lon;
        let (sn, cn) = this_lon.sin_cos();
        let alpha = ring_spatial_alpha(lon_c.ring);

        // vertex = sin(theta0+t) * (cn*col_x + sn*col_z) + cos(theta0+t) * col_y + delta
        let axis = col_x * cn + col_z * sn;
        positions.clear();
        colors.clear();
        for &(sin_t, cos_t) in trig {
            let s_th = sin_t0 * cos_t + cos_t0 * sin_t;
            let c_th = cos_t0 * cos_t - sin_t0 * sin_t;
            let v = axis * s_th + col_y * c_th + delta;
            positions.push([v.x as f32, v.y as f32, v.z as f32]);
            colors.push([1.0, 1.0, 1.0, alpha]);
        }
        if let Some(mesh) = meshes.get_mut(&mesh_h.0) {
            *mesh = make_line_mesh(&positions, &colors);
        }
    }
}

fn make_line_mesh(positions: &[[f32; 3]], colors: &[[f32; 4]]) -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::LineStrip,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions.to_vec());
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors.to_vec());
    mesh
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
