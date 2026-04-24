//! Inspector panel (right dock) — strictly **read-only**.
//!
//! Displays vehicle + world state. Any setting the user can edit
//! lives in the [`super::properties`] panel. Layout is built on the
//! widget primitives in `super::widgets` so every row / bar / section
//! shares the same spacing + typography.

use bevy_egui::egui;

use gearbox_physics::VehicleSpec;

use gearbox_viz::GearboxSim;

use super::selection::Selection;
use super::style::{
    caption, font, space, title_text, AXIS_X, AXIS_Y, AXIS_Z, TEXT_SECONDARY,
};
use super::widgets::{
    axis_readout_row, pretty_progressbar_text, readout_row, section, sub_caption, subsection,
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
        sub_caption(ui, "Selected vehicle no longer exists.");
        return;
    };

    let pose = sim.0.vehicle_pose(id);
    let linvel = sim.0.vehicle_linvel(id);
    let ctrl = sim.0.control(id);
    let speed = (linvel.vx * linvel.vx + linvel.vy * linvel.vy + linvel.vz * linvel.vz).sqrt();
    let size = state.spec.chassis.size;
    let (fp_x, fp_z) = top_down_footprint(&state.spec);

    // ═══ Info ═══════════════════════════════════════════════════════
    section(ui, "insp_info", "Info", accent, true, |ui| {
        // Name + #id header.
        ui.horizontal(|ui| {
            ui.label(title_text(&state.spec.name));
            ui.label(
                egui::RichText::new(format!("#{}", id.0))
                    .size(font::CAPTION)
                    .color(TEXT_SECONDARY),
            );
        });
        ui.add_space(space::ROW);

        readout_row(ui, "mass",      &format!("{:.0} kg", state.spec.chassis.mass));
        readout_row(ui, "wheels",    &state.spec.wheels.len().to_string());
        readout_row(ui, "parts",     &state.spec.parts.len().to_string());
        readout_row(ui, "footprint", &format!("{:.2} × {:.2} m", fp_x, fp_z));

        // Power reservoirs — one labelled progress bar per source,
        // active fully saturated, inactive faded.
        if !state.spec.power.sources.is_empty() {
            let active = state.spec.power.active_source();
            for (idx, src) in state.spec.power.sources.iter().enumerate() {
                let is_active = active == Some(idx);
                let bar_col = if is_active {
                    accent
                } else {
                    accent.linear_multiply(0.45)
                };
                let text = format!("{:.0} / {:.0}", src.current, src.capacity);
                pretty_progressbar_text(
                    ui,
                    &src.label,
                    src.fraction() as f32,
                    &text,
                    bar_col,
                );
            }
            let power = &state.spec.power;
            let tag = if !power.turned_on {
                "engine off"
            } else if power.last_moving {
                "moving"
            } else {
                "parked"
            };
            readout_row(
                ui,
                tag,
                &format!(
                    "{:.2} m/s · {:.2} u/s",
                    power.last_horiz_speed, power.last_drain_rate
                ),
            );
        }

        // Containers — same labelled bar language.
        if !state.spec.containers.is_empty() {
            for (idx, container) in state.spec.containers.iter().enumerate() {
                let name = format!("container #{}", idx + 1);
                let text = format!("{:.0} / {:.0}", container.amount, container.capacity);
                pretty_progressbar_text(
                    ui,
                    &name,
                    container.fraction() as f32,
                    &text,
                    accent,
                );
            }
        }
    });

    ui.add_space(space::SECTION);

    // ═══ Geo ════════════════════════════════════════════════════════
    let geo = sim.0.vehicle_geo(id);
    let heading = sim.0.vehicle_heading(id);
    section(ui, "insp_geo", "Geo", accent, false, |ui| {
        axis_readout_row(ui, "lat", AXIS_Z, &format!("{:+.10}°", geo.latitude));
        axis_readout_row(ui, "lon", AXIS_X, &format!("{:+.10}°", geo.longitude));
        axis_readout_row(ui, "alt", AXIS_Y, &format!("{:+.4} m", geo.altitude));
        readout_row(ui, "heading", &format!("{:6.2}°  {}", heading, compass_letter(heading)));
    });

    ui.add_space(space::SECTION);

    // ═══ Transform (read-only; edit in Properties) ═════════════════
    let q = pose.rotation;
    let (rx, ry, rz) = quat_to_euler_xyz(q.w, q.x, q.y, q.z);
    section(ui, "insp_tr", "Transform", accent, false, |ui| {
        subsection(ui, "insp_tr_position", "Position", None, accent, true, |ui| {
            axis_readout_row(ui, "X", AXIS_X, &format!("{:+.3} m", pose.point.x));
            axis_readout_row(ui, "Y", AXIS_Y, &format!("{:+.3} m", pose.point.y));
            axis_readout_row(ui, "Z", AXIS_Z, &format!("{:+.3} m", pose.point.z));
        });

        subsection(
            ui,
            "insp_tr_rotation",
            "Rotation",
            Some("Euler XYZ"),
            accent,
            true,
            |ui| {
                axis_readout_row(ui, "X", AXIS_X, &format!("{:+.2}°", rx.to_degrees()));
                axis_readout_row(ui, "Y", AXIS_Y, &format!("{:+.2}°", ry.to_degrees()));
                axis_readout_row(ui, "Z", AXIS_Z, &format!("{:+.2}°", rz.to_degrees()));
            },
        );

        subsection(
            ui,
            "insp_tr_scale",
            "Scale",
            Some("chassis size — baked at spawn"),
            accent,
            true,
            |ui| {
                axis_readout_row(ui, "X", AXIS_X, &format!("{:.3} m", size.x));
                axis_readout_row(ui, "Y", AXIS_Y, &format!("{:.3} m", size.y));
                axis_readout_row(ui, "Z", AXIS_Z, &format!("{:.3} m", size.z));
            },
        );
    });

    ui.add_space(space::SECTION);

    // ═══ Velocity ═══════════════════════════════════════════════════
    section(ui, "insp_vel", "Velocity", accent, false, |ui| {
        axis_readout_row(ui, "X", AXIS_X, &format!("{:+.2} m/s", linvel.vx as f32));
        axis_readout_row(ui, "Y", AXIS_Y, &format!("{:+.2} m/s", linvel.vy as f32));
        axis_readout_row(ui, "Z", AXIS_Z, &format!("{:+.2} m/s", linvel.vz as f32));
        readout_row(ui, "|v|", &format!("{:.2} m/s", speed));
    });

    ui.add_space(space::SECTION);

    // ═══ Control ════════════════════════════════════════════════════
    section(ui, "insp_ctl", "Control", accent, false, |ui| {
        bar_row(ui, "throttle", ctrl.throttle, -1.0, 1.0, accent);
        bar_row(ui, "steer",    ctrl.steer,    -1.0, 1.0, accent);
        bar_row(ui, "brake",    ctrl.brake,     0.0, 1.0, accent);
    });
}

// ─── World (nothing-selected) view ──────────────────────────────────

fn world_info(ui: &mut egui::Ui, sim: &mut GearboxSim, accent: egui::Color32) {
    let planet = sim.0.planet;
    let gravity = sim.0.gravity;
    let vehicle_count = sim.0.vehicles().count();

    section(ui, "world_summary", "World", accent, true, |ui| {
        ui.label(title_text("No selection"));
        ui.add_space(space::ROW);
        readout_row(ui, "vehicles", &vehicle_count.to_string());
        ui.add_space(space::BLOCK);
        ui.label(caption("Click a vehicle in the viewport, or pick one from Scene."));
    });

    ui.add_space(space::SECTION);

    section(ui, "world_planet", "Planet", accent, false, |ui| {
        readout_row(ui, "radius",        &format!("{:.0} m",  planet.radius));
        readout_row(
            ui,
            "circumference",
            &format!("{:.0} km", planet.radius * std::f64::consts::TAU / 1_000.0),
        );
        subsection(ui, "insp_world_datum", "Datum", None, accent, true, |ui| {
            axis_readout_row(ui, "lat", AXIS_Z, &format!("{:+.6}°",  planet.datum.latitude));
            axis_readout_row(ui, "lon", AXIS_X, &format!("{:+.6}°",  planet.datum.longitude));
            axis_readout_row(ui, "alt", AXIS_Y, &format!("{:+.2} m", planet.datum.altitude));
        });
    });

    ui.add_space(space::SECTION);

    section(ui, "world_physics", "Physics", accent, false, |ui| {
        axis_readout_row(ui, "gx", AXIS_X, &format!("{:+.2} m/s²", gravity.x));
        axis_readout_row(ui, "gy", AXIS_Y, &format!("{:+.2} m/s²", gravity.y));
        axis_readout_row(ui, "gz", AXIS_Z, &format!("{:+.2} m/s²", gravity.z));
    });
}

// ─── Helpers ────────────────────────────────────────────────────────

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

/// Signed-bar row: two-line progressbar module (label above, bar below)
/// filled proportional to `v` within `[min, max]`. Used for the Control
/// section (throttle/steer/brake readouts).
fn bar_row(ui: &mut egui::Ui, label: &str, v: f64, min: f64, max: f64, accent: egui::Color32) {
    let frac = ((v - min) / (max - min)).clamp(0.0, 1.0) as f32;
    let text = format!("{:+.2}", v);
    pretty_progressbar_text(ui, label, frac, &text, accent);
}

/// Cardinal/intercardinal letter for a heading in degrees.
fn compass_letter(h: f64) -> &'static str {
    let idx = ((h / 45.0).round() as i32).rem_euclid(8);
    ["N", "NE", "E", "SE", "S", "SW", "W", "NW"][idx as usize]
}

/// Quaternion → intrinsic XYZ Euler angles (radians).
fn quat_to_euler_xyz(w: f64, x: f64, y: f64, z: f64) -> (f64, f64, f64) {
    let sy = 2.0 * (w * y + x * z).clamp(-1.0, 1.0);
    let ey = sy.asin();
    let (ex, ez) = if sy.abs() > 0.9999 {
        (0.0, (-2.0 * (x * y - w * z)).atan2(1.0 - 2.0 * (y * y + z * z)))
    } else {
        (
            (-2.0 * (y * z - w * x)).atan2(1.0 - 2.0 * (x * x + y * y)),
            (-2.0 * (x * y - w * z)).atan2(1.0 - 2.0 * (y * y + z * z)),
        )
    };
    (ex, ey, ez)
}
