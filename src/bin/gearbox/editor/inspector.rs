//! Inspector panel (right dock).
//!
//! Content layout matches the left panels (left-aligned) — only the
//! panel title itself is right-aligned, which is handled in
//! `float::floating_window`.

use bevy_egui::egui;

use crate::viz::GearboxSim;

use super::selection::Selection;
use super::style::{
    accent_color, fg_dim, section_caps, AXIS_X, AXIS_Y, AXIS_Z, TEXT_PRIMARY, TEXT_SECONDARY,
};

pub fn draw_content(
    ui: &mut egui::Ui,
    sim: &GearboxSim,
    selection: &Selection,
) {
    let Some(id) = selection.vehicle else {
        empty_state(ui);
        return;
    };
    let Some(state) = sim.0.vehicle(id) else {
        ui.label(egui::RichText::new("Selected vehicle no longer exists.").color(fg_dim()));
        return;
    };
    let pose = sim.0.vehicle_pose(id);
    let linvel = sim.0.vehicle_linvel(id);
    let ctrl = sim.0.control(id);
    let speed = (linvel.vx * linvel.vx + linvel.vy * linvel.vy + linvel.vz * linvel.vz).sqrt();

    // ─── Name / id row (left-aligned, flat) ───────────────────────
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(&state.spec.name)
                .strong()
                .size(13.0)
                .color(TEXT_PRIMARY),
        );
        ui.label(
            egui::RichText::new(format!("#{}", id.0))
                .small()
                .color(fg_dim()),
        );
    });
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("mass").small().color(fg_dim()));
        ui.label(
            egui::RichText::new(format!("{:.0} kg", state.spec.chassis.mass))
                .strong()
                .size(11.0),
        );
        ui.add_space(10.0);
        ui.label(egui::RichText::new("wheels").small().color(fg_dim()));
        ui.label(
            egui::RichText::new(state.spec.wheels.len().to_string())
                .strong()
                .size(11.0),
        );
    });
    ui.add_space(2.0);

    // ─── Geo ──────────────────────────────────────────────────────
    let geo = sim.0.vehicle_geo(id);
    let heading = sim.0.vehicle_heading(id);
    section(ui, "insp_geo", "Geo", |ui| {
        axis_row(ui, "lat", AXIS_Z, &format!("{:+.10}°", geo.latitude));
        axis_row(ui, "lon", AXIS_X, &format!("{:+.10}°", geo.longitude));
        axis_row(ui, "alt", AXIS_Y, &format!("{:+.4} m",  geo.altitude));
        plain_row(ui, "hdg", &format!("{:6.2}°  {}", heading, compass_letter(heading)));
    });

    // ─── Transform ────────────────────────────────────────────────
    section(ui, "insp_tr", "Transform", |ui| {
        sub_label(ui, "position");
        axis_row(ui, "X", AXIS_X, &format!("{:+.3} m", pose.point.x as f32));
        axis_row(ui, "Y", AXIS_Y, &format!("{:+.3} m", pose.point.y as f32));
        axis_row(ui, "Z", AXIS_Z, &format!("{:+.3} m", pose.point.z as f32));
        ui.add_space(2.0);
        sub_label(ui, "rotation");
        let q = pose.rotation;
        let (roll, pitch, yaw) = quat_to_euler(
            q.w as f32, q.x as f32, q.y as f32, q.z as f32,
        );
        axis_row(ui, "X", AXIS_X, &format!("{:+.2}°", roll.to_degrees()));
        axis_row(ui, "Y", AXIS_Y, &format!("{:+.2}°", pitch.to_degrees()));
        axis_row(ui, "Z", AXIS_Z, &format!("{:+.2}°", yaw.to_degrees()));
    });

    // ─── Velocity ─────────────────────────────────────────────────
    section(ui, "insp_vel", "Velocity", |ui| {
        axis_row(ui, "X", AXIS_X, &format!("{:+.2} m/s", linvel.vx as f32));
        axis_row(ui, "Y", AXIS_Y, &format!("{:+.2} m/s", linvel.vy as f32));
        axis_row(ui, "Z", AXIS_Z, &format!("{:+.2} m/s", linvel.vz as f32));
        ui.add_space(2.0);
        plain_row(ui, "|v|", &format!("{:.2} m/s", speed));
    });

    // ─── Control ──────────────────────────────────────────────────
    section(ui, "insp_ctl", "Control", |ui| {
        bar_row(ui, "throttle", ctrl.throttle, -1.0, 1.0);
        bar_row(ui, "steer",    ctrl.steer,    -1.0, 1.0);
        bar_row(ui, "brake",    ctrl.brake,     0.0, 1.0);
    });
}

// ─── Sections (egui built-in, caret-on-left, accent UPPERCASE label) ─

fn section(
    ui: &mut egui::Ui,
    id_src: &str,
    name: &str,
    add: impl FnOnce(&mut egui::Ui),
) {
    egui::CollapsingHeader::new(section_caps(name))
        .id_salt(id_src)
        .default_open(false)
        .show(ui, |ui| add(ui));
}

// ─── Rows (left-aligned, like spawn/tree panels) ──────────────────

/// `coloured-label  value` — left-aligned label, value flush right.
fn axis_row(
    ui: &mut egui::Ui,
    glyph: &str,
    color: egui::Color32,
    value: &str,
) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(glyph)
                .strong()
                .monospace()
                .size(11.0)
                .color(color),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(value)
                    .monospace()
                    .size(11.0)
                    .color(TEXT_PRIMARY),
            );
        });
    });
}

fn plain_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).small().color(TEXT_SECONDARY));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(value)
                    .monospace()
                    .size(11.0)
                    .color(TEXT_PRIMARY),
            );
        });
    });
}

fn sub_label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .small()
            .italics()
            .color(TEXT_SECONDARY),
    );
}

fn bar_row(ui: &mut egui::Ui, label: &str, v: f32, min: f32, max: f32) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).small().color(TEXT_SECONDARY));
        let frac = ((v - min) / (max - min)).clamp(0.0, 1.0);
        ui.add(
            egui::ProgressBar::new(frac)
                .text(egui::RichText::new(format!("{:+.2}", v)).monospace().small())
                .fill(accent_color())
                .corner_radius(egui::CornerRadius::same(3)),
        );
    });
}

/// Cardinal/intercardinal letter for a heading in degrees, e.g.
/// 0 → "N", 45 → "NE", 90 → "E", 185 → "S", 260 → "W".
fn compass_letter(h: f64) -> &'static str {
    // 22.5° arcs, centered on each of the 16 points — but we only
    // surface 8 (N/NE/E/SE/S/SW/W/NW) because that's what fits the
    // "6.2°  XX" column cleanly.
    let idx = ((h / 45.0).round() as i32).rem_euclid(8);
    ["N", "NE", "E", "SE", "S", "SW", "W", "NW"][idx as usize]
}

fn quat_to_euler(w: f32, x: f32, y: f32, z: f32) -> (f32, f32, f32) {
    let sinp = (2.0 * (w * y - z * x)).clamp(-1.0, 1.0);
    let pitch = sinp.asin();
    let roll = (2.0 * (w * x + y * z)).atan2(1.0 - 2.0 * (x * x + y * y));
    let yaw = (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z));
    (roll, pitch, yaw)
}

fn empty_state(ui: &mut egui::Ui) {
    ui.add_space(8.0);
    ui.vertical_centered(|ui| {
        ui.label(
            egui::RichText::new("No selection")
                .strong()
                .color(fg_dim()),
        );
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new(
                "Click a vehicle in the viewport,\nor pick one from Scene.",
            )
            .small()
            .color(fg_dim()),
        );
    });
}
