//! Persistent scene lifecycle, camera controls, USD terrain collision,
//! object placement, and world-event publishing.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, RwLock};

use crate::physics::PhysicsWorld;
use bevy::ecs::entity::Entities;
use bevy::light::NotShadowCaster;
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use bevy::transform::TransformSystems;
use gearbox_api::{GearboxBus, SceneEvent, event_kind};
use mara::ui::modules::bevy::{
    BevyViewportInput, BevyViewportRenderTarget, BevyViewportSet, ChaseCamera, GroundGrid,
    apply_rig,
};
use rapier3d::math::{Rotation as DQuat, Vector as DVec3};
use rapier3d::prelude::{
    ColliderBuilder, ColliderHandle, Pose, RigidBodyBuilder, RigidBodyHandle, RigidBodyType,
};

/// Earth-radius planet sphere. The simulator was tuned for this
/// radius — vehicle wheel friction, camera fog distances, cloud
/// altitude, and shadow cascades all assume ~6 371 km.
const PLANET_RADIUS_M: f32 = 6_371_000.0;
/// The planet cap lies under the deepest valley of the distant land, with
/// room for the meadow's own hollows; any shallower and it shows through the
/// valleys as flat pale islands.
const PLANET_VISUAL_DROP_M: f32 = 0.5 * crate::terrain::HORIZON_RELIEF_M + 30.0;
const TERRAIN_FLAT_SPAWN_RADIUS_M: f32 = 24.0;
const TERRAIN_FULL_RELIEF_RADIUS_M: f32 = 55.0;
const TERRAIN_MIN_HEIGHT_M: f32 = -5.0;
const TERRAIN_MAX_HEIGHT_M: f32 = 10.0;
const FLAT_GROUND_HALF_EXTENT_M: f64 = 10_000.0;
const FLAT_GROUND_VISUAL_SIZE_M: f32 = 10_000.0;
const USD_TERRAIN_ACTIVATION_WARN_FRAMES: u32 = 120;
const CAMERA_HALF_SPAN_M: f32 = 5_000.0;
const CAMERA_MAX_DISTANCE_M: f32 = 5_000.0;
/// With the ceiling lifted: far enough for the whole planet to fit the view.
const CAMERA_ORBIT_DISTANCE_M: f32 = 40_000_000.0;
const CAMERA_MAX_HEIGHT_M: f32 = 3_000.0;

static USD_TERRAIN_LOADED: AtomicBool = AtomicBool::new(false);
/// Set when the loaded USD terrain mesh is level; `terrain_height_m` then
/// returns its height instead of the procedural hill formula.
static USD_TERRAIN_IS_FLAT: AtomicBool = AtomicBool::new(false);
static USD_TERRAIN_FLAT_Y_BITS: AtomicU32 = AtomicU32::new(0);
const USD_TERRAIN_FLAT_TOLERANCE_M: f32 = 0.05;
/// The loaded terrain's exact surface, so placement uses the same shape the
/// collider has rather than the procedural formula it approximates.
static USD_TERRAIN_MESH: RwLock<Option<Arc<TerrainHeightMesh>>> = RwLock::new(None);
const TERRAIN_HEIGHT_BINS: f32 = 512.0;

struct TerrainHeightMesh {
    vertices: Vec<[f32; 3]>,
    triangles: Vec<[u32; 3]>,
    min_x: f32,
    min_z: f32,
    cell: f32,
    cols: usize,
    rows: usize,
    bins: Vec<Vec<u32>>,
}

impl TerrainHeightMesh {
    fn build(vertices: &[DVec3], triangles: &[[u32; 3]]) -> Option<Self> {
        let vertices: Vec<[f32; 3]> = vertices
            .iter()
            .map(|v| [v.x as f32, v.y as f32, v.z as f32])
            .collect();
        let triangles: Vec<[u32; 3]> = triangles
            .iter()
            .copied()
            .filter(|t| t.iter().all(|&i| (i as usize) < vertices.len()))
            .collect();
        if vertices.is_empty() || triangles.is_empty() {
            return None;
        }
        let (mut min_x, mut min_z, mut max_x, mut max_z) = (
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        );
        for v in &vertices {
            min_x = min_x.min(v[0]);
            max_x = max_x.max(v[0]);
            min_z = min_z.min(v[2]);
            max_z = max_z.max(v[2]);
        }
        let span = (max_x - min_x).max(max_z - min_z);
        if !(span > 0.0) {
            return None;
        }
        let cell = (span / TERRAIN_HEIGHT_BINS).max(1.0);
        let cols = ((max_x - min_x) / cell).floor() as usize + 1;
        let rows = ((max_z - min_z) / cell).floor() as usize + 1;
        let mut bins = vec![Vec::new(); cols * rows];
        for (t, tri) in triangles.iter().enumerate() {
            let pts = tri.map(|i| vertices[i as usize]);
            let lo_x = pts.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min);
            let hi_x = pts.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max);
            let lo_z = pts.iter().map(|p| p[2]).fold(f32::INFINITY, f32::min);
            let hi_z = pts.iter().map(|p| p[2]).fold(f32::NEG_INFINITY, f32::max);
            let c0 = ((lo_x - min_x) / cell).floor() as usize;
            let c1 = (((hi_x - min_x) / cell).floor() as usize).min(cols - 1);
            let r0 = ((lo_z - min_z) / cell).floor() as usize;
            let r1 = (((hi_z - min_z) / cell).floor() as usize).min(rows - 1);
            for r in r0..=r1 {
                for c in c0..=c1 {
                    bins[r * cols + c].push(t as u32);
                }
            }
        }
        Some(Self {
            vertices,
            triangles,
            min_x,
            min_z,
            cell,
            cols,
            rows,
            bins,
        })
    }

    fn height_at(&self, x: f32, z: f32) -> Option<f32> {
        let c = ((x - self.min_x) / self.cell).floor();
        let r = ((z - self.min_z) / self.cell).floor();
        if c < 0.0 || r < 0.0 || c as usize >= self.cols || r as usize >= self.rows {
            return None;
        }
        let mut best: Option<f32> = None;
        for &t in &self.bins[r as usize * self.cols + c as usize] {
            let [a, b, c] = self.triangles[t as usize].map(|i| self.vertices[i as usize]);
            if let Some(y) = triangle_height_at(a, b, c, x, z) {
                best = Some(best.map_or(y, |h| h.max(y)));
            }
        }
        best
    }
}

fn triangle_height_at(a: [f32; 3], b: [f32; 3], c: [f32; 3], x: f32, z: f32) -> Option<f32> {
    let det = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
    if det.abs() < 1e-9 {
        return None;
    }
    let l1 = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / det;
    let l2 = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / det;
    let l3 = 1.0 - l1 - l2;
    let eps = -1e-4;
    if l1 < eps || l2 < eps || l3 < eps {
        return None;
    }
    Some(l1 * a[1] + l2 * b[1] + l3 * c[1])
}

fn set_usd_terrain_height_profile(min_y: f32, max_y: f32) {
    let flat = max_y - min_y <= USD_TERRAIN_FLAT_TOLERANCE_M;
    USD_TERRAIN_FLAT_Y_BITS.store(max_y.to_bits(), Ordering::Relaxed);
    USD_TERRAIN_IS_FLAT.store(flat, Ordering::Relaxed);
}

fn set_usd_terrain_height_mesh(vertices: &[DVec3], triangles: &[[u32; 3]]) {
    if let Ok(mut slot) = USD_TERRAIN_MESH.write() {
        *slot = TerrainHeightMesh::build(vertices, triangles).map(Arc::new);
    }
}

fn clear_usd_terrain_height_profile() {
    USD_TERRAIN_IS_FLAT.store(false, Ordering::Relaxed);
    if let Ok(mut slot) = USD_TERRAIN_MESH.write() {
        *slot = None;
    }
}

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StaticUsdPropBodies>()
            .init_resource::<PublishedUsdPoses>()
            .add_systems(
                Startup,
                (
                    spawn_world.after(BevyViewportSet::SetupTarget),
                    spawn_flat_ground,
                ),
            )
            .add_systems(Update, mark_new_usd_terrain_roots)
            .add_systems(
                Update,
                (chase_camera_control, chase_camera_zoom, chase_camera_keys).chain(),
            )
            .add_systems(Update, snap_new_usd_roots_to_terrain)
            .add_systems(Update, freeze_settled_static_usd_prop_bodies)
            .add_systems(Update, publish_loaded_usd_poses)
            .add_systems(Update, harvest_bales_on_machine_contact)
            .add_systems(Update, cleanup_terrain_collision_without_usd_terrain)
            .add_systems(Update, cleanup_static_usd_prop_bodies)
            .add_systems(
                PostUpdate,
                (
                    chase_camera_floor.before(TransformSystems::Propagate),
                    activate_usd_terrain_when_collider_ready.after(TransformSystems::Propagate),
                    align_new_grounded_usd_bounds_to_terrain
                        .after(activate_usd_terrain_when_collider_ready),
                ),
            );
    }
}

#[derive(Component, Debug, Clone, Copy)]
struct TerrainBoundsSnapPending {
    frames_waited: u32,
}

#[derive(Component, Debug, Clone, Copy)]
struct PendingUsdTerrainActivation {
    frames_waited: u32,
}

#[derive(Component, Debug, Clone, Copy)]
struct StaticUsdPhysicsProp {
    body: RigidBodyHandle,
    visual_top_offset_y: f32,
    frames_alive: u32,
}

#[derive(Resource, Debug, Clone, Copy)]
pub(crate) struct FlatGround {
    entity: Entity,
    collider: ColliderHandle,
}

#[derive(Resource, Debug, Clone, Copy)]
pub(crate) struct TerrainCollision {
    terrain: ColliderHandle,
    safety_floor: ColliderHandle,
}

#[derive(Resource, Default)]
struct StaticUsdPropBodies {
    handles: HashMap<Entity, (RigidBodyHandle, ColliderHandle)>,
}

/// Prop entities whose settled pose has already been published. Keyed by
/// `Entity` (not runtime id) so that re-loading an id — a fresh entity — is
/// reported anew instead of being silently suppressed across script runs.
#[derive(Resource, Default)]
struct PublishedUsdPoses {
    published: std::collections::HashSet<Entity>,
}

const TERRAIN_BOUNDS_SNAP_SETTLE_FRAMES: u32 = 5;
const TERRAIN_PROP_CONTACT_CLEARANCE_M: f32 = 0.015;
const STATIC_PROP_MIN_DYNAMIC_FRAMES: u32 = 12;
const STATIC_PROP_FORCE_FREEZE_FRAMES: u32 = 45;
const STATIC_PROP_SETTLED_LINEAR_SPEED_MPS: f64 = 0.12;
const STATIC_PROP_SETTLED_ANGULAR_SPEED_RPS: f64 = 0.25;

/// The planet: fine rings in a cap around the pole under the field, where it is
/// seen from near, and coarse ones round the rest for a camera in orbit. Rings
/// tighten towards the field, keeping the true curvature with ~35k vertices
/// where a full 1024x512 sphere uploaded 29 MB at every launch.
fn planet_cap_mesh(radius: f32) -> Mesh {
    const CAP_RAD: f32 = 3.0 * std::f32::consts::PI / 180.0;
    const CAP_RINGS: u32 = 48;
    // Past the cap, coarse rings close the globe for a camera in orbit.
    const GLOBE_RINGS: u32 = 90;
    const RINGS: u32 = CAP_RINGS + GLOBE_RINGS;
    const SEGMENTS: u32 = 256;
    let mut positions = vec![[0.0, radius, 0.0]];
    let mut normals = vec![[0.0, 1.0, 0.0]];
    for ring in 1..=RINGS {
        let theta = if ring <= CAP_RINGS {
            CAP_RAD * (ring as f32 / CAP_RINGS as f32).powi(2)
        } else {
            let t = (ring - CAP_RINGS) as f32 / (GLOBE_RINGS + 1) as f32;
            CAP_RAD + (std::f32::consts::PI - CAP_RAD) * t
        };
        for segment in 0..SEGMENTS {
            let phi = segment as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            let normal = [theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin()];
            positions.push(normal.map(|c| c * radius));
            normals.push(normal);
        }
    }
    let at = |ring: u32, segment: u32| 1 + (ring - 1) * SEGMENTS + segment % SEGMENTS;
    let mut indices = Vec::new();
    for segment in 0..SEGMENTS {
        indices.extend([0, at(1, segment + 1), at(1, segment)]);
    }
    for ring in 1..RINGS {
        for segment in 0..SEGMENTS {
            let (a, b) = (at(ring, segment), at(ring, segment + 1));
            let (c, d) = (at(ring + 1, segment), at(ring + 1, segment + 1));
            indices.extend([a, b, c, b, d, c]);
        }
    }
    // The far pole closes it.
    let south = positions.len() as u32;
    positions.push([0.0, -radius, 0.0]);
    normals.push([0.0, -1.0, 0.0]);
    for segment in 0..SEGMENTS {
        indices.extend([south, at(RINGS, segment), at(RINGS, segment + 1)]);
    }
    Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(bevy::mesh::Indices::U32(indices))
}

fn spawn_world(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    render_target: Option<Res<BevyViewportRenderTarget>>,
    globe: Res<crate::globe::Globe>,
) {
    let radius = PLANET_RADIUS_M;

    // ── Planet ───────────────────────────────────────────────────────
    // Warm sandy / tan ground colour, filling the horizon. It is lowered
    // below the local terrain so it cannot appear as a second flat
    // ground plate under the hilly field mesh.
    let planet_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.62, 0.48, 0.33),
        perceptual_roughness: 0.95,
        ..default()
    });
    let planet_mesh = meshes.add(planet_cap_mesh(radius));
    commands.spawn((
        Name::new("Planet"),
        // At the centre of its own frame, shrunk to sit under the land everywhere.
        Transform::from_scale(Vec3::splat(1.0 - PLANET_VISUAL_DROP_M / radius)),
        big_space::prelude::CellCoord::default(),
        ChildOf(globe.root),
        Mesh3d(planet_mesh),
        MeshMaterial3d(planet_mat.clone()),
        NotShadowCaster,
        bevy::light::NotShadowReceiver,
    ));

    // ── Ground grid: off-looking flat grids make hilly terrain read as
    // floating over a plate, so keep it effectively invisible by
    // default; the UI toggle can still enable the grid resource.
    commands.insert_resource(GroundGrid {
        color: Color::srgba(80.0 / 255.0, 70.0 / 255.0, 70.0 / 255.0, 0.0),
        ..GroundGrid::default()
    });

    // ── Camera ──────────────────────────────────────────────────────
    // Scripted views: `GEARBOX_CAMERA_DISTANCE` (m), `GEARBOX_CAMERA_ELEVATION` (deg)
    // and `GEARBOX_CAMERA_FOCUS` ("x,z" in m).
    let view = |name: &str, fallback: f32| {
        std::env::var(name).ok().and_then(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite()).unwrap_or(fallback)
    };
    let chase = ChaseCamera {
        focus: std::env::var("GEARBOX_CAMERA_FOCUS")
            .ok()
            .and_then(|v| {
                let (x, z) = v.split_once(',')?;
                Some(Vec3::new(x.trim().parse().ok()?, 0.5, z.trim().parse().ok()?))
            })
            .unwrap_or(Vec3::new(0.0, 0.5, 0.0)),
        distance: view("GEARBOX_CAMERA_DISTANCE", 14.0).clamp(1.0, CAMERA_ORBIT_DISTANCE_M),
        elevation: view("GEARBOX_CAMERA_ELEVATION", 15.0).clamp(-10.0, 89.0).to_radians(),
        max_distance: CAMERA_MAX_DISTANCE_M,
        ..default()
    };
    let mut camera_transform = Transform::from_xyz(0.0, 8.0, -15.0).looking_at(Vec3::ZERO, Vec3::Y);
    apply_rig(&chase, &mut camera_transform);

    let mut camera = commands.spawn((
        Name::new("Camera"),
        Camera3d::default(),
        bevy::render::view::NoIndirectDrawing,
        // Sunlight only: no point or spot lights to cluster, and building
        // the (empty) clusters cost ~18 ms a frame.
        bevy::light::cluster::ClusterConfig::None,
        camera_transform,
        Projection::Perspective(PerspectiveProjection {
            near: 0.1,
            // Only culling reads it (depth is reversed and unbounded): past the
            // far side of the planet from the highest orbit.
            far: 1.0e8,
            ..default()
        }),
        chase,
        big_space::prelude::CellCoord::default(),
        big_space::prelude::FloatingOrigin,
        ChildOf(globe.home),
    ));
    if let Some(target) = render_target {
        camera.insert(bevy::camera::RenderTarget::from(target.0.clone()));
    }
}

/// Pointer input arrives from the mara viewport in render-target pixels:
/// a primary drag orbits, a middle drag pans, Shift with a middle drag
/// lifts the focus instead.
fn chase_camera_control(
    input: Res<BevyViewportInput>,
    keys: Res<ButtonInput<KeyCode>>,
    grab: Res<crate::viewer::systems::GizmoGrab>,
    context: Res<crate::viewer::machine_context::MachineHover>,
    mut cameras: Query<(&mut ChaseCamera, &mut Transform)>,
) {
    if context.captures_pointer {
        return;
    }
    let orbit_delta = if grab.0 {
        Vec2::ZERO
    } else {
        Vec2::from(input.drag_delta)
    };
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let (pan_delta, lift_delta) = if shift {
        (Vec2::ZERO, input.pan_delta[1])
    } else {
        (Vec2::from(input.pan_delta), 0.0)
    };
    if pan_delta == Vec2::ZERO && lift_delta == 0.0 && orbit_delta == Vec2::ZERO {
        return;
    }

    for (mut cam, mut transform) in &mut cameras {
        if pan_delta != Vec2::ZERO {
            let pan_speed = cam.distance * cam.pan_sensitivity;
            let forward = Vec3::new(cam.yaw.sin(), 0.0, cam.yaw.cos());
            let right = Vec3::new(forward.z, 0.0, -forward.x);
            cam.focus += (-right * pan_delta.x - forward * pan_delta.y) * pan_speed;
        }
        if lift_delta != 0.0 {
            cam.focus.y += lift_delta * cam.distance * cam.pan_sensitivity;
        }
        if orbit_delta != Vec2::ZERO {
            cam.yaw -= orbit_delta.x * cam.orbit_speed;
            cam.elevation += orbit_delta.y * cam.orbit_speed;
            cam.elevation = cam.elevation.clamp(cam.min_elevation, cam.max_elevation);
        }
        apply_rig(&cam, &mut transform);
    }
}

/// How much of the view distance a second of a held key moves the view.
const KEY_PAN_PER_SEC: f32 = 0.4;

/// How far above the actual terrain surface (hills included, not just
/// `y=0`) the camera's eye is kept. Three degrees above level keeps the
/// camera itself above a focus that is on the ground, whatever the distance.
const CAMERA_TERRAIN_CLEARANCE_M: f32 = 0.5;
const CAMERA_MIN_ELEVATION: f32 = 3.0_f32.to_radians();

/// Bounds the camera after controls and fly-to updates; Alt bypasses only the
/// floor. The floor tracks the terrain surface under the camera's eye (not
/// `focus`, and not a flat `y=0`), so flying over a hill rises with it and
/// looking straight down stops right at the ground instead of clipping
/// through — `apply_rig` puts the eye at `focus + distance` along
/// `yaw`/`elevation`, so the same offset is used here to find what's
/// actually under the eye before solving back for the `focus.y` that keeps
/// it clear.
pub(crate) fn chase_camera_floor(
    keys: Res<ButtonInput<KeyCode>>,
    toggles: Res<crate::viewer::overlays::DisplayToggles>,
    mut cameras: Query<(&mut ChaseCamera, &mut Transform, &mut Projection)>,
) {
    let (ceiling, max_height) = if toggles.unlimited_zoom {
        (CAMERA_ORBIT_DISTANCE_M, CAMERA_ORBIT_DISTANCE_M)
    } else {
        (CAMERA_MAX_DISTANCE_M, CAMERA_MAX_HEIGHT_M)
    };
    let below_ground = keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight);
    for (mut cam, mut transform, mut projection) in &mut cameras {
        cam.max_distance = ceiling;
        cam.distance = cam.distance.clamp(cam.min_distance, ceiling);
        // Depth precision falls with the square of distance over the near
        // plane, so the near plane backs off as the camera does; nothing is
        // ever that close to a camera that far out.
        let near = (cam.distance / 200.0).clamp(0.1, 100_000.0);
        if let Projection::Perspective(lens) = projection.as_mut()
            && (lens.near - near).abs() > near * 0.05
        {
            lens.near = near;
        }
        cam.focus.x = cam.focus.x.clamp(-CAMERA_HALF_SPAN_M, CAMERA_HALF_SPAN_M);
        cam.focus.z = cam.focus.z.clamp(-CAMERA_HALF_SPAN_M, CAMERA_HALF_SPAN_M);
        if !below_ground {
            cam.elevation = cam.elevation.max(CAMERA_MIN_ELEVATION);
            let horizontal = cam.elevation.cos();
            let rise = cam.distance * cam.elevation.sin();
            let eye_x = cam.focus.x + cam.distance * cam.yaw.sin() * horizontal;
            let eye_z = cam.focus.z + cam.distance * cam.yaw.cos() * horizontal;
            let ground = terrain_height_m(eye_x, eye_z);
            cam.focus.y = cam
                .focus
                .y
                .max(ground + CAMERA_TERRAIN_CLEARANCE_M - rise);
        }
        let rise = cam.distance * cam.elevation.sin().max(0.0);
        if rise > max_height {
            cam.distance = max_height / cam.elevation.sin();
        }
        cam.focus.y = cam
            .focus
            .y
            .min(max_height - cam.distance * cam.elevation.sin());
        apply_rig(&cam, &mut transform);
    }
}

/// W/S fly the camera forward/back along its view, translating it rather
/// than zooming — distance to focus is untouched, so this never changes
/// perspective, only position. A/D slide over the ground, Q/E go down and
/// up, Shift speeds all of it. W/S drop any follow, since a followed
/// target's position would otherwise fight the translation every frame.
/// Yields WASD to `viewer::drive::keyboard` while it's driving the selected
/// machine, so the keys don't do both at once.
fn chase_camera_keys(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    panel: Res<crate::viewer::drive::MachinePanel>,
    mut follow: ResMut<crate::viewer::state::FollowTarget>,
    mut cameras: Query<(&mut ChaseCamera, &mut Transform)>,
) {
    if panel.keyboard.is_some() {
        return;
    }
    let axis =
        |neg: KeyCode, pos: KeyCode| (keys.pressed(pos) as i8 - keys.pressed(neg) as i8) as f32;
    // `forward` below points from the focus back to the camera, so W is the
    // negative direction along it.
    let ahead = axis(KeyCode::KeyW, KeyCode::KeyS);
    let aside = axis(KeyCode::KeyA, KeyCode::KeyD);
    let up = axis(KeyCode::KeyQ, KeyCode::KeyE);
    if ahead == 0.0 && aside == 0.0 && up == 0.0 {
        return;
    }
    let boost = if keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight) {
        3.0
    } else {
        1.0
    };
    if ahead != 0.0 {
        follow.set(None);
    }
    for (mut cam, mut transform) in &mut cameras {
        let forward = Vec3::new(cam.yaw.sin(), 0.0, cam.yaw.cos());
        let right = Vec3::new(forward.z, 0.0, -forward.x);
        let offset = Vec3::new(
            forward.x * cam.elevation.cos(),
            cam.elevation.sin(),
            forward.z * cam.elevation.cos(),
        );
        let speed = cam.distance.max(2.0) * KEY_PAN_PER_SEC * boost * time.delta_secs();
        cam.focus += offset * ahead * speed + (right * aside + Vec3::Y * up) * speed;
        apply_rig(&cam, &mut transform);
    }
}

/// The viewport reports scroll in 120-point units; one wheel notch is 50
/// points in egui, so this brings it back to notches.
const SCROLL_NOTCHES_PER_UNIT: f64 = 120.0 / 50.0;

fn chase_camera_zoom(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    input: Res<BevyViewportInput>,
    context: Res<crate::viewer::machine_context::MachineHover>,
    mut zoom_target: Local<Option<f64>>,
    mut last_written: Local<Option<f32>>,
    mut cameras: Query<(&mut ChaseCamera, &mut Transform)>,
) {
    if keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight) {
        return;
    }
    let scroll_delta = if context.captures_pointer {
        0.0
    } else {
        input.scroll_delta as f64 * SCROLL_NOTCHES_PER_UNIT
    };

    let Ok((mut cam, mut transform)) = cameras.single_mut() else {
        return;
    };
    let target = zoom_target.get_or_insert(cam.distance as f64);
    // A distance set elsewhere (fly, fit) becomes the new zoom target instead
    // of being pulled back to the last scrolled one.
    if last_written.is_some_and(|d| (d - cam.distance).abs() > 1e-4) {
        *target = cam.distance as f64;
    }
    if scroll_delta != 0.0 {
        let log_target = target.max(0.1).log10();
        let new_log = log_target - scroll_delta * cam.zoom_step;
        *target = 10f64
            .powf(new_log)
            .clamp(cam.min_distance as f64, cam.max_distance as f64);
    }

    let dt = time.delta_secs_f64();
    let log_current = (cam.distance as f64).max(0.1).ln();
    let log_target = target.max(0.1).ln();
    let log_diff = log_target - log_current;
    if log_diff.abs() > 1e-4 {
        let new_log = log_current + log_diff * (6.0 * dt).min(0.9);
        cam.distance = new_log.exp() as f32;
        apply_rig(&cam, &mut transform);
    } else if log_diff.abs() > 1e-5 {
        cam.distance = *target as f32;
        apply_rig(&cam, &mut transform);
    }
    *last_written = Some(cam.distance);
}

fn spawn_flat_ground(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut physics: ResMut<PhysicsWorld>,
) {
    USD_TERRAIN_LOADED.store(false, Ordering::Relaxed);
    clear_usd_terrain_height_profile();

    let material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.48, 0.42, 0.30),
        perceptual_roughness: 0.98,
        ..default()
    });
    let mesh = meshes.add(Cuboid::new(
        FLAT_GROUND_VISUAL_SIZE_M,
        0.02,
        FLAT_GROUND_VISUAL_SIZE_M,
    ));
    let entity = commands
        .spawn((
            Name::new("FlatGround"),
            Transform::from_xyz(0.0, -0.01, 0.0),
            Mesh3d(mesh),
            MeshMaterial3d(material),
            NotShadowCaster,
        ))
        .id();

    let collider =
        ColliderBuilder::cuboid(FLAT_GROUND_HALF_EXTENT_M, 0.02, FLAT_GROUND_HALF_EXTENT_M)
            .translation(DVec3::new(0.0, -0.02, 0.0))
            .friction(ground_friction(1.0))
            .restitution(0.0)
            .build();
    let collider = physics.colliders.insert(collider);
    commands.insert_resource(FlatGround { entity, collider });
}

fn mark_new_usd_terrain_roots(
    mut commands: Commands,
    terrain_roots: Query<(Entity, &Name), Added<usd_bevy::UsdSceneRoot>>,
) {
    for (entity, name) in terrain_roots.iter() {
        if !is_usd_terrain_root_name(name.as_str()) {
            continue;
        }
        commands
            .entity(entity)
            .insert(PendingUsdTerrainActivation { frames_waited: 0 });
        info!(
            "world: USD terrain scene loaded; keeping flat fallback until terrain collider is ready"
        );
    }
}

fn activate_usd_terrain_when_collider_ready(
    mut commands: Commands,
    mut terrain_roots: Query<(Entity, &Name, &mut PendingUsdTerrainActivation)>,
    children: Query<&Children>,
    meshes: Res<Assets<Mesh>>,
    terrain_meshes: Query<(Entity, &GlobalTransform, &Mesh3d)>,
    flat: Option<Res<FlatGround>>,
    terrain_collision: Option<Res<TerrainCollision>>,
    mut physics: ResMut<PhysicsWorld>,
) {
    let mut flat_ground = flat.as_deref().copied();
    let mut terrain_collision_ready = terrain_collision.is_some();
    for (root, name, mut pending) in terrain_roots.iter_mut() {
        pending.frames_waited = pending.frames_waited.saturating_add(1);
        if !terrain_collision_ready && is_usd_terrain_scene_instantiated(root, &children) {
            if let Some(collision) = attach_gearbox_terrain_trimesh(
                root,
                &children,
                &meshes,
                &terrain_meshes,
                physics.as_mut(),
            ) {
                commands.insert_resource(collision);
                terrain_collision_ready = true;
            }
        }
        if !terrain_collision_ready {
            if pending.frames_waited == USD_TERRAIN_ACTIVATION_WARN_FRAMES {
                warn!(
                    "world: still waiting for visible USD terrain mesh on {}; keeping flat fallback active",
                    name.as_str()
                );
            }
            continue;
        }

        USD_TERRAIN_LOADED.store(true, Ordering::Relaxed);
        let removed_flat = if let Some(flat) = flat_ground.take() {
            remove_flat_ground(&mut commands, physics.as_mut(), flat);
            commands.remove_resource::<FlatGround>();
            true
        } else {
            false
        };
        commands
            .entity(root)
            .remove::<PendingUsdTerrainActivation>();
        if removed_flat {
            info!(
                "world: USD terrain collider ready for {}; removed default flat ground",
                name.as_str()
            );
        } else {
            info!(
                "world: USD terrain collider ready for {}; default flat ground was already gone",
                name.as_str()
            );
        }
    }
}

pub(crate) fn is_usd_terrain_root_name(name: &str) -> bool {
    name == "WorldTerrain" || name.to_ascii_lowercase().contains("terrain")
}

pub(crate) fn is_usd_terrain_scene_instantiated(root: Entity, children: &Query<&Children>) -> bool {
    collect_descendants(root, children).len() > 1
}

pub(crate) fn asset_path(relative: &str) -> String {
    crate::load::default_asset_root()
        .join(relative)
        .to_string_lossy()
        .into_owned()
}

fn attach_gearbox_terrain_trimesh(
    root: Entity,
    children: &Query<&Children>,
    meshes: &Assets<Mesh>,
    terrain_meshes: &Query<(Entity, &GlobalTransform, &Mesh3d)>,
    physics: &mut PhysicsWorld,
) -> Option<TerrainCollision> {
    let Some((vertices, indices)) =
        terrain_trimesh_from_visible_mesh(root, children, meshes, terrain_meshes)
    else {
        return None;
    };
    remove_terrain_descendant_colliders(root, children, physics);
    let (min_y, max_y) = vertices
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(v.y as f32), hi.max(v.y as f32))
        });
    set_usd_terrain_height_profile(min_y, max_y);
    set_usd_terrain_height_mesh(&vertices, &indices);
    let Some(terrain) = ColliderBuilder::trimesh(vertices, indices).ok() else {
        warn!("world: failed to build exact Rapier trimesh collider for USD terrain");
        return None;
    };
    let terrain = physics.colliders.insert(
        terrain
            .friction(ground_friction(1.4))
            .restitution(0.0)
            .build(),
    );
    physics.entity_to_collider.insert(root, terrain);

    // Belt-and-braces catch floor below the lowest authored terrain. It
    // should never be contacted in normal use, but it prevents assets from
    // disappearing forever if a future USD terrain asset has a hole or loads
    // slower than its dynamic bodies.
    let safety_y = TERRAIN_MIN_HEIGHT_M as f64 - 1.0;
    let safety_floor =
        ColliderBuilder::cuboid(FLAT_GROUND_HALF_EXTENT_M, 0.10, FLAT_GROUND_HALF_EXTENT_M)
            .translation(DVec3::new(0.0, safety_y, 0.0))
            .friction(1.2)
            .restitution(0.0)
            .build();
    let safety_floor = physics.colliders.insert(safety_floor);

    info!("world: attached exact visible-mesh Rapier trimesh collider for USD terrain");
    Some(TerrainCollision {
        terrain,
        safety_floor,
    })
}

fn terrain_trimesh_from_visible_mesh(
    root: Entity,
    children: &Query<&Children>,
    meshes: &Assets<Mesh>,
    terrain_meshes: &Query<(Entity, &GlobalTransform, &Mesh3d)>,
) -> Option<(Vec<DVec3>, Vec<[u32; 3]>)> {
    let mut best = None;
    let mut best_area = 0.0f32;
    for entity in collect_descendants(root, children) {
        let Ok((_entity, gt, mesh3d)) = terrain_meshes.get(entity) else {
            continue;
        };
        let Some(mesh) = meshes.get(&mesh3d.0) else {
            continue;
        };
        let Some(candidate) = trimesh_candidate_from_mesh(gt, mesh) else {
            continue;
        };
        if candidate.area > best_area {
            best_area = candidate.area;
            best = Some(candidate);
        }
    }
    best.map(|candidate| (candidate.vertices, candidate.indices))
}

struct TrimeshCandidate {
    vertices: Vec<DVec3>,
    indices: Vec<[u32; 3]>,
    area: f32,
}

fn trimesh_candidate_from_mesh(gt: &GlobalTransform, mesh: &Mesh) -> Option<TrimeshCandidate> {
    let positions = match mesh.attribute(Mesh::ATTRIBUTE_POSITION)? {
        VertexAttributeValues::Float32x3(positions) => positions,
        _ => return None,
    };
    if positions.len() < 4 {
        return None;
    }

    let m = gt.to_matrix();
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let vertices = positions
        .iter()
        .map(|p| {
            let w = m.transform_point3(Vec3::from(*p));
            min = min.min(w);
            max = max.max(w);
            DVec3::new(w.x as f64, w.y as f64, w.z as f64)
        })
        .collect::<Vec<_>>();

    let indices = if let Some(indices) = mesh.indices() {
        let raw = indices.iter().map(|i| i as u32).collect::<Vec<_>>();
        raw.chunks_exact(3)
            .map(|tri| [tri[0], tri[1], tri[2]])
            .collect::<Vec<_>>()
    } else {
        (0..vertices.len() / 3)
            .map(|i| [(i * 3) as u32, (i * 3 + 1) as u32, (i * 3 + 2) as u32])
            .collect::<Vec<_>>()
    };
    if indices.is_empty() {
        return None;
    }

    let size = max - min;
    let area = size.x.abs() * size.z.abs();
    if area <= 0.0 {
        return None;
    }
    Some(TrimeshCandidate {
        vertices,
        indices,
        area,
    })
}

fn remove_terrain_descendant_colliders(
    root: Entity,
    children: &Query<&Children>,
    physics: &mut PhysicsWorld,
) {
    let stale = collect_descendants(root, children)
        .into_iter()
        .filter_map(|entity| {
            physics
                .entity_to_collider
                .remove(&entity)
                .map(|handle| (entity, handle))
        })
        .collect::<Vec<_>>();
    let colliders = &mut physics.colliders;
    let islands = &mut physics.islands;
    let bodies = &mut physics.bodies;
    for (_entity, handle) in stale {
        colliders.remove(handle, islands, bodies, true);
    }
}

pub(crate) fn remove_flat_ground(
    commands: &mut Commands,
    physics: &mut PhysicsWorld,
    flat: FlatGround,
) {
    commands.entity(flat.entity).despawn();
    let colliders = &mut physics.colliders;
    let islands = &mut physics.islands;
    let bodies = &mut physics.bodies;
    colliders.remove(flat.collider, islands, bodies, true);
}

fn snap_new_usd_roots_to_terrain(
    mut commands: Commands,
    mut roots: Query<(Entity, &Name, &mut Transform), Added<Name>>,
) {
    for (entity, name, mut tr) in roots.iter_mut() {
        if !name.as_str().starts_with("UsdLoad[") {
            continue;
        }
        // The generic USD loader API documents Y as "height above ground".
        // Convert that offset to absolute Bevy Y for every static load, not
        // just y=0. This keeps target indicators above bales on hills instead
        // of disappearing inside the terrain.
        let requested_ground_offset = tr.translation.y;
        tr.translation.y =
            terrain_height_m(tr.translation.x, tr.translation.z) + requested_ground_offset;

        // For grounded USD assets (bales use y=0), do a second pass after the
        // scene is instantiated and mesh AABBs exist. Asset origins are not
        // guaranteed to be at the bottom, so origin-only terrain snapping can
        // still leave them floating or half buried.
        if requested_ground_offset.abs() < 0.001 {
            commands
                .entity(entity)
                .insert(TerrainBoundsSnapPending { frames_waited: 0 });
        }
    }
}

fn align_new_grounded_usd_bounds_to_terrain(
    mut commands: Commands,
    mut roots: Query<(Entity, &Name, &mut Transform, &mut TerrainBoundsSnapPending)>,
    children: Query<&Children>,
    meshes: Res<Assets<Mesh>>,
    mut physics: ResMut<PhysicsWorld>,
    mut prop_bodies: ResMut<StaticUsdPropBodies>,
    bounds: Query<(
        &GlobalTransform,
        Option<&usd_bevy::UsdSceneRoot>,
        Option<&StaticUsdPhysicsProp>,
        Option<&Mesh3d>,
        Option<&bevy::camera::primitives::Aabb>,
    )>,
) {
    for (root, name, mut root_transform, mut pending) in roots.iter_mut() {
        pending.frames_waited += 1;
        let Some(extent) = loaded_usd_world_extent(root, &children, &meshes, &bounds) else {
            if pending.frames_waited > 240 {
                warn!(
                    "world: no bounds found to terrain-align {}; keeping origin snap",
                    name.as_str()
                );
                commands.entity(root).remove::<TerrainBoundsSnapPending>();
            }
            continue;
        };
        if pending.frames_waited < TERRAIN_BOUNDS_SNAP_SETTLE_FRAMES {
            continue;
        }

        let root_translation_before_adjustment = root_transform.translation;
        let delta_y = TERRAIN_PROP_CONTACT_CLEARANCE_M - extent.min_terrain_clearance;
        if delta_y.abs() > 0.002 {
            root_transform.translation.y += delta_y;
            info!(
                "world: terrain contact adjusted {} by {delta_y:+.3} m (clearance={:+.3})",
                name.as_str(),
                extent.min_terrain_clearance
            );
        }
        if extent.has_scene_root && !extent.has_static_prop_body {
            let (body, collider) = attach_static_usd_prop_body(
                root,
                &root_transform,
                root_translation_before_adjustment,
                &extent,
                physics.as_mut(),
            );
            commands.entity(root).insert(StaticUsdPhysicsProp {
                body,
                visual_top_offset_y: extent.max.y - root_translation_before_adjustment.y,
                frames_alive: 0,
            });
            prop_bodies.handles.insert(root, (body, collider));
            info!(
                "world: attached gravity collider to {} (radius={:.3}, half_length={:.3})",
                name.as_str(),
                extent.collider_radius,
                extent.collider_half_length
            );
        }
        commands.entity(root).remove::<TerrainBoundsSnapPending>();
    }
}

#[derive(Debug, Clone, Copy)]
struct WorldExtent {
    min: Vec3,
    max: Vec3,
    min_terrain_clearance: f32,
    has_scene_root: bool,
    has_static_prop_body: bool,
    collider_radius: f32,
    collider_half_length: f32,
}

fn loaded_usd_world_extent(
    root: Entity,
    children: &Query<&Children>,
    meshes: &Assets<Mesh>,
    bounds: &Query<(
        &GlobalTransform,
        Option<&usd_bevy::UsdSceneRoot>,
        Option<&StaticUsdPhysicsProp>,
        Option<&Mesh3d>,
        Option<&bevy::camera::primitives::Aabb>,
    )>,
) -> Option<WorldExtent> {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut min_terrain_clearance = f32::INFINITY;
    let mut count = 0usize;
    let mut has_scene_root = false;
    let mut has_static_prop_body = false;
    for entity in collect_descendants(root, children) {
        let Ok((gt, scene_root, prop_body, mesh3d, aabb)) = bounds.get(entity) else {
            continue;
        };
        has_scene_root |= scene_root.is_some();
        has_static_prop_body |= prop_body.is_some();
        if let Some(mesh3d) = mesh3d
            && let Some(mesh) = meshes.get(&mesh3d.0)
            && let Some((mesh_min, mesh_max, mesh_clearance)) =
                mesh_world_bounds_and_terrain_clearance(gt, mesh)
        {
            min = min.min(mesh_min);
            max = max.max(mesh_max);
            min_terrain_clearance = min_terrain_clearance.min(mesh_clearance);
            count += 1;
        } else if let Some(aabb) = aabb {
            // Prefer Bevy's AABB because it is computed from the actual mesh
            // vertices after usd_bevy has converted USD Z-up geometry into
            // Gearbox/Bevy Y-up geometry. Authored USD extents stay in the
            // original prim space on some assets (notably the bale.usdz), so
            // using them for ground contact can put the object visibly above
            // or below the terrain.
            let center = Vec3::from(aabb.center);
            let half = Vec3::from(aabb.half_extents);
            let (box_min, box_max, box_clearance) =
                local_box_world_bounds_and_terrain_clearance(gt, center - half, center + half);
            min = min.min(box_min);
            max = max.max(box_max);
            min_terrain_clearance = min_terrain_clearance.min(box_clearance);
            count += 1;
        }
    }
    if !(count > 0 && min_terrain_clearance.is_finite()) {
        return None;
    }
    let size = max - min;
    let collider_radius = (size.y * 0.5).clamp(0.1, 2.0);
    let horizontal_long = size.x.max(size.z);
    let collider_half_length = (horizontal_long * 0.5 - collider_radius).max(0.0);
    Some(WorldExtent {
        min,
        max,
        min_terrain_clearance,
        has_scene_root,
        has_static_prop_body,
        collider_radius,
        collider_half_length,
    })
}

fn attach_static_usd_prop_body(
    root: Entity,
    root_transform: &Transform,
    root_translation_before_adjustment: Vec3,
    extent: &WorldExtent,
    physics: &mut PhysicsWorld,
) -> (RigidBodyHandle, ColliderHandle) {
    let center = (extent.min + extent.max) * 0.5;
    let root_pos = root_transform.translation;
    let local_center =
        root_transform.rotation.inverse() * (center - root_translation_before_adjustment);
    let size = extent.max - extent.min;
    let along_x = size.x >= size.z;
    let body = RigidBodyBuilder::dynamic()
        .pose(Pose {
            translation: DVec3::new(root_pos.x as f64, root_pos.y as f64, root_pos.z as f64),
            rotation: DQuat::from_xyzw(
                root_transform.rotation.x as f64,
                root_transform.rotation.y as f64,
                root_transform.rotation.z as f64,
                root_transform.rotation.w as f64,
            ),
        })
        .linvel(DVec3::ZERO)
        .angvel(DVec3::ZERO)
        .linear_damping(4.0)
        .angular_damping(8.0)
        .can_sleep(true)
        .build();
    let body_handle = physics.bodies.insert(body);
    let mut collider = if along_x {
        ColliderBuilder::capsule_x(
            extent.collider_half_length as f64,
            extent.collider_radius as f64,
        )
    } else {
        ColliderBuilder::capsule_z(
            extent.collider_half_length as f64,
            extent.collider_radius as f64,
        )
    };
    collider = collider
        .translation(DVec3::new(
            local_center.x as f64,
            local_center.y as f64,
            local_center.z as f64,
        ))
        .density(80.0)
        .friction(1.2)
        .restitution(0.05);
    let collider_handle =
        physics
            .colliders
            .insert_with_parent(collider.build(), body_handle, &mut physics.bodies);
    physics.entity_to_body.insert(root, body_handle);
    physics.entity_to_collider.insert(root, collider_handle);
    (body_handle, collider_handle)
}

/// Publish the settled world pose of every loader-spawned static USD prop —
/// once, when its physics body freezes. Scripts subscribe to
/// `gearbox/usd/pose/**` and drive off these authoritative positions instead
/// of guessing where a terrain-snapped, physics-settled asset ended up. This
/// replaces the old proximity-matching marker system: the world reports where
/// things *are*; it never decides what anything targets.
fn publish_loaded_usd_poses(
    physics: Res<PhysicsWorld>,
    bus: Option<ResMut<GearboxBus>>,
    mut published: ResMut<PublishedUsdPoses>,
    props: Query<(Entity, &Name, &Transform, &StaticUsdPhysicsProp)>,
) {
    let Some(mut bus) = bus else {
        return;
    };
    // Forget props that no longer exist (harvested / unloaded). A later load
    // that reuses the same runtime id is a fresh entity, so its pose is then
    // published anew rather than suppressed.
    let live: std::collections::HashSet<Entity> = props.iter().map(|(e, ..)| e).collect();
    published.published.retain(|entity| live.contains(entity));

    for (entity, name, tr, prop) in props.iter() {
        if published.published.contains(&entity) {
            continue;
        }
        let Some(id) = parse_loaded_usd_id(name.as_str()) else {
            continue;
        };
        // Wait for the prop to come to rest. `freeze_settled_static_usd_prop_bodies`
        // flips a settled body to `Fixed`, so a non-dynamic body means the
        // pose reported here is the final resting pose.
        let settled = physics
            .bodies
            .get(prop.body)
            .is_some_and(|body| !body.is_dynamic());
        if !settled {
            continue;
        }
        let pos = tr.translation;
        let top_y = pos.y + prop.visual_top_offset_y;
        bus.publish_event(
            SceneEvent::new(event_kind::POSE, id)
                .at(pos.x, pos.y, pos.z)
                .with_top(top_y),
        );
        published.published.insert(entity);
    }
}

fn harvest_bales_on_machine_contact(
    mut commands: Commands,
    mut physics: ResMut<PhysicsWorld>,
    mut prop_bodies: ResMut<StaticUsdPropBodies>,
    bus: Option<ResMut<GearboxBus>>,
    bales: Query<(Entity, &Name, &Transform, &StaticUsdPhysicsProp)>,
) {
    let mut bus = bus;
    let prop_body_handles = prop_bodies
        .handles
        .values()
        .map(|(body, _)| *body)
        .collect::<std::collections::HashSet<_>>();
    let mut touched = Vec::new();
    for (entity, name, tr, prop) in bales.iter() {
        let Some(bale_id) = parse_loaded_bale_id(name.as_str()) else {
            continue;
        };
        let Some((_body, collider)) = prop_bodies.handles.get(&entity).copied() else {
            continue;
        };
        let hit_non_prop_body = physics
            .narrow_phase
            .contact_pairs_with(collider)
            .filter(|pair| pair.has_any_active_contact())
            .any(|pair| {
                let other_collider = if pair.collider1 == collider {
                    pair.collider2
                } else {
                    pair.collider1
                };
                physics
                    .colliders
                    .get(other_collider)
                    .and_then(|collider| collider.parent())
                    .is_some_and(|body| {
                        body != prop.body
                            && !prop_body_handles.contains(&body)
                            && physics
                                .bodies
                                .get(body)
                                .is_some_and(|body| body.is_dynamic())
                    })
            });
        if hit_non_prop_body {
            touched.push((entity, bale_id, tr.translation));
        }
    }

    for (entity, bale_id, pos) in touched {
        remove_static_prop_body(entity, physics.as_mut(), prop_bodies.as_mut());
        commands.entity(entity).despawn();
        if let Some(bus) = bus.as_deref_mut() {
            bus.publish_event(
                SceneEvent::new(event_kind::HARVESTED, &format!("bale_{bale_id}"))
                    .at(pos.x, pos.y, pos.z)
                    .with_prop("bale_id", &bale_id),
            );
        }
    }
}

fn parse_loaded_bale_id(name: &str) -> Option<String> {
    let rest = name.strip_prefix("UsdLoad[bale_")?;
    let end = rest.find(']')?;
    Some(rest[..end].to_string())
}

/// Extract the loader runtime id from a `UsdLoad[<id>]::…` entity name.
fn parse_loaded_usd_id(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("UsdLoad[")?;
    let end = rest.find(']')?;
    Some(&rest[..end])
}

fn freeze_settled_static_usd_prop_bodies(
    mut props: Query<(Entity, &mut StaticUsdPhysicsProp)>,
    mut physics: ResMut<PhysicsWorld>,
) {
    for (_entity, mut prop) in props.iter_mut() {
        prop.frames_alive = prop.frames_alive.saturating_add(1);
        let Some(body) = physics.bodies.get_mut(prop.body) else {
            continue;
        };
        if !body.is_dynamic() {
            continue;
        }
        if prop.frames_alive < STATIC_PROP_MIN_DYNAMIC_FRAMES {
            continue;
        }

        let linear_speed = body.linvel().length();
        let angular_speed = body.angvel().length();
        let settled = linear_speed < STATIC_PROP_SETTLED_LINEAR_SPEED_MPS
            && angular_speed < STATIC_PROP_SETTLED_ANGULAR_SPEED_RPS;
        let timed_out = prop.frames_alive >= STATIC_PROP_FORCE_FREEZE_FRAMES;
        if settled || timed_out {
            body.set_linvel(DVec3::ZERO, true);
            body.set_angvel(DVec3::ZERO, true);
            body.set_body_type(RigidBodyType::Fixed, true);
        }
    }
}

fn cleanup_static_usd_prop_bodies(
    entities: &Entities,
    mut physics: ResMut<PhysicsWorld>,
    mut prop_bodies: ResMut<StaticUsdPropBodies>,
) {
    let stale = prop_bodies
        .handles
        .keys()
        .copied()
        .filter(|entity| !entities.contains(*entity))
        .collect::<Vec<_>>();
    for entity in stale {
        remove_static_prop_body(entity, physics.as_mut(), prop_bodies.as_mut());
    }
}

fn cleanup_terrain_collision_without_usd_terrain(
    mut commands: Commands,
    terrain_collision: Option<Res<TerrainCollision>>,
    terrain_roots: Query<&Name, With<usd_bevy::UsdSceneRoot>>,
    mut physics: ResMut<PhysicsWorld>,
) {
    let Some(terrain_collision) = terrain_collision.as_deref().copied() else {
        return;
    };
    if terrain_roots
        .iter()
        .any(|name| is_usd_terrain_root_name(name.as_str()))
    {
        return;
    }

    physics
        .entity_to_collider
        .retain(|_, handle| *handle != terrain_collision.terrain);
    let physics = physics.as_mut();
    let colliders = &mut physics.colliders;
    let islands = &mut physics.islands;
    let bodies = &mut physics.bodies;
    colliders.remove(terrain_collision.terrain, islands, bodies, true);
    colliders.remove(terrain_collision.safety_floor, islands, bodies, true);
    commands.remove_resource::<TerrainCollision>();
    USD_TERRAIN_LOADED.store(false, Ordering::Relaxed);
    clear_usd_terrain_height_profile();
    info!("world: removed terrain collision because USD terrain is no longer loaded");
}

fn remove_static_prop_body(
    entity: Entity,
    physics: &mut PhysicsWorld,
    prop_bodies: &mut StaticUsdPropBodies,
) {
    if let Some((body, _collider)) = prop_bodies.handles.remove(&entity) {
        physics.entity_to_body.remove(&entity);
        physics.entity_to_collider.remove(&entity);
        physics.bodies.remove(
            body,
            &mut physics.islands,
            &mut physics.colliders,
            &mut physics.impulse_joints,
            &mut physics.multibody_joints,
            true,
        );
    }
}

fn mesh_world_bounds_and_terrain_clearance(
    gt: &GlobalTransform,
    mesh: &Mesh,
) -> Option<(Vec3, Vec3, f32)> {
    let positions = match mesh.attribute(Mesh::ATTRIBUTE_POSITION)? {
        VertexAttributeValues::Float32x3(positions) => positions,
        _ => return None,
    };
    if positions.is_empty() {
        return None;
    }

    let m = gt.to_matrix();
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut min_clearance = f32::INFINITY;
    for p in positions {
        let w = m.transform_point3(Vec3::from(*p));
        min = min.min(w);
        max = max.max(w);
        min_clearance = min_clearance.min(w.y - terrain_height_m(w.x, w.z));
    }
    Some((min, max, min_clearance))
}

fn local_box_world_bounds_and_terrain_clearance(
    gt: &GlobalTransform,
    local_min: Vec3,
    local_max: Vec3,
) -> (Vec3, Vec3, f32) {
    let m = gt.to_matrix();
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut min_clearance = f32::INFINITY;
    for i in 0..8 {
        let c = Vec3::new(
            if i & 1 == 0 { local_min.x } else { local_max.x },
            if i & 2 == 0 { local_min.y } else { local_max.y },
            if i & 4 == 0 { local_min.z } else { local_max.z },
        );
        let w = m.transform_point3(c);
        min = min.min(w);
        max = max.max(w);
        min_clearance = min_clearance.min(w.y - terrain_height_m(w.x, w.z));
    }
    (min, max, min_clearance)
}

pub(crate) fn collect_descendants(root: Entity, children: &Query<&Children>) -> Vec<Entity> {
    let mut out = vec![root];
    let mut cursor = 0usize;
    while cursor < out.len() {
        let entity = out[cursor];
        cursor += 1;
        if let Ok(kids) = children.get(entity) {
            out.extend(kids.iter());
        }
    }
    out
}

pub fn terrain_height_m(x: f32, z: f32) -> f32 {
    if let Some(h) = crate::terrain::procedural_height_m(x, z) {
        return h;
    }
    if !USD_TERRAIN_LOADED.load(Ordering::Relaxed) {
        return 0.0;
    }
    if USD_TERRAIN_IS_FLAT.load(Ordering::Relaxed) {
        return f32::from_bits(USD_TERRAIN_FLAT_Y_BITS.load(Ordering::Relaxed));
    }
    let mesh = USD_TERRAIN_MESH.read().ok().and_then(|slot| slot.clone());
    if let Some(h) = mesh.and_then(|m| m.height_at(x, z)) {
        return h;
    }
    terrain_height_formula_m(x, z)
}

fn terrain_height_formula_m(x: f32, z: f32) -> f32 {
    let raw = terrain_hills_raw_m(x, z) + terrain_visible_local_relief_m(x, z);
    let origin = terrain_hills_raw_m(0.0, 0.0) + terrain_visible_local_relief_m(0.0, 0.0);
    let distance_from_spawn = (x * x + z * z).sqrt();
    let spawn_fade = smoothstep_range(
        TERRAIN_FLAT_SPAWN_RADIUS_M,
        TERRAIN_FULL_RELIEF_RADIUS_M,
        distance_from_spawn,
    );

    ((raw - origin) * spawn_fade).clamp(TERRAIN_MIN_HEIGHT_M, TERRAIN_MAX_HEIGHT_M)
}

fn terrain_hills_raw_m(x: f32, z: f32) -> f32 {
    // Visible crop-field rolls. Keep this CPU-side so the visible mesh
    // and Rapier heightfield collider are exactly the same shape.
    //
    // The first attempt used 3-6 m hills over hundreds of metres; on an
    // 8 km field from the default camera that reads basically flat. Use
    // stronger nearby rolls so the terrain is unmistakably a mesh, not a
    // texture floating over a plate.
    let rolling_noise = (fbm_world(x * 0.0017 + 12.7, z * 0.0017 - 8.4, 5) - 0.5) * 4.0;
    rolling_noise
        + smooth_hill(x, z, 45.0, 45.0, 70.0, 4.0)
        + smooth_hill(x, z, -70.0, 55.0, 85.0, 3.4)
        + smooth_hill(x, z, 80.0, -65.0, 95.0, 3.0)
        + smooth_hill(x, z, 260.0, -360.0, 300.0, -2.8)
        + smooth_hill(x, z, -340.0, 260.0, 340.0, -2.3)
        + smooth_hill(x, z, 620.0, -430.0, 520.0, 3.5)
        + smooth_hill(x, z, -720.0, 560.0, 560.0, 3.3)
        + smooth_hill(x, z, 1_250.0, 900.0, 850.0, 2.8)
        + smooth_hill(x, z, -1_250.0, -850.0, 900.0, 2.6)
        + smooth_hill(x, z, 2_050.0, -1_450.0, 1_100.0, 2.4)
        + smooth_hill(x, z, -2_150.0, 1_450.0, 1_150.0, 2.3)
}

fn terrain_visible_local_relief_m(x: f32, z: f32) -> f32 {
    // Deliberately obvious local relief near the default camera/tractor.
    // These are real vertex heights, not shader displacement. The
    // `terrain_height_m` origin subtraction keeps exact spawn at y=0 while
    // leaving a clearly visible hill/valley profile tens of metres around
    // the origin.
    let rolling_ridges = (x * 0.020).sin() * (z * 0.015 + 0.7).sin() * 1.2;
    rolling_ridges
        + smooth_hill(x, z, 32.0, 42.0, 38.0, 8.8)
        + smooth_hill(x, z, -44.0, 36.0, 44.0, -4.8)
        + smooth_hill(x, z, 64.0, -42.0, 52.0, 6.2)
        + smooth_hill(x, z, -72.0, -58.0, 60.0, -3.9)
        + smooth_hill(x, z, 0.0, 115.0, 80.0, 5.4)
}

fn smoothstep_range(edge0: f32, edge1: f32, value: f32) -> f32 {
    let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub(crate) fn smooth_hill(x: f32, z: f32, cx: f32, cz: f32, radius: f32, height: f32) -> f32 {
    let dx = x - cx;
    let dz = z - cz;
    let d2 = dx * dx + dz * dz;
    height * (-d2 / (2.0 * radius * radius)).exp()
}

pub(crate) fn fbm_world(x: f32, z: f32, octaves: u32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut freq = 1.0;
    let mut norm = 0.0;
    for _ in 0..octaves {
        sum += amp * value_noise_2d(x * freq, z * freq);
        norm += amp;
        amp *= 0.5;
        freq *= 2.03;
    }
    (sum / norm).clamp(0.0, 1.0)
}

fn value_noise_2d(x: f32, z: f32) -> f32 {
    let xi = x.floor() as i32;
    let zi = z.floor() as i32;
    let xf = x - xi as f32;
    let zf = z - zi as f32;
    let sx = smoothstep(xf);
    let sz = smoothstep(zf);

    let a = hash2(xi, zi);
    let b = hash2(xi + 1, zi);
    let c = hash2(xi, zi + 1);
    let d = hash2(xi + 1, zi + 1);
    let ab = a + (b - a) * sx;
    let cd = c + (d - c) * sx;
    ab + (cd - ab) * sz
}

fn hash2(x: i32, z: i32) -> f32 {
    let mut n = x as u32;
    n = n.wrapping_mul(0x9E37_79B1);
    n ^= (z as u32).wrapping_mul(0x85EB_CA77);
    n ^= n >> 16;
    n = n.wrapping_mul(0xC2B2_AE3D);
    n ^= n >> 15;
    n as f32 / u32::MAX as f32
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::Ordering;

    use super::{TERRAIN_MAX_HEIGHT_M, TERRAIN_MIN_HEIGHT_M, USD_TERRAIN_LOADED, terrain_height_m};

    static TERRAIN_FLAG_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn terrain_mesh_heights_interpolate_the_loaded_surface() {
        use super::{DVec3, TerrainHeightMesh};
        let vertices = [
            DVec3::new(-10.0, 0.0, -10.0),
            DVec3::new(10.0, 4.0, -10.0),
            DVec3::new(10.0, 8.0, 10.0),
            DVec3::new(-10.0, 4.0, 10.0),
        ];
        let triangles = [[0, 1, 2], [0, 2, 3]];
        let mesh = TerrainHeightMesh::build(&vertices, &triangles).expect("mesh");
        assert!((mesh.height_at(0.0, 0.0).unwrap() - 4.0).abs() < 1e-3);
        assert!((mesh.height_at(9.9, -9.9).unwrap() - 4.0).abs() < 0.05);
        assert!((mesh.height_at(5.0, 5.0).unwrap() - 6.0).abs() < 1e-3);
        assert!(mesh.height_at(50.0, 0.0).is_none());
    }

    #[test]
    fn terrain_height_is_flat_until_usd_terrain_is_active() {
        let _lock = TERRAIN_FLAG_TEST_LOCK.lock().unwrap();
        USD_TERRAIN_LOADED.store(false, Ordering::Relaxed);
        assert_eq!(terrain_height_m(35.0, 45.0), 0.0);
    }

    #[test]
    fn terrain_height_has_visible_local_relief() {
        let _lock = TERRAIN_FLAG_TEST_LOCK.lock().unwrap();
        USD_TERRAIN_LOADED.store(true, Ordering::Relaxed);
        let mut min_h = f32::MAX;
        let mut max_h = f32::MIN;
        for zi in -20..=20 {
            for xi in -20..=20 {
                let h = terrain_height_m(xi as f32 * 25.0, zi as f32 * 25.0);
                min_h = min_h.min(h);
                max_h = max_h.max(h);
            }
        }

        assert!(
            terrain_height_m(0.0, 0.0).abs() < 0.001,
            "terrain origin must stay at y=0 for sane default spawning"
        );
        assert!(
            terrain_height_m(5.0, 0.0).abs() < 0.001 && terrain_height_m(0.0, 5.0).abs() < 0.001,
            "exact spawn pad must stay flat so tractors can spawn cleanly"
        );
        assert!(
            max_h <= TERRAIN_MAX_HEIGHT_M + 0.001,
            "terrain max height exceeded cap: max={max_h:.2}"
        );
        assert!(
            min_h >= TERRAIN_MIN_HEIGHT_M - 0.001,
            "terrain min height exceeded cap: min={min_h:.2}"
        );
        assert!(
            max_h - min_h > 4.0,
            "local terrain relief is too subtle: min={min_h:.2}, max={max_h:.2}, span={:.2}",
            max_h - min_h
        );
        USD_TERRAIN_LOADED.store(false, Ordering::Relaxed);
    }
}

/// Friction of the generated grounds; `GEARBOX_GROUND_FRICTION` overrides
/// it to test slippery surfaces.
pub fn ground_friction(default: f64) -> f64 {
    std::env::var("GEARBOX_GROUND_FRICTION")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|f| *f >= 0.0)
        .unwrap_or(default)
}
