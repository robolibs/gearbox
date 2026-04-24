//! Tiny persistence for editor UI state: which menu was open on
//! each side last session and the size of each floating window.
//!
//! Written to `$HOME/.config/gearbox/editor-state.txt` as a simple
//! `key=value` list. No extra crate deps — just `std::fs` + `std::env`.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use bevy::prelude::*;

use bevy_frost::SideActive;

/// UI state snapshot written to disk on change.
#[derive(Resource, Clone, Debug)]
pub struct EditorUiState {
    /// Menu ids to restore open on each rail. `None` = closed.
    /// Stored as strings so the save format is agnostic of which
    /// menus exist — adding a new menu doesn't need a persist-
    /// format migration.
    pub left_active: Option<String>,
    pub right_active: Option<String>,
    pub spawn_size: Vec2,
    pub workspace_size: Vec2,
    pub inspector_size: Vec2,
}

impl Default for EditorUiState {
    fn default() -> Self {
        Self {
            left_active: None,
            right_active: None,
            // Every left-rail panel uses the SAME width so toggling
            // between Spawn ↔ Workspace doesn't make the pane visibly
            // resize. Heights can differ because the content heights
            // differ, but the width is fixed design-wide.
            spawn_size: Vec2::new(210.0, 300.0),
            workspace_size: Vec2::new(210.0, 340.0),
            inspector_size: Vec2::new(230.0, 440.0),
        }
    }
}

fn state_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let mut dir = PathBuf::from(home);
    dir.push(".config");
    dir.push("gearbox");
    let _ = fs::create_dir_all(&dir);
    Some(dir.join("editor-state.txt"))
}

impl EditorUiState {
    pub fn load() -> Self {
        let mut out = Self::default();
        let Some(path) = state_path() else { return out };
        let Ok(text) = fs::read_to_string(&path) else { return out };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            match k.trim() {
                // Back-compat: old "left=workspace", "right=inspector".
                "left" | "left_active" => out.left_active = parse_menu(v.trim()),
                "right" | "right_active" => out.right_active = parse_menu(v.trim()),
                _ => {}
            }
        }
        out
    }

    pub fn save(&self) {
        let Some(path) = state_path() else { return };
        let Ok(mut f) = fs::File::create(&path) else { return };
        let _ = writeln!(f, "left_active={}", fmt_menu(&self.left_active));
        let _ = writeln!(f, "right_active={}", fmt_menu(&self.right_active));
    }

    /// Seed the provided `SideActive` with the persisted menu ids.
    /// Intended for one-shot use at app startup, *before* any dock
    /// system runs — otherwise `invalidate_stale` might clear these
    /// before the buttons they refer to have registered into the
    /// layout.
    pub fn seed_side_active(&self, active: &mut SideActive) {
        active.left = self.left_active.clone();
        active.right = self.right_active.clone();
    }
}

fn parse_menu(s: &str) -> Option<String> {
    match s {
        "" | "none" => None,
        // Back-compat: the old "spawn" label was renamed to "library"
        // and "ui" was renamed to "properties".
        "spawn" => Some("library".into()),
        "ui" => Some("properties".into()),
        other => Some(other.into()),
    }
}

fn fmt_menu(m: &Option<String>) -> &str {
    m.as_deref().unwrap_or("none")
}

/// Flush state to disk whenever `SideActive` changes. Simple &
/// robust — no app-exit hook needed.
pub fn save_state_on_change(
    active: Res<SideActive>,
    mut state: ResMut<EditorUiState>,
) {
    let mut changed = false;
    if active.left != state.left_active {
        state.left_active = active.left.clone();
        changed = true;
    }
    if active.right != state.right_active {
        state.right_active = active.right.clone();
        changed = true;
    }
    if changed {
        state.save();
    }
}
