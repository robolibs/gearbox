//! UI-settings panel (right dock, "U" button).
//!
//! Collapsible sections let the user tweak presentation layers —
//! currently the world grid (on/off, opacity, colour).

use bevy::prelude::*;
use bevy_egui::egui;

use crate::viz::GroundGrid;

use super::style::{fg_dim, section_caps};
use super::transform_gizmos::GizmoScale;

pub fn draw_content(
    ui: &mut egui::Ui,
    grid: &mut GroundGrid,
    gizmo_scale: &mut GizmoScale,
    accent: egui::Color32,
) {
    egui::CollapsingHeader::new(section_caps("Grid", accent))
        .id_salt("ui_grid")
        .default_open(true)
        .show(ui, |ui| {
            // ─── visible toggle ─────────────────────────────────
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("visible").small().color(fg_dim()));
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        ui.checkbox(&mut grid.visible, "");
                    },
                );
            });

            // ─── opacity slider ─────────────────────────────────
            let mut alpha = grid.color.alpha();
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("opacity").small().color(fg_dim()));
                if ui
                    .add(
                        egui::Slider::new(&mut alpha, 0.0..=1.0)
                            .show_value(true)
                            .fixed_decimals(2),
                    )
                    .changed()
                {
                    grid.color = grid.color.with_alpha(alpha);
                }
            });

            // ─── colour picker (RGB only — alpha is the slider) ─
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("colour").small().color(fg_dim()));
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        let mut c32 = bevy_to_egui_rgb(grid.color);
                        if ui.color_edit_button_srgb(&mut c32).changed() {
                            grid.color = egui_rgb_to_bevy(c32, alpha);
                        }
                    },
                );
            });
        });

    egui::CollapsingHeader::new(section_caps("Gizmos", accent))
        .id_salt("ui_gizmos")
        .default_open(false)
        .show(ui, |ui| {
            // One control — makes the transform gizmos uniformly
            // thicker *and* bigger. The whole handle transform is
            // scaled by this, so shafts, tips, rings and cubes all
            // grow together.
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("size").small().color(fg_dim()));
                ui.add(
                    egui::Slider::new(&mut gizmo_scale.0, 0.25..=4.0)
                        .show_value(true)
                        .fixed_decimals(2),
                );
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
