//! Floating right dock — mirror of the left, currently just the Inspector.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::viz::GearboxSim;

use super::{float, inspector};
use super::persist::EditorUiState;
use super::selection::Selection;

#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum RightTab {
    #[default]
    Inspector,
    None,
}

pub fn right_dock_ui(
    mut contexts: EguiContexts,
    mut active: ResMut<RightTab>,
    mut ui_state: ResMut<EditorUiState>,
    sim: Res<GearboxSim>,
    selection: Res<Selection>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };

    float::icon_rail("right_rail", ctx, egui::Align2::RIGHT_TOP, |ui| {
        float::icon_button(ui, "I", "Inspector",
            matches!(*active, RightTab::Inspector),
            || *active = if *active == RightTab::Inspector { RightTab::None } else { RightTab::Inspector });
    });

    if *active == RightTab::None { return; }

    let size_ref = &mut ui_state.inspector_size;
    let size = egui::vec2(size_ref.x, size_ref.y);

    let mut open = true;
    let new_size = float::floating_window(
        ctx,
        "right_window_inspector",
        "Inspector",
        egui::Align2::RIGHT_TOP,
        size,
        &mut open,
        |ui| inspector::draw_content(ui, &sim, &selection),
    );

    if let Some(ns) = new_size {
        let new = Vec2::new(ns.x, ns.y);
        if (size_ref.x - new.x).abs() > 0.5 || (size_ref.y - new.y).abs() > 0.5 {
            *size_ref = new;
            ui_state.save();
        }
    }
    if !open { *active = RightTab::None; }
}
