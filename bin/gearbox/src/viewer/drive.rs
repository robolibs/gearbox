//! Who drives a machine from the viewer: the Machine pane's sliders, or the
//! first connected gamepad. `MachinePanel` remembers which drive controllers
//! the viewer holds and which one the gamepad steers; `gamepad_drive` turns
//! pad input into a `CmdVel` on the same path the bus uses.

use std::collections::HashSet;

use bevy::prelude::*;

use crate::controller::{CmdVel, ControllerKey, UiDrive};

pub struct DrivePlugin;

impl Plugin for DrivePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MachinePanel>()
            .add_systems(Update, gamepad_drive);
    }
}

#[derive(Resource, Default)]
pub struct MachinePanel {
    pub last_selection: Option<Entity>,
    /// Drive controllers the viewer currently holds.
    pub holding: HashSet<ControllerKey>,
    /// The drive controller the gamepad steers, if any.
    pub gamepad: Option<ControllerKey>,
    /// Machines whose containers have been folded once.
    pub folded: HashSet<String>,
}

impl MachinePanel {
    pub fn holds(&self, key: &ControllerKey) -> bool {
        self.holding.contains(key)
    }

    pub fn gamepad_on(&self, key: &ControllerKey) -> bool {
        self.gamepad.as_ref() == Some(key)
    }

    /// Take or release a drive controller for the viewer's own commands.
    pub fn set_viewer_drive(&mut self, ui_drive: &mut UiDrive, key: &ControllerKey, on: bool) {
        if on {
            self.holding.insert(key.clone());
            ui_drive.0.entry(key.clone()).or_insert(CmdVel::default());
        } else {
            self.holding.remove(key);
            ui_drive.0.remove(key);
            if self.gamepad.as_ref() == Some(key) {
                self.gamepad = None;
            }
        }
    }

    /// Hand a drive controller to the gamepad, or take it back to the sliders.
    pub fn set_gamepad(&mut self, ui_drive: &mut UiDrive, key: &ControllerKey, on: bool) {
        if on {
            self.gamepad = Some(key.clone());
            self.holding.insert(key.clone());
        } else if self.gamepad.as_ref() == Some(key) {
            self.gamepad = None;
            ui_drive.0.insert(key.clone(), CmdVel::default());
        }
    }
}

pub const DRIVE_TYPES: [&str; 2] = ["builtin:ackermann_cmd_vel", "builtin:diff_drive_cmd_vel"];
const PAD_SPEED_MPS: f32 = 4.0;
const PAD_YAW_RPS: f32 = 1.0;
const PAD_DEADZONE: f32 = 0.12;

/// The first connected gamepad drives the held controller while the pane's
/// switch is on: right trigger forward, left trigger back (or a stick's Y,
/// or the D-pad), left stick steers (or the D-pad), south button stops.
/// Inputs are logged once a second while a stick or trigger is off centre.
fn gamepad_drive(
    time: Res<Time>,
    gamepads: Query<(&Gamepad, Option<&Name>)>,
    panel: Res<MachinePanel>,
    mut ui_drive: ResMut<UiDrive>,
    mut log_at: Local<f32>,
) {
    let Some(key) = panel.gamepad.clone() else {
        return;
    };
    let Some((pad, name)) = gamepads.iter().next() else {
        if time.elapsed_secs() >= *log_at {
            *log_at = time.elapsed_secs() + 5.0;
            warn!("gearbox-pad: the Gamepad switch is on but no gamepad is connected");
        }
        return;
    };
    let dead = |v: f32| if v.abs() < PAD_DEADZONE { 0.0 } else { v };
    let axis = |a: GamepadAxis| dead(pad.get(a).unwrap_or(0.0));
    let button = |b: GamepadButton| dead(pad.get(b).unwrap_or(0.0));
    let throttle = button(GamepadButton::RightTrigger2) - button(GamepadButton::LeftTrigger2);
    let stick_y = if axis(GamepadAxis::LeftStickY) != 0.0 {
        axis(GamepadAxis::LeftStickY)
    } else {
        axis(GamepadAxis::RightStickY)
    };
    let dpad_y = button(GamepadButton::DPadUp) - button(GamepadButton::DPadDown);
    let forward = [throttle, stick_y, dpad_y]
        .into_iter()
        .find(|v| *v != 0.0)
        .unwrap_or(0.0);
    let dpad_x = button(GamepadButton::DPadRight) - button(GamepadButton::DPadLeft);
    let steer = [
        axis(GamepadAxis::LeftStickX),
        axis(GamepadAxis::RightStickX),
        dpad_x,
    ]
    .into_iter()
    .find(|v| *v != 0.0)
    .unwrap_or(0.0);
    if (forward != 0.0 || steer != 0.0) && time.elapsed_secs() >= *log_at {
        *log_at = time.elapsed_secs() + 1.0;
        info!(
            "gearbox-pad: {} → forward {forward:.2} steer {steer:.2} (RT {:.2} LT {:.2} LS {:.2},{:.2} RS {:.2},{:.2} dpad {dpad_x:.0},{dpad_y:.0})",
            name.map(|n| n.as_str()).unwrap_or("gamepad"),
            button(GamepadButton::RightTrigger2),
            button(GamepadButton::LeftTrigger2),
            axis(GamepadAxis::LeftStickX),
            axis(GamepadAxis::LeftStickY),
            axis(GamepadAxis::RightStickX),
            axis(GamepadAxis::RightStickY),
        );
    }
    let cmd = if pad.pressed(GamepadButton::South) {
        CmdVel {
            linear_mps: 0.0,
            angular_rps: 0.0,
        }
    } else {
        CmdVel {
            linear_mps: forward * PAD_SPEED_MPS,
            angular_rps: -steer * PAD_YAW_RPS,
        }
    };
    ui_drive.0.insert(key, cmd);
}
