//! Log: recent gearbox events, then the keyboard and camera bindings.

use mara::ui::mara_core;
use mara_core::pane::PaneBody;
use mara_core::pod::Pod;

use super::{PaneCtx, cid, pid};
use crate::host::PANE_LOG as P;

pub fn show(body: &mut PaneBody<'_, '_>, ctx: &PaneCtx) {
    let lines = ctx.log.tail(40);
    let events = if lines.is_empty() {
        Pod::new(pid(P, "lines", 0)).with_readout("status", "no events yet")
    } else {
        Pod::new(pid(P, "lines", 0))
            .fill()
            .with_select_list(lines, None, ctx.accent)
    };
    body.add_normal(cid(P, "lines"), "Events", "document", vec![events]);
    body.add_normal(
        cid(P, "keys"),
        "Keys",
        "keyboard",
        vec![Pod::new(pid(P, "keys", 0)).with_keybindings(vec![
            ("M", "Machines"),
            ("F", "Scene"),
            ("O", "View"),
            ("?", "Controller and keyboard"),
            ("R", "Reload"),
            ("Ctrl+K", "Command palette"),
            ("G / X", "Ground grid / axes"),
            ("Y / C", "Physics / colliders"),
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
            ("Click", "Select the object under the cursor"),
        ])],
    );
}
