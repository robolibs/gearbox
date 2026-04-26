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
/// focus" camera move. Parametric: every frame we derive
/// `(focus, yaw, distance, elevation)` from four smoothstep tracks
/// and let the rig place the camera from those. Because distance
/// and yaw are independent tracks, the spin can start *while the
/// camera is still rising* — it simultaneously pulls back and
/// begins to arc, then curves back in as the distance track
/// reverses.
///
///   Focus pan  [0.10, 0.50]  — old focus → vehicle position.
///   Distance   [0.00, 0.50]  — start → apex  (going up / out).
///              [0.50, 1.00]  — apex  → final (coming down / in).
///   Yaw        [0.375, 1.00] — start_yaw → behind-vehicle yaw.
///                             Kicks in 1/4 of the pull-back window
///                             *before* the pull-back finishes, so
///                             the camera is spinning **and** still
///                             rising through the overlap.
///   Elevation: held at `start_elevation`.
#[derive(Copy, Clone, Debug)]
pub struct FlyTarget {
    pub vehicle: VehicleId,
    pub distance: f32,
    pub duration: f32,
    pub elapsed: f32,
    pub start_focus: Vec3,
    pub start_distance: f32,
    pub start_yaw: f32,
    pub start_elevation: f32,
    pub apex_distance: f32,
}

impl FlyTarget {
    /// Apex distance reached at the midpoint of the fly. **Fixed** —
    /// not a function of machine size — so every machine's cinematic
    /// arc has the same shape (pull back ~45 m, hover, spiral in to
    /// `distance`). Small robots and big harvesters share the same
    /// route; only the final focus point differs.
    pub const APEX_DISTANCE: f32 = 45.0;

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
            start_elevation: cam.elevation,
            apex_distance: Self::APEX_DISTANCE,
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

        // Focus eases onto the target during the pull-back.
        let s_focus = smoothstep(sub_progress(t, 0.1, 0.5));
        let focus = target.start_focus.lerp(target_focus, s_focus);

        // Distance: up for the first half, down for the second.
        let distance = if t < 0.5 {
            let s = smoothstep(sub_progress(t, 0.0, 0.5));
            target.start_distance + (target.apex_distance - target.start_distance) * s
        } else {
            let s = smoothstep(sub_progress(t, 0.5, 1.0));
            target.apex_distance + (target.distance - target.apex_distance) * s
        };

        // Yaw: starts at t = 0.375 (1/4 of the pull-back window
        // before the pull-back finishes).
        let s_yaw = smoothstep(sub_progress(t, 0.375, 1.0));
        let tau = std::f32::consts::TAU;
        let mut yaw_gap = (target_cam_yaw - target.start_yaw) % tau;
        if yaw_gap > std::f32::consts::PI {
            yaw_gap -= tau;
        } else if yaw_gap < -std::f32::consts::PI {
            yaw_gap += tau;
        }
        let yaw = target.start_yaw + yaw_gap * s_yaw;

        cam.focus = focus;
        cam.yaw = yaw;
        cam.distance = distance;
        cam.elevation = target.start_elevation;

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
