//! Shortcuts: every key/mouse/gamepad binding in one place, read-only.

use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::container::Tab;
use mara_core::pod::{Pod, PodResponse};
use mara_core::shelf::ShelfContainer;
use mara_core::vocab::Id as MaraId;
use std::collections::HashMap;

use super::{PaneCtx, cid, pid};
use crate::host::PANE_SHORTCUTS as P;

pub fn container(_world: &World, _ctx: &PaneCtx) -> ShelfContainer<'static> {
    let gamepad = Pod::new(pid(P, "gamepad", 0)).with_keybindings(vec![
        ("Right stick", "Orbit camera"),
        ("Left stick", "Strafe / forward-back (free camera only)"),
        ("R2 / L2", "Raise / lower camera (free camera only)"),
        ("D-pad ← / →", "Switch machine using chosen camera mode"),
        ("D-pad ↑", "Toggle camera follow"),
        ("D-pad ↓", "Toggle cinematic transitions"),
        ("Hold R1 + left stick", "Drive selected machine"),
        ("Release R1", "Stop local drive; allow external control"),
        ("Hold L1", "Machine layer · reserved for internals"),
        ("Cross / A", "Stop; release R1 to re-arm"),
    ]);

    let keyboard = Pod::new(pid(P, "keyboard", 0)).with_keybindings(vec![
        ("M", "Machines"),
        ("F", "Scene"),
        ("O", "View"),
        ("?", "Controller and keyboard"),
        ("R", "Reload"),
        ("Ctrl+K", "Command palette"),
        ("G / X", "Ground grid / axes"),
        ("Y / C", "Physics / colliders"),
        ("Esc", "Clear selection"),
    ]);

    let mouse = Pod::new(pid(P, "mouse", 0)).with_keybindings(vec![
        ("Drag", "Orbit"),
        ("Middle drag", "Pan"),
        ("Shift+Middle drag", "Lift"),
        ("Wheel", "Zoom"),
        ("W A S D", "Move over the ground"),
        ("Q / E", "Down / up"),
        ("Shift", "Faster"),
        ("Click", "Select the object under the cursor"),
    ]);

    let tab =
        Tab::new(cid(P, "shortcuts"), "Shortcuts", "keyboard").pods(vec![gamepad, keyboard, mouse]);
    ShelfContainer::tabbed(cid(P, "root"), "Shortcuts", "keyboard", vec![tab])
}

pub fn apply(_responses: &HashMap<MaraId, Vec<PodResponse>>, _world: &mut World, _ctx: &PaneCtx) {}
