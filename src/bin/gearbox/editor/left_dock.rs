//! Floating left dock — letter buttons at top-left, content window next to them.
//!
//! Layout mirrors VS Code's activity bar + panel idea, but fully floating.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::viz::{GearboxSim, PlayerControlled, VehicleBody};
use crate::BigSpaceRoot;

use super::persist::EditorUiState;
use super::selection::Selection;
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
    mut ui_state: ResMut<EditorUiState>,
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
    big_space_root: Res<BigSpaceRoot>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };

    // --- Letter rail (top-left) ---
    float::icon_rail("left_rail", ctx, egui::Align2::LEFT_TOP, |ui| {
        float::icon_button(ui, "W", "Workspace",
            matches!(*active, LeftTab::Workspace),
            || *active = if *active == LeftTab::Workspace { LeftTab::None } else { LeftTab::Workspace });
        float::icon_button(ui, "S", "Spawn",
            matches!(*active, LeftTab::Spawn),
            || *active = if *active == LeftTab::Spawn { LeftTab::None } else { LeftTab::Spawn });
    });

    if *active == LeftTab::None { return; }

    let (id, title, size_ref) = match *active {
        LeftTab::Workspace => ("left_window_workspace", "Workspace", &mut ui_state.workspace_size),
        LeftTab::Spawn     => ("left_window_spawn",     "Spawn",     &mut ui_state.spawn_size),
        LeftTab::None      => unreachable!(),
    };
    let size = egui::vec2(size_ref.x, size_ref.y);

    let mut open = true;
    let new_size = float::floating_window(
        ctx, id, title,
        egui::Align2::LEFT_TOP,
        size,
        &mut open,
        |ui| match *active {
            LeftTab::Workspace => tree::draw_content(
                ui,
                &mut commands,
                &sim,
                &bodies,
                &mut selection,
            ),
            LeftTab::Spawn => spawn_panel::draw_content(
                ui,
                &mut commands,
                &mut sim,
                &mut meshes,
                &mut materials,
                &existing_bodies,
                &player_tagged,
                &mut selection,
                big_space_root.0,
            ),
            LeftTab::None => {}
        },
    );
    if let Some(ns) = new_size {
        let new = Vec2::new(ns.x, ns.y);
        if (size_ref.x - new.x).abs() > 0.5 || (size_ref.y - new.y).abs() > 0.5 {
            *size_ref = new;
            ui_state.save();
        }
    }
    if !open { *active = LeftTab::None; }
}
