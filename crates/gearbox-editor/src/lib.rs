//! # Editor UI — egui panels sitting on top of the **renderer**.
//!
//! Layer in the gearbox stack: runs inside the same process as the
//! **simulator** (owned by `gearbox_viz`) and the **tool API**
//! (zenoh, owned by `gearbox_api`). This crate is purely the
//! floating-dock UI: selection, gizmos, inspector, properties,
//! persistence — no sim stepping, no network transport. It mutates
//! the simulator through Bevy resources; any changes it needs to
//! broadcast *outside* the process go through the tool-API crate.

#![allow(
    deprecated,
    dead_code,
    clippy::collapsible_if,
    clippy::doc_overindented_list_items,
    clippy::too_many_arguments,
    clippy::unnecessary_cast,
    clippy::unnecessary_map_or
)]

// Re-export the generic UI kit under the module names the editor
// source already uses (`super::float`, `super::widgets`,
// `super::gizmo_material`) so the in-crate modules didn't need their
// import paths rewritten during extraction. `float` is an alias for
// `bevy_frost::floating` after the upstream rename.
pub use bevy_frost::floating as float;
pub use bevy_frost::gizmo_material;
pub use bevy_frost::widgets;

pub mod dock_ribbons;
pub mod usd_load;
pub mod usd_tree;
pub use usd_load::{LoadUsdQueue, PendingUsdRemoval, UsdSelectable, UsdTreeExpanded};
pub use usd_tree::UsdTreeFilter;
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

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use bevy_frost::FrostPlugin;

/// Ordering hooks for the editor's egui-pass systems. Uses a
/// `SystemSet` enum rather than chaining the fn items directly
/// because the system function types carry enough lifetime and
/// `ResMut` params that Bevy 0.18's `.chain()` method resolution
/// mis-dispatches to the `Curve` trait on a tuple of them.
#[derive(SystemSet, Debug, Clone, Copy, Eq, PartialEq, Hash)]
enum EditorUiSet {
    AccentUpdate,
    Transport,
    DockRibbons,
    LeftDock,
    RightDock,
}

pub struct EditorPlugin;

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        // Load persisted state up-front so the initial active menus
        // match last run. `SideActive` itself is initialised by
        // `FrostPlugin`; we seed it with the persisted values here.
        let state = persist::EditorUiState::load();
        let mut seeded_side_active = bevy_frost::SideActive::default();
        state.seed_side_active(&mut seeded_side_active);

        app.add_plugins(FrostPlugin)
            .add_plugins(selection_ring::SelectionRingPlugin)
            .add_plugins(heading_arrows::HeadingArrowsPlugin)
            // Transform gizmos: upstream `transform_gizmo_bevy` plugin
            // plus the editor's bridge that ties it to the
            // selection / sim.
            .add_plugins(transform_gizmos::EditorGizmoBridgePlugin)
            .insert_resource(state)
            .insert_resource(seeded_side_active)
            .insert_resource(preset_registry::PresetRegistry::with_defaults())
            .init_resource::<properties::PendingColorChange>()
            .init_resource::<selection::Selection>()
            .init_resource::<pending_spawn::PendingSpawn>()
            .init_resource::<LoadUsdQueue>()
            .init_resource::<PendingUsdRemoval>()
            .init_resource::<UsdTreeExpanded>()
            .init_resource::<UsdTreeFilter>()
            // `PostStartup` so `main::setup_scene` has already run.
            .add_systems(PostStartup, (heading_arrows::setup_heading_arrows,))
            // The new bevy_frost runs `apply_theme` from inside its
            // own `ThemePlugin` system in `EguiPrimaryContextPass`
            // (the system fn is private — no `.before()` hook), so
            // we update accent in `Update` instead. `Update` runs
            // before any egui pass each frame, so the theme picks up
            // the fresh accent value the same frame.
            .configure_sets(
                EguiPrimaryContextPass,
                (
                    EditorUiSet::Transport,
                    EditorUiSet::DockRibbons,
                    EditorUiSet::LeftDock,
                    EditorUiSet::RightDock,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                style::update_accent_from_selection.in_set(EditorUiSet::AccentUpdate),
            )
            .add_systems(
                EguiPrimaryContextPass,
                transport::transport_bar.in_set(EditorUiSet::Transport),
            )
            .add_systems(
                EguiPrimaryContextPass,
                dock_ribbons::draw_dock_ribbons_ui.in_set(EditorUiSet::DockRibbons),
            )
            .add_systems(
                EguiPrimaryContextPass,
                left_dock::left_dock_ui.in_set(EditorUiSet::LeftDock),
            )
            .add_systems(
                EguiPrimaryContextPass,
                usd_tree::draw_usd_tree_panel.in_set(EditorUiSet::LeftDock),
            )
            .add_systems(
                EguiPrimaryContextPass,
                right_dock::right_dock_ui.in_set(EditorUiSet::RightDock),
            )
            // Editor systems. The transform-gizmo plugin owns its
            // own draw / picking / drag pipeline (registered in
            // `EditorGizmoBridgePlugin`), so this chain only
            // contains gearbox-side logic.
            .add_systems(
                Update,
                (
                    selection::pick_and_drag_system,
                    player_sync::sync_player_to_selection_system,
                    persist::save_state_on_change,
                    pending_spawn::spawn_ghost_if_needed,
                    pending_spawn::rotate_ghost_on_ctrl_wheel,
                    pending_spawn::update_ghost_position,
                    pending_spawn::commit_or_cancel_ghost,
                    selection_ring::update_selection_ring,
                    heading_arrows::update_heading_arrows,
                )
                    .chain(),
            )
            // Properties panel's colour picker queues a pending
            // change; this consumer system writes it to the live
            // material each frame.
            .add_systems(Update, properties::apply_vehicle_color_changes);
    }
}
