//! WASD keyboard → `ControlInput` for every `PlayerControlled` vehicle.
//!
//! A companion system, `sync_player_to_selection_system`, keeps the
//! `PlayerControlled` tag aligned with the editor's current selection:
//! clicking a vehicle in the viewport (or picking one in the scene
//! tree) automatically hands WASD control to that vehicle.

use bevy::prelude::*;

use super::{GearboxSim, PlayerControlled, VehicleBody};
use crate::editor::selection::Selection;
use gearbox::ControlInput;

pub fn wasd_input_system(
    keys: Res<ButtonInput<KeyCode>>,
    mut sim: ResMut<GearboxSim>,
    players: Query<&VehicleBody, With<PlayerControlled>>,
) {
    let throttle = axis(&keys, KeyCode::KeyW, KeyCode::KeyS);
    // A steers left, D steers right. Rapier treats positive
    // `wheel.steering` as a rotation around -suspension (i.e. +up),
    // which pivots the wheels left — so A (turn left) maps to
    // +steer. Drones reuse the same axis for strafe.
    let steer = axis(&keys, KeyCode::KeyA, KeyCode::KeyD);
    let brake = if keys.pressed(KeyCode::Space) { 1.0 } else { 0.0 };
    // Drone-only axes (zero for ground vehicles):
    //   Q/E — yaw left/right
    //   Z/X — ascend/descend
    let yaw  = axis(&keys, KeyCode::KeyQ, KeyCode::KeyE);
    let lift = axis(&keys, KeyCode::KeyZ, KeyCode::KeyX);
    let ctrl = ControlInput { throttle, brake, steer, yaw, lift };

    for body in &players {
        sim.0.set_control(body.id, ctrl);
    }
}

/// Rewrite the `PlayerControlled` tag so the selected vehicle is
/// always the one WASD drives. Nothing selected → nothing tagged →
/// WASD has no effect (remote-controlled vehicles will use a
/// different tag and won't need to be selected).
pub fn sync_player_to_selection_system(
    mut commands: Commands,
    selection: Res<Selection>,
    bodies: Query<(Entity, &VehicleBody, Has<PlayerControlled>)>,
) {
    if !selection.is_changed() {
        return;
    }
    let target_id = selection.vehicle;
    for (entity, body, is_player) in &bodies {
        let should_drive = target_id == Some(body.id);
        if should_drive && !is_player {
            commands.entity(entity).insert(PlayerControlled);
        } else if !should_drive && is_player {
            commands.entity(entity).remove::<PlayerControlled>();
        }
    }
}

fn axis(keys: &ButtonInput<KeyCode>, pos: KeyCode, neg: KeyCode) -> f32 {
    let mut v = 0.0;
    if keys.pressed(pos) {
        v += 1.0;
    }
    if keys.pressed(neg) {
        v -= 1.0;
    }
    v
}
