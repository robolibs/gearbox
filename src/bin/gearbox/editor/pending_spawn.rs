//! "Drag-to-place" vehicle spawning.
//!
//! Flow:
//!   1. Spawn-panel button sets `PendingSpawn::spec` to a VehicleSpec.
//!   2. `spawn_ghost_if_needed` creates a translucent ghost of that
//!      spec under the BigSpace root.
//!   3. `update_ghost_position` moves the ghost to the cursor's
//!      ground-plane projection every frame.
//!   4. `commit_or_cancel_ghost` watches for:
//!        LMB (not over UI) → commit: despawn ghost, spawn real vehicle
//!        Esc / RMB          → cancel: despawn ghost, clear request

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::EguiContexts;

use gearbox::{
    datapod::{Point, Pose, Quaternion},
    VehicleSpec,
};

use crate::BigSpaceRoot;
use crate::viz::{
    spawn_height_for, spawn_vehicle_ghost, spawn_vehicle_visuals, GearboxSim, GhostTag,
    PlayerControlled,
};

use super::selection::{cursor_ray_to_ground, Selection};

/// Pending vehicle-placement request. A `Some(spec)` means "the user
/// has picked a preset and is currently dragging it in the viewport".
#[derive(Resource, Default)]
pub struct PendingSpawn {
    pub spec: Option<VehicleSpec>,
    pub ghost_root: Option<Entity>,
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
    }
}

/// If a spec is requested but we haven't materialised a ghost yet,
/// build it now.
pub fn spawn_ghost_if_needed(
    mut pending: ResMut<PendingSpawn>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    big_space_root: Res<BigSpaceRoot>,
) {
    if pending.ghost_root.is_some() || pending.spec.is_none() {
        return;
    }
    let Some(spec) = pending.spec.clone() else { return };
    let root = spawn_vehicle_ghost(
        &mut commands,
        &mut meshes,
        &mut materials,
        &spec,
        big_space_root.0,
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
    let Some(spec) = pending.spec.as_ref() else { return };
    let Some(ghost) = pending.ghost_root else { return };
    let Ok(window) = windows.single() else { return };
    let Some(cursor) = window.cursor_position() else { return };
    let Ok((camera, cam_tr)) = cameras.single() else { return };
    let Some(hit) = cursor_ray_to_ground(camera, cam_tr, cursor, 0.0) else {
        return;
    };
    if let Ok(mut tr) = ghost_q.get_mut(ghost) {
        tr.translation = Vec3::new(hit.x, spawn_height_for(spec) as f32, hit.z);
    }
}

/// Watch for the click that either commits (LMB in-viewport) or
/// cancels (Esc / RMB / Middle).
pub fn commit_or_cancel_ghost(
    mut pending: ResMut<PendingSpawn>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut contexts: EguiContexts,
    mut commands: Commands,
    mut sim: ResMut<GearboxSim>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    player_tagged: Query<Entity, With<PlayerControlled>>,
    mut selection: ResMut<Selection>,
    big_space_root: Res<BigSpaceRoot>,
) {
    let Some(spec) = pending.spec.clone() else { return };

    // Cancel?
    let cancel = keys.just_pressed(KeyCode::Escape)
        || mouse.just_pressed(MouseButton::Right)
        || mouse.just_pressed(MouseButton::Middle);
    if cancel {
        if let Some(e) = pending.ghost_root.take() {
            commands.entity(e).despawn();
        }
        pending.spec = None;
        return;
    }

    // Don't commit when the pointer is over an egui panel.
    let over_ui = contexts
        .ctx_mut()
        .map(|c| c.wants_pointer_input())
        .unwrap_or(false);
    if over_ui {
        return;
    }

    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }

    // Commit: project the cursor to the ground and spawn the real vehicle.
    let Ok(window) = windows.single() else { return };
    let Some(cursor) = window.cursor_position() else { return };
    let Ok((camera, cam_tr)) = cameras.single() else { return };
    let Some(hit) = cursor_ray_to_ground(camera, cam_tr, cursor, 0.0) else { return };

    if let Some(e) = pending.ghost_root.take() {
        commands.entity(e).despawn();
    }
    pending.spec = None;

    let pose = Pose {
        point: Point::new(hit.x as f64, spawn_height_for(&spec), hit.z as f64),
        rotation: Quaternion::identity(),
    };
    let id = sim.0.spawn_vehicle(spec.clone(), pose);
    let root = spawn_vehicle_visuals(
        &mut commands,
        &mut meshes,
        &mut materials,
        id,
        &spec,
        big_space_root.0,
    );
    for e in player_tagged.iter() {
        commands.entity(e).remove::<PlayerControlled>();
    }
    commands.entity(root).insert(PlayerControlled);
    selection.vehicle = Some(id);
}
