//! Floating left dock — letter buttons at top-left, content window next to them.
//!
//! Layout mirrors VS Code's activity bar + panel idea, but fully floating.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use gearbox::VehicleId;

use crate::viz::{ChaseCamera, GearboxSim, PlayerControlled, VehicleBody};
use crate::viz::camera::FlyTarget;

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
    mut cameras: Query<&mut ChaseCamera>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let accent_col = accent.0;

    // Filled by `tree::draw_content` when the user double-clicks a
    // row; applied below after the UI borrow ends.
    let mut frame_to: Option<VehicleId> = None;

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
                &mut frame_to,
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

    // ─── Apply the "frame this vehicle" request ──────────────────
    //
    // Double-clicking a row in the workspace tree tells the chase
    // camera to SMOOTHLY fly to that vehicle — no teleport. The
    // camera's `chase_camera_fly` system reads `fly_target` and
    // eases focus + distance toward it over a few tenths of a
    // second. Distance is 3× the vehicle's longest dimension so you
    // end up close enough to see it, not in orbit.
    if let Some(id) = frame_to {
        if let Some(state) = sim.0.vehicle(id) {
            let size = state.spec.chassis.size;
            let max_dim = size.x.max(size.y).max(size.z) as f32;
            let target_dist = (max_dim * 3.0).max(4.0);
            if let Ok(mut cam) = cameras.single_mut() {
                // ~3 s flight — pull back, eye the machine, spiral in.
                let target = FlyTarget::new(id, target_dist, 3.0, &cam);
                cam.fly_target = Some(target);
            }
        }
    }
}
