//! Inspector panel (right dock) — strictly **read-only**.
//!
//! Displays vehicle + world state; any setting that the user can edit
//! lives in the [`super::properties`] panel instead.

use bevy_egui::egui;

use gearbox::VehicleSpec;

use crate::viz::GearboxSim;

use super::selection::Selection;
use super::style::{
    contrast_text_for, fg_dim, section_caps, AXIS_X, AXIS_Y, AXIS_Z, TEXT_PRIMARY, TEXT_SECONDARY,
};

pub fn draw_content(
    ui: &mut egui::Ui,
    sim: &mut GearboxSim,
    selection: &Selection,
    accent: egui::Color32,
) {
    let Some(id) = selection.vehicle else {
        world_info(ui, sim, accent);
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
    let size = state.spec.chassis.size;

    // Top-down footprint: chassis + every part, projected onto XZ and
    // unioned. Gives the "looking from above" bounding box.
    let (fp_x, fp_z) = top_down_footprint(&state.spec);

    // ─── Info ─────────────────────────────────────────────────────
    section(ui, "insp_info", "Info", true, accent, |ui| {
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
        plain_row(ui, "mass", &format!("{:.0} kg", state.spec.chassis.mass));
        plain_row(ui, "wheels", &state.spec.wheels.len().to_string());
        plain_row(ui, "parts", &state.spec.parts.len().to_string());
        plain_row(
            ui,
            "footprint",
            &format!("{:.2} × {:.2} m", fp_x, fp_z),
        );

        // Power reservoir(s). One bar per source so Robotti shows
        // TWO bars stacked — no labels; the bar fill itself is the
        // state indicator. The active source (the one currently
        // draining) is drawn with a full-saturation accent; idle
        // sources fade to 45 %.
        let active = state.spec.power.active_source();
        for (idx, src) in state.spec.power.sources.iter().enumerate() {
            ui.add_space(2.0);
            let frac = src.fraction();
            let is_active = active == Some(idx);
            let bar_col = if is_active {
                accent
            } else {
                accent.linear_multiply(0.45)
            };
            ui.add(
                egui::ProgressBar::new(frac)
                    .text(
                        egui::RichText::new(format!(
                            "{:.0} / {:.0}",
                            src.current, src.capacity
                        ))
                        .monospace()
                        .small()
                        .color(contrast_text_for(bar_col)),
                    )
                    .fill(bar_col)
                    .corner_radius(egui::CornerRadius::same(3)),
            );
        }
        if !state.spec.power.sources.is_empty() {
            ui.add_space(2.0);
            // Surface the tick's live belief — what speed it saw,
            // whether it classified as "moving", and the drain rate
            // applied. If this says PARKED while you're actually
            // driving, the physics body's linvel is lower than the
            // 0.35 m/s threshold; if it says MOVING while you're
            // idle, something is perturbing the chassis.
            let power = &state.spec.power;
            let tag = if !power.turned_on {
                "ENGINE OFF"
            } else if power.last_moving {
                "MOVING"
            } else {
                "PARKED"
            };
            plain_row(
                ui,
                tag,
                &format!(
                    "{:.2} m/s · {:.2} u/s",
                    power.last_horiz_speed, power.last_drain_rate
                ),
            );
        }

        // Container(s) — stacked under the power bars. Same visual
        // language: just a bar with the numeric readout inside.
        for container in &state.spec.containers {
            ui.add_space(2.0);
            let frac = container.fraction();
            ui.add(
                egui::ProgressBar::new(frac)
                    .text(
                        egui::RichText::new(format!(
                            "{:.0} / {:.0}",
                            container.amount, container.capacity
                        ))
                        .monospace()
                        .small()
                        .color(contrast_text_for(accent)),
                    )
                    .fill(accent)
                    .corner_radius(egui::CornerRadius::same(3)),
            );
        }
    });

    // ─── Geo ──────────────────────────────────────────────────────
    let geo = sim.0.vehicle_geo(id);
    let heading = sim.0.vehicle_heading(id);
    section(ui, "insp_geo", "Geo", false, accent, |ui| {
        axis_row(ui, "lat", AXIS_Z, &format!("{:+.10}°", geo.latitude));
        axis_row(ui, "lon", AXIS_X, &format!("{:+.10}°", geo.longitude));
        axis_row(ui, "alt", AXIS_Y, &format!("{:+.4} m",  geo.altitude));
        plain_row(ui, "hdg", &format!("{:6.2}°  {}", heading, compass_letter(heading)));
    });

    // ─── Transform (read-only; edit in Properties) ──────────────────
    section(ui, "insp_tr", "Transform", false, accent, |ui| {
        sub_label(ui, "position");
        axis_row(ui, "X", AXIS_X, &format!("{:+.3} m", pose.point.x));
        axis_row(ui, "Y", AXIS_Y, &format!("{:+.3} m", pose.point.y));
        axis_row(ui, "Z", AXIS_Z, &format!("{:+.3} m", pose.point.z));

        let q = pose.rotation;
        let (rx, ry, rz) = quat_to_euler_xyz(q.w as f32, q.x as f32, q.y as f32, q.z as f32);
        ui.add_space(2.0);
        sub_label(ui, "rotation  (Euler XYZ)");
        axis_row(ui, "X", AXIS_X, &format!("{:+.2}°", rx.to_degrees()));
        axis_row(ui, "Y", AXIS_Y, &format!("{:+.2}°", ry.to_degrees()));
        axis_row(ui, "Z", AXIS_Z, &format!("{:+.2}°", rz.to_degrees()));

        ui.add_space(2.0);
        sub_label(ui, "scale  (chassis size — baked at spawn)");
        axis_row(ui, "X", AXIS_X, &format!("{:.3} m", size.x));
        axis_row(ui, "Y", AXIS_Y, &format!("{:.3} m", size.y));
        axis_row(ui, "Z", AXIS_Z, &format!("{:.3} m", size.z));
    });

    // ─── Velocity ─────────────────────────────────────────────────
    section(ui, "insp_vel", "Velocity", false, accent, |ui| {
        axis_row(ui, "X", AXIS_X, &format!("{:+.2} m/s", linvel.vx as f32));
        axis_row(ui, "Y", AXIS_Y, &format!("{:+.2} m/s", linvel.vy as f32));
        axis_row(ui, "Z", AXIS_Z, &format!("{:+.2} m/s", linvel.vz as f32));
        ui.add_space(2.0);
        plain_row(ui, "|v|", &format!("{:.2} m/s", speed));
    });

    // ─── Control ──────────────────────────────────────────────────
    section(ui, "insp_ctl", "Control", false, accent, |ui| {
        bar_row(ui, "throttle", ctrl.throttle, -1.0, 1.0, accent);
        bar_row(ui, "steer",    ctrl.steer,    -1.0, 1.0, accent);
        bar_row(ui, "brake",    ctrl.brake,     0.0, 1.0, accent);
    });
}

// ─── Sections (egui built-in, caret-on-left, accent UPPERCASE label) ─

fn section(
    ui: &mut egui::Ui,
    id_src: &str,
    name: &str,
    default_open: bool,
    accent: egui::Color32,
    add: impl FnOnce(&mut egui::Ui),
) {
    egui::CollapsingHeader::new(section_caps(name, accent))
        .id_salt(id_src)
        .default_open(default_open)
        .show(ui, |ui| add(ui));
}

/// Top-down (XZ) footprint of a vehicle — union of chassis + every
/// part projected onto the ground. Returns `(width_x, length_z)`.
fn top_down_footprint(spec: &VehicleSpec) -> (f64, f64) {
    let hx = spec.chassis.size.x * 0.5;
    let hz = spec.chassis.size.z * 0.5;
    let (mut x_min, mut x_max) = (-hx, hx);
    let (mut z_min, mut z_max) = (-hz, hz);
    for p in &spec.parts {
        let phx = p.size.x * 0.5;
        let phz = p.size.z * 0.5;
        x_min = x_min.min(p.position.x - phx);
        x_max = x_max.max(p.position.x + phx);
        z_min = z_min.min(p.position.z - phz);
        z_max = z_max.max(p.position.z + phz);
    }
    (x_max - x_min, z_max - z_min)
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

fn bar_row(ui: &mut egui::Ui, label: &str, v: f32, min: f32, max: f32, accent: egui::Color32) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).small().color(TEXT_SECONDARY));
        let frac = ((v - min) / (max - min)).clamp(0.0, 1.0);
        ui.add(
            egui::ProgressBar::new(frac)
                .text(
                    egui::RichText::new(format!("{:+.2}", v))
                        .monospace()
                        .small()
                        .color(contrast_text_for(accent)),
                )
                .fill(accent)
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

/// Quaternion → intrinsic XYZ Euler angles (radians). Inverse of
/// `euler_xyz_to_quat` so round-tripping through the inspector keeps
/// the rotation stable.
fn quat_to_euler_xyz(w: f32, x: f32, y: f32, z: f32) -> (f32, f32, f32) {
    // Intrinsic rotations: Rz · Ry · Rx (applied R_x first when we
    // build the quaternion below).
    let sy = 2.0 * (w * y + x * z).clamp(-1.0, 1.0);
    let ey = sy.asin();
    let ex;
    let ez;
    if sy.abs() > 0.9999 {
        // Gimbal lock — fold all roll into yaw.
        ex = 0.0;
        ez = (-2.0 * (x * y - w * z)).atan2(1.0 - 2.0 * (y * y + z * z));
    } else {
        ex = (-2.0 * (y * z - w * x)).atan2(1.0 - 2.0 * (x * x + y * y));
        ez = (-2.0 * (x * y - w * z)).atan2(1.0 - 2.0 * (y * y + z * z));
    }
    (ex, ey, ez)
}

// Transform EDITING lives in `editor::properties`. The inspector
// only shows read-only values, so the drag/euler helpers have moved
// there.

fn world_info(ui: &mut egui::Ui, sim: &mut GearboxSim, accent: egui::Color32) {
    let planet = sim.0.planet;
    let gravity = sim.0.gravity;
    let vehicle_count = sim.0.vehicles().count();

    // ─── Summary (first — default-open) ───────────────────────────
    section(ui, "world_summary", "World", true, accent, |ui| {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("No selection")
                    .strong()
                    .size(13.0)
                    .color(TEXT_PRIMARY),
            );
        });
        plain_row(ui, "vehicles", &vehicle_count.to_string());
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new("Click a vehicle in the viewport, or pick one from Scene.")
                .small()
                .color(fg_dim()),
        );
    });

    // ─── Planet ───────────────────────────────────────────────────
    section(ui, "world_planet", "Planet", false, accent, |ui| {
        plain_row(ui, "radius", &format!("{:.0} m", planet.radius));
        plain_row(
            ui,
            "circumference",
            &format!("{:.0} km", planet.radius * std::f64::consts::TAU / 1_000.0),
        );
        ui.add_space(2.0);
        sub_label(ui, "datum");
        axis_row(
            ui,
            "lat",
            AXIS_Z,
            &format!("{:+.6}°", planet.datum.latitude),
        );
        axis_row(
            ui,
            "lon",
            AXIS_X,
            &format!("{:+.6}°", planet.datum.longitude),
        );
        axis_row(
            ui,
            "alt",
            AXIS_Y,
            &format!("{:+.2} m", planet.datum.altitude),
        );
    });

    // ─── Physics ──────────────────────────────────────────────────
    section(ui, "world_physics", "Physics", false, accent, |ui| {
        axis_row(ui, "gx", AXIS_X, &format!("{:+.2} m/s²", gravity.x));
        axis_row(ui, "gy", AXIS_Y, &format!("{:+.2} m/s²", gravity.y));
        axis_row(ui, "gz", AXIS_Z, &format!("{:+.2} m/s²", gravity.z));
    });
}
