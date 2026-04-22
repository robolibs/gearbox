//! Tiny persistence for editor UI state: which tabs were open last session
//! and the size of each floating window.
//!
//! Written to `$HOME/.config/gearbox/editor-state.txt` as a simple
//! `key=value` list. No extra crate deps — just `std::fs` + `std::env`.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use bevy::prelude::*;

use super::left_dock::LeftTab;
use super::right_dock::RightTab;

/// UI state snapshot written to disk on change.
#[derive(Resource, Clone, Copy, Debug)]
pub struct EditorUiState {
    pub left: LeftTab,
    pub right: RightTab,
    pub spawn_size: Vec2,
    pub workspace_size: Vec2,
    pub inspector_size: Vec2,
}

impl Default for EditorUiState {
    fn default() -> Self {
        Self {
            left: LeftTab::default(),
            right: RightTab::default(),
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
        // Sizes are fixed at the struct defaults now — we only restore
        // which tab was open last session. Legacy `*_w`/`*_h` keys are
        // intentionally ignored so no old corrupted state can override
        // the panel widths.
        let mut out = Self::default();
        let Some(path) = state_path() else { return out };
        let Ok(text) = fs::read_to_string(&path) else { return out };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            match k.trim() {
                "left"  => out.left  = parse_left(v.trim()),
                "right" => out.right = parse_right(v.trim()),
                _ => {}
            }
        }
        out
    }

    pub fn save(&self) {
        let Some(path) = state_path() else { return };
        let Ok(mut f) = fs::File::create(&path) else { return };
        let _ = writeln!(f, "left={}",  fmt_left(self.left));
        let _ = writeln!(f, "right={}", fmt_right(self.right));
    }
}

fn parse_left(s: &str) -> LeftTab {
    match s {
        // Accept "spawn" for back-compat with pre-rename state files.
        "library" | "spawn" => LeftTab::Library,
        "workspace"         => LeftTab::Workspace,
        _                   => LeftTab::None,
    }
}
fn fmt_left(t: LeftTab) -> &'static str {
    match t {
        LeftTab::Library   => "library",
        LeftTab::Workspace => "workspace",
        LeftTab::None      => "none",
    }
}
fn parse_right(s: &str) -> RightTab {
    match s {
        "inspector"              => RightTab::Inspector,
        // Back-compat: the old "ui" tab became "properties".
        "properties" | "ui"      => RightTab::Properties,
        _                        => RightTab::None,
    }
}
fn fmt_right(t: RightTab) -> &'static str {
    match t {
        RightTab::Inspector  => "inspector",
        RightTab::Properties => "properties",
        RightTab::None       => "none",
    }
}

/// Periodically flush state to disk whenever it changes. Simple & robust —
/// no app-exit hook needed.
pub fn save_state_on_change(
    left: Res<LeftTab>,
    right: Res<RightTab>,
    mut state: ResMut<EditorUiState>,
) {
    let mut changed = false;
    if *left != state.left   { state.left  = *left;  changed = true; }
    if *right != state.right { state.right = *right; changed = true; }
    if changed {
        state.save();
    }
}
