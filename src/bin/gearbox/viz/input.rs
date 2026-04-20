//! WASD keyboard → `ControlInput` for every `PlayerControlled` vehicle.

use bevy::prelude::*;

use super::{GearboxSim, PlayerControlled, VehicleBody};
use gearbox::ControlInput;

pub fn wasd_input_system(
    keys: Res<ButtonInput<KeyCode>>,
    mut sim: ResMut<GearboxSim>,
    players: Query<&VehicleBody, With<PlayerControlled>>,
) {
    let throttle = axis(&keys, KeyCode::KeyW, KeyCode::KeyS);
    // A steers left, D steers right. Rapier treats positive `wheel.steering`
    // as a rotation around -suspension (i.e. +up), which pivots the wheels
    // left — so A (turn left) maps to +steer.
    let steer = axis(&keys, KeyCode::KeyA, KeyCode::KeyD);
    let brake = if keys.pressed(KeyCode::Space) { 1.0 } else { 0.0 };
    let ctrl = ControlInput { throttle, brake, steer };

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
