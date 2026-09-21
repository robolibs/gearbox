//! Capture pane: HDR screenshots and video of the viewport, without the UI.

use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::container::Tab;
use mara_core::pod::{Pod, PodResponse};
use mara_core::vocab::Id as MaraId;
use std::collections::HashMap;

use super::{PaneCtx, button_clicked, cid, pick_folder, pid, short_path};
use crate::host::PANE_CAPTURE as P;
use crate::viewer::recorder::Recorder;

pub fn tab(world: &World, ctx: &PaneCtx) -> Tab {
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
    Tab::new(cid(P, "capture"), "Capture", "record").pods(vec![
        Pod::new(pid(P, "capture", 0))
            .with_readout("status", status)
            .with_readout("last", last)
            .with_readout("folder", folder)
            .with_readout("formats", "EXR + PNG, HDR10 HEVC video")
            .with_button("Screenshot", ctx.accent)
            .with_button(if recording { "Stop recording" } else { "Record video" }, ctx.accent)
            .with_button("Choose folder…", ctx.accent)
            .with_button("Default folders", ctx.accent),
    ])
}

pub fn apply(responses: &HashMap<MaraId, Vec<PodResponse>>, world: &mut World, _ctx: &PaneCtx) {
    let section = cid(P, "capture");
    let chosen = button_clicked(responses, section, 0, 2)
        .then(|| pick_folder(world.resource::<Recorder>().folder()))
        .flatten();
    let mut recorder = world.resource_mut::<Recorder>();
    if chosen.is_some() {
        recorder.set_folder(chosen);
    }
    if button_clicked(responses, section, 0, 3) {
        recorder.set_folder(None);
    }
    if button_clicked(responses, section, 0, 0) {
        recorder.request_still();
    }
    if button_clicked(responses, section, 0, 1) {
        recorder.toggle_recording();
    }
}
