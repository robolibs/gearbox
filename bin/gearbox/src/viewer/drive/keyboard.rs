use bevy::prelude::*;

use super::{DRIVE_TYPES, HostInputFocus, MachinePanel};
use crate::controller::{CmdVel, ControllerInventory, ControllerKey, UiDrive};
use crate::viewer::systems::Selection;

/// WASD drives whichever machine is selected in the viewport; with nothing
/// selected (or the selection not a drivable machine), the keys fall
/// through untouched to the free camera fly controls in `world.rs`. A held
/// gamepad trigger still owns the machine it's driving, same as the
/// on-screen slider does.
pub(super) fn route(
    keys: Res<ButtonInput<KeyCode>>,
    focus: Res<HostInputFocus>,
    inventory: Res<ControllerInventory>,
    selection: Res<Selection>,
    mut panel: ResMut<MachinePanel>,
    mut ui: ResMut<UiDrive>,
) {
    let root = focus.0.then_some(selection.0).flatten();
    let target = root.and_then(|root| {
        let machine = inventory
            .machines
            .iter()
            .find(|m| m.scene_root == Some(root))?;
        let controller = machine
            .controllers
            .iter()
            .find(|c| c.enabled && DRIVE_TYPES.contains(&c.controller_type.as_str()))?;
        Some((
            ControllerKey::new(root, &machine.id, &controller.instance),
            controller.controller_type == DRIVE_TYPES[0],
        ))
    });

    if let Some(previous) = panel.keyboard.clone()
        && target.as_ref().map(|(key, _)| key) != Some(&previous)
    {
        panel.keyboard = None;
        panel.holding.remove(&previous);
        ui.stop_once(&previous);
    }

    let Some((key, ackermann)) = target else { return };
    if panel.gamepad_on(&key) {
        // The gamepad's trigger is already armed for this machine; don't
        // stomp its command with a zero one just because no key is down.
        return;
    }

    let axis =
        |neg: KeyCode, pos: KeyCode| (keys.pressed(pos) as i8 - keys.pressed(neg) as i8) as f32;
    let throttle = axis(KeyCode::KeyS, KeyCode::KeyW);
    let steering = axis(KeyCode::KeyD, KeyCode::KeyA);

    panel.keyboard = Some(key.clone());
    panel.holding.insert(key.clone());
    ui.drive(
        &key,
        CmdVel {
            linear_mps: throttle * 4.0,
            angular_rps: steering,
        },
        ackermann.then_some(steering),
    );
}
