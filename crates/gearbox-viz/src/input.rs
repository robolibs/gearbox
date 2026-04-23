//! Keyboard (WASD) → `ControlInput` for every `PlayerControlled`
//! vehicle.
//!
//! Gamepad support used to live here via `gilrs`; it was removed when
//! we moved external control onto the robot-API layer (zenoh). The
//! editor keeps keyboard teleop as a dev convenience; anything else —
//! gamepad, joystick, scripted agents — is expected to publish to the
//! same control channel over the network.
//!
//! The companion system that ties the `PlayerControlled` tag to the
//! editor's current selection lives in `gearbox-editor` (viz has no
//! knowledge of selection state — that's an editor concern).

use bevy::prelude::*;

use super::{GearboxSim, PlayerControlled, VehicleBody};
use gearbox_core::ControlInput;

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
    let yaw = axis(&keys, KeyCode::KeyQ, KeyCode::KeyE);
    let lift = axis(&keys, KeyCode::KeyZ, KeyCode::KeyX);

    // ControlInput is f64 (matches rapier-f64). Inputs are f32, so
    // upcast at the boundary.
    let ctrl = ControlInput {
        throttle: throttle as f64,
        steer: steer as f64,
        brake: brake as f64,
        yaw: yaw as f64,
        lift: lift as f64,
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
