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
    ui_state: Res<EditorUiState>,
    sim: Res<GearboxSim>,
    selection: Res<Selection>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };

    float::side_button(
        "right_btn_inspector", ctx, egui::Align2::RIGHT_TOP, 0,
        "I", "Inspector",
        matches!(*active, RightTab::Inspector),
        || *active = if *active == RightTab::Inspector { RightTab::None } else { RightTab::Inspector },
    );

    if *active == RightTab::None { return; }

    let size = ui_state.inspector_size;
    let mut open = true;
    float::floating_window(
        ctx,
        "right_window_inspector",
        "Inspector",
        egui::Align2::RIGHT_TOP,
        egui::vec2(size.x, size.y),
        &mut open,
        |ui| inspector::draw_content(ui, &sim, &selection),
    );
}
