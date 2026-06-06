//! Workspace + Library panel bodies. Ribbon buttons are declared in
//! `super::dock_ribbons` and drawn in a separate system; this file
//! only renders the two panels (their floating windows anchor +
//! width follow whichever cluster the user has dragged the button
//! into at the moment).

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use gearbox_physics::VehicleId;

use bevy_frost::{PaneBuilder, RibbonOpen, RibbonPlacement, floating_window_for_item};
use gearbox_viz::camera::{ChaseCameraFly, FlyTarget};
use gearbox_viz::{ChaseCamera, FollowTarget, GearboxSim, PlayerControlled, VehicleBody};

use super::dock_ribbons::{ID_LIBRARY, ID_WORKSPACE, RIBBON_ITEMS, RIBBONS, is_menu_open};
use super::pending_spawn::PendingSpawn;
use super::persist::EditorUiState;
use super::preset_registry::PresetRegistry;
use super::selection::Selection;
use super::style::AccentColor;
use super::usd_load::UsdSelectable;
use super::{spawn_panel, tree};

/// Asset + tag-query bundle — collapses param into one so
/// `left_dock_ui` stays under Bevy's 16-param tuple cap.
#[derive(bevy::ecs::system::SystemParam)]
pub struct LeftDockAssets<'w, 's> {
    pub meshes: ResMut<'w, Assets<Mesh>>,
    pub materials: ResMut<'w, Assets<StandardMaterial>>,
    pub existing_bodies: Query<'w, 's, Entity, With<VehicleBody>>,
    pub player_tagged: Query<'w, 's, Entity, With<PlayerControlled>>,
    pub pending_usd_removal: ResMut<'w, super::usd_load::PendingUsdRemoval>,
    pub usd_tree_expanded: ResMut<'w, super::usd_load::UsdTreeExpanded>,
    pub usd_assets: Query<'w, 's, (Entity, Option<&'static Name>), With<UsdSelectable>>,
    pub usd_prims: Query<'w, 's, (Option<&'static Name>, &'static usd_bevy::UsdPrimRef)>,
    pub usd_children: Query<'w, 's, &'static Children>,
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
    cameras: Query<&ChaseCamera>,
    mut chase_fly: ResMut<ChaseCameraFly>,
    mut follow: ResMut<FollowTarget>,
) {
    let LeftDockAssets {
        mut meshes,
        mut materials,
        existing_bodies,
        usd_assets,
        usd_prims,
        usd_children,
        pending_usd_removal: mut pending_usd_removal_inner,
        usd_tree_expanded: mut usd_tree_expanded_inner,
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
            |pane: &mut PaneBuilder| {
                tree::draw_content(
                    pane,
                    &mut commands,
                    &sim,
                    &bodies,
                    &usd_assets,
                    &usd_prims,
                    &usd_children,
                    &mut selection,
                    &mut follow,
                    &mut pending_usd_removal_inner,
                    &mut usd_tree_expanded_inner,
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
            |pane: &mut PaneBuilder| {
                spawn_panel::draw_content(
                    pane,
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
            if let Ok(cam) = cameras.single() {
                chase_fly.target = Some(FlyTarget::new(id, FINAL_DISTANCE, FLY_DURATION, cam));
            }
        }
    }
}
