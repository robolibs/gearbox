//! Cinematic fly-to-vehicle camera animation, layered on top of
//! `bevy_glacial`'s base [`ChaseCamera`].
//!
//! The base orbit / pan / zoom rig — including the [`ChaseCamera`]
//! component itself, [`chase_camera_control`] and
//! [`chase_camera_zoom`] systems, and the `apply_rig` /
//! `cursor_ray_to_ground` helpers — lives in
//! [`bevy_glacial`](https://github.com/bresilla/bevy_glacial).
//! This module only adds the gearbox-specific fly-to-vehicle
//! cinematic.
//!
//! State is held on a [`ChaseCameraFly`] resource (one camera per
//! app, so a singleton fits cleaner than a per-entity component).
//! Set `fly.target = Some(FlyTarget::new(...))` and the
//! [`chase_camera_fly`] system animates the camera over the next
//! `duration` seconds. Any user mouse gesture (middle-drag,
//! left+right-drag) cancels the in-flight animation.

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;

pub use bevy_glacial::{
    apply_rig, chase_camera_control, chase_camera_zoom, cursor_ray_to_ground, ChaseCamera,
};

use gearbox_core::VehicleId;

use super::GearboxSim;

/// Destination + scripted-animation state for the "double-click to
/// focus" camera move. Two-phase animation:
///
///   **Phase A — pure rotation** (`t ∈ [0, PHASE_A_END]`)
///   The camera's *world position* is pinned to `start_cam_world`
///   while the focus point lerps from `start_focus` → `target_focus`.
///   `(yaw, distance, elevation)` are *derived* each frame from
///   `(start_cam_world − focus)` so `apply_rig` rotates around the
///   pinned camera position. Net effect: the camera turns its head
///   to look at the machine without translating. Front-loaded so the
///   "I want to see the thing" motion is the first thing the eye
///   notices.
///
///   **Phase B — orbit-in arc** (`t ∈ [PHASE_A_END, 1]`)
///   Standard rig animation. Focus is locked on the live vehicle
///   position; distance pulls back to apex then in to final; yaw
///   arcs to behind-vehicle. The Phase-A end-state is the starting
///   point for these tracks (derived per-frame from the pinned
///   `start_cam_world` and the live `target_focus`, so a moving
///   vehicle stays consistent).
#[derive(Copy, Clone, Debug)]
pub struct FlyTarget {
    pub vehicle: VehicleId,
    pub distance: f32,
    pub duration: f32,
    pub elapsed: f32,
    pub start_focus: Vec3,
    /// Camera's world position at fly-start. Phase A keeps the
    /// camera pinned here while the focus rotates; Phase B uses it
    /// to derive the rig start-state.
    pub start_cam_world: Vec3,
    pub start_distance: f32,
    pub start_yaw: f32,
    pub start_elevation: f32,
    pub apex_distance: f32,
    /// Previous-frame world position of the followed vehicle. Used
    /// to slide `start_focus` and `start_cam_world` by the per-frame
    /// target delta so the lerp / pin reference frame stays anchored
    /// to a moving vehicle (otherwise the camera judders because the
    /// gap geometry shifts under the interpolation).
    pub last_target_pos: Option<Vec3>,
}

impl FlyTarget {
    /// Apex distance reached at the midpoint of the fly. **Fixed** —
    /// not a function of machine size — so every machine's cinematic
    /// arc has the same shape (pull back ~45 m, hover, spiral in to
    /// `distance`). Small robots and big harvesters share the same
    /// route; only the final focus point differs.
    pub const APEX_DISTANCE: f32 = 45.0;

    /// Fraction of the duration spent on the rotate-only phase.
    /// Snappy enough to feel like the priority motion, smooth enough
    /// not to whip-pan.
    pub const PHASE_A_END: f32 = 0.30;

    /// Build a fresh target, snapshotting the camera's current pose
    /// as the animation's starting point. `start_cam_world` is
    /// derived from the rig (focus + offset(yaw, distance,
    /// elevation)) so we know exactly where the camera is in world
    /// space without needing the Transform at the call site.
    pub fn new(vehicle: VehicleId, distance: f32, duration: f32, cam: &ChaseCamera) -> Self {
        let horizontal = cam.distance * cam.elevation.cos();
        let vertical = cam.distance * cam.elevation.sin();
        let offset = Vec3::new(
            horizontal * cam.yaw.sin(),
            vertical,
            horizontal * cam.yaw.cos(),
        );
        Self {
            vehicle,
            distance,
            duration: duration.max(0.01),
            elapsed: 0.0,
            start_focus: cam.focus,
            start_cam_world: cam.focus + offset,
            start_distance: cam.distance,
            start_yaw: cam.yaw,
            start_elevation: cam.elevation,
            apex_distance: Self::APEX_DISTANCE,
            last_target_pos: None,
        }
    }
}

/// Per-app fly-target slot. `None` = no animation in flight.
#[derive(Resource, Default, Debug)]
pub struct ChaseCameraFly {
    pub target: Option<FlyTarget>,
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
/// Cancels when the user gives any camera input (middle-drag,
/// left+right-drag, or scroll-wheel zoom).
pub fn chase_camera_fly(
    time: Res<Time>,
    sim: Res<GearboxSim>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut wheel: MessageReader<MouseWheel>,
    mut fly: ResMut<ChaseCameraFly>,
    mut cameras: Query<(&mut ChaseCamera, &mut Transform)>,
) {
    if fly.target.is_none() {
        // Drain any wheel events so the next frame doesn't see them
        // as "happened during fly".
        wheel.read().for_each(drop);
        return;
    }

    // Cancel on any user camera input this frame.
    let middle = mouse_buttons.pressed(MouseButton::Middle);
    let both_lr = mouse_buttons.pressed(MouseButton::Left)
        && mouse_buttons.pressed(MouseButton::Right);
    let scrolled = wheel
        .read()
        .any(|e| match e.unit {
            MouseScrollUnit::Line | MouseScrollUnit::Pixel => e.y.abs() > f32::EPSILON,
        });
    if middle || both_lr || scrolled {
        fly.target = None;
        return;
    }

    let dt = time.delta_secs();

    for (mut cam, mut tr) in &mut cameras {
        let Some(mut target) = fly.target else { continue };
        if sim.0.vehicle(target.vehicle).is_none() {
            fly.target = None;
            continue;
        }

        target.elapsed += dt;
        let t = (target.elapsed / target.duration).clamp(0.0, 1.0);

        // Live target pose — the vehicle can move during the fly.
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
        let target_cam_yaw = vehicle_yaw + std::f32::consts::PI;

        // Slide `start_focus` AND `start_cam_world` by the vehicle's
        // per-frame motion so both reference points stay anchored
        // relative to the moving vehicle. Without this, on a moving
        // target the lerp endpoints (Phase A) and the rig-start
        // derivation (Phase B) drift out from under the interpolation
        // and the camera judders. With it, the animation behaves on
        // a driving vehicle the same way it does on a parked one.
        if let Some(last) = target.last_target_pos {
            let delta = target_focus - last;
            target.start_focus += delta;
            target.start_cam_world += delta;
        }
        target.last_target_pos = Some(target_focus);

        let phase_a_end = FlyTarget::PHASE_A_END;

        if t < phase_a_end {
            // ── Phase A: pure rotation ───────────────────────────
            // Camera world position pinned to `start_cam_world`.
            // Focus lerps from `start_focus` to `target_focus`. We
            // *derive* yaw/distance/elevation from the geometry
            // `(start_cam_world - focus)` so `apply_rig` produces
            // a Transform whose translation equals start_cam_world
            // and whose rotation looks at the lerped focus — i.e.
            // the camera turns its head without translating.
            let s = smoothstep((t / phase_a_end).clamp(0.0, 1.0));
            let focus = target.start_focus.lerp(target_focus, s);
            let off = target.start_cam_world - focus;
            let dist = off.length().max(0.1);
            cam.focus = focus;
            cam.distance = dist;
            cam.yaw = off.x.atan2(off.z);
            cam.elevation = (off.y / dist).clamp(-1.0, 1.0).asin();
        } else {
            // ── Phase B: orbit-in arc ────────────────────────────
            // Focus locked on the live vehicle position. Derive the
            // Phase-B start state per-frame from `start_cam_world`
            // (vehicle-anchored) and `target_focus` (live), so the
            // rig params at tb=0 reproduce the Phase-A end pose
            // even after vehicle motion. Then lerp to (apex →
            // final, behind-vehicle yaw, original elevation).
            let tb = ((t - phase_a_end) / (1.0 - phase_a_end)).clamp(0.0, 1.0);

            let off = target.start_cam_world - target_focus;
            let dist_b_start = off.length().max(0.1);
            let yaw_b_start = off.x.atan2(off.z);
            let elev_b_start = (off.y / dist_b_start).clamp(-1.0, 1.0).asin();

            // Distance: rise to apex by tb=0.5, fall to final by 1.0.
            let distance = if tb < 0.5 {
                let s = smoothstep(sub_progress(tb, 0.0, 0.5));
                dist_b_start + (target.apex_distance - dist_b_start) * s
            } else {
                let s = smoothstep(sub_progress(tb, 0.5, 1.0));
                target.apex_distance + (target.distance - target.apex_distance) * s
            };

            // Yaw: shortest-arc lerp from derived start to behind-
            // vehicle, spread across most of Phase B so the orbit
            // happens *concurrently* with the pull-back-then-pull-in
            // rather than as a late tack-on. Finishes at tb=0.85 so
            // the last 15% is a clean settle with no more rotation.
            let s_yaw = smoothstep(sub_progress(tb, 0.0, 0.85));
            let tau = std::f32::consts::TAU;
            let mut yaw_gap = (target_cam_yaw - yaw_b_start) % tau;
            if yaw_gap > std::f32::consts::PI {
                yaw_gap -= tau;
            } else if yaw_gap < -std::f32::consts::PI {
                yaw_gap += tau;
            }
            let yaw = yaw_b_start + yaw_gap * s_yaw;

            // Elevation: ease from derived back to the user's
            // original elevation across the full Phase-B duration.
            let s_elev = smoothstep(tb);
            let elevation = elev_b_start + (target.start_elevation - elev_b_start) * s_elev;

            cam.focus = target_focus;
            cam.yaw = yaw;
            cam.distance = distance;
            cam.elevation = elevation;
        }

        if t >= 1.0 {
            cam.focus = target_focus;
            cam.yaw = target_cam_yaw;
            cam.distance = target.distance;
            cam.elevation = target.start_elevation;
            fly.target = None;
        } else {
            fly.target = Some(target);
        }

        apply_rig(&cam, &mut tr);
    }
}

fn sub_progress(t: f32, a: f32, b: f32) -> f32 {
    ((t - a) / (b - a)).clamp(0.0, 1.0)
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}
