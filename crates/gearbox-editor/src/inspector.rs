//! Inspector panel (right dock) — strictly **read-only**.
//!
//! Displays vehicle + world state. Any setting the user can edit
//! lives in the [`super::properties`] panel. Layout is built on the
//! widget primitives in `super::widgets` so every row / bar / section
//! shares the same spacing + typography.

use bevy::prelude::*;
use bevy_egui::egui;
use bevy_frost::PaneBuilder;

use gearbox_physics::VehicleSpec;

use gearbox_viz::GearboxSim;

use super::selection::Selection;
use super::style::{AXIS_X, AXIS_Y, AXIS_Z, TEXT_SECONDARY, caption, font, space, title_text};
use super::widgets::{
    axis_readout_row, pretty_progressbar_text, readout_row, sub_caption, subsection,
};

/// What the inspector needs to show for a `Load USD…`-spawned asset.
/// Built in `right_dock_ui` from the selected entity's components and
/// passed in so the inspector stays a pure rendering function.
pub struct UsdInspect {
    pub name: String,
    /// World-space pose. For top-level `SceneRoot`s (no parent) this
    /// is the same as the entity's local `Transform`; for nested
    /// loads it'd come from `GlobalTransform.compute_transform()`.
    pub world_translation: Vec3,
    pub world_rotation: Quat,
    pub world_scale: Vec3,
    /// Lat / lon / alt under the editor's planet datum, computed
    /// from `world_translation`.
    pub geo_latitude: f64,
    pub geo_longitude: f64,
    pub geo_altitude: f64,
}

pub fn draw_content(
    pane: &mut PaneBuilder,
    sim: &mut GearboxSim,
    selection: &Selection,
    usd_inspect: Option<UsdInspect>,
    accent: egui::Color32,
) {
    if let Some(info) = usd_inspect {
        usd_info(pane, &info, accent);
        return;
    }
    let Some(id) = selection.vehicle else {
        world_info(pane, sim, accent);
        return;
    };
    let Some(_state) = sim.0.vehicle(id) else {
        pane.section("insp_missing", "Selection", true, |ui| {
            sub_caption(ui, "Selected vehicle no longer exists.");
        });
        return;
    };

    let pose = sim.0.vehicle_pose(id);
    let linvel = sim.0.vehicle_linvel(id);
    let ctrl = sim.0.control(id);
    let speed = (linvel.vx * linvel.vx + linvel.vy * linvel.vy + linvel.vz * linvel.vz).sqrt();
    let state = sim.0.vehicle(id).unwrap();
    let size = state.spec.chassis.size;
    let (fp_x, fp_z) = top_down_footprint(&state.spec);

    // ═══ Info ═══════════════════════════════════════════════════════
    pane.section("insp_info", "Info", true, |ui| {
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

        readout_row(ui, "mass", &format!("{:.0} kg", state.spec.chassis.mass));
        readout_row(ui, "wheels", &state.spec.wheels.len().to_string());
        readout_row(ui, "parts", &state.spec.parts.len().to_string());
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
                pretty_progressbar_text(ui, &src.label, src.fraction() as f32, &text, bar_col);
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
                pretty_progressbar_text(ui, &name, container.fraction() as f32, &text, accent);
            }
        }
    });

    // ═══ Geo ════════════════════════════════════════════════════════
    let geo = sim.0.vehicle_geo(id);
    let heading = sim.0.vehicle_heading(id);
    pane.section("insp_geo", "Geo", false, |ui| {
        axis_readout_row(ui, "lat", AXIS_Z, &format!("{:+.10}°", geo.latitude));
        axis_readout_row(ui, "lon", AXIS_X, &format!("{:+.10}°", geo.longitude));
        axis_readout_row(ui, "alt", AXIS_Y, &format!("{:+.4} m", geo.altitude));
        readout_row(
            ui,
            "heading",
            &format!("{:6.2}°  {}", heading, compass_letter(heading)),
        );
    });

    // ═══ Transform (read-only; edit in Properties) ═════════════════
    let q = pose.rotation;
    let (rx, ry, rz) = quat_to_euler_xyz(q.w, q.x, q.y, q.z);
    pane.section("insp_tr", "Transform", false, |ui| {
        subsection(
            ui,
            "insp_tr_position",
            "Position",
            None,
            accent,
            true,
            |ui| {
                axis_readout_row(ui, "X", AXIS_X, &format!("{:+.3} m", pose.point.x));
                axis_readout_row(ui, "Y", AXIS_Y, &format!("{:+.3} m", pose.point.y));
                axis_readout_row(ui, "Z", AXIS_Z, &format!("{:+.3} m", pose.point.z));
            },
        );

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

    // ═══ Velocity ═══════════════════════════════════════════════════
    pane.section("insp_vel", "Velocity", false, |ui| {
        axis_readout_row(ui, "X", AXIS_X, &format!("{:+.2} m/s", linvel.vx as f32));
        axis_readout_row(ui, "Y", AXIS_Y, &format!("{:+.2} m/s", linvel.vy as f32));
        axis_readout_row(ui, "Z", AXIS_Z, &format!("{:+.2} m/s", linvel.vz as f32));
        readout_row(ui, "|v|", &format!("{:.2} m/s", speed));
    });

    // ═══ Control ════════════════════════════════════════════════════
    pane.section("insp_ctl", "Control", false, |ui| {
        bar_row(ui, "throttle", ctrl.throttle, -1.0, 1.0, accent);
        bar_row(ui, "steer", ctrl.steer, -1.0, 1.0, accent);
        bar_row(ui, "brake", ctrl.brake, 0.0, 1.0, accent);
    });
}

// ─── USD (Load-USD-spawned asset) view ─────────────────────────────

fn usd_info(pane: &mut PaneBuilder, info: &UsdInspect, accent: egui::Color32) {
    let q = info.world_rotation;
    let (rx, ry, rz) = quat_to_euler_xyz(q.w as f64, q.x as f64, q.y as f64, q.z as f64);

    pane.section("usd_info", "USD Asset", true, |ui| {
        ui.label(title_text(&info.name));
        ui.add_space(space::ROW);
        sub_caption(ui, "Loaded via 📂 Load USD…");
    });

    pane.section("usd_geo", "Geo", true, |ui| {
        axis_readout_row(ui, "lat", AXIS_Z, &format!("{:+.10}°", info.geo_latitude));
        axis_readout_row(ui, "lon", AXIS_X, &format!("{:+.10}°", info.geo_longitude));
        axis_readout_row(ui, "alt", AXIS_Y, &format!("{:+.4} m", info.geo_altitude));
    });

    pane.section("usd_tr", "Transform (world)", true, |ui| {
        subsection(ui, "usd_tr_pos", "Position", None, accent, true, |ui| {
            axis_readout_row(
                ui,
                "X",
                AXIS_X,
                &format!("{:+.3} m", info.world_translation.x),
            );
            axis_readout_row(
                ui,
                "Y",
                AXIS_Y,
                &format!("{:+.3} m", info.world_translation.y),
            );
            axis_readout_row(
                ui,
                "Z",
                AXIS_Z,
                &format!("{:+.3} m", info.world_translation.z),
            );
        });
        subsection(
            ui,
            "usd_tr_rot",
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
        subsection(ui, "usd_tr_scale", "Scale", None, accent, true, |ui| {
            axis_readout_row(ui, "X", AXIS_X, &format!("{:.3}", info.world_scale.x));
            axis_readout_row(ui, "Y", AXIS_Y, &format!("{:.3}", info.world_scale.y));
            axis_readout_row(ui, "Z", AXIS_Z, &format!("{:.3}", info.world_scale.z));
        });
    });
}

// ─── World (nothing-selected) view ──────────────────────────────────

fn world_info(pane: &mut PaneBuilder, sim: &mut GearboxSim, accent: egui::Color32) {
    let planet = sim.0.planet;
    let gravity = sim.0.gravity;
    let vehicle_count = sim.0.vehicles().count();

    pane.section("world_summary", "World", true, |ui| {
        ui.label(title_text("No selection"));
        ui.add_space(space::ROW);
        readout_row(ui, "vehicles", &vehicle_count.to_string());
        ui.add_space(space::BLOCK);
        ui.label(caption(
            "Click a vehicle in the viewport, or pick one from Scene.",
        ));
    });

    pane.section("world_planet", "Planet", false, |ui| {
        readout_row(ui, "radius", &format!("{:.0} m", planet.radius));
        readout_row(
            ui,
            "circumference",
            &format!("{:.0} km", planet.radius * std::f64::consts::TAU / 1_000.0),
        );
        subsection(ui, "insp_world_datum", "Datum", None, accent, true, |ui| {
            axis_readout_row(
                ui,
                "lat",
                AXIS_Z,
                &format!("{:+.6}°", planet.datum.latitude),
            );
            axis_readout_row(
                ui,
                "lon",
                AXIS_X,
                &format!("{:+.6}°", planet.datum.longitude),
            );
            axis_readout_row(
                ui,
                "alt",
                AXIS_Y,
                &format!("{:+.2} m", planet.datum.altitude),
            );
        });
    });

    pane.section("world_physics", "Physics", false, |ui| {
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
        (
            0.0,
            (-2.0 * (x * y - w * z)).atan2(1.0 - 2.0 * (y * y + z * z)),
        )
    } else {
        (
            (-2.0 * (y * z - w * x)).atan2(1.0 - 2.0 * (x * x + y * y)),
            (-2.0 * (x * y - w * z)).atan2(1.0 - 2.0 * (y * y + z * z)),
        )
    };
    (ex, ey, ez)
}
