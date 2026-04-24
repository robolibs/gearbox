//! Workspace + Library panel bodies. Ribbon buttons are declared in
//! `super::dock_ribbons` and drawn in a separate system; this file
//! only renders the two panels (their floating windows anchor +
//! width follow whichever cluster the user has dragged the button
//! into at the moment).

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use gearbox_physics::VehicleId;

use bevy_frost::{floating_window_for_item, RibbonOpen, RibbonPlacement};
use gearbox_viz::{ChaseCamera, FollowTarget, GearboxSim, PlayerControlled, VehicleBody};
use gearbox_viz::camera::FlyTarget;

use super::dock_ribbons::{is_menu_open, ID_LIBRARY, ID_WORKSPACE, RIBBONS, RIBBON_ITEMS};
use super::pending_spawn::PendingSpawn;
use super::persist::EditorUiState;
use super::preset_registry::PresetRegistry;
use super::selection::Selection;
use super::style::AccentColor;
use super::{spawn_panel, tree};

/// Asset + tag-query bundle — collapses four system params into one
/// so `left_dock_ui` stays under Bevy's 16-param tuple cap.
#[derive(bevy::ecs::system::SystemParam)]
pub struct LeftDockAssets<'w, 's> {
    pub meshes: ResMut<'w, Assets<Mesh>>,
    pub materials: ResMut<'w, Assets<StandardMaterial>>,
    pub existing_bodies: Query<'w, 's, Entity, With<VehicleBody>>,
    pub player_tagged: Query<'w, 's, Entity, With<PlayerControlled>>,
}

pub fn left_dock_ui(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    ui_state: Res<EditorUiState>,
    // spawn
    mut commands: Commands,
    mut sim: ResMut<GearboxSim>,
    assets: LeftDockAssets,
    mut selection: ResMut<Selection>,
    // tree
    bodies: Query<(Entity, &VehicleBody, Option<&Name>, Has<PlayerControlled>)>,
    mut pending: ResMut<PendingSpawn>,
    registry: Res<PresetRegistry>,
    accent: Res<AccentColor>,
    mut cameras: Query<&mut ChaseCamera>,
    mut follow: ResMut<FollowTarget>,
) {
    let LeftDockAssets {
        mut meshes,
        mut materials,
        existing_bodies,
        player_tagged,
    } = assets;
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let accent_col = accent.0;

    let mut frame_to: Option<VehicleId> = None;

    if is_menu_open(&open, &placement, ID_WORKSPACE) {
        let size = ui_state.workspace_size;
        let mut keep_open = true;
        floating_window_for_item(
            ctx,
            RIBBONS,
            RIBBON_ITEMS,
            &placement,
            ID_WORKSPACE,
            "Workspace",
            egui::vec2(size.x, size.y),
            &mut keep_open,
            accent_col,
            |ui| {
                tree::draw_content(
                    ui,
                    &mut commands,
                    &sim,
                    &bodies,
                    &mut selection,
                    &mut follow,
                    accent_col,
                    &mut frame_to,
                );
            },
        );
    }
    if is_menu_open(&open, &placement, ID_LIBRARY) {
        let size = ui_state.spawn_size;
        let mut keep_open = true;
        floating_window_for_item(
            ctx,
            RIBBONS,
            RIBBON_ITEMS,
            &placement,
            ID_LIBRARY,
            "Library",
            egui::vec2(size.x, size.y),
            &mut keep_open,
            accent_col,
            |ui| {
                spawn_panel::draw_content(
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
                );
            },
        );
    }

    // ─── Apply the "frame this vehicle" request ──────────────────
    if let Some(id) = frame_to {
        if sim.0.vehicle(id).is_some() {
            const FINAL_DISTANCE: f32 = 18.0;
            const FLY_DURATION: f32 = 3.0;
            if let Ok(mut cam) = cameras.single_mut() {
                let target = FlyTarget::new(id, FINAL_DISTANCE, FLY_DURATION, &cam);
                cam.fly_target = Some(target);
            }
        }
    }
}
