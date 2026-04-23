//! # Editor UI — egui panels sitting on top of the **renderer**.
//!
//! Layer in the gearbox stack: runs inside the same process as the
//! **simulator** (owned by `gearbox_viz`) and the **tool API**
//! (zenoh, owned by `gearbox_api`). This crate is purely the
//! floating-dock UI: selection, gizmos, inspector, properties,
//! persistence — no sim stepping, no network transport. It mutates
//! the simulator through Bevy resources; any changes it needs to
//! broadcast *outside* the process go through the tool-API crate.

pub mod float;
pub mod gizmo;
pub mod gizmo_material;
pub mod heading_arrows;
pub mod inspector;
pub mod left_dock;
pub mod pending_spawn;
pub mod persist;
pub mod player_sync;
pub mod preset_registry;
pub mod properties;
pub mod right_dock;
pub mod selection;
pub mod selection_ring;
pub mod spawn_panel;
pub mod style;
pub mod transform_gizmos;
pub mod transport;
pub mod tree;
pub mod ui_panel;
pub mod widgets;

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;

pub struct EditorPlugin;

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        // Load persisted state up-front so the initial tabs match last run.
        let state = persist::EditorUiState::load();
        let left = state.left;
        let right = state.right;

        app.add_plugins(selection_ring::SelectionRingPlugin)
            .add_plugins(heading_arrows::HeadingArrowsPlugin)
            // Always-on-top mesh material for the (now hidden) legacy
            // gizmo meshes that hold pick metadata.
            .add_plugins(
                bevy::pbr::MaterialPlugin::<gizmo_material::GizmoMaterial>::default(),
            )
            // 2D overlay gizmo — the actual visual UI, modelled after
            // urholaukkarinen/transform-gizmo (flat filled shapes in
            // screen space, depth-test off).
            .add_plugins(gizmo::GizmoOverlayPlugin)
            .insert_resource(state)
            .insert_resource(left)
            .insert_resource(right)
            .insert_resource(preset_registry::PresetRegistry::with_defaults())
            .init_resource::<properties::PendingColorChange>()
            .init_resource::<selection::Selection>()
            .init_resource::<pending_spawn::PendingSpawn>()
            .init_resource::<style::AccentColor>()
            .init_resource::<style::GlassOpacity>()
            // Mirror the slider's value into the `GLASS_OPACITY`
            // atomic every frame so `section`, `floating_window` and
            // friends (plain helpers, not Bevy systems) pick up the
            // current alpha without plumbing a resource reference
            // through every UI call.
            .add_systems(PreUpdate, style::sync_glass_opacity_system)
            .init_resource::<transform_gizmos::GizmoMode>()
            .init_resource::<transform_gizmos::HoveredGizmo>()
            .init_resource::<transform_gizmos::GizmoDrag>()
            .init_resource::<transform_gizmos::GizmoScale>()
            .init_resource::<transform_gizmos::GizmoModesEnabled>()
            // `PostStartup` so `main::setup_scene` has already run.
            .add_systems(
                PostStartup,
                (
                    selection_ring::setup_selection_ring,
                    heading_arrows::setup_heading_arrows,
                    transform_gizmos::setup_transform_gizmos,
                    gizmo::setup_gizmo_overlay,
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
                    // the same frame as the click. Tab cycling is gone
                    // now: the gizmo shows translate + rotate + scale
                    // simultaneously (transform-gizmo convention), and
                    // the clicked handle's `GizmoMode` feeds the drag
                    // system directly.
                    transform_gizmos::hover_transform_gizmos,
                    transform_gizmos::gizmo_drag_system,
                    selection::pick_and_drag_system,
                    player_sync::sync_player_to_selection_system,
                    persist::save_state_on_change,
                    pending_spawn::spawn_ghost_if_needed,
                    pending_spawn::rotate_ghost_on_ctrl_wheel,
                    pending_spawn::update_ghost_position,
                    pending_spawn::commit_or_cancel_ghost,
                    selection_ring::update_selection_ring,
                    heading_arrows::update_heading_arrows,
                    // Regen gizmo meshes before the visuals are
                    // re-synced so the new mesh data and positions
                    // land on the same frame.
                    transform_gizmos::regenerate_gizmo_meshes,
                    transform_gizmos::update_transform_gizmos,
                    gizmo::draw_gizmo_system,
                )
                    .chain(),
            )
            // Properties panel's colour picker queues a pending
            // change; this consumer system writes it to the live
            // material each frame.
            .add_systems(Update, properties::apply_vehicle_color_changes);
    }
}
