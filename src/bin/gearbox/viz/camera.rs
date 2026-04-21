//! Free orbit camera — astrocraft-style mouse navigation.
//!
//! Bindings:
//!   Scroll wheel                — zoom (logarithmic, smoothed)
//!   Middle-click + drag         — pan (translate focus in world XZ plane)
//!   Left + Right pressed + drag — orbit (yaw + pitch)
//!   Double middle-click         — snap focus to cursor's world-point
//!
//! The camera has NO automatic follow; driving a vehicle with WASD does not
//! move the view. Use double-middle-click to re-centre.

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use big_space::prelude::{BigSpace, CellCoord, Grid};

use gearbox::VehicleId;

use super::GearboxSim;

/// Attach this to a `Camera3d` entity. Fully user-driven.
#[derive(Component, Clone)]
pub struct ChaseCamera {
    /// World-space focus point.
    pub focus: Vec3,

    /// Orbit angle around world +Y, radians. 0 looks along +Z.
    pub yaw: f32,
    /// Elevation above the horizon, radians.
    pub elevation: f32,
    pub distance: f32,

    pub min_distance: f32,
    pub max_distance: f32,

    pub pan_sensitivity: f32,
    pub orbit_speed: f32,
    /// Exponential zoom coefficient — 0.05 = 5% per scroll line.
    pub zoom_step: f64,
    /// Smoothing for zoom (exponential toward target distance).
    pub zoom_smoothing: f64,

    pub last_middle_click_secs: f32,

    /// When set, the camera cinematically flies to behind the given
    /// vehicle at the given distance. Focus, yaw and distance are
    /// all eased toward their targets every frame, so the camera
    /// arcs around the machine instead of flying straight through
    /// it. Cleared on arrival, on user gesture, or if the vehicle
    /// disappears.
    pub fly_target: Option<FlyTarget>,
}

/// Destination + scripted-animation state for the "double-click to
/// focus" camera move. Three sub-phases run on overlapping sub-
/// intervals of a single `duration`:
///
///   0-40 %   — pull back: camera zooms out (distance grows to
///              `start_distance.max(target_distance) * 1.8`) while
///              its focus eases onto the target so "eye on the
///              machine" is locked in.
///   40-50 %  — hold at the apex; camera now faces the target.
///   50-100 % — orbit in: yaw arcs (shortest angle) to behind the
///              vehicle while distance shrinks to `distance`.
#[derive(Copy, Clone, Debug)]
pub struct FlyTarget {
    pub vehicle: VehicleId,
    pub distance: f32,
    pub duration: f32,
    pub elapsed: f32,
    pub start_focus: Vec3,
    pub start_distance: f32,
    pub start_yaw: f32,
}

impl FlyTarget {
    /// Build a fresh target, snapshotting the camera's current pose
    /// as the animation's starting point.
    pub fn new(vehicle: VehicleId, distance: f32, duration: f32, cam: &ChaseCamera) -> Self {
        Self {
            vehicle,
            distance,
            duration: duration.max(0.01),
            elapsed: 0.0,
            start_focus: cam.focus,
            start_distance: cam.distance,
            start_yaw: cam.yaw,
        }
    }
}

impl Default for ChaseCamera {
    fn default() -> Self {
        Self {
            focus: Vec3::ZERO,
            yaw: 0.0,
            elevation: 25f32.to_radians(),
            distance: 14.0,
            min_distance: 3.0,
            max_distance: 120.0,
            pan_sensitivity: 0.0012,
            orbit_speed: 0.005,
            zoom_step: 0.05,
            zoom_smoothing: 6.0,
            last_middle_click_secs: -10.0,
            fly_target: None,
        }
    }
}

/// Cinematic "fly to behind the selected vehicle". Runs a scripted
/// animation over `FlyTarget::duration`:
///
///   1. 0–40 %  — pull back (distance → apex) while the focus
///                eases onto the target, so the camera is "looking
///                at the machine" well before it moves in.
///   2. 40–50 % — hold at the apex, briefly.
///   3. 50–100 %— orbit in: yaw arcs toward `vehicle_yaw + π` while
///                distance shrinks to `FlyTarget::distance`.
///
/// Target focus/yaw are re-read from the vehicle's current pose every
/// frame, so the camera stays locked on even if the machine moves
/// during the flight.
pub fn chase_camera_fly(
    time: Res<Time>,
    sim: Res<GearboxSim>,
    mut cameras: Query<(&mut ChaseCamera, &mut Transform, &mut CellCoord)>,
    root_grid: Query<&Grid, With<BigSpace>>,
) {
    let cell_size = root_grid.single().map(|g| g.cell_edge_length()).unwrap_or(2000.0);
    let dt = time.delta_secs();
    let tau = std::f32::consts::TAU;

    for (mut cam, mut tr, mut cell) in &mut cameras {
        let Some(mut target) = cam.fly_target else { continue };
        if sim.0.vehicle(target.vehicle).is_none() {
            cam.fly_target = None;
            continue;
        }

        target.elapsed += dt;
        let t = (target.elapsed / target.duration).clamp(0.0, 1.0);

        // Live target pose.
        let pose = sim.0.vehicle_pose(target.vehicle);
        let target_focus = Vec3::new(
            pose.point.x as f32,
            pose.point.y as f32,
            pose.point.z as f32,
        );
        let q = Quat::from_xyzw(
            pose.rotation.x as f32,
            pose.rotation.y as f32,
            pose.rotation.z as f32,
            pose.rotation.w as f32,
        );
        let fwd = q * Vec3::Z;
        let vehicle_yaw = fwd.x.atan2(fwd.z);
        let target_yaw = vehicle_yaw + std::f32::consts::PI;

        // ── Focus: eye on the machine during the pull-back phase ──
        let s_focus = smoothstep(sub_progress(t, 0.0, 0.4));
        cam.focus = target.start_focus.lerp(target_focus, s_focus);

        // ── Distance: bell with a brief hold at the apex ──
        let apex = target.start_distance.max(target.distance) * 1.8;
        cam.distance = if t < 0.4 {
            // Phase 1: zoom out, start → apex.
            let s = smoothstep(sub_progress(t, 0.0, 0.4));
            lerp_f32(target.start_distance, apex, s)
        } else if t < 0.5 {
            apex
        } else {
            // Phase 3: zoom in, apex → target distance.
            let s = smoothstep(sub_progress(t, 0.5, 1.0));
            lerp_f32(apex, target.distance, s)
        };

        // ── Yaw: spin from 30 % onward (after the eye is settled) ──
        let s_yaw = smoothstep(sub_progress(t, 0.3, 1.0));
        let mut yaw_gap = (target_yaw - target.start_yaw) % tau;
        if yaw_gap > std::f32::consts::PI {
            yaw_gap -= tau;
        } else if yaw_gap < -std::f32::consts::PI {
            yaw_gap += tau;
        }
        cam.yaw = target.start_yaw + yaw_gap * s_yaw;

        if t >= 1.0 {
            // Snap to final targets + clear.
            cam.focus = target_focus;
            cam.distance = target.distance;
            cam.yaw = target_yaw;
            cam.fly_target = None;
        } else {
            cam.fly_target = Some(target);
        }

        apply_rig_big_space(&cam, cell_size, &mut tr, &mut cell);
    }
}

fn sub_progress(t: f32, a: f32, b: f32) -> f32 {
    ((t - a) / (b - a)).clamp(0.0, 1.0)
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn lerp_f32(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Handles pan (middle drag), orbit (L+R drag), and double-middle-click
/// re-centring (ray-casts the cursor to the ground plane).
pub fn chase_camera_control(
    time: Res<Time>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    primary_window: Query<&Window, With<PrimaryWindow>>,
    bevy_cameras: Query<(&Camera, &GlobalTransform)>,
    mut pan_anchor: Local<Option<Vec2>>,
    mut orbit_anchor: Local<Option<Vec2>>,
    mut cameras: Query<(&mut ChaseCamera, &mut Transform, &mut CellCoord)>,
    root_grid: Query<&Grid, With<BigSpace>>,
) {
    let middle_pressed = mouse_buttons.pressed(MouseButton::Middle);
    let left_pressed = mouse_buttons.pressed(MouseButton::Left);
    let right_pressed = mouse_buttons.pressed(MouseButton::Right);
    let both_lr = left_pressed && right_pressed;

    if !middle_pressed {
        *pan_anchor = None;
    }
    if !both_lr {
        *orbit_anchor = None;
    }

    let cursor_position = primary_window.single().ok().and_then(|w| w.cursor_position());

    // --- Pan: middle-click drag ---
    let mut pan_delta = Vec2::ZERO;
    if middle_pressed {
        if let Some(pos) = cursor_position {
            if let Some(anchor) = *pan_anchor {
                pan_delta = pos - anchor;
            }
            *pan_anchor = Some(pos);
        }
    }

    // --- Orbit: left+right click drag ---
    let mut orbit_delta = Vec2::ZERO;
    if both_lr {
        if let Some(pos) = cursor_position {
            if orbit_anchor.is_none() {
                *orbit_anchor = Some(pos);
            }
            if let Some(anchor) = *orbit_anchor {
                orbit_delta = pos - anchor;
            }
            *orbit_anchor = Some(pos);
        }
    }

    let now = time.elapsed_secs();
    let cell_size = root_grid.single().map(|g| g.cell_edge_length()).unwrap_or(2000.0);

    for (mut cam, mut tr, mut cell) in &mut cameras {
        // Double-middle-click → re-centre focus on cursor-to-ground point.
        if mouse_buttons.just_pressed(MouseButton::Middle) {
            let is_double = now - cam.last_middle_click_secs < 0.35;
            cam.last_middle_click_secs = now;
            if is_double {
                if let (Some(cursor), Ok((camera, cam_tr))) = (cursor_position, bevy_cameras.single()) {
                    if let Some(hit) = cursor_ray_to_ground(camera, cam_tr, cursor) {
                        cam.focus = hit;
                    }
                }
            }
        }

        // Pan → slide focus in world XZ plane, aligned to current yaw.
        if pan_delta != Vec2::ZERO {
            let pan_speed = cam.distance * cam.pan_sensitivity;
            let forward = Vec3::new(cam.yaw.sin(), 0.0, cam.yaw.cos());
            let right = Vec3::new(forward.z, 0.0, -forward.x);
            cam.focus += (-right * pan_delta.x - forward * pan_delta.y) * pan_speed;
            cam.fly_target = None; // user took over → cancel auto-fly
        }

        // Orbit.
        if orbit_delta != Vec2::ZERO {
            cam.yaw -= orbit_delta.x * cam.orbit_speed;
            cam.elevation += orbit_delta.y * cam.orbit_speed;
            cam.elevation = cam.elevation.clamp(
                5f32.to_radians(),
                89f32.to_radians(),
            );
            cam.fly_target = None;
        }

        apply_rig_big_space(&cam, cell_size, &mut tr, &mut cell);
    }
}

/// Scroll-wheel zoom — logarithmic with exponential smoothing.
/// Skips when Ctrl is held: that gesture is reserved for rotating
/// the ghost during drag-to-place.
pub fn chase_camera_zoom(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut wheel: MessageReader<MouseWheel>,
    mut zoom_target: Local<Option<f64>>,
    mut cameras: Query<(&mut ChaseCamera, &mut Transform, &mut CellCoord)>,
    root_grid: Query<&Grid, With<BigSpace>>,
) {
    if keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight) {
        // Drain so the event doesn't accumulate while Ctrl is held.
        wheel.read().for_each(drop);
        return;
    }
    let mut scroll_delta = 0.0_f64;
    for event in wheel.read() {
        scroll_delta += match event.unit {
            MouseScrollUnit::Line => event.y as f64,
            MouseScrollUnit::Pixel => event.y as f64 / 32.0,
        };
    }

    let Ok((mut cam, mut tr, mut cell)) = cameras.single_mut() else { return };
    let cell_size = root_grid.single().map(|g| g.cell_edge_length()).unwrap_or(2000.0);

    let target = zoom_target.get_or_insert(cam.distance as f64);
    let min = cam.min_distance as f64;
    let max = cam.max_distance as f64;

    if scroll_delta != 0.0 {
        let log_target = target.max(0.1).log10();
        let new_log = log_target - scroll_delta * cam.zoom_step;
        *target = 10f64.powf(new_log).clamp(min, max);
        cam.fly_target = None; // scrolling cancels an auto-fly
    }

    // Keep the zoom target synced with the distance being written by
    // the fly system; otherwise the first post-fly scroll would snap
    // back to the pre-fly distance.
    if cam.fly_target.is_some() {
        *target = cam.distance as f64;
    }

    let dt = time.delta_secs_f64();
    let log_current = (cam.distance as f64).max(0.1).ln();
    let log_target = target.max(0.1).ln();
    let log_diff = log_target - log_current;
    if log_diff.abs() > 1e-4 {
        let new_log = log_current + log_diff * (cam.zoom_smoothing * dt).min(0.9);
        cam.distance = new_log.exp() as f32;
        apply_rig_big_space(&cam, cell_size, &mut tr, &mut cell);
    } else if log_diff.abs() > 1e-5 {
        cam.distance = *target as f32;
        apply_rig_big_space(&cam, cell_size, &mut tr, &mut cell);
    }
}

/// Place the camera in world coords, split into `(CellCoord, Transform)`
/// so f32 precision stays inside one grid cell regardless of zoom level.
fn apply_rig_big_space(
    cam: &ChaseCamera,
    cell_size: f32,
    tr: &mut Transform,
    cell: &mut CellCoord,
) {
    let horizontal = cam.distance * cam.elevation.cos();
    let vertical = cam.distance * cam.elevation.sin();
    let offset = Vec3::new(
        horizontal * cam.yaw.sin(),
        vertical,
        horizontal * cam.yaw.cos(),
    );
    let cam_world = cam.focus + offset;

    // Split the camera's world-space position into an integer cell index
    // and a small local offset — this is what big_space's internal
    // recentering would do, but we write it up-front so our own writes
    // don't race with the recentre system.
    let new_cell = CellCoord::new(
        (cam_world.x / cell_size).round() as i32,
        (cam_world.y / cell_size).round() as i32,
        (cam_world.z / cell_size).round() as i32,
    );
    let cell_origin = Vec3::new(
        new_cell.x as f32 * cell_size,
        new_cell.y as f32 * cell_size,
        new_cell.z as f32 * cell_size,
    );

    let cam_local = cam_world - cell_origin;
    let focus_local = cam.focus - cell_origin;

    *tr = Transform::from_translation(cam_local).looking_at(focus_local, Vec3::Y);
    *cell = new_cell;
}

fn cursor_ray_to_ground(
    camera: &Camera,
    cam_tr: &GlobalTransform,
    cursor: Vec2,
) -> Option<Vec3> {
    let ray = camera.viewport_to_world(cam_tr, cursor).ok()?;
    let origin = ray.origin;
    let direction = *ray.direction;
    if direction.y.abs() < 1e-6 {
        return None;
    }
    let t = -origin.y / direction.y;
    if t < 0.0 {
        return None;
    }
    Some(origin + direction * t)
}
