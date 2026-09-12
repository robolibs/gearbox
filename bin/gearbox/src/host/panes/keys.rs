//! The Controls pane: keyboard and mouse bindings.

use mara::ui::mara_core;
use mara_core::pane::PaneBody;
use mara_core::pod::Pod;

use super::{PaneCtx, cid, pid};
use crate::host::PANE_KEYS as P;

pub fn show(body: &mut PaneBody<'_, '_>, _ctx: &PaneCtx) {
    body.add_normal(
        cid(P, "keys"),
        "Keyboard",
        "keyboard",
        vec![Pod::new(pid(P, "keys", 0)).with_keybindings(vec![
            ("F", "Open selection"),
            ("T", "Open outliner"),
            ("N", "Open agents"),
            ("I", "Open stage info"),
            ("O", "Open overlays"),
            ("M", "Open machine"),
            ("?", "Open controls"),
            ("R", "Reload active stage"),
            ("Ctrl+K", "Command palette"),
            ("G / X / P", "Grid / axes / prim markers"),
            ("B / Y / C", "Skeleton / physics / colliders"),
            ("Esc", "Clear selection"),
        ])],
    );
    body.add_normal(
        cid(P, "camera"),
        "Camera",
        "camera",
        vec![Pod::new(pid(P, "camera", 0)).with_keybindings(vec![
            ("Drag", "Orbit"),
            ("Middle drag", "Pan"),
            ("Shift+Middle drag", "Lift"),
            ("Wheel", "Zoom"),
            ("W A S D", "Move over the ground"),
            ("Q / E", "Down / up"),
            ("Shift", "Faster"),
            ("Alt", "Allow the view under the ground"),
            ("Click", "Select the asset under the cursor"),
        ])],
    );
}
