//! World / editor-tool controls — rendered from the Properties panel's
//! `world_section` when no vehicle is selected.
//!
//! Lays out three contained sections (Grid, Gizmos, Selection Ring)
//! using the shared `section()` / row-module primitives so the
//! styling matches every other panel.

use bevy::prelude::*;
use bevy_egui::egui;
use bevy_frost::PaneBuilder;

use gearbox_viz::GroundGrid;

use super::selection_ring::SelectionRingSettings;
use super::style::GlassOpacity;
use super::transform_gizmos::{GizmoModesEnabled, GizmoScale};
use super::widgets::{dual_pane_labelled, pretty_slider, toggle};

pub fn draw_content(
    pane: &mut PaneBuilder,
    grid: &mut GroundGrid,
    gizmo_scale: &mut GizmoScale,
    gizmo_modes: &mut GizmoModesEnabled,
    ring: &mut SelectionRingSettings,
    glass_opacity: &mut GlassOpacity,
    accent: egui::Color32,
) {
    pane.section("ui_theme", "Theme", false, |ui| {
        let mut v = glass_opacity.0 as f64;
        if pretty_slider(ui, "opacity", &mut v, 1.0..=100.0, 0, "%", accent).changed() {
            glass_opacity.0 = v.round().clamp(1.0, 100.0) as u8;
        }
    });

    pane.section("ui_grid", "Grid", true, |ui| {
        toggle(ui, "visible", &mut grid.visible, accent);

        let mut alpha = grid.color.alpha() as f64;
        if pretty_slider(ui, "opacity", &mut alpha, 0.0..=1.0, 2, "", accent).changed() {
            grid.color = grid.color.with_alpha(alpha as f32);
        }

        dual_pane_labelled(ui, "colour", |ui| {
            let alpha = grid.color.alpha();
            let mut c32 = bevy_to_egui_rgb(grid.color);
            if ui.color_edit_button_srgb(&mut c32).changed() {
                grid.color = egui_rgb_to_bevy(c32, alpha);
            }
        });
    });

    pane.section("ui_gizmos", "Gizmos", false, |ui| {
        let mut v = gizmo_scale.0 as f64;
        if pretty_slider(ui, "size", &mut v, 0.25..=4.0, 2, "", accent).changed() {
            gizmo_scale.0 = v as f32;
        }
        toggle(ui, "translate", &mut gizmo_modes.translate, accent);
        toggle(ui, "rotate", &mut gizmo_modes.rotate, accent);
        toggle(ui, "scale", &mut gizmo_modes.scale, accent);
        if !gizmo_modes.translate && !gizmo_modes.rotate && !gizmo_modes.scale {
            gizmo_modes.translate = true;
        }
    });

    pane.section("ui_selection_ring", "Selection Ring", false, |ui| {
        let mut t = ring.thickness as f64;
        if pretty_slider(ui, "thickness", &mut t, 0.02..=1.0, 2, "m", accent).changed() {
            ring.thickness = t as f32;
        }
    });
}

// ─── colour conversion helpers ──────────────────────────────────────

fn bevy_to_egui_rgb(c: Color) -> [u8; 3] {
    let s = c.to_srgba();
    [
        (s.red * 255.0).round().clamp(0.0, 255.0) as u8,
        (s.green * 255.0).round().clamp(0.0, 255.0) as u8,
        (s.blue * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

fn egui_rgb_to_bevy(rgb: [u8; 3], alpha: f32) -> Color {
    Color::srgba(
        rgb[0] as f32 / 255.0,
        rgb[1] as f32 / 255.0,
        rgb[2] as f32 / 255.0,
        alpha,
    )
}
