//! Floating left dock — letter buttons at top-left, content window next to them.
//!
//! Layout mirrors VS Code's activity bar + panel idea, but fully floating.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use gearbox_physics::VehicleId;

use gearbox_viz::{ChaseCamera, FollowTarget, GearboxSim, PlayerControlled, VehicleBody};
use gearbox_viz::camera::FlyTarget;

use super::pending_spawn::PendingSpawn;
use super::persist::EditorUiState;
use super::preset_registry::PresetRegistry;
use super::selection::Selection;
use super::style::AccentColor;
use super::{float, spawn_panel, tree};

#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum LeftTab {
    #[default]
    None,
    Workspace,
    Library,
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
    registry: Res<PresetRegistry>,
    accent: Res<AccentColor>,
    mut cameras: Query<&mut ChaseCamera>,
    mut follow: ResMut<FollowTarget>,
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
        "left_btn_library", ctx, egui::Align2::LEFT_TOP, 1,
        "L", "Library",
        matches!(*active, LeftTab::Library),
        accent_col,
        || *active = if *active == LeftTab::Library { LeftTab::None } else { LeftTab::Library },
    );

    if *active == LeftTab::None { return; }

    let (id, title, size) = match *active {
        LeftTab::Workspace => ("left_window_workspace", "Workspace", ui_state.workspace_size),
        LeftTab::Library   => ("left_window_library",   "Library",   ui_state.spawn_size),
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
                &mut follow,
                accent_col,
                &mut frame_to,
            ),
            LeftTab::Library => spawn_panel::draw_content(
                ui,
                &mut commands,
                &mut pending,
                &existing_bodies,
                &registry,
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
    // camera to SMOOTHLY fly to that vehicle. The cinematic arc is
    // **size-independent**: every machine pulls back to the same
    // apex and settles at the same final distance, so the "route"
    // looks identical whether you frame a drone or a harvester.
    // Final framing size difference is just the natural consequence
    // of the machine being bigger in world-space.
    if let Some(id) = frame_to {
        if sim.0.vehicle(id).is_some() {
            // Cinematic constants — tuned to read well from a drone
            // up to an oxbo-sized machine at typical screen sizes.
            const FINAL_DISTANCE: f32 = 18.0;
            const FLY_DURATION:  f32 = 3.0;
            if let Ok(mut cam) = cameras.single_mut() {
                let target = FlyTarget::new(id, FINAL_DISTANCE, FLY_DURATION, &cam);
                cam.fly_target = Some(target);
            }
        }
    }
}
