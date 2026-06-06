//! "Drag-to-place" vehicle spawning.
//!
//! Flow:
//!   1. Spawn-panel button sets `PendingSpawn::spec` to a VehicleSpec.
//!   2. `spawn_ghost_if_needed` creates a translucent ghost of that
//!      spec.
//!   3. `update_ghost_position` moves the ghost to the cursor's
//!      ground-plane projection every frame.
//!   4. `commit_or_cancel_ghost` watches for:
//!        LMB (not over UI) → commit: despawn ghost, spawn real vehicle
//!        Esc / RMB          → cancel: despawn ghost, clear request

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::EguiContexts;

use gearbox_physics::{
    VehicleSpec,
    datapod::{Point, Pose, Quaternion},
};

use gearbox_viz::{
    GearboxSim, GhostTag, PlayerControlled, spawn_height_for, spawn_vehicle_ghost,
    spawn_vehicle_visuals,
};

use super::selection::{Selection, cursor_ray_to_ground};

/// Pending vehicle-placement request. A `Some(spec)` means "the user
/// has picked a preset and is currently dragging it in the viewport".
#[derive(Resource, Default)]
pub struct PendingSpawn {
    pub spec: Option<VehicleSpec>,
    pub ghost_root: Option<Entity>,
    /// Yaw (rad, around +Y) applied to the ghost and to the committed
    /// vehicle. Adjusted with Ctrl + mouse-wheel while placing.
    pub yaw: f32,
}

impl PendingSpawn {
    /// Queue a new placement; if a ghost is already up, cancel it
    /// first (called from the spawn panel when the user picks a new
    /// preset mid-drag).
    pub fn request(&mut self, spec: VehicleSpec, commands: &mut Commands) {
        if let Some(old) = self.ghost_root.take() {
            commands.entity(old).despawn();
        }
        self.spec = Some(spec);
        self.yaw = 0.0;
    }
}

/// Rotation rate per scroll-line when Ctrl is held.
const ROTATE_PER_LINE: f32 = 0.20; // ≈11°/line — 32 clicks = full turn

/// If a spec is requested but we haven't materialised a ghost yet,
/// build it now.
pub fn spawn_ghost_if_needed(
    mut pending: ResMut<PendingSpawn>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<bevy::image::Image>>,
) {
    if pending.ghost_root.is_some() || pending.spec.is_none() {
        return;
    }
    let Some(spec) = pending.spec.clone() else {
        return;
    };
    let root = spawn_vehicle_ghost(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut images,
        &spec,
    );
    pending.ghost_root = Some(root);
}

/// Move the ghost's root Transform to the cursor-ground-plane hit
/// every frame. Does nothing when no placement is active.
pub fn update_ghost_position(
    pending: Res<PendingSpawn>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    mut ghost_q: Query<&mut Transform, With<GhostTag>>,
) {
    let Some(spec) = pending.spec.as_ref() else {
        return;
    };
    let Some(ghost) = pending.ghost_root else {
        return;
    };
    let Ok(window) = windows.single() else { return };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let Ok((camera, cam_tr)) = cameras.single() else {
        return;
    };
    let Some(hit) = cursor_ray_to_ground(camera, cam_tr, cursor, 0.0) else {
        return;
    };
    if let Ok(mut tr) = ghost_q.get_mut(ghost) {
        tr.translation = Vec3::new(hit.x, spawn_height_for(spec) as f32, hit.z);
        tr.rotation = Quat::from_rotation_y(pending.yaw);
    }
}

/// Ctrl + mouse-wheel rotates the ghost around its vertical axis.
/// Also intercepts the wheel event so `chase_camera_zoom` doesn't
/// also zoom at the same time (handled on that side — see
/// `chase_camera_zoom`'s `wants_rotate_ghost` early-return).
pub fn rotate_ghost_on_ctrl_wheel(
    mut pending: ResMut<PendingSpawn>,
    keys: Res<ButtonInput<KeyCode>>,
    mut wheel: MessageReader<MouseWheel>,
) {
    if pending.spec.is_none() {
        // Drain so we don't carry a stale event into the next placement.
        wheel.read().for_each(drop);
        return;
    }
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    if !ctrl {
        return;
    }
    let mut delta = 0.0_f32;
    for event in wheel.read() {
        delta += match event.unit {
            MouseScrollUnit::Line => event.y,
            MouseScrollUnit::Pixel => event.y / 32.0,
        };
    }
    if delta != 0.0 {
        pending.yaw += delta * ROTATE_PER_LINE;
    }
}

/// Tracked across frames while the user holds LMB. Used to tell a
/// clean click ("place here") from a drag-to-orbit gesture ("don't
/// you dare place it while I'm moving the camera").
#[derive(Default)]
pub struct ClickState {
    press_cursor: Option<Vec2>,
    saw_rmb_while_held: bool,
    saw_drag: bool,
}

const CLICK_DRAG_THRESHOLD_PX: f32 = 5.0;

/// Commit on a *clean* LMB click in the viewport; cancel ONLY on Esc.
/// Middle-click panning and L+R orbiting both leave the ghost alone.
pub fn commit_or_cancel_ghost(
    mut pending: ResMut<PendingSpawn>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut contexts: EguiContexts,
    mut commands: Commands,
    mut sim: ResMut<GearboxSim>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<bevy::image::Image>>,
    asset_server: Res<bevy::asset::AssetServer>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    player_tagged: Query<Entity, With<PlayerControlled>>,
    mut selection: ResMut<Selection>,
    mut state: Local<ClickState>,
) {
    let Some(spec) = pending.spec.clone() else {
        // No pending placement → clear any stale click state.
        *state = ClickState::default();
        return;
    };

    // Cancel: Esc only. MMB and RMB stay free for camera pan/orbit.
    if keys.just_pressed(KeyCode::Escape) {
        if let Some(e) = pending.ghost_root.take() {
            commands.entity(e).despawn();
        }
        pending.spec = None;
        *state = ClickState::default();
        return;
    }

    let Ok(window) = windows.single() else { return };
    let Some(cursor) = window.cursor_position() else {
        return;
    };

    // Don't start a click while the pointer is hovering a panel.
    let over_ui = contexts
        .ctx_mut()
        .map(|c| c.wants_pointer_input())
        .unwrap_or(false);

    // LMB pressed now → start tracking a potential click.
    if mouse.just_pressed(MouseButton::Left) && !over_ui {
        state.press_cursor = Some(cursor);
        state.saw_rmb_while_held = mouse.pressed(MouseButton::Right);
        state.saw_drag = false;
    }

    // LMB currently held → watch for RMB (orbit) or drag motion
    // (camera manipulation), either of which invalidates "this was a
    // click to place".
    if mouse.pressed(MouseButton::Left) {
        if mouse.pressed(MouseButton::Right) {
            state.saw_rmb_while_held = true;
        }
        if let Some(press) = state.press_cursor {
            if press.distance(cursor) > CLICK_DRAG_THRESHOLD_PX {
                state.saw_drag = true;
            }
        }
    }

    // LMB just released → commit iff it looked like a clean click.
    if !mouse.just_released(MouseButton::Left) {
        return;
    }
    let was_clean_click =
        state.press_cursor.is_some() && !state.saw_rmb_while_held && !state.saw_drag;
    let press_started_in_viewport = state.press_cursor.is_some();
    *state = ClickState::default();
    if !was_clean_click || !press_started_in_viewport {
        return;
    }

    // Commit at the release position.
    let Ok((camera, cam_tr)) = cameras.single() else {
        return;
    };
    let Some(hit) = cursor_ray_to_ground(camera, cam_tr, cursor, 0.0) else {
        return;
    };

    if let Some(e) = pending.ghost_root.take() {
        commands.entity(e).despawn();
    }
    let yaw = pending.yaw as f64;
    pending.spec = None;

    // Yaw around +Y → quaternion (cos(θ/2), 0, sin(θ/2), 0).
    let half = yaw * 0.5;
    let rotation = Quaternion::new(half.cos(), 0.0, half.sin(), 0.0);

    let pose = Pose {
        point: Point::new(hit.x as f64, spawn_height_for(&spec), hit.z as f64),
        rotation,
    };
    let id = sim.0.spawn_vehicle(spec.clone(), pose);
    let root = spawn_vehicle_visuals(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut images,
        &asset_server,
        id,
        &spec,
    );
    for e in player_tagged.iter() {
        commands.entity(e).remove::<PlayerControlled>();
    }
    commands.entity(root).insert(PlayerControlled);
    selection.vehicle = Some(id);
}
