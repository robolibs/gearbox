//! Inspector + Properties panel bodies. Ribbon buttons are declared
//! in `super::dock_ribbons` and drawn in a separate system; this
//! file only renders the two panels.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use bevy_frost::{floating_window_for_item, PaneBuilder, RibbonOpen, RibbonPlacement};
use gearbox_viz::{GearboxSim, GroundGrid};

use super::dock_ribbons::{is_menu_open, ID_INSPECTOR, ID_PROPERTIES, RIBBONS, RIBBON_ITEMS};
use super::inspector::UsdInspect;
use super::persist::EditorUiState;
use super::properties::PendingColorChange;
use super::selection::Selection;
use super::selection_ring::SelectionRingSettings;
use super::style::AccentColor;
use super::transform_gizmos::{GizmoModesEnabled, GizmoScale};
use super::usd_load::UsdSelectable;
use super::{inspector, properties};

pub fn right_dock_ui(
    mut contexts: EguiContexts,
    open: Res<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    ui_state: Res<EditorUiState>,
    mut sim: ResMut<GearboxSim>,
    selection: Res<Selection>,
    mut grid: ResMut<GroundGrid>,
    mut gizmo_scale: ResMut<GizmoScale>,
    mut gizmo_modes: ResMut<GizmoModesEnabled>,
    mut ring_settings: ResMut<SelectionRingSettings>,
    mut glass_opacity: ResMut<super::style::GlassOpacity>,
    mut pending_color: ResMut<PendingColorChange>,
    accent: Res<AccentColor>,
    usd_query: Query<(&GlobalTransform, Option<&Name>), With<UsdSelectable>>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let accent_col = accent.0;

    // Build USD inspector data when a USD asset is the active
    // selection. Read `GlobalTransform` so the readout reflects the
    // composed world pose (not just `Transform` local to a parent).
    let usd_inspect = selection.usd_entity.and_then(|e| {
        usd_query.get(e).ok().map(|(gt, name)| {
            let t = gt.compute_transform();
            // Project world position onto the planet datum for a
            // lat/lon/alt readout matching what vehicles show.
            let geo = sim.0.planet.local_to_geo(gearbox_physics::datapod::Point::new(
                t.translation.x as f64,
                t.translation.y as f64,
                t.translation.z as f64,
            ));
            UsdInspect {
                name: name.map(|n| n.as_str().to_string()).unwrap_or_else(|| "USD".to_string()),
                world_translation: t.translation,
                world_rotation: t.rotation,
                world_scale: t.scale,
                geo_latitude: geo.latitude,
                geo_longitude: geo.longitude,
                geo_altitude: geo.altitude,
            }
        })
    });

    if is_menu_open(&open, &placement, ID_INSPECTOR) {
        let size = ui_state.inspector_size;
        let mut keep_open = true;
        floating_window_for_item(
            ctx,
            RIBBONS,
            RIBBON_ITEMS,
            &placement,
            ID_INSPECTOR,
            "Inspector",
            egui::vec2(size.x, size.y),
            &mut keep_open,
            accent_col,
            |pane: &mut PaneBuilder| {
                inspector::draw_content(pane, &mut sim, &selection, usd_inspect, accent_col)
            },
        );
    }
    if is_menu_open(&open, &placement, ID_PROPERTIES) {
        let size = ui_state.inspector_size; // reuse default
        let mut keep_open = true;
        floating_window_for_item(
            ctx,
            RIBBONS,
            RIBBON_ITEMS,
            &placement,
            ID_PROPERTIES,
            "Properties",
            egui::vec2(size.x, size.y),
            &mut keep_open,
            accent_col,
            |pane: &mut PaneBuilder| {
                properties::draw_content(
                    pane,
                    &mut sim,
                    &selection,
                    &mut grid,
                    &mut gizmo_scale,
                    &mut gizmo_modes,
                    &mut ring_settings,
                    &mut glass_opacity,
                    &mut pending_color,
                    accent_col,
                )
            },
        );
    }
}
