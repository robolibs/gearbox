//! Capture pane: HDR screenshots and video of the viewport, without the UI.

use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::{pane::PaneBody, pod::Pod};

use super::{PaneCtx, button_clicked, cid, pick_folder, pid, short_path};
use crate::host::PANE_CAPTURE as P;
use crate::viewer::recorder::Recorder;

pub fn show(body: &mut PaneBody<'_, '_>, world: &mut World, ctx: &PaneCtx) {
    let recorder = world.resource::<Recorder>();
    let recording = recorder.is_recording();
    let status = recorder.status();
    let last = recorder
        .last()
        .and_then(|path| path.file_name())
        .map_or_else(|| "nothing yet".to_string(), |name| name.to_string_lossy().into_owned());
    let folder = recorder.folder().map_or_else(
        || "~/Pictures, ~/Videos /gearbox".to_string(),
        |folder| short_path(&folder.display().to_string(), 34),
    );
    let section = cid(P, "capture");
    body.add_normal(
        section,
        "Capture",
        "record",
        vec![
            Pod::new(pid(P, "capture", 0))
                .with_readout("status", status)
                .with_readout("last", last)
                .with_readout("folder", folder)
                .with_readout("formats", "EXR + PNG, HDR10 HEVC video")
                .with_button("Screenshot", ctx.accent)
                .with_button(if recording { "Stop recording" } else { "Record video" }, ctx.accent)
                .with_button("Choose folder…", ctx.accent)
                .with_button("Default folders", ctx.accent),
        ],
    );
    let responses = body.render();
    let chosen = button_clicked(&responses, section, 0, 2)
        .then(|| pick_folder(world.resource::<Recorder>().folder()))
        .flatten();
    let mut recorder = world.resource_mut::<Recorder>();
    if chosen.is_some() {
        recorder.set_folder(chosen);
    }
    if button_clicked(&responses, section, 0, 3) {
        recorder.set_folder(None);
    }
    if button_clicked(&responses, section, 0, 0) {
        recorder.request_still();
    }
    if button_clicked(&responses, section, 0, 1) {
        recorder.toggle_recording();
    }
}
