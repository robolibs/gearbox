//! The panes, one module each. Every `show` gets the pane body, the Bevy
//! world and the frame context; it reads resources directly, builds Mara
//! pods, renders them and turns the responses into resource writes or
//! `HostCommand`s.

pub mod agents;
pub mod cameras;
pub mod controllers;
pub mod info;
pub mod keys;
pub mod log;
pub mod machine;
pub mod outliner;
pub mod overlays;
pub mod selection;
pub mod timeline;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use mara::ui::mara_core;
use mara_core::pod::PodResponse;
use mara_core::vocab::{Color32, Id as MaraId};

use crate::viewer::commands::HostCommand;
use crate::viewer::log::LoaderLog;

/// Commands collected while the panes draw; `'static` so tree closures can
/// hold a clone. Drained into `HostCommands` after the rails are shown.
#[derive(Clone, Default)]
pub struct Outbox(Arc<Mutex<Vec<HostCommand>>>);

impl Outbox {
    pub fn push(&self, command: HostCommand) {
        if let Ok(mut queue) = self.0.lock() {
            queue.push(command);
        }
    }

    pub fn drain(&self) -> Vec<HostCommand> {
        self.0.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
    }
}

/// What every pane gets besides the world.
pub struct PaneCtx<'a> {
    pub accent: Color32,
    pub egui: &'a egui::Context,
    pub outbox: Outbox,
    pub log: &'a LoaderLog,
}

impl PaneCtx<'_> {
    pub fn send(&self, command: HostCommand) {
        self.outbox.push(command);
    }

    /// Mara toggles keep their own persisted state; write the pane's truth
    /// over it so a switch never shows a state the sim does not hold.
    pub fn sync_toggles(&self, pod: MaraId, states: &[bool]) {
        self.egui.data_mut(|d| {
            for (i, on) in states.iter().enumerate() {
                let key: egui::Id = pod.with(("mara_pod_toggle_state", i)).into();
                d.insert_persisted(key, *on);
            }
        });
    }

    /// Same for a hybrid select list's selected and pinned rows.
    pub fn sync_list_memory(&self, pod: MaraId, selected: Option<usize>, pinned: Option<usize>) {
        let sel_key: egui::Id = pod.with(("mara_pod_hybrid_select_list_sel", 0usize)).into();
        let pin_key: egui::Id = pod.with(("mara_pod_hybrid_select_list_pin", 0usize)).into();
        self.egui.data_mut(|d| {
            d.insert_persisted(sel_key, selected);
            d.insert_persisted(pin_key, pinned);
        });
    }

    /// Fold a container once, by id.
    pub fn fold_container(&self, container: MaraId) {
        let id: egui::Id = container.into();
        self.egui
            .data_mut(|d| d.insert_persisted(id.with("body_open"), false));
    }
}

pub fn cid(pane: &'static str, section: &str) -> MaraId {
    MaraId::new(("gearbox", pane, section.to_string()))
}

pub fn pid(pane: &'static str, section: &str, idx: usize) -> MaraId {
    MaraId::new(("gearbox", pane, section.to_string(), idx))
}

pub fn nonempty_or<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.trim().is_empty() {
        fallback
    } else {
        value
    }
}

pub fn pod_response(
    responses: &HashMap<MaraId, Vec<PodResponse>>,
    container: MaraId,
    pod: usize,
) -> Option<&PodResponse> {
    responses.get(&container).and_then(|pods| pods.get(pod))
}

pub fn button_clicked(
    responses: &HashMap<MaraId, Vec<PodResponse>>,
    container: MaraId,
    pod: usize,
    button: usize,
) -> bool {
    pod_response(responses, container, pod)
        .and_then(|resp| resp.buttons.get(button))
        .is_some_and(|button| button.clicked)
}

pub fn select_list_clicked(
    responses: &HashMap<MaraId, Vec<PodResponse>>,
    container: MaraId,
    pod: usize,
) -> Option<usize> {
    pod_response(responses, container, pod)
        .and_then(|resp| resp.select_lists.first())
        .and_then(|select| select.clicked)
}

pub fn set_toggle(target: &mut bool, response: &PodResponse, index: usize) {
    if let Some(toggle) = response.toggles.get(index)
        && toggle.changed
    {
        *target = toggle.on;
    }
}

/// The native file picker for stages.
pub fn pick_usd_file() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("USD stages", &["usda", "usdc", "usd", "usdz"])
        .pick_file()
}

/// A path that fits a readout: the file name when it fits, else the tail
/// of the path with an ellipsis in front.
pub fn short_path(path: &str, max_chars: usize) -> String {
    let count = path.chars().count();
    if count <= max_chars {
        return path.to_string();
    }
    let tail: String = path
        .chars()
        .skip(count.saturating_sub(max_chars.saturating_sub(1)))
        .collect();
    format!("…{tail}")
}

impl PaneCtx<'_> {
    /// Overwrite a slider's persisted value, for a control that resets it
    /// (a Stop button under speed sliders).
    pub fn set_slider(&self, pod: MaraId, index: usize, value: f64) {
        let key: egui::Id = pod.with(("mara_pod_slider_val", index)).into();
        self.egui.data_mut(|d| d.insert_persisted(key, value));
    }
}
