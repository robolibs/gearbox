//! Two line-list MESHES (one per circle) that track the machine.
//!
//! Astrocraft-style: the circles are real meshes inside the
//! `BigSpace` hierarchy, so big_space's floating-origin transform
//! propagation handles positioning — no per-frame manual rebasing,
//! no gizmo jitter. Each frame we:
//!
//!   - read the machine's world position and distance-from-planet-centre,
//!   - compute its geographic lat/lon on the rotated sphere,
//!   - set the circle entity's Transform to the machine's position
//!     (so the mesh's origin IS the machine, keeping vertex magnitudes
//!     small even at Earth radius — f32 cm precision works),
//!   - rewrite the mesh's vertex buffer in f64, then cast the small
//!     vertex-minus-anchor offsets to f32.

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
        let blue = Color::srgba(0.10, 0.45, 1.00, 1.0);
        Self { lat_color: blue, lon_color: blue }
    }
}

#[derive(Component, Copy, Clone)]
pub struct LatCircle;
#[derive(Component, Copy, Clone)]
pub struct LonCircle;

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
    let empty: Vec<[f32; 3]> = Vec::new();
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, empty);
    mesh
}

/// Build a line-strip mesh from unrotated-sphere unit vectors. Each
/// vertex is stored as an offset from `anchor_world` to keep f32
/// magnitudes small near the anchor (cm precision at the machine,
/// deteriorating to ≲1 m at the antipode 12 000 km away — which is
/// always behind the planet's occlusion horizon anyway).
fn build_line_strip(
    us: &[DVec3],
    planet_rot: DQuat,
    r_circle: f64,
    sphere_centre: DVec3,
    anchor_world: DVec3,
) -> Mesh {
    let positions: Vec<[f32; 3]> = us
        .iter()
        .map(|u| {
            let world = planet_rot * *u * r_circle + sphere_centre;
            let offset = world - anchor_world;
            [offset.x as f32, offset.y as f32, offset.z as f32]
        })
        .collect();

    let mut mesh = Mesh::new(
        PrimitiveTopology::LineStrip,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh
}

/// Spawn the two circle meshes once, under the BigSpace root.
/// Positions are filled in by `update_circle_meshes` every frame.
pub fn spawn_circle_meshes(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    big_space_root: Entity,
    cfg: &GroundGrid,
) {
    let lat_mesh = meshes.add(empty_line_mesh());
    let lon_mesh = meshes.add(empty_line_mesh());
    // `depth_bias` is a wgpu `Constant` depth offset that StandardMaterial
    // casts to i32, so anything astronomical overflows or clamps — keep
    // it modest. 1000 is plenty to beat the tractor chassis mesh without
    // pushing the fragments past the near plane or wrapping the bias.
    let lat_mat = materials.add(StandardMaterial {
        base_color: cfg.lat_color,
        unlit: true,
        depth_bias: 1000.0,
        ..default()
    });
    let lon_mat = materials.add(StandardMaterial {
        base_color: cfg.lon_color,
        unlit: true,
        depth_bias: 1000.0,
        ..default()
    });

    // `NoFrustumCulling` — the mesh's AABB is computed once from the
    // (initially empty) vertex buffer, and Bevy's frustum culling
    // doesn't recompute it as we rewrite positions each frame. Without
    // this component the entity disappears as soon as the camera gets
    // close enough that the stale AABB falls outside the view frustum.
    commands
        .spawn((
            Name::new("LatCircle"),
            BigSpatialBundle::default(),
            Mesh3d(lat_mesh),
            MeshMaterial3d(lat_mat),
            NoFrustumCulling,
            LatCircle,
        ))
        .insert(ChildOf(big_space_root));
    commands
        .spawn((
            Name::new("LonCircle"),
            BigSpatialBundle::default(),
            Mesh3d(lon_mesh),
            MeshMaterial3d(lon_mat),
            NoFrustumCulling,
            LonCircle,
        ))
        .insert(ChildOf(big_space_root));
}

/// Build the two circle meshes ONCE, anchored to the machine's initial
/// world position, then leave them alone. Subsequent frames don't touch
/// the meshes or their entity transforms, so as the machine drives the
/// circles stay pinned to their original location — not following.
///
/// `done` is a Bevy `Local` that latches after the first successful
/// build, so the work (mesh rebuild + transform write) happens exactly
/// once per editor session.
pub fn update_circle_meshes(
    mut meshes: ResMut<Assets<Mesh>>,
    sim: Res<GearboxSim>,
    players: Query<&VehicleBody, With<PlayerControlled>>,
    mut lat_q: Query<(&Mesh3d, &mut Transform), (With<LatCircle>, Without<LonCircle>, Without<ChaseCamera>)>,
    mut lon_q: Query<(&Mesh3d, &mut Transform), (With<LonCircle>, Without<LatCircle>, Without<ChaseCamera>)>,
    mut done: Local<bool>,
) {
    if *done { return; }
    let Ok(player) = players.single() else { return };
    let Ok((lat_h, mut lat_tr)) = lat_q.single_mut() else { return };
    let Ok((lon_h, mut lon_tr)) = lon_q.single_mut() else { return };

    let veh_pose = sim.0.vehicle_pose(player.id);
    let veh_world = DVec3::new(veh_pose.point.x, veh_pose.point.y, veh_pose.point.z);

    let sphere_centre = DVec3::new(0.0, -sim.0.planet.radius, 0.0);
    let planet_rot = rotation_f64(
        sim.0.planet.datum.latitude,
        sim.0.planet.datum.longitude,
    );
    let inv_rot = planet_rot.inverse();

    let to_veh = veh_world - sphere_centre;
    let r_machine = to_veh.length().max(1.0);
    // Circles sit basically ON the sphere's surface — 1 cm lift is
    // just enough to clear the sphere mesh's vertex-level z-fighting
    // without being noticeably above the ground.
    let r_circle = sim.0.planet.radius + 0.01;
    let dir = to_veh / r_machine;
    let unrotated = inv_rot * dir;
    let lat_rad = unrotated.y.clamp(-1.0, 1.0).asin();
    let lon_rad = unrotated.z.atan2(unrotated.x);

    // Anchor the circle entities at the machine so vertices are small.
    let machine_pos_f32 = veh_world.as_vec3();
    lat_tr.translation = machine_pos_f32;
    lat_tr.rotation = Quat::IDENTITY;
    lat_tr.scale = Vec3::ONE;
    lon_tr.translation = machine_pos_f32;
    lon_tr.rotation = Quat::IDENTITY;
    lon_tr.scale = Vec3::ONE;

    // The parameter sweep is ANCHORED at the machine: vertex 0 sits
    // exactly on the machine's longitude (for the latitude circle) or
    // on the machine itself (for the meridian). Without this, the
    // machine's position falls mid-segment and the straight LineStrip
    // chord dives ~r·(1−cos(Δ/2)) metres BELOW the arc — at Earth
    // radius with 2048 segments that's still a few cm, but at 512
    // segments it was ~70 m and the circle vanished INTO the ground
    // right where the tractor stood.

    // --- Latitude circle: thin-quad strip along lat = const ---
    {
        let cl = lat_rad.cos();
        let sl = lat_rad.sin();
        let mut us: Vec<DVec3> = Vec::with_capacity(SEGS as usize + 1);
        for i in 0..=SEGS {
            let phi = lon_rad + (i as f64 / SEGS as f64) * std::f64::consts::TAU;
            us.push(DVec3::new(cl * phi.cos(), sl, cl * phi.sin()));
        }
        let new_mesh = build_line_strip(&us, planet_rot, r_circle, sphere_centre, veh_world);
        if let Some(mesh) = meshes.get_mut(&lat_h.0) {
            *mesh = new_mesh;
        }
    }

    // --- Longitude great circle: thin-quad strip through both poles + machine ---
    //     u(θ) = (sin θ · cosLon, cos θ, sin θ · sinLon). Machine at θ0 = π/2 − lat.
    {
        let cn = lon_rad.cos();
        let sn = lon_rad.sin();
        let theta0 = std::f64::consts::FRAC_PI_2 - lat_rad;
        let mut us: Vec<DVec3> = Vec::with_capacity(SEGS as usize + 1);
        for i in 0..=SEGS {
            let theta = theta0 + (i as f64 / SEGS as f64) * std::f64::consts::TAU;
            let (st, ct) = theta.sin_cos();
            us.push(DVec3::new(st * cn, ct, st * sn));
        }
        let new_mesh = build_line_strip(&us, planet_rot, r_circle, sphere_centre, veh_world);
        if let Some(mesh) = meshes.get_mut(&lon_h.0) {
            *mesh = new_mesh;
        }
    }

    *done = true;
}
