//! World / editor-tool controls — rendered from the Properties panel's
//! `world_section` when no vehicle is selected.
//!
//! Lays out three contained sections (Grid, Gizmos, Selection Ring)
//! using the shared `section()` / `labelled_row()` primitives so the
//! styling matches every other panel.

use bevy::prelude::*;
use bevy_egui::egui;

use gearbox_viz::GroundGrid;

use super::selection_ring::SelectionRingSettings;
use super::style::space;
use super::transform_gizmos::{GizmoModesEnabled, GizmoScale};
use super::widgets::{labelled_row, pretty_slider, section, toggle};

pub fn draw_content(
    ui: &mut egui::Ui,
    grid: &mut GroundGrid,
    gizmo_scale: &mut GizmoScale,
    gizmo_modes: &mut GizmoModesEnabled,
    ring: &mut SelectionRingSettings,
    accent: egui::Color32,
) {
    section(ui, "ui_grid", "Grid", accent, true, |ui| {
        labelled_row(ui, "visible", |ui| {
            toggle(ui, &mut grid.visible, accent);
        });
        labelled_row(ui, "opacity", |ui| {
            let mut alpha = grid.color.alpha();
            if pretty_slider(ui, &mut alpha, 0.0..=1.0, 2, "", accent).changed() {
                grid.color = grid.color.with_alpha(alpha);
            }
        });
        labelled_row(ui, "colour", |ui| {
            let alpha = grid.color.alpha();
            let mut c32 = bevy_to_egui_rgb(grid.color);
            if ui.color_edit_button_srgb(&mut c32).changed() {
                grid.color = egui_rgb_to_bevy(c32, alpha);
            }
        });
    });

    ui.add_space(space::SECTION);

    section(ui, "ui_gizmos", "Gizmos", accent, false, |ui| {
        labelled_row(ui, "size", |ui| {
            pretty_slider(ui, &mut gizmo_scale.0, 0.25..=4.0, 2, "", accent);
        });
        // Per-mode enable toggles — Tab only cycles between the modes
        // with a checked toggle. Translate + Rotate stay available for
        // day-to-day work; Scale is the one most users want to disable
        // (tractors don't grow).
        labelled_row(ui, "translate", |ui| {
            toggle(ui, &mut gizmo_modes.translate, accent);
        });
        labelled_row(ui, "rotate", |ui| {
            toggle(ui, &mut gizmo_modes.rotate, accent);
        });
        labelled_row(ui, "scale", |ui| {
            toggle(ui, &mut gizmo_modes.scale, accent);
        });
        // Guardrail: at least one mode must stay on, otherwise Tab
        // has nothing to cycle and the gizmo vanishes. Snap Translate
        // back on if the user tries to disable them all.
        if !gizmo_modes.translate && !gizmo_modes.rotate && !gizmo_modes.scale {
            gizmo_modes.translate = true;
        }
    });

    ui.add_space(space::SECTION);

    section(ui, "ui_selection_ring", "Selection Ring", accent, false, |ui| {
        labelled_row(ui, "thickness", |ui| {
            pretty_slider(ui, &mut ring.thickness, 0.02..=1.0, 2, "m", accent);
        });
    });
}

// ─── colour conversion helpers ──────────────────────────────────────

fn bevy_to_egui_rgb(c: Color) -> [u8; 3] {
    let s = c.to_srgba();
    [
        (s.red   * 255.0).round().clamp(0.0, 255.0) as u8,
        (s.green * 255.0).round().clamp(0.0, 255.0) as u8,
        (s.blue  * 255.0).round().clamp(0.0, 255.0) as u8,
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
