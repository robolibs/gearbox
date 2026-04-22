//! Local LOD ground grid — a stack of flat square grids of lines
//! centred on the chase-camera's focus.
//!
//! Each level uses a fixed decade spacing (1 m, 10 m, 100 m, 1 km);
//! the level whose cell size best matches the current view scale
//! fades in, neighbours fade down, and far levels disappear. Unlike
//! the old sphere-surface system, the meshes here are built once at
//! startup and just translate with the camera — there's no per-frame
//! vertex rebuild, so panning stays smooth.
//!
//! Fades:
//!   - Per-level Gaussian on `log(cam_dist / step)` — peaks at the
//!     level whose cell size is ~10× the camera distance.
//!   - Radial in the mesh: alpha drops with distance from centre, so
//!     the outer edge dissolves into the ground instead of ending in
//!     a hard square border.
//!   - Major-line boost every 10th line reads as a "chapter" tick
//!     without becoming noisy.

use bevy::asset::RenderAssetUsages;
use bevy::light::NotShadowCaster;
use bevy::mesh::PrimitiveTopology;
use bevy::prelude::*;
use big_space::prelude::BigSpatialBundle;

use super::camera::ChaseCamera;

// ── User-visible settings ───────────────────────────────────────────

#[derive(Resource, Clone, Copy)]
pub struct GroundGrid {
    pub visible: bool,
    /// Base RGB + alpha. Alpha scales everything.
    pub color: Color,
}

impl Default for GroundGrid {
    fn default() -> Self {
        Self {
            visible: true,
            color: Color::srgba(80.0 / 255.0, 70.0 / 255.0, 70.0 / 255.0, 0.35),
        }
    }
}

// ── LOD levels ──────────────────────────────────────────────────────

/// Cell size per level (metres). Decades so neighbouring levels sit
/// an order of magnitude apart — matches the old sphere grid's feel.
pub const LEVEL_STEPS: [f32; 4] = [1.0, 10.0, 100.0, 1_000.0];
/// Half-extent of each level's square (metres). Same "lines per
/// side" across levels, so the per-level mesh cost is constant.
pub const LEVEL_HALF: [f32; 4] = [50.0, 500.0, 5_000.0, 50_000.0];
/// Every Nth line is a major line (brighter alpha).
const MAJOR_EVERY: i32 = 10;
/// Major-line alpha boost (multiplied against the base colour alpha).
const MAJOR_BOOST: f32 = 2.2;
/// Grid rides this height above the tangent plane.
const GRID_Y: f32 = 0.05;

/// Peak fade at `log10(cam_dist / step) ≈ GAUSS_PEAK`. 1.0 means a
/// level is at full strength when the camera is 10× its cell size
/// away — i.e. when the cells look ~10 %-of-view-width in size.
const GAUSS_PEAK: f32 = 1.0;
/// Bell width. 0.55 puts the neighbour levels at ~5 % of peak, so
/// the active level clearly dominates but transitions aren't abrupt.
const GAUSS_WIDTH: f32 = 0.55;

// ── Components ──────────────────────────────────────────────────────

#[derive(Component)]
pub struct LocalGrid {
    pub level: u8,
    pub material: Handle<StandardMaterial>,
}

// ── Planet-datum rotation (preserved from the old module) ──────────

pub fn rotation_from_latlon_to_top(lat_deg: f64, lon_deg: f64) -> Quat {
    let lat = (lat_deg as f32).to_radians();
    let lon = (lon_deg as f32).to_radians();
    let dir = Vec3::new(
        lat.cos() * lon.cos(),
        lat.sin(),
        lat.cos() * lon.sin(),
    )
    .normalize();
    Quat::from_rotation_arc(dir, Vec3::Y)
}

// ── Spawn ───────────────────────────────────────────────────────────

/// Spawn one grid entity per LOD level. Name kept for back-compat
/// with `main::setup_scene`.
pub fn spawn_circle_meshes(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    big_space_root: Entity,
    cfg: &GroundGrid,
) {
    for level in 0..LEVEL_STEPS.len() {
        let step = LEVEL_STEPS[level];
        let half = LEVEL_HALF[level];
        let mesh = meshes.add(build_level_mesh(cfg, step, half));
        let mat = materials.add(StandardMaterial {
            base_color: Color::WHITE,
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            cull_mode: None,
            ..default()
        });
        commands
            .spawn((
                Name::new(format!("LocalGrid[L{level}]")),
                LocalGrid { level: level as u8, material: mat.clone() },
                BigSpatialBundle {
                    transform: Transform::from_xyz(0.0, GRID_Y, 0.0),
                    ..default()
                },
                Mesh3d(mesh),
                MeshMaterial3d(mat),
                NotShadowCaster,
                Visibility::Visible,
            ))
            .insert(ChildOf(big_space_root));
    }
}

// ── Per-frame systems ──────────────────────────────────────────────

/// Camera-follow: slide every grid level with the chase-camera
/// focus, snapped to that level's minor step so lines stay
/// world-aligned. Also writes each level's fade to its material's
/// alpha — the level whose cell size matches the current zoom blends
/// in, the rest fade out.
pub fn build_grid_meshes(
    cameras: Query<&ChaseCamera>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    cfg: Res<GroundGrid>,
    mut grids: Query<(&LocalGrid, &mut Transform, &mut Visibility)>,
) {
    let Ok(cam) = cameras.single() else { return };
    let cam_dist = cam.distance.max(0.1);

    for (grid, mut tr, mut vis) in grids.iter_mut() {
        let step = LEVEL_STEPS[grid.level as usize];
        // Snap the grid to the *major* step (10 × minor). That's the
        // spacing at which the mesh's stripe pattern repeats, so the
        // whole grid — minor *and* major lines — always lands on the
        // same world positions and the mesh just translates as a
        // rigid sheet. (Snapping to the minor step shifts which
        // intra-mesh lines are "major" every snap, which read as the
        // grid jumping. Not snapping at all makes it look glued to
        // the camera.) Effective jump = major-step once per
        // major-step of pan.
        let snap_step = step * MAJOR_EVERY as f32;
        tr.translation.x = (cam.focus.x / snap_step).round() * snap_step;
        tr.translation.y = GRID_Y;
        tr.translation.z = (cam.focus.z / snap_step).round() * snap_step;

        let fade = level_fade(cam_dist, step);
        let a = cfg.color.alpha() * fade;
        *vis = if cfg.visible && a > 0.005 {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if let Some(m) = materials.get_mut(&grid.material) {
            let srgba = cfg.color.to_srgba();
            m.base_color = Color::srgba(srgba.red, srgba.green, srgba.blue, a);
        }
    }
}

/// When the UI changes grid colour, rebuild the tiny per-level line
/// meshes so the vertex-colour alpha pattern updates. Infrequent.
pub fn update_grid_alpha(
    cfg: Res<GroundGrid>,
    mut meshes: ResMut<Assets<Mesh>>,
    grids: Query<(&LocalGrid, &Mesh3d)>,
) {
    if !cfg.is_changed() {
        return;
    }
    for (grid, mesh_h) in grids.iter() {
        let step = LEVEL_STEPS[grid.level as usize];
        let half = LEVEL_HALF[grid.level as usize];
        if let Some(m) = meshes.get_mut(&mesh_h.0) {
            *m = build_level_mesh(&cfg, step, half);
        }
    }
}

// ── LOD fade ───────────────────────────────────────────────────────

fn level_fade(cam_dist: f32, step: f32) -> f32 {
    // Gaussian bell in log-space over the ratio `cam_dist / step`.
    // Peak at GAUSS_PEAK, width GAUSS_WIDTH. Near zero outside
    // ~2 × width from the peak.
    let log_r = (cam_dist / step).max(1e-3).log10();
    let z = (log_r - GAUSS_PEAK) / GAUSS_WIDTH;
    (-0.5 * z * z).exp()
}

// ── Mesh generation ────────────────────────────────────────────────

fn build_level_mesh(cfg: &GroundGrid, step: f32, half: f32) -> Mesh {
    let s = cfg.color.to_srgba();
    let base_rgba = [s.red, s.green, s.blue, s.alpha];

    let n = (half / step) as i32;
    let total_lines = (2 * n + 1) * 2;
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity((total_lines * 2) as usize);
    let mut colors:    Vec<[f32; 4]> = Vec::with_capacity((total_lines * 2) as usize);

    // Radial alpha: lines near the outer edge dissolve into the
    // ground so the square boundary isn't visible. `t ∈ [0, 1]` is
    // normalised distance from centre; smoothstep-cubed fades sharply
    // once past ~2/3 of the radius.
    let line_color = |i: i32| -> [f32; 4] {
        let t = (i.abs() as f32) / (n as f32);
        let edge_fade = {
            let u = (1.0 - t).clamp(0.0, 1.0);
            u * u * (3.0 - 2.0 * u) // smoothstep
        };
        let major = i.rem_euclid(MAJOR_EVERY) == 0;
        let boost = if major { MAJOR_BOOST } else { 1.0 };
        [
            base_rgba[0],
            base_rgba[1],
            base_rgba[2],
            (base_rgba[3] * edge_fade * boost).clamp(0.0, 1.0),
        ]
    };

    // Lines running along +X (constant Z).
    for i in -n..=n {
        let z = i as f32 * step;
        let c = line_color(i);
        positions.push([-half, 0.0, z]);
        positions.push([ half, 0.0, z]);
        colors.push(c);
        colors.push(c);
    }
    // Lines running along +Z (constant X).
    for i in -n..=n {
        let x = i as f32 * step;
        let c = line_color(i);
        positions.push([x, 0.0, -half]);
        positions.push([x, 0.0,  half]);
        colors.push(c);
        colors.push(c);
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::LineList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh
}
