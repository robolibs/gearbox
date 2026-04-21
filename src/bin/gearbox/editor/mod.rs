//! Egui editor — floating docks, selection, drag, persistence.

pub mod float;
pub mod inspector;
pub mod left_dock;
pub mod pending_spawn;
pub mod persist;
pub mod right_dock;
pub mod selection;
pub mod selection_ring;
pub mod spawn_panel;
pub mod style;
pub mod transform_gizmos;
pub mod transport;
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
            .init_resource::<pending_spawn::PendingSpawn>()
            .init_resource::<style::AccentColor>()
            .init_resource::<transform_gizmos::GizmoMode>()
            .init_resource::<transform_gizmos::HoveredGizmo>()
            .init_resource::<transform_gizmos::GizmoDrag>()
            .init_resource::<transform_gizmos::GizmoScale>()
            // `PostStartup` so `main::setup_scene` has already inserted
            // the `BigSpaceRoot` resource that these entities parent under.
            .add_systems(
                PostStartup,
                (
                    selection_ring::setup_selection_ring,
                    transform_gizmos::setup_transform_gizmos,
                ),
            )
            .add_systems(
                EguiPrimaryContextPass,
                (
                    style::update_accent_from_selection,
                    style::apply_theme,
                    transport::transport_bar,
                    left_dock::left_dock_ui,
                    right_dock::right_dock_ui,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                (
                    // Gizmo input runs BEFORE pick_and_drag so hover +
                    // active drag block the vehicle-picker cleanly in
                    // the same frame as the click.
                    transform_gizmos::cycle_gizmo_mode,
                    transform_gizmos::hover_transform_gizmos,
                    transform_gizmos::gizmo_drag_system,
                    selection::pick_and_drag_system,
                    persist::save_state_on_change,
                    pending_spawn::spawn_ghost_if_needed,
                    pending_spawn::rotate_ghost_on_ctrl_wheel,
                    pending_spawn::update_ghost_position,
                    pending_spawn::commit_or_cancel_ghost,
                    selection_ring::update_selection_ring,
                    // Regen gizmo meshes before the visuals are
                    // re-synced so the new mesh data and positions
                    // land on the same frame.
                    transform_gizmos::regenerate_gizmo_meshes,
                    transform_gizmos::update_transform_gizmos,
                )
                    .chain(),
            );
    }
}
