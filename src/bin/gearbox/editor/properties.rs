//! Properties panel (right dock).
//!
//! Editable counterpart to the read-only Inspector. Exposes:
//!   - Selected vehicle: colour, mass, per-wheel engine force / brake,
//!     damping, transform (position + rotation).
//!   - Editor/world tools: grid density, gizmo scale, selection-ring
//!     thickness (previously lived in the standalone "UI" panel).
//!
//! When nothing is selected, only the editor/world section shows.

use bevy::prelude::*;
use bevy_egui::egui;

use gearbox::{
    datapod::{Point, Pose, Quaternion},
    VehicleId,
};

use crate::viz::gamepad::GamepadSelection;
use crate::viz::{ChassisTinted, GearboxSim, GroundGrid};

use super::selection::Selection;
use super::selection_ring::SelectionRingSettings;
use super::style::{
    contrast_text_for, fg_dim, section_caps, AXIS_X, AXIS_Y, AXIS_Z, TEXT_PRIMARY,
};
use super::transform_gizmos::GizmoScale;
use super::ui_panel;

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
    ring_settings: &mut SelectionRingSettings,
    pending_color: &mut PendingColorChange,
    gamepad_selection: &mut GamepadSelection,
    accent: egui::Color32,
) {
    // Per-object properties vs world properties are mutually
    // exclusive: a selected vehicle gets its own panel; empty
    // selection falls back to the editor/world tools.
    if let Some(id) = selection.vehicle {
        vehicle_section(ui, sim, id, pending_color, accent);
    } else {
        world_section(
            ui,
            sim,
            grid,
            gizmo_scale,
            ring_settings,
            gamepad_selection,
            accent,
        );
    }
}

fn world_section(
    ui: &mut egui::Ui,
    sim: &mut GearboxSim,
    grid: &mut GroundGrid,
    gizmo_scale: &mut GizmoScale,
    ring_settings: &mut SelectionRingSettings,
    gamepad_selection: &mut GamepadSelection,
    accent: egui::Color32,
) {
    // Sandbox-mode toggles. Kept above the grid/gizmo/ring block so
    // they're the first thing you see in the world panel.
    egui::CollapsingHeader::new(section_caps("Sandbox", accent))
        .id_salt("world_sandbox")
        .default_open(true)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("unlimited")
                        .small()
                        .strong()
                        .color(TEXT_PRIMARY),
                );
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        ui.checkbox(&mut sim.0.unlimited_power, "");
                    },
                );
            });
            ui.label(
                egui::RichText::new("Power drain is suspended on every vehicle.")
                    .small()
                    .italics()
                    .color(fg_dim()),
            );
        });
    ui.add_space(4.0);

    // Gamepad chooser — refreshed every frame so hot-plugging
    // shows up without restarting.
    gamepad_section(ui, gamepad_selection, accent);
    ui.add_space(4.0);

    editor_section(ui, grid, gizmo_scale, ring_settings, accent);
}

fn gamepad_section(
    ui: &mut egui::Ui,
    selection: &mut GamepadSelection,
    accent: egui::Color32,
) {
    egui::CollapsingHeader::new(section_caps("Gamepad", accent))
        .id_salt("world_gamepad")
        .default_open(true)
        .show(ui, |ui| {
            if let Some(err) = &selection.init_error {
                ui.label(
                    egui::RichText::new("Gamepad backend failed to init")
                        .small()
                        .strong()
                        .color(fg_dim()),
                );
                ui.label(
                    egui::RichText::new(err).small().monospace().color(fg_dim()),
                );
                ui.label(
                    egui::RichText::new(
                        "Linux fix: sudo usermod -aG input $USER  (log out + back in)",
                    )
                    .small()
                    .italics()
                    .color(fg_dim()),
                );
                return;
            }
            if selection.detected.is_empty() {
                ui.label(
                    egui::RichText::new("No controllers detected.")
                        .small()
                        .italics()
                        .color(fg_dim()),
                );
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
                    .map(|gi| gi.name.clone())
                    .unwrap_or_else(|| "—".into()),
                None => "—".into(),
            };
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("device").small().color(TEXT_PRIMARY));
                egui::ComboBox::from_id_salt("gamepad_pick")
                    .selected_text(active_label)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut selection.selected, None, "(auto)");
                        for info in &selection.detected {
                            ui.selectable_value(
                                &mut selection.selected,
                                Some(info.id),
                                &info.name,
                            );
                        }
                    });
            });
        });
}

fn vehicle_section(
    ui: &mut egui::Ui,
    sim: &mut GearboxSim,
    id: VehicleId,
    pending_color: &mut PendingColorChange,
    accent: egui::Color32,
) {
    // Snapshot every field the UI needs into owned locals, then drop
    // the `&VehicleState` borrow so the closure below can take `sim`
    // mutably (for `set_vehicle_mass`, colour change, etc.).
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
            ui.label(egui::RichText::new("Selected vehicle no longer exists.").color(fg_dim()));
            return;
        };
        let name = state.spec.name.clone();
        let driven_wheels: Vec<usize> = state
            .spec
            .wheels
            .iter()
            .enumerate()
            .filter(|(_, w)| w.driven)
            .map(|(i, _)| i)
            .collect();
        let mean_engine_force = if driven_wheels.is_empty() {
            0.0
        } else {
            driven_wheels
                .iter()
                .map(|&i| state.spec.wheels[i].max_engine_force)
                .sum::<f32>()
                / driven_wheels.len() as f32
        };
        let mean_brake = if state.spec.wheels.is_empty() {
            0.0
        } else {
            state.spec.wheels.iter().map(|w| w.max_brake).sum::<f32>()
                / state.spec.wheels.len() as f32
        };
        (
            name,
            state.spec.chassis.color,
            state.spec.chassis.mass,
            state.spec.chassis.linear_damping,
            state.spec.chassis.angular_damping,
            driven_wheels,
            mean_engine_force,
            mean_brake,
            state.spec.wheels.len(),
        )
    };

    egui::CollapsingHeader::new(section_caps(&format!("Vehicle · {}", name), accent))
        .id_salt("prop_vehicle")
        .default_open(true)
        .show(ui, |ui| {
            // --- Colour ---
            let mut rgb: [f32; 3] = current_color;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("colour").small().color(TEXT_PRIMARY));
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if ui.color_edit_button_rgb(&mut rgb).changed() {
                            if let Some(v) = sim.0.vehicle_mut(id) {
                                v.spec.chassis.color = rgb;
                            }
                            pending_color.pending = Some((id, rgb));
                        }
                    },
                );
            });

            ui.add_space(2.0);

            // --- Mass ---
            let mut mass = current_mass;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("mass (kg)").small().color(TEXT_PRIMARY));
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        let resp = ui.add(
                            egui::DragValue::new(&mut mass)
                                .speed(1.0)
                                .range(0.1..=100_000.0)
                                .fixed_decimals(1),
                        );
                        if resp.changed() {
                            sim.0.set_vehicle_mass(id, mass);
                        }
                    },
                );
            });

            // --- Engine force (mean across driven wheels) ---
            if !driven_wheels.is_empty() {
                let mut ef = mean_engine_force;
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("max engine (N/wheel)")
                            .small()
                            .color(TEXT_PRIMARY),
                    );
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
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
                        },
                    );
                });
            }

            // --- Brake (mean across all wheels) ---
            if wheels_count > 0 {
                let mut br = mean_brake;
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("max brake (N·m/wheel)")
                            .small()
                            .color(TEXT_PRIMARY),
                    );
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
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
                        },
                    );
                });
            }

            ui.add_space(2.0);

            // --- Damping ---
            let mut lin = current_linear_damping;
            let mut ang = current_angular_damping;
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("damping (lin / ang)")
                        .small()
                        .color(TEXT_PRIMARY),
                );
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
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
                    },
                );
            });
        });

    // --- Work ---
    work_section(ui, sim, id, accent);

    // --- Power ---
    power_section(ui, sim, id, accent);

    // --- Container ---
    container_section(ui, sim, id, accent);

    // --- Transform (moved from Inspector — it's the editable side) ---
    egui::CollapsingHeader::new(section_caps("Transform", accent))
        .id_salt("prop_transform")
        .default_open(false)
        .show(ui, |ui| {
            let pose = sim.0.vehicle_pose(id);
            let mut px = pose.point.x as f32;
            let mut py = pose.point.y as f32;
            let mut pz = pose.point.z as f32;
            let q = pose.rotation;
            let (mut rx, mut ry, mut rz) = {
                let (x, y, z) =
                    quat_to_euler_xyz(q.w as f32, q.x as f32, q.y as f32, q.z as f32);
                (x.to_degrees(), y.to_degrees(), z.to_degrees())
            };
            let mut changed = false;

            ui.label(
                egui::RichText::new("position  (drag to move, double-click to type)")
                    .small()
                    .italics()
                    .color(fg_dim()),
            );
            changed |= axis_drag_row(ui, "X", AXIS_X, &mut px, 0.05, " m");
            changed |= axis_drag_row(ui, "Y", AXIS_Y, &mut py, 0.05, " m");
            changed |= axis_drag_row(ui, "Z", AXIS_Z, &mut pz, 0.05, " m");

            ui.add_space(2.0);
            ui.label(
                egui::RichText::new("rotation  (Euler XYZ, degrees)")
                    .small()
                    .italics()
                    .color(fg_dim()),
            );
            changed |= axis_drag_row(ui, "X", AXIS_X, &mut rx, 1.0, "°");
            changed |= axis_drag_row(ui, "Y", AXIS_Y, &mut ry, 1.0, "°");
            changed |= axis_drag_row(ui, "Z", AXIS_Z, &mut rz, 1.0, "°");

            if changed {
                let nq = euler_xyz_to_quat(rx.to_radians(), ry.to_radians(), rz.to_radians());
                sim.0.set_vehicle_pose(
                    id,
                    Pose {
                        point: Point::new(px as f64, py as f64, pz as f64),
                        rotation: Quaternion::new(
                            nq.0 as f64,
                            nq.1 as f64,
                            nq.2 as f64,
                            nq.3 as f64,
                        ),
                    },
                );
            }
        });
}

fn editor_section(
    ui: &mut egui::Ui,
    grid: &mut GroundGrid,
    gizmo_scale: &mut GizmoScale,
    ring_settings: &mut SelectionRingSettings,
    accent: egui::Color32,
) {
    // Reuse the existing grid / gizmo / ring section layout — these
    // are editor-wide tools, not per-vehicle, and their UI already
    // exists in `ui_panel`.
    ui_panel::draw_content(ui, grid, gizmo_scale, ring_settings, accent);
}

fn work_section(
    ui: &mut egui::Ui,
    sim: &mut GearboxSim,
    id: VehicleId,
    accent: egui::Color32,
) {
    // Snapshot current values without holding a borrow into sim.
    let Some(state) = sim.0.vehicle(id) else { return };
    if state.spec.power.sources.is_empty() {
        return; // nothing to drive — hide the section
    }
    let mut work = state.spec.power.work;
    let mut resistance = state.spec.power.work_resistance;

    egui::CollapsingHeader::new(section_caps("Work", accent))
        .id_salt("prop_work")
        .default_open(false)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("work").small().color(TEXT_PRIMARY));
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if ui.checkbox(&mut work, "").changed() {
                            if let Some(v) = sim.0.vehicle_mut(id) {
                                v.spec.power.work = work;
                            }
                        }
                    },
                );
            });
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("resistance")
                        .small()
                        .color(TEXT_PRIMARY),
                );
                if ui
                    .add(
                        egui::Slider::new(&mut resistance, 0.0..=1.0)
                            .show_value(true)
                            .fixed_decimals(2),
                    )
                    .changed()
                {
                    if let Some(v) = sim.0.vehicle_mut(id) {
                        v.spec.power.work_resistance = resistance;
                    }
                }
            });
        });
}

fn power_section(
    ui: &mut egui::Ui,
    sim: &mut GearboxSim,
    id: VehicleId,
    accent: egui::Color32,
) {
    // Snapshot everything the UI needs so we don't keep an immutable
    // borrow into sim while the closure takes mut access below.
    struct Snap {
        turned_on: bool,
        primary: usize,
        entries: Vec<(String, f32, f32)>,
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

    egui::CollapsingHeader::new(section_caps("Power", accent))
        .id_salt("prop_power")
        .default_open(false)
        .show(ui, |ui| {
            // --- TURN ON switch ---
            let mut turned_on = snap.turned_on;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("TURN ON").small().strong().color(TEXT_PRIMARY));
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if ui.checkbox(&mut turned_on, "").changed() {
                            if let Some(v) = sim.0.vehicle_mut(id) {
                                v.spec.power.turned_on = turned_on;
                            }
                        }
                    },
                );
            });
            ui.add_space(2.0);

            // --- Primary source radio (only if there are 2+) ---
            if snap.entries.len() >= 2 {
                ui.label(
                    egui::RichText::new("primary (drains first)")
                        .small()
                        .color(fg_dim()),
                );
                let mut primary = snap.primary.min(snap.entries.len() - 1);
                for (idx, (label, _, _)) in snap.entries.iter().enumerate() {
                    ui.radio_value(&mut primary, idx, label);
                }
                if primary != snap.primary {
                    if let Some(v) = sim.0.vehicle_mut(id) {
                        v.spec.power.primary = primary;
                    }
                }
                ui.add_space(4.0);
            }

            // --- Per-source capacity sliders ---
            for (idx, (label, capacity_snapshot, current_snapshot)) in
                snap.entries.iter().enumerate()
            {
                let mut capacity = *capacity_snapshot;
                ui.label(
                    egui::RichText::new(format!(
                        "{} · {:.0} / {:.0}",
                        label, current_snapshot, capacity_snapshot
                    ))
                    .small()
                    .color(TEXT_PRIMARY),
                );
                if ui
                    .add(
                        egui::Slider::new(&mut capacity, 10.0..=5000.0)
                            .show_value(true)
                            .fixed_decimals(0)
                            .text("capacity"),
                    )
                    .changed()
                {
                    if let Some(v) = sim.0.vehicle_mut(id) {
                        if let Some(src) = v.spec.power.sources.get_mut(idx) {
                            src.capacity = capacity;
                            if src.current > src.capacity {
                                src.current = src.capacity;
                            }
                        }
                    }
                }
                ui.add_space(2.0);
            }

            // --- Refuel / Repower button (fills every source) ---
            if ui
                .add(
                    egui::Button::new("Refuel / Repower")
                        .min_size(egui::vec2(ui.available_width(), 22.0)),
                )
                .clicked()
            {
                if let Some(v) = sim.0.vehicle_mut(id) {
                    v.spec.power.refuel();
                }
            }
        });
}

#[derive(Clone)]
struct ContainerSnap {
    amount: f32,
    capacity: f32,
    fill_rate_frac: f32,
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

    egui::CollapsingHeader::new(section_caps("Container", accent))
        .id_salt("prop_container")
        .default_open(false)
        .show(ui, |ui| {
            for (idx, s) in snaps.iter().enumerate() {
                // Step size for +/- is 5 % of capacity, floored at 1
                // so tiny-capacity containers still bump by whole units.
                let step = (s.capacity * 0.05).max(1.0);

                // 1. FILL  — progress-bar readout of the current amount.
                let frac = if s.capacity > 0.0 {
                    (s.amount / s.capacity).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                ui.add(
                    egui::ProgressBar::new(frac)
                        .text(
                            egui::RichText::new(format!(
                                "{:.0} / {:.0}",
                                s.amount, s.capacity
                            ))
                            .monospace()
                            .small()
                            .color(contrast_text_for(accent)),
                        )
                        .fill(accent)
                        .corner_radius(egui::CornerRadius::same(3)),
                );

                // 2. CAPACITY slider.
                let mut capacity = s.capacity;
                if ui
                    .add(
                        egui::Slider::new(&mut capacity, 1.0..=5000.0)
                            .show_value(true)
                            .fixed_decimals(0)
                            .text("capacity"),
                    )
                    .changed()
                {
                    if let Some(v) = sim.0.vehicle_mut(id) {
                        if let Some(c) = v.spec.containers.get_mut(idx) {
                            c.set_capacity(capacity);
                        }
                    }
                }

                // 3. +/- buttons (step = 5 % of capacity, floored at 1).
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

                // 4. RATE slider — 0 %..5 % of capacity per second.
                //    `0` means no auto-fill; anything above turns it on.
                let mut rate_pct = s.fill_rate_frac * 100.0;
                if ui
                    .add(
                        egui::Slider::new(&mut rate_pct, 0.0..=5.0)
                            .show_value(true)
                            .suffix(" %/s")
                            .fixed_decimals(2)
                            .text("rate"),
                    )
                    .changed()
                {
                    if let Some(v) = sim.0.vehicle_mut(id) {
                        if let Some(c) = v.spec.containers.get_mut(idx) {
                            c.fill_rate_frac = (rate_pct / 100.0).clamp(0.0, 0.05);
                        }
                    }
                }
                ui.add_space(4.0);
            }
        });
}

// --- Shared math helpers (mirror of the inspector's, kept local so
// the two panels don't cross-import UI utilities) ---

fn quat_to_euler_xyz(w: f32, x: f32, y: f32, z: f32) -> (f32, f32, f32) {
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

fn euler_xyz_to_quat(ex: f32, ey: f32, ez: f32) -> (f32, f32, f32, f32) {
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
    value: &mut f32,
    speed: f64,
    suffix: &str,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(glyph)
                .strong()
                .monospace()
                .size(11.0)
                .color(color),
        );
        ui.with_layout(
            egui::Layout::right_to_left(egui::Align::Center),
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
    });
    changed
}

/// Bevy system that applies a queued colour change to EVERY tinted
/// piece of a vehicle — chassis mesh plus every part whose declared
/// colour matched the chassis colour at spawn (cab, side beams,
/// crossbars, body panels). Contrast parts (dark roofs, wheels,
/// hitches) keep their own colour because they weren't tagged with
/// `ChassisTinted` in `spawn_vehicle_visuals`.
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
