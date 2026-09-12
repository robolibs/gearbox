//! Saved camera viewpoints.

use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::pane::PaneBody;
use mara_core::pod::Pod;

use super::{PaneCtx, button_clicked, cid, pid, select_list_clicked};
use crate::host::PANE_CAMERAS as P;
use crate::viewer::commands::HostCommand;
use crate::viewer::state::CameraBookmarks;

pub fn show(body: &mut PaneBody<'_, '_>, world: &mut World, ctx: &PaneCtx) {
    let accent = ctx.accent;
    let bookmarks = world.resource::<CameraBookmarks>();
    let actions_id = cid(P, "actions");
    body.add_normal(
        actions_id,
        "Bookmarks",
        "camera",
        vec![
            Pod::new(pid(P, "actions", 0))
                .with_readout("saved", bookmarks.items.len().to_string())
                .with_button("Save current camera", accent)
                .with_button("Clear bookmarks", accent),
        ],
    );
    let saved_id = cid(P, "saved");
    if !bookmarks.items.is_empty() {
        let rows: Vec<String> = bookmarks.items.iter().map(|b| b.name.clone()).collect();
        let trailing: Vec<String> = bookmarks
            .items
            .iter()
            .map(|b| format!("{:.1} m", b.distance))
            .collect();
        body.add_normal(
            saved_id,
            "Saved views",
            "list",
            vec![Pod::new(pid(P, "saved", 0)).with_select_list(rows, Some(trailing), accent)],
        );
    }
    let responses = body.render();
    if button_clicked(&responses, actions_id, 0, 0) {
        ctx.send(HostCommand::SaveBookmark);
    }
    if button_clicked(&responses, actions_id, 0, 1) {
        ctx.send(HostCommand::ClearBookmarks);
    }
    if let Some(index) = select_list_clicked(&responses, saved_id, 0) {
        ctx.send(HostCommand::RecallBookmark(index));
    }
}
