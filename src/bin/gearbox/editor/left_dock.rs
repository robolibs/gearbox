//! Floating left dock — letter buttons at top-left, content window next to them.
//!
//! Layout mirrors VS Code's activity bar + panel idea, but fully floating.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::viz::{GearboxSim, PlayerControlled, VehicleBody};

use super::pending_spawn::PendingSpawn;
use super::persist::EditorUiState;
use super::selection::Selection;
use super::style::AccentColor;
use super::{float, spawn_panel, tree};

#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum LeftTab {
    #[default]
    None,
    Workspace,
    Spawn,
}

pub fn left_dock_ui(
    mut contexts: EguiContexts,
    mut active: ResMut<LeftTab>,
    ui_state: Res<EditorUiState>,
    // spawn
    mut commands: Commands,
    mut sim: ResMut<GearboxSim>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    existing_bodies: Query<Entity, With<VehicleBody>>,
    player_tagged: Query<Entity, With<PlayerControlled>>,
    mut selection: ResMut<Selection>,
    // tree
    bodies: Query<(Entity, &VehicleBody, Option<&Name>, Has<PlayerControlled>)>,
    mut pending: ResMut<PendingSpawn>,
    accent: Res<AccentColor>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let accent_col = accent.0;

    // --- Side buttons (separated, stacked vertically on the left edge) ---
    float::side_button(
        "left_btn_workspace", ctx, egui::Align2::LEFT_TOP, 0,
        "W", "Workspace",
        matches!(*active, LeftTab::Workspace),
        accent_col,
        || *active = if *active == LeftTab::Workspace { LeftTab::None } else { LeftTab::Workspace },
    );
    float::side_button(
        "left_btn_spawn", ctx, egui::Align2::LEFT_TOP, 1,
        "S", "Spawn",
        matches!(*active, LeftTab::Spawn),
        accent_col,
        || *active = if *active == LeftTab::Spawn { LeftTab::None } else { LeftTab::Spawn },
    );

    if *active == LeftTab::None { return; }

    let (id, title, size) = match *active {
        LeftTab::Workspace => ("left_window_workspace", "Workspace", ui_state.workspace_size),
        LeftTab::Spawn     => ("left_window_spawn",     "Spawn",     ui_state.spawn_size),
        LeftTab::None      => unreachable!(),
    };

    let mut open = true;
    float::floating_window(
        ctx, id, title,
        egui::Align2::LEFT_TOP,
        egui::vec2(size.x, size.y),
        &mut open,
        accent_col,
        |ui| match *active {
            LeftTab::Workspace => tree::draw_content(
                ui,
                &mut commands,
                &sim,
                &bodies,
                &mut selection,
                accent_col,
            ),
            LeftTab::Spawn => spawn_panel::draw_content(
                ui,
                &mut commands,
                &mut pending,
                &existing_bodies,
                &mut sim,
                &mut meshes,
                &mut materials,
                &player_tagged,
                accent_col,
            ),
            LeftTab::None => {}
        },
    );
}
