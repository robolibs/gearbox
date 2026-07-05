//! Keyboard shortcuts: panel toggles + overlay toggles + Ctrl+K
//! command palette. Adapted from the bevy_openusd viewer.

use bevy::prelude::*;
use bevy_egui::input::egui_wants_any_keyboard_input;

use crate::viewer::mara_ui::RibbonOpen;
use crate::viewer::overlays::DisplayToggles;
use crate::viewer::state::ReloadRequest;
use crate::viewer::ui::{
    RIB_INFO, RIB_KEYS, RIB_OVERLAYS, RIB_TREE, RIBBON_LEFT, ViewerCommandPalette,
};

pub struct ViewerKeyboardPlugin;

impl Plugin for ViewerKeyboardPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                handle_keys.run_if(not(egui_wants_any_keyboard_input)),
                handle_palette_shortcut,
            ),
        );
    }
}

fn handle_palette_shortcut(
    keys: Res<ButtonInput<KeyCode>>,
    mut palette: ResMut<ViewerCommandPalette>,
) {
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    if !ctrl {
        return;
    }
    if keys.just_pressed(KeyCode::KeyK) || keys.just_pressed(KeyCode::KeyP) {
        palette.0.open = !palette.0.open;
        if palette.0.open {
            palette.0.query.clear();
            palette.0.selected = 0;
        }
    }
}

fn handle_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut ribbon: ResMut<RibbonOpen>,
    mut toggles: ResMut<DisplayToggles>,
    mut reload: ResMut<ReloadRequest>,
) {
    if keys.just_pressed(KeyCode::KeyT) {
        ribbon.toggle(RIBBON_LEFT, RIB_TREE);
    }
    if keys.just_pressed(KeyCode::KeyI) {
        ribbon.toggle(RIBBON_LEFT, RIB_INFO);
    }
    if keys.just_pressed(KeyCode::KeyO) {
        ribbon.toggle(RIBBON_LEFT, RIB_OVERLAYS);
    }
    if keys.just_pressed(KeyCode::Slash) {
        ribbon.toggle(RIBBON_LEFT, RIB_KEYS);
    }
    if keys.just_pressed(KeyCode::KeyG) {
        toggles.show_world_grid = !toggles.show_world_grid;
    }
    if keys.just_pressed(KeyCode::KeyX) {
        toggles.show_world_axes = !toggles.show_world_axes;
    }
    if keys.just_pressed(KeyCode::KeyP) {
        toggles.show_prim_markers = !toggles.show_prim_markers;
    }
    if keys.just_pressed(KeyCode::KeyB) {
        toggles.show_skeleton = !toggles.show_skeleton;
    }
    if keys.just_pressed(KeyCode::KeyY) {
        toggles.show_physics = !toggles.show_physics;
    }
    if keys.just_pressed(KeyCode::KeyC) {
        toggles.show_colliders = !toggles.show_colliders;
    }
    if keys.just_pressed(KeyCode::KeyR) {
        reload.requested = true;
    }
}
