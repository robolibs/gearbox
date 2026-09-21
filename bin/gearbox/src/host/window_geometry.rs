//! Remembers the native window's size and position across launches, in
//! `~/.config/gearbox/window.txt` — same plain `key=value` format as the
//! other small per-user files next to it (`controls.json` is the one
//! exception, JSON, from an earlier pane).

use std::path::PathBuf;

#[derive(Clone, Copy, Debug)]
pub struct WindowGeometry {
    pub width: f32,
    pub height: f32,
    pub x: f32,
    pub y: f32,
}

fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("gearbox").join("window.txt"))
}

pub fn load() -> Option<WindowGeometry> {
    let content = std::fs::read_to_string(config_path()?).ok()?;
    let mut width = None;
    let mut height = None;
    let mut x = None;
    let mut y = None;
    for line in content.lines() {
        let (key, value) = line.split_once('=')?;
        let value: f32 = value.trim().parse().ok()?;
        match key.trim() {
            "width" => width = Some(value),
            "height" => height = Some(value),
            "x" => x = Some(value),
            "y" => y = Some(value),
            _ => {}
        }
    }
    // A window smaller than this is almost certainly a corrupt/leftover
    // file, not a size anyone actually wants back.
    const MIN: f32 = 200.0;
    let (width, height, x, y) = (width?, height?, x?, y?);
    if width < MIN || height < MIN {
        return None;
    }
    Some(WindowGeometry { width, height, x, y })
}

pub fn save(geometry: WindowGeometry) {
    let Some(path) = config_path() else { return };
    let Some(parent) = path.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let content = format!(
        "width={}\nheight={}\nx={}\ny={}\n",
        geometry.width as i32, geometry.height as i32, geometry.x as i32, geometry.y as i32
    );
    // Best-effort: a window move/resize is not the place to surface a
    // disk-full or read-only-config-dir error.
    let _ = std::fs::write(path, content);
}
