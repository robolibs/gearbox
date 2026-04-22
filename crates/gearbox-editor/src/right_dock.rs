//! Floating right dock — Inspector (I, read-only) and Properties (P, editable).

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use gearbox_viz::gamepad::GamepadSelection;
use gearbox_viz::{GearboxSim, GroundGrid};

use super::{float, inspector, properties};
use super::persist::EditorUiState;
use super::properties::PendingColorChange;
use super::selection::Selection;
use super::selection_ring::SelectionRingSettings;
use super::style::AccentColor;
use super::transform_gizmos::{GizmoModesEnabled, GizmoScale};

#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum RightTab {
    #[default]
    Inspector,
    Properties,
    None,
}

pub fn right_dock_ui(
    mut contexts: EguiContexts,
    mut active: ResMut<RightTab>,
    ui_state: Res<EditorUiState>,
    mut sim: ResMut<GearboxSim>,
    selection: Res<Selection>,
    mut grid: ResMut<GroundGrid>,
    mut gizmo_scale: ResMut<GizmoScale>,
    mut gizmo_modes: ResMut<GizmoModesEnabled>,
    mut ring_settings: ResMut<SelectionRingSettings>,
    mut pending_color: ResMut<PendingColorChange>,
    mut gamepad_selection: ResMut<GamepadSelection>,
    accent: Res<AccentColor>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let accent_col = accent.0;

    float::side_button(
        "right_btn_inspector", ctx, egui::Align2::RIGHT_TOP, 0,
        "I", "Inspector",
        matches!(*active, RightTab::Inspector),
        accent_col,
        || *active = if *active == RightTab::Inspector { RightTab::None } else { RightTab::Inspector },
    );
    float::side_button(
        "right_btn_properties", ctx, egui::Align2::RIGHT_TOP, 1,
        "P", "Properties",
        matches!(*active, RightTab::Properties),
        accent_col,
        || *active = if *active == RightTab::Properties { RightTab::None } else { RightTab::Properties },
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
                accent_col,
                |ui| inspector::draw_content(ui, &mut sim, &selection, accent_col),
            );
        }
        RightTab::Properties => {
            let size = ui_state.inspector_size; // reuse the same default
            let mut open = true;
            float::floating_window(
                ctx,
                "right_window_properties",
                "Properties",
                egui::Align2::RIGHT_TOP,
                egui::vec2(size.x, size.y),
                &mut open,
                accent_col,
                |ui| properties::draw_content(
                    ui,
                    &mut sim,
                    &selection,
                    &mut grid,
                    &mut gizmo_scale,
                    &mut gizmo_modes,
                    &mut ring_settings,
                    &mut pending_color,
                    &mut gamepad_selection,
                    accent_col,
                ),
            );
        }
    }
}
