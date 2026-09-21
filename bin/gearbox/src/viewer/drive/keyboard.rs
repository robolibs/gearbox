use bevy::prelude::*;

use super::{DRIVE_TYPES, HostInputFocus, MachinePanel};
use crate::controller::{CmdVel, ControllerInventory, ControllerKey, UiDrive};
use crate::viewer::systems::Selection;

/// Held WASD keys drive the selected machine; release emits a one-shot stop.
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

    let Some((key, ackermann)) = target else {
        return;
    };
    if panel.gamepad_on(&key) {
        panel.keyboard = None;
        return;
    }
    if ![KeyCode::KeyW, KeyCode::KeyA, KeyCode::KeyS, KeyCode::KeyD]
        .into_iter()
        .any(|key| keys.pressed(key))
    {
        if panel.keyboard.as_ref() == Some(&key) {
            panel.keyboard = None;
            panel.holding.remove(&key);
            ui.stop_once(&key);
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::{ControllerCommands, ControllerRuntimeState, apply_ui_drive};

    fn fixture() -> (App, [ControllerKey; 2]) {
        let mut app = App::new();
        let roots = [
            app.world_mut().spawn_empty().id(),
            app.world_mut().spawn_empty().id(),
        ];
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/tractor.usd");
        let machine = crate::controller::discover_machines_from_usd(&path)
            .unwrap()
            .remove(0);
        let machines: Vec<_> = roots
            .iter()
            .enumerate()
            .map(|(i, &root)| {
                let mut machine = machine.clone();
                machine.id = format!("tractor_{i}");
                machine.scene_root = Some(root);
                machine
            })
            .collect();
        let keys = std::array::from_fn(|i| ControllerKey::new(roots[i], &machines[i].id, "drive"));
        app.init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(HostInputFocus(true))
            .insert_resource(Selection(Some(roots[0])))
            .insert_resource(ControllerInventory { machines })
            .init_resource::<MachinePanel>()
            .init_resource::<UiDrive>()
            .init_resource::<ControllerCommands>()
            .init_resource::<ControllerRuntimeState>()
            .add_systems(Update, (route, apply_ui_drive).chain());
        (app, keys)
    }

    fn frame(app: &mut App, key: &ControllerKey) -> (f32, f32) {
        app.world_mut()
            .resource_mut::<ControllerCommands>()
            .cmd_vel
            .insert(
                key.clone(),
                CmdVel {
                    linear_mps: 2.0,
                    angular_rps: 0.4,
                },
            );
        app.update();
        let command = app.world().resource::<ControllerCommands>().cmd_vel[key];
        (command.linear_mps, command.angular_rps)
    }

    #[test]
    fn idle_selection_does_not_override_remote_or_slider_commands() {
        let (mut app, keys) = fixture();
        for _ in 0..3 {
            assert_eq!(frame(&mut app, &keys[0]), (2.0, 0.4));
            assert!(app.world().resource::<MachinePanel>().keyboard.is_none());
        }
        app.world_mut().resource_mut::<UiDrive>().drive(
            &keys[0],
            CmdVel {
                linear_mps: 1.5,
                angular_rps: -0.2,
            },
            Some(-0.2),
        );
        assert_eq!(frame(&mut app, &keys[0]), (1.5, -0.2));
    }

    #[test]
    fn releasing_keyboard_stops_once_then_yields_to_remote() {
        let (mut app, keys) = fixture();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyW);
        assert_eq!(frame(&mut app, &keys[0]), (4.0, 0.0));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::KeyW);
        assert_eq!(frame(&mut app, &keys[0]), (0.0, 0.0));
        assert_eq!(frame(&mut app, &keys[0]), (2.0, 0.4));
        assert!(!app.world().resource::<MachinePanel>().holds(&keys[0]));
    }

    #[test]
    fn focus_loss_stops_only_keyboard_owned_commands() {
        let (mut app, keys) = fixture();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyW);
        assert_eq!(frame(&mut app, &keys[0]), (4.0, 0.0));
        app.world_mut().resource_mut::<HostInputFocus>().0 = false;
        assert_eq!(frame(&mut app, &keys[0]), (0.0, 0.0));
        assert_eq!(frame(&mut app, &keys[0]), (2.0, 0.4));
        app.world_mut().resource_mut::<HostInputFocus>().0 = true;
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release_all();
        assert_eq!(frame(&mut app, &keys[0]), (2.0, 0.4));
    }

    #[test]
    fn selection_changes_release_old_keyboard_without_claiming_idle_target() {
        let (mut app, keys) = fixture();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyW);
        assert_eq!(frame(&mut app, &keys[0]), (4.0, 0.0));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release_all();
        app.world_mut().resource_mut::<Selection>().0 = Some(keys[1].scene_root);
        assert_eq!(frame(&mut app, &keys[0]), (0.0, 0.0));
        assert_eq!(frame(&mut app, &keys[1]), (2.0, 0.4));
        assert_eq!(frame(&mut app, &keys[0]), (2.0, 0.4));
        app.world_mut().resource_mut::<Selection>().0 = None;
        assert_eq!(frame(&mut app, &keys[1]), (2.0, 0.4));
    }

    #[test]
    fn gamepad_takes_precedence_and_clears_keyboard_ownership() {
        let (mut app, keys) = fixture();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyW);
        assert_eq!(frame(&mut app, &keys[0]), (4.0, 0.0));
        {
            let mut panel = app.world_mut().resource_mut::<MachinePanel>();
            panel.gamepad = Some(keys[0].clone());
            panel.holding.clear();
        }
        app.world_mut().resource_mut::<UiDrive>().drive(
            &keys[0],
            CmdVel {
                linear_mps: 1.5,
                angular_rps: -0.2,
            },
            Some(-0.2),
        );
        assert_eq!(frame(&mut app, &keys[0]), (1.5, -0.2));
        assert!(app.world().resource::<MachinePanel>().keyboard.is_none());
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release_all();
        assert_eq!(frame(&mut app, &keys[0]), (1.5, -0.2));
    }

    #[test]
    fn opposing_keys_still_explicitly_own_zero_command() {
        let (mut app, keys) = fixture();
        for key in [KeyCode::KeyW, KeyCode::KeyS] {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(key);
        }
        assert_eq!(frame(&mut app, &keys[0]), (0.0, 0.0));
        assert!(app.world().resource::<MachinePanel>().keyboard_on(&keys[0]));
    }
}
