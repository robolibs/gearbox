//! Floating right dock — Inspector (I) and UI settings (U).

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::viz::{GearboxSim, GroundGrid};

use super::{float, inspector, ui_panel};
use super::persist::EditorUiState;
use super::selection::Selection;

#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum RightTab {
    #[default]
    Inspector,
    Ui,
    None,
}

pub fn right_dock_ui(
    mut contexts: EguiContexts,
    mut active: ResMut<RightTab>,
    ui_state: Res<EditorUiState>,
    sim: Res<GearboxSim>,
    selection: Res<Selection>,
    mut grid: ResMut<GroundGrid>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };

    float::side_button(
        "right_btn_inspector", ctx, egui::Align2::RIGHT_TOP, 0,
        "I", "Inspector",
        matches!(*active, RightTab::Inspector),
        || *active = if *active == RightTab::Inspector { RightTab::None } else { RightTab::Inspector },
    );
    float::side_button(
        "right_btn_ui", ctx, egui::Align2::RIGHT_TOP, 1,
        "U", "UI settings",
        matches!(*active, RightTab::Ui),
        || *active = if *active == RightTab::Ui { RightTab::None } else { RightTab::Ui },
    );

    match *active {
        RightTab::None => {}
        RightTab::Inspector => {
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
        RightTab::Ui => {
            let size = ui_state.inspector_size; // reuse same default width
            let mut open = true;
            float::floating_window(
                ctx,
                "right_window_ui",
                "UI",
                egui::Align2::RIGHT_TOP,
                egui::vec2(size.x, size.y),
                &mut open,
                |ui| ui_panel::draw_content(ui, &mut grid),
            );
        }
    }
}
