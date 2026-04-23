//! Properties panel (right dock).
//!
//! Editable counterpart to the read-only Inspector. Layout follows
//! the design-system primitives in `super::widgets`: every section
//! uses `section(...)`, every label+control pair uses
//! `labelled_row(...)`, spacing is driven by the `style::space`
//! tokens, never ad-hoc `add_space`.

use bevy::prelude::*;
use bevy_egui::egui;

use gearbox_physics::{
    datapod::{Point, Pose, Quaternion},
    VehicleId,
};

use gearbox_viz::gamepad::GamepadSelection;
use gearbox_viz::{ChassisTinted, GearboxSim, GroundGrid};

use super::selection::Selection;
use super::selection_ring::SelectionRingSettings;
use super::style::{
    contrast_text_for, font, space, AXIS_X, AXIS_Y, AXIS_Z, TEXT_PRIMARY,
};
use super::transform_gizmos::{GizmoModesEnabled, GizmoScale};
use super::ui_panel;
use super::widgets::{
    group_frame, labelled_row, pretty_slider, section, sub_caption, toggle, wide_button,
};

/// Bevy `Resource` the UI writes to request a live colour change on
/// a vehicle. The `apply_vehicle_color_changes` system consumes and
/// drains it each frame.
#[derive(Resource, Default, Debug, Clone)]
pub struct PendingColorChange {
    pub pending: Option<(VehicleId, [f32; 3])>,
}

pub fn draw_content(
    ui: &mut egui::Ui,
    sim: &mut GearboxSim,
    selection: &Selection,
    grid: &mut GroundGrid,
    gizmo_scale: &mut GizmoScale,
    gizmo_modes: &mut GizmoModesEnabled,
    ring_settings: &mut SelectionRingSettings,
    glass_opacity: &mut super::style::GlassOpacity,
    pending_color: &mut PendingColorChange,
    gamepad_selection: &mut GamepadSelection,
    accent: egui::Color32,
) {
    if let Some(id) = selection.vehicle {
        vehicle_section(ui, sim, id, pending_color, accent);
    } else {
        world_section(
            ui, sim, grid, gizmo_scale, gizmo_modes, ring_settings, glass_opacity,
            gamepad_selection, accent,
        );
    }
}

// ═══ World panel ════════════════════════════════════════════════════

fn world_section(
    ui: &mut egui::Ui,
    sim: &mut GearboxSim,
    grid: &mut GroundGrid,
    gizmo_scale: &mut GizmoScale,
    gizmo_modes: &mut GizmoModesEnabled,
    ring_settings: &mut SelectionRingSettings,
    glass_opacity: &mut super::style::GlassOpacity,
    gamepad_selection: &mut GamepadSelection,
    accent: egui::Color32,
) {
    section(ui, "world_sandbox", "Sandbox", accent, true, |ui| {
        labelled_row(ui, "unlimited power", |ui| {
            toggle(ui, &mut sim.0.unlimited_power, accent);
        });
        ui.add_space(space::TIGHT);
        sub_caption(ui, "Power drain is suspended on every vehicle.");
    });

    ui.add_space(space::SECTION);

    gamepad_section(ui, gamepad_selection, accent);

    ui.add_space(space::SECTION);

    // Grid / gizmo / ring still live in `ui_panel::draw_content`
    // (refactored itself when Inspector gets done).
    ui_panel::draw_content(
        ui, grid, gizmo_scale, gizmo_modes, ring_settings, glass_opacity, accent,
    );
}

fn gamepad_section(
    ui: &mut egui::Ui,
    selection: &mut GamepadSelection,
    accent: egui::Color32,
) {
    section(ui, "world_gamepad", "Gamepad", accent, true, |ui| {
        if let Some(err) = &selection.init_error {
            sub_caption(ui, "Gamepad backend failed to init");
            ui.label(
                egui::RichText::new(err)
                    .monospace()
                    .size(font::CAPTION)
                    .color(TEXT_PRIMARY),
            );
            ui.add_space(space::ROW);
            sub_caption(
                ui,
                "Linux fix: sudo usermod -aG input $USER  (log out + back in)",
            );
            return;
        }
        if selection.detected.is_empty() {
            sub_caption(ui, "No controllers detected.");
            return;
        }

        let active_id = selection
            .selected
            .filter(|id| selection.detected.iter().any(|gi| gi.id == *id))
            .or_else(|| selection.detected.first().map(|gi| gi.id));
        let active_label = match active_id {
            Some(id) => selection
                .detected
                .iter()
                .find(|gi| gi.id == id)
                .map(|gi| truncate_ellipsis(&gi.name, 18))
                .unwrap_or_else(|| "—".into()),
            None => "—".into(),
        };
        labelled_row(ui, "device", |ui| {
            egui::ComboBox::from_id_salt("gamepad_pick")
                .width(150.0)
                .selected_text(active_label)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut selection.selected, None, "(auto)");
                    for info in &selection.detected {
                        ui.selectable_value(&mut selection.selected, Some(info.id), &info.name);
                    }
                });
        });
    });
}

// ═══ Vehicle panel ══════════════════════════════════════════════════

fn vehicle_section(
    ui: &mut egui::Ui,
    sim: &mut GearboxSim,
    id: VehicleId,
    pending_color: &mut PendingColorChange,
    accent: egui::Color32,
) {
    // Snapshot every field the UI needs into owned locals so the
    // sim can be borrowed mutably inside closures below.
    let (
        name,
        current_color,
        current_mass,
        current_linear_damping,
        current_angular_damping,
        driven_wheels,
        mean_engine_force,
        mean_brake,
        wheels_count,
    ) = {
        let Some(state) = sim.0.vehicle(id) else {
            sub_caption(ui, "Selected vehicle no longer exists.");
            return;
        };
        let name = state.spec.name.clone();
        let driven: Vec<usize> = state
            .spec
            .wheels
            .iter()
            .enumerate()
            .filter(|(_, w)| w.driven)
            .map(|(i, _)| i)
            .collect();
        let engine = if driven.is_empty() {
            0.0
        } else {
            driven
                .iter()
                .map(|&i| state.spec.wheels[i].max_engine_force)
                .sum::<f64>()
                / driven.len() as f64
        };
        let brake = if state.spec.wheels.is_empty() {
            0.0
        } else {
            state.spec.wheels.iter().map(|w| w.max_brake).sum::<f64>()
                / state.spec.wheels.len() as f64
        };
        (
            name,
            state.spec.chassis.color,
            state.spec.chassis.mass,
            state.spec.chassis.linear_damping,
            state.spec.chassis.angular_damping,
            driven,
            engine,
            brake,
            state.spec.wheels.len(),
        )
    };

    section(
        ui,
        "prop_vehicle",
        &format!("Vehicle · {}", name),
        accent,
        true,
        |ui| {
            // --- Colour ---
            let mut rgb = current_color;
            labelled_row(ui, "colour", |ui| {
                if ui.color_edit_button_rgb(&mut rgb).changed() {
                    if let Some(v) = sim.0.vehicle_mut(id) {
                        v.spec.chassis.color = rgb;
                    }
                    pending_color.pending = Some((id, rgb));
                }
            });

            // --- Mass ---
            let mut mass = current_mass;
            labelled_row(ui, "mass (kg)", |ui| {
                let resp = ui.add(
                    egui::DragValue::new(&mut mass)
                        .speed(1.0)
                        .range(0.1..=100_000.0)
                        .fixed_decimals(1),
                );
                if resp.changed() {
                    sim.0.set_vehicle_mass(id, mass);
                }
            });

            // --- Engine force (mean across driven wheels) ---
            if !driven_wheels.is_empty() {
                let mut ef = mean_engine_force;
                labelled_row(ui, "max engine (N/wheel)", |ui| {
                    let resp = ui.add(
                        egui::DragValue::new(&mut ef)
                            .speed(1.0)
                            .range(0.0..=1_000_000.0)
                            .fixed_decimals(1),
                    );
                    if resp.changed() {
                        if let Some(v) = sim.0.vehicle_mut(id) {
                            for idx in &driven_wheels {
                                v.spec.wheels[*idx].max_engine_force = ef;
                            }
                        }
                    }
                });
            }

            // --- Brake (mean across all wheels) ---
            if wheels_count > 0 {
                let mut br = mean_brake;
                labelled_row(ui, "max brake (N·m/wheel)", |ui| {
                    let resp = ui.add(
                        egui::DragValue::new(&mut br)
                            .speed(1.0)
                            .range(0.0..=1_000_000.0)
                            .fixed_decimals(1),
                    );
                    if resp.changed() {
                        if let Some(v) = sim.0.vehicle_mut(id) {
                            for w in v.spec.wheels.iter_mut() {
                                w.max_brake = br;
                            }
                        }
                    }
                });
            }

            // --- Damping (linear / angular) ---
            let mut lin = current_linear_damping;
            let mut ang = current_angular_damping;
            labelled_row(ui, "damping (lin / ang)", |ui| {
                let r_ang = ui.add(
                    egui::DragValue::new(&mut ang)
                        .speed(0.05)
                        .range(0.0..=50.0)
                        .fixed_decimals(2),
                );
                let r_lin = ui.add(
                    egui::DragValue::new(&mut lin)
                        .speed(0.05)
                        .range(0.0..=50.0)
                        .fixed_decimals(2),
                );
                if r_lin.changed() || r_ang.changed() {
                    if let Some(v) = sim.0.vehicle_mut(id) {
                        v.spec.chassis.linear_damping = lin;
                        v.spec.chassis.angular_damping = ang;
                    }
                }
            });
        },
    );

    // Each helper decides whether it renders at all (Work / Power /
    // Container sections early-return when the selected vehicle has
    // nothing of that kind). The inter-section gap is their own
    // responsibility — they insert `space::SECTION` right before they
    // draw, so a section that doesn't render leaves no orphan gap
    // above it. Transform is always rendered and emits its own gap.
    work_section(ui, sim, id, accent);
    power_section(ui, sim, id, accent);
    container_section(ui, sim, id, accent);
    transform_section(ui, sim, id, accent);
}

fn work_section(ui: &mut egui::Ui, sim: &mut GearboxSim, id: VehicleId, accent: egui::Color32) {
    let Some(state) = sim.0.vehicle(id) else { return };
    if state.spec.power.sources.is_empty() {
        return;
    }
    let mut work = state.spec.power.work;
    let mut resistance = state.spec.power.work_resistance;

    ui.add_space(space::SECTION);
    section(ui, "prop_work", "Work", accent, false, |ui| {
        labelled_row(ui, "work", |ui| {
            if toggle(ui, &mut work, accent).changed() {
                if let Some(v) = sim.0.vehicle_mut(id) {
                    v.spec.power.work = work;
                }
            }
        });
        labelled_row(ui, "resistance", |ui| {
            if pretty_slider(ui, &mut resistance, 0.0..=1.0, 2, "", accent).changed() {
                if let Some(v) = sim.0.vehicle_mut(id) {
                    v.spec.power.work_resistance = resistance;
                }
            }
        });
    });
}

fn power_section(ui: &mut egui::Ui, sim: &mut GearboxSim, id: VehicleId, accent: egui::Color32) {
    struct Snap {
        turned_on: bool,
        primary: usize,
        entries: Vec<(String, f64, f64)>,
    }
    let snap: Snap = {
        let Some(state) = sim.0.vehicle(id) else { return };
        if state.spec.power.sources.is_empty() {
            return;
        }
        Snap {
            turned_on: state.spec.power.turned_on,
            primary: state.spec.power.primary,
            entries: state
                .spec
                .power
                .sources
                .iter()
                .map(|s| (s.label.clone(), s.capacity, s.current))
                .collect(),
        }
    };

    ui.add_space(space::SECTION);
    section(ui, "prop_power", "Power", accent, false, |ui| {
        // TURN ON
        let mut turned_on = snap.turned_on;
        labelled_row(ui, "turn on", |ui| {
            if toggle(ui, &mut turned_on, accent).changed() {
                if let Some(v) = sim.0.vehicle_mut(id) {
                    v.spec.power.turned_on = turned_on;
                }
            }
        });

        // Primary selector (only 2+ sources) — grouped in a subtle
        // frame so it reads as one decision, not three loose radios.
        if snap.entries.len() >= 2 {
            ui.add_space(space::BLOCK);
            sub_caption(ui, "primary (drains first)");
            ui.add_space(space::TIGHT);
            group_frame(ui, accent, |ui| {
                let mut primary = snap.primary.min(snap.entries.len() - 1);
                for (idx, (label, _, _)) in snap.entries.iter().enumerate() {
                    ui.radio_value(&mut primary, idx, label);
                }
                if primary != snap.primary {
                    if let Some(v) = sim.0.vehicle_mut(id) {
                        v.spec.power.primary = primary;
                    }
                }
            });
        }

        // Per-source capacity sliders. Each source gets a small header
        // (name + current / capacity readout) followed by its
        // "capacity" slider in a labelled row — so the slider's
        // purpose is always visible without pushing the card wider.
        for (idx, (label, capacity_snapshot, current_snapshot)) in snap.entries.iter().enumerate() {
            ui.add_space(space::BLOCK);
            sub_caption(
                ui,
                &format!("{} · {:.0} / {:.0}", label, current_snapshot, capacity_snapshot),
            );
            ui.add_space(space::TIGHT);
            let mut capacity = *capacity_snapshot;
            labelled_row(ui, "capacity", |ui| {
                if pretty_slider(ui, &mut capacity, 10.0..=5000.0, 0, "", accent).changed() {
                    if let Some(v) = sim.0.vehicle_mut(id) {
                        if let Some(src) = v.spec.power.sources.get_mut(idx) {
                            src.capacity = capacity;
                            if src.current > src.capacity {
                                src.current = src.capacity;
                            }
                        }
                    }
                }
            });
        }

        ui.add_space(space::BLOCK);
        if wide_button(ui, "Refuel / Repower", accent).clicked() {
            if let Some(v) = sim.0.vehicle_mut(id) {
                v.spec.power.refuel();
            }
        }
    });
}

#[derive(Clone)]
struct ContainerSnap {
    amount: f64,
    capacity: f64,
    fill_rate_frac: f64,
}

fn container_section(
    ui: &mut egui::Ui,
    sim: &mut GearboxSim,
    id: VehicleId,
    accent: egui::Color32,
) {
    let snaps: Vec<ContainerSnap> = {
        let Some(state) = sim.0.vehicle(id) else { return };
        if state.spec.containers.is_empty() {
            return;
        }
        state
            .spec
            .containers
            .iter()
            .map(|c| ContainerSnap {
                amount: c.amount,
                capacity: c.capacity,
                fill_rate_frac: c.fill_rate_frac,
            })
            .collect()
    };

    ui.add_space(space::SECTION);
    section(ui, "prop_container", "Container", accent, false, |ui| {
        for (idx, s) in snaps.iter().enumerate() {
            if idx > 0 {
                ui.add_space(space::BLOCK);
            }
            // FILL bar
            let frac = if s.capacity > 0.0 {
                (s.amount / s.capacity).clamp(0.0, 1.0)
            } else {
                0.0
            };
            ui.add(
                egui::ProgressBar::new(frac as f32)
                    .text(
                        egui::RichText::new(format!("{:.0} / {:.0}", s.amount, s.capacity))
                            .monospace()
                            .size(font::NUMERIC)
                            .color(contrast_text_for(accent)),
                    )
                    .fill(accent)
                    .corner_radius(egui::CornerRadius::same(super::style::radius::SM)),
            );

            // CAPACITY slider — labelled so its purpose is clear
            // without eating width with an inline `.text(...)`.
            ui.add_space(space::TIGHT);
            let mut capacity = s.capacity;
            labelled_row(ui, "capacity", |ui| {
                if pretty_slider(ui, &mut capacity, 1.0..=5000.0, 0, "", accent).changed() {
                    if let Some(v) = sim.0.vehicle_mut(id) {
                        if let Some(c) = v.spec.containers.get_mut(idx) {
                            c.set_capacity(capacity);
                        }
                    }
                }
            });

            // +/- / empty
            ui.add_space(space::TIGHT);
            let step = (s.capacity * 0.05).max(1.0);
            ui.horizontal(|ui| {
                if ui.button(format!("− {:.0}", step)).clicked() {
                    if let Some(v) = sim.0.vehicle_mut(id) {
                        if let Some(c) = v.spec.containers.get_mut(idx) {
                            c.bump(-1.0);
                        }
                    }
                }
                if ui.button(format!("+ {:.0}", step)).clicked() {
                    if let Some(v) = sim.0.vehicle_mut(id) {
                        if let Some(c) = v.spec.containers.get_mut(idx) {
                            c.bump(1.0);
                        }
                    }
                }
                if ui.button("empty").clicked() {
                    if let Some(v) = sim.0.vehicle_mut(id) {
                        if let Some(c) = v.spec.containers.get_mut(idx) {
                            c.empty_out();
                        }
                    }
                }
            });

            // RATE slider — auto-fill rate as % of capacity per sec.
            ui.add_space(space::TIGHT);
            let mut rate_pct = s.fill_rate_frac * 100.0;
            labelled_row(ui, "rate (%/s)", |ui| {
                if pretty_slider(ui, &mut rate_pct, 0.0..=5.0, 1, "", accent).changed() {
                    if let Some(v) = sim.0.vehicle_mut(id) {
                        if let Some(c) = v.spec.containers.get_mut(idx) {
                            c.fill_rate_frac = (rate_pct / 100.0).clamp(0.0, 0.05);
                        }
                    }
                }
            });
        }
    });
}

fn transform_section(
    ui: &mut egui::Ui,
    sim: &mut GearboxSim,
    id: VehicleId,
    accent: egui::Color32,
) {
    ui.add_space(space::SECTION);
    section(ui, "prop_transform", "Transform", accent, false, |ui| {
        let pose = sim.0.vehicle_pose(id);
        let mut px = pose.point.x;
        let mut py = pose.point.y;
        let mut pz = pose.point.z;
        let q = pose.rotation;
        let (mut rx, mut ry, mut rz) = {
            let (x, y, z) = quat_to_euler_xyz(q.w, q.x, q.y, q.z);
            (x.to_degrees(), y.to_degrees(), z.to_degrees())
        };
        let mut changed = false;

        sub_caption(ui, "position  (drag, double-click to type)");
        ui.add_space(space::TIGHT);
        changed |= axis_drag_row(ui, "X", AXIS_X, &mut px, 0.05, " m");
        changed |= axis_drag_row(ui, "Y", AXIS_Y, &mut py, 0.05, " m");
        changed |= axis_drag_row(ui, "Z", AXIS_Z, &mut pz, 0.05, " m");

        ui.add_space(space::BLOCK);
        sub_caption(ui, "rotation  (Euler XYZ, degrees)");
        ui.add_space(space::TIGHT);
        changed |= axis_drag_row(ui, "X", AXIS_X, &mut rx, 1.0, "°");
        changed |= axis_drag_row(ui, "Y", AXIS_Y, &mut ry, 1.0, "°");
        changed |= axis_drag_row(ui, "Z", AXIS_Z, &mut rz, 1.0, "°");

        if changed {
            let nq = euler_xyz_to_quat(rx.to_radians(), ry.to_radians(), rz.to_radians());
            sim.0.set_vehicle_pose(
                id,
                Pose {
                    point: Point::new(px as f64, py as f64, pz as f64),
                    rotation: Quaternion::new(nq.0 as f64, nq.1 as f64, nq.2 as f64, nq.3 as f64),
                },
            );
        }
    });
}

// ═══ Helpers ════════════════════════════════════════════════════════

/// Shorten a string to at most `max_chars` Unicode scalar values,
/// appending an ellipsis when truncated. Used so a 40-character
/// gamepad name doesn't overflow the combo's selected-text cell.
fn truncate_ellipsis(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    let kept: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{}…", kept)
}

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

fn euler_xyz_to_quat(ex: f64, ey: f64, ez: f64) -> (f64, f64, f64, f64) {
    let (sx, cx) = ((ex * 0.5).sin(), (ex * 0.5).cos());
    let (sy, cy) = ((ey * 0.5).sin(), (ey * 0.5).cos());
    let (sz, cz) = ((ez * 0.5).sin(), (ez * 0.5).cos());
    let w = cx * cy * cz - sx * sy * sz;
    let x = sx * cy * cz + cx * sy * sz;
    let y = cx * sy * cz - sx * cy * sz;
    let z = cx * cy * sz + sx * sy * cz;
    (w, x, y, z)
}

fn axis_drag_row(
    ui: &mut egui::Ui,
    glyph: &str,
    color: egui::Color32,
    value: &mut f64,
    speed: f64,
    suffix: &str,
) -> bool {
    // Use the same labelled-row skeleton as every other Properties row:
    // a fixed-width label cell on the left (so the column of controls
    // stays aligned) with the coloured axis glyph inside it, and the
    // DragValue in the right cell — which `labelled_row` explicitly
    // right-aligns against the card edge.
    let mut changed = false;
    super::widgets::labelled_row_custom_left(
        ui,
        |ui| {
            ui.label(
                egui::RichText::new(glyph)
                    .strong()
                    .monospace()
                    .size(font::NUMERIC)
                    .color(color),
            );
        },
        |ui| {
            let resp = ui.add(
                egui::DragValue::new(value)
                    .speed(speed)
                    .suffix(suffix)
                    .fixed_decimals(3),
            );
            if resp.changed() {
                changed = true;
            }
        },
    );
    changed
}

/// Bevy system that applies a queued colour change to EVERY tinted
/// piece of a vehicle — chassis mesh plus every part whose declared
/// colour matched the chassis colour at spawn.
pub fn apply_vehicle_color_changes(
    mut pending: ResMut<PendingColorChange>,
    tinted: Query<(&ChassisTinted, &MeshMaterial3d<StandardMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Some((target_id, [r, g, b])) = pending.pending else {
        return;
    };
    let new_color = Color::srgb(r, g, b);
    for (tag, material) in &tinted {
        if tag.id != target_id {
            continue;
        }
        if let Some(mat) = materials.get_mut(&material.0) {
            mat.base_color = new_color;
        }
    }
    pending.pending = None;
}
