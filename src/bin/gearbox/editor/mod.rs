//! Egui editor — floating docks, selection, drag, persistence.

pub mod float;
pub mod gizmos;
pub mod inspector;
pub mod left_dock;
pub mod persist;
pub mod right_dock;
pub mod selection;
pub mod spawn_panel;
pub mod style;
pub mod tree;

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;

pub struct EditorPlugin;

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        // Load persisted state up-front so the initial tabs match last run.
        let state = persist::EditorUiState::load();
        let left = state.left;
        let right = state.right;

        app.insert_resource(state)
            .insert_resource(left)
            .insert_resource(right)
            .init_resource::<selection::Selection>()
            .add_systems(
                EguiPrimaryContextPass,
                (
                    style::apply_theme_once,
                    left_dock::left_dock_ui,
                    right_dock::right_dock_ui,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                (
                    selection::pick_and_drag_system,
                    gizmos::selection_gizmos_system,
                    persist::save_state_on_change,
                ),
            );
    }
}
