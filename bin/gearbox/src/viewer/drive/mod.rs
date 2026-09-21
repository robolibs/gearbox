//! Viewer ownership and the Bevy adapter for layered operator controls.

mod gamepad;
mod keyboard;

use crate::controller::ControllerKey;
use bevy::prelude::*;
use std::collections::HashSet;

pub struct DrivePlugin;

impl Plugin for DrivePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MachinePanel>()
            .init_resource::<HostInputFocus>()
            .init_resource::<gamepad::GamepadState>()
            .insert_resource(ControlSettings::load())
            .add_systems(
                Update,
                (gamepad::route, keyboard::route, gamepad::camera)
                    .chain()
                    .before(crate::controller::apply_ui_drive)
                    .after(crate::viewer::systems::pick_on_click),
            );
    }
}

#[derive(Resource, Default)]
pub(crate) struct HostInputFocus(pub bool);

#[derive(Resource, Default)]
pub struct MachinePanel {
    pub holding: HashSet<ControllerKey>,
    pub gamepad: Option<ControllerKey>,
    pub keyboard: Option<ControllerKey>,
    pub layer: gearbox_controls::Layer,
    pub deadman_active: bool,
}

impl MachinePanel {
    pub fn holds(&self, key: &ControllerKey) -> bool {
        self.holding.contains(key)
    }

    pub fn gamepad_on(&self, key: &ControllerKey) -> bool {
        self.gamepad.as_ref() == Some(key)
    }

    pub fn keyboard_on(&self, key: &ControllerKey) -> bool {
        self.keyboard.as_ref() == Some(key)
    }
}

#[derive(Resource, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ControlSettings {
    #[serde(alias = "invert_right_y")]
    pub invert_look_y: bool,
    pub cinematic_transitions: bool,
}

impl Default for ControlSettings {
    fn default() -> Self {
        Self {
            invert_look_y: true,
            cinematic_transitions: true,
        }
    }
}

impl ControlSettings {
    fn path() -> Option<std::path::PathBuf> {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|p| std::path::PathBuf::from(p).join(".config"))
            })
            .map(|p| p.join("gearbox/controls.json"))
    }

    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                warn!(
                    "Cannot read controller settings {}: {error}",
                    path.display()
                );
                Self::default()
            }),
            Err(error) => {
                if error.kind() != std::io::ErrorKind::NotFound {
                    warn!(
                        "Cannot read controller settings {}: {error}",
                        path.display()
                    );
                }
                Self::default()
            }
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path().ok_or_else(|| std::io::Error::other("No config directory"))?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        let temp = path.with_extension(format!("{}.tmp", std::process::id()));
        std::fs::write(&temp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(temp, path)
    }
}

pub const DRIVE_TYPES: [&str; 3] = ["builtin:ackermann_cmd_vel", "builtin:diff_drive_cmd_vel", "builtin:tracked_cmd_vel"];
