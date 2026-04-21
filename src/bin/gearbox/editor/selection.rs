//! Click-to-select + drag-to-move.
//!
//! Left-click on a vehicle's chassis selects it. Dragging the mouse while
//! left is held (and right isn't — that combo is orbit) teleports the
//! vehicle to the cursor's projection on the ground plane, zeroing its
//! velocities so it doesn't shoot off.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::EguiContexts;

use gearbox::{
    datapod::{Point, Pose, Quaternion},
    VehicleId,
};

use crate::viz::GearboxSim;

/// What, if anything, is currently selected.
#[derive(Resource, Default)]
pub struct Selection {
    pub vehicle: Option<VehicleId>,
    drag: Option<DragState>,
}

struct DragState {
    /// Cursor position at press — used to distinguish a click from a drag.
    press_cursor: Vec2,
    /// True once we've moved far enough to count as a drag.
    active: bool,
    /// Drop height above ground so the wheels aren't embedded.
    drop_y: f32,
}

const DRAG_THRESHOLD_PX: f32 = 4.0;

pub fn pick_and_drag_system(
    mut selection: ResMut<Selection>,
    mut sim: ResMut<GearboxSim>,
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    mut contexts: EguiContexts,
) {
    // Don't grab input when the cursor is over an egui panel.
    let over_ui = contexts
        .ctx_mut()
        .map(|ctx| ctx.wants_pointer_input())
        .unwrap_or(false);

    let right_held = buttons.pressed(MouseButton::Right);
    let Ok(window) = windows.single() else { return };
    let Some(cursor) = window.cursor_position() else {
        selection.drag = None;
        return;
    };
    let Ok((camera, cam_tr)) = cameras.single() else { return };

    // Release → clear drag.
    if buttons.just_released(MouseButton::Left) {
        selection.drag = None;
    }

    if over_ui {
        // Selection changes only happen in-viewport.
        return;
    }

    // Press: raycast and pick.
    if buttons.just_pressed(MouseButton::Left) && !right_held {
        let id = cursor_pick_vehicle(&sim, camera, cam_tr, cursor);
        selection.vehicle = id;
        if let Some(id) = id {
            let drop_y = sim.0.vehicle_pose(id).point.y.max(0.8) as f32;
            selection.drag = Some(DragState {
                press_cursor: cursor,
                active: false,
                drop_y,
            });
        } else {
            selection.drag = None;
        }
    }

    // Drag: teleport the selected vehicle to the ground under the cursor.
    let vehicle_id = selection.vehicle;
    if let Some(drag) = selection.drag.as_mut() {
        if !buttons.pressed(MouseButton::Left) || right_held {
            return;
        }
        if !drag.active && cursor.distance(drag.press_cursor) > DRAG_THRESHOLD_PX {
            drag.active = true;
        }
        if drag.active {
            if let Some(id) = vehicle_id {
                if let Some(ground_hit) = cursor_ray_to_ground(camera, cam_tr, cursor, 0.0) {
                    let pose = Pose {
                        point: Point::new(
                            ground_hit.x as f64,
                            drag.drop_y as f64,
                            ground_hit.z as f64,
                        ),
                        rotation: Quaternion::identity(),
                    };
                    sim.0.set_vehicle_pose(id, pose);
                }
            }
        }
    }
}

fn cursor_pick_vehicle(
    sim: &GearboxSim,
    camera: &Camera,
    cam_tr: &GlobalTransform,
    cursor: Vec2,
) -> Option<VehicleId> {
    let ray = camera.viewport_to_world(cam_tr, cursor).ok()?;
    let origin = ray.origin;
    let direction = *ray.direction;

    let mut best: Option<(VehicleId, f32)> = None;
    for (id, state) in sim.0.vehicles() {
        let pose = sim.0.vehicle_pose(id);
        let centre = Vec3::new(
            pose.point.x as f32,
            pose.point.y as f32,
            pose.point.z as f32,
        );
        let rot = Quat::from_xyzw(
            pose.rotation.x as f32,
            pose.rotation.y as f32,
            pose.rotation.z as f32,
            pose.rotation.w as f32,
        );
        let half = Vec3::new(
            (state.spec.chassis.size.x * 0.5) as f32,
            (state.spec.chassis.size.y * 0.5) as f32,
            (state.spec.chassis.size.z * 0.5) as f32,
        );
        if let Some(t) = ray_obb_intersect(origin, direction, centre, rot, half) {
            if best.map_or(true, |(_, bt)| t < bt) {
                best = Some((id, t));
            }
        }
    }
    best.map(|(id, _)| id)
}

pub fn cursor_ray_to_ground(
    camera: &Camera,
    cam_tr: &GlobalTransform,
    cursor: Vec2,
    plane_y: f32,
) -> Option<Vec3> {
    let ray = camera.viewport_to_world(cam_tr, cursor).ok()?;
    let origin = ray.origin;
    let direction = *ray.direction;
    if direction.y.abs() < 1e-6 {
        return None;
    }
    let t = (plane_y - origin.y) / direction.y;
    if t < 0.0 {
        return None;
    }
    Some(origin + direction * t)
}

/// Slab-test ray vs oriented box. Returns the near-hit `t` if the ray
/// enters the box, ignoring hits strictly behind the origin.
fn ray_obb_intersect(
    origin: Vec3,
    dir: Vec3,
    centre: Vec3,
    rot: Quat,
    half: Vec3,
) -> Option<f32> {
    let inv = rot.inverse();
    let local_origin = inv * (origin - centre);
    let local_dir = inv * dir;

    let mut t_min = f32::NEG_INFINITY;
    let mut t_max = f32::INFINITY;
    for i in 0..3 {
        let o = local_origin[i];
        let d = local_dir[i];
        let h = half[i];
        if d.abs() < 1e-8 {
            if o < -h || o > h {
                return None;
            }
        } else {
            let t1 = (-h - o) / d;
            let t2 = (h - o) / d;
            let (tn, tx) = if t1 < t2 { (t1, t2) } else { (t2, t1) };
            t_min = t_min.max(tn);
            t_max = t_max.min(tx);
            if t_min > t_max {
                return None;
            }
        }
    }
    if t_max < 0.0 {
        return None;
    }
    Some(t_min.max(0.0))
}
