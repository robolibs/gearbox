//! Keyboard (WASD) + gamepad → `ControlInput` for every
//! `PlayerControlled` vehicle.
//!
//! The keyboard and gamepad axes are merged by larger-magnitude-wins
//! (see `gamepad::merge_axis`), so either input source can take over
//! without stomping the other on partial input.
//!
//! The companion system that ties the `PlayerControlled` tag to the
//! editor's current selection lives in `gearbox-editor` (viz has no
//! knowledge of selection state — that's an editor concern).

use bevy::prelude::*;

use super::gamepad::{merge_axis, GamepadState};
use super::{GearboxSim, PlayerControlled, VehicleBody};
use gearbox_core::ControlInput;

pub fn wasd_input_system(
    keys: Res<ButtonInput<KeyCode>>,
    gamepad: Res<GamepadState>,
    mut sim: ResMut<GearboxSim>,
    players: Query<&VehicleBody, With<PlayerControlled>>,
) {
    let kb_throttle = axis(&keys, KeyCode::KeyW, KeyCode::KeyS);
    // A steers left, D steers right. Rapier treats positive
    // `wheel.steering` as a rotation around -suspension (i.e. +up),
    // which pivots the wheels left — so A (turn left) maps to
    // +steer. Drones reuse the same axis for strafe.
    let kb_steer = axis(&keys, KeyCode::KeyA, KeyCode::KeyD);
    let kb_brake = if keys.pressed(KeyCode::Space) { 1.0 } else { 0.0 };
    // Drone-only axes (zero for ground vehicles):
    //   Q/E — yaw left/right
    //   Z/X — ascend/descend
    let kb_yaw = axis(&keys, KeyCode::KeyQ, KeyCode::KeyE);
    let kb_lift = axis(&keys, KeyCode::KeyZ, KeyCode::KeyX);

    // ControlInput is f64 (matches rapier-f64). Inputs come from
    // keyboard + gamepad as f32, so upcast at the boundary.
    let ctrl = ControlInput {
        throttle: merge_axis(kb_throttle, gamepad.throttle)              as f64,
        steer:    merge_axis(kb_steer,    gamepad.steer)                 as f64,
        brake:    merge_axis(kb_brake,    gamepad.brake).max(0.0)        as f64,
        yaw:      merge_axis(kb_yaw,      gamepad.yaw)                   as f64,
        lift:     merge_axis(kb_lift,     gamepad.lift)                  as f64,
    };

    for body in &players {
        sim.0.set_control(body.id, ctrl);
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
