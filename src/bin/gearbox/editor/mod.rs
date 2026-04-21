//! Egui editor — floating docks, selection, drag, persistence.

pub mod float;
pub mod gizmos;
pub mod inspector;
pub mod left_dock;
pub mod pending_spawn;
pub mod persist;
pub mod right_dock;
pub mod selection;
pub mod spawn_panel;
pub mod style;
pub mod tree;
pub mod ui_panel;

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
            .init_resource::<gizmos::GizmoMode>()
            .init_resource::<pending_spawn::PendingSpawn>()
            .init_resource::<style::AccentColor>()
            .add_systems(Startup, gizmos::configure_gizmos)
            .add_systems(
                EguiPrimaryContextPass,
                (
                    style::update_accent_from_selection,
                    style::apply_theme,
                    left_dock::left_dock_ui,
                    right_dock::right_dock_ui,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                (
                    selection::pick_and_drag_system,
                    gizmos::gizmo_mode_input,
                    persist::save_state_on_change,
                    pending_spawn::spawn_ghost_if_needed,
                    pending_spawn::rotate_ghost_on_ctrl_wheel,
                    pending_spawn::update_ghost_position,
                    pending_spawn::commit_or_cancel_ghost,
                )
                    .chain(),
            )
            // Gizmos run after transform propagation so they read the
            // big_space-rebased `GlobalTransform`, not a stale one.
            .add_systems(PostUpdate, gizmos::selection_gizmos_system);
    }
}
