//! Window-geometry persistence — remembers the Bevy primary window's
//! size + position across runs so you don't start every session with
//! the default tiny 1280 × 800 pane in the top-left.
//!
//! Ported verbatim from the `astrocraft` project
//! (`/home/bresilla/data/code/game/astrocraft/src/ui/window_settings.rs`).
//! Plain-text `key=value` format so the config is diffable / editable
//! by hand if ever needed.

use bevy::prelude::*;
use bevy::window::{PrimaryWindow, WindowMoved, WindowPosition, WindowResized};
use std::env;
use std::fs;
use std::path::PathBuf;

pub struct WindowSettingsPlugin;

impl Plugin for WindowSettingsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, persist_window_geometry);
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WindowGeometry {
    pub width: f32,
    pub height: f32,
    pub position: IVec2,
}

impl Default for WindowGeometry {
    fn default() -> Self {
        Self {
            width: 1280.0,
            height: 800.0,
            position: IVec2::new(120, 120),
        }
    }
}

pub fn load_window_geometry() -> WindowGeometry {
    let path = config_path();
    let Ok(contents) = fs::read_to_string(path) else {
        return WindowGeometry::default();
    };
    parse_geometry(&contents).unwrap_or_default()
}

fn persist_window_geometry(
    primary_window: Single<(Entity, &Window), With<PrimaryWindow>>,
    mut moved_events: MessageReader<WindowMoved>,
    mut resized_events: MessageReader<WindowResized>,
) {
    let (window_entity, window) = *primary_window;
    let mut dirty = false;

    for event in moved_events.read() {
        if event.window == window_entity {
            dirty = true;
        }
    }
    for event in resized_events.read() {
        if event.window == window_entity {
            dirty = true;
        }
    }
    if !dirty {
        return;
    }

    let WindowPosition::At(position) = window.position else {
        return;
    };

    let geometry = WindowGeometry {
        width: window.resolution.width(),
        height: window.resolution.height(),
        position,
    };
    let _ = save_geometry(geometry);
}

fn save_geometry(geometry: WindowGeometry) -> std::io::Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        path,
        format!(
            "width={}\nheight={}\nx={}\ny={}\n",
            geometry.width, geometry.height, geometry.position.x, geometry.position.y
        ),
    )
}

fn parse_geometry(contents: &str) -> Option<WindowGeometry> {
    let mut width = None;
    let mut height = None;
    let mut x = None;
    let mut y = None;

    for line in contents.lines() {
        let (key, value) = line.split_once('=')?;
        match key.trim() {
            "width" => width = value.trim().parse::<f32>().ok(),
            "height" => height = value.trim().parse::<f32>().ok(),
            "x" => x = value.trim().parse::<i32>().ok(),
            "y" => y = value.trim().parse::<i32>().ok(),
            _ => {}
        }
    }

    Some(WindowGeometry {
        width: width?,
        height: height?,
        position: IVec2::new(x?, y?),
    })
}

fn config_path() -> PathBuf {
    let base = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("gearbox").join("window.txt")
}

pub fn geometry_to_window(geometry: WindowGeometry) -> Window {
    Window {
        title: "gearbox editor".to_string(),
        name: Some("gearbox".to_string()),
        resolution: (
            geometry.width.round() as u32,
            geometry.height.round() as u32,
        )
            .into(),
        position: WindowPosition::At(geometry.position),
        ..default()
    }
}
