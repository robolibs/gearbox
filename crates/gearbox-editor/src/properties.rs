//! Properties panel (right dock).
//!
//! Editable counterpart to the read-only Inspector. Layout follows
//! the design-system primitives in `super::widgets`: every section
//! uses `section(...)`, every label+control pair uses
//! `labelled_row(...)`, spacing is driven by the `style::space`
//! tokens, never ad-hoc `add_space`.

use bevy::prelude::*;
use bevy_egui::egui;
use bevy_frost::PaneBuilder;

use gearbox_physics::{
    VehicleId,
    datapod::{Point, Pose, Quaternion},
};

use gearbox_viz::{ChassisTinted, GearboxSim, GroundGrid};

use super::selection::Selection;
use super::selection_ring::SelectionRingSettings;
use super::style::{AXIS_X, AXIS_Y, AXIS_Z, space};
use super::transform_gizmos::{GizmoModesEnabled, GizmoScale};
use super::ui_panel;
use super::usd_load::PendingUsdRemoval;
use super::widgets::{
    axis_drag, color_rgb, drag_value, group_frame, pretty_progressbar_text, pretty_slider,
    row_separator, sub_caption, subsection, toggle, wide_button,
};

/// Bevy `Resource` the UI writes to request a live colour change on
/// a vehicle. The `apply_vehicle_color_changes` system consumes and
/// drains it each frame.
#[derive(Resource, Default, Debug, Clone)]
pub struct PendingColorChange {
    pub pending: Option<(VehicleId, [f32; 3])>,
}

pub fn draw_content(
    pane: &mut PaneBuilder,
    sim: &mut GearboxSim,
    selection: &mut Selection,
    grid: &mut GroundGrid,
    gizmo_scale: &mut GizmoScale,
    gizmo_modes: &mut GizmoModesEnabled,
    ring_settings: &mut SelectionRingSettings,
    glass_opacity: &mut super::style::GlassOpacity,
    pending_color: &mut PendingColorChange,
    pending_usd_removal: &mut PendingUsdRemoval,
    accent: egui::Color32,
) {
    if let Some(id) = selection.vehicle {
        vehicle_section(pane, sim, id, pending_color, accent);
    } else if let Some(usd_entity) = selection.usd_entity {
        usd_section(pane, selection, usd_entity, pending_usd_removal, accent);
    } else {
        world_section(
            pane,
            sim,
            grid,
            gizmo_scale,
            gizmo_modes,
            ring_settings,
            glass_opacity,
            accent,
        );
    }
}

// ═══ USD asset panel ════════════════════════════════════════════════

fn usd_section(
    pane: &mut PaneBuilder,
    selection: &mut Selection,
    usd_entity: Entity,
    pending_removal: &mut PendingUsdRemoval,
    accent: egui::Color32,
) {
    pane.section("usd_actions", "Asset Actions", true, |ui| {
        sub_caption(ui, "USD-loaded asset.");
        ui.add_space(space::BLOCK);
        if wide_button(ui, "🗑  Remove from scene", accent).clicked() {
            pending_removal.0.push(usd_entity);
            // Clear selection — the gizmo / inspector / ring all
            // gracefully fall back to nothing-selected next frame.
            selection.usd_entity = None;
        }
    });
}

// ═══ World panel ════════════════════════════════════════════════════

fn world_section(
    pane: &mut PaneBuilder,
    sim: &mut GearboxSim,
    grid: &mut GroundGrid,
    gizmo_scale: &mut GizmoScale,
    gizmo_modes: &mut GizmoModesEnabled,
    ring_settings: &mut SelectionRingSettings,
    glass_opacity: &mut super::style::GlassOpacity,
    accent: egui::Color32,
) {
    pane.section("world_sandbox", "Sandbox", true, |ui| {
        toggle(ui, "unlimited power", &mut sim.0.unlimited_power, accent);
        ui.add_space(space::TIGHT);
        sub_caption(ui, "Power drain is suspended on every vehicle.");
    });

    // Grid / gizmo / ring still live in `ui_panel::draw_content`.
    ui_panel::draw_content(
        pane,
        grid,
        gizmo_scale,
        gizmo_modes,
        ring_settings,
        glass_opacity,
        accent,
    );
}

// ═══ Vehicle panel ══════════════════════════════════════════════════

fn vehicle_section(
    pane: &mut PaneBuilder,
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
            pane.section("prop_missing", "Vehicle", true, |ui| {
                sub_caption(ui, "Selected vehicle no longer exists.");
            });
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

    pane.section("prop_vehicle", &format!("Vehicle · {}", name), true, |ui| {
        // --- Colour ---
        let mut rgb = current_color;
        if color_rgb(ui, "colour", &mut rgb, accent).changed() {
            if let Some(v) = sim.0.vehicle_mut(id) {
                v.spec.chassis.color = rgb;
            }
            pending_color.pending = Some((id, rgb));
        }

        // --- Mass ---
        let mut mass = current_mass;
        if drag_value(ui, "mass (kg)", &mut mass, 1.0, 0.1..=100_000.0, 1, "").changed() {
            sim.0.set_vehicle_mass(id, mass);
        }

        // --- Engine force (mean across driven wheels) ---
        if !driven_wheels.is_empty() {
            let mut ef = mean_engine_force;
            if drag_value(
                ui,
                "max engine (N/wheel)",
                &mut ef,
                1.0,
                0.0..=1_000_000.0,
                1,
                "",
            )
            .changed()
            {
                if let Some(v) = sim.0.vehicle_mut(id) {
                    for idx in &driven_wheels {
                        v.spec.wheels[*idx].max_engine_force = ef;
                    }
                }
            }
        }

        // --- Brake (mean across all wheels) ---
        if wheels_count > 0 {
            let mut br = mean_brake;
            if drag_value(
                ui,
                "max brake (N·m/wheel)",
                &mut br,
                1.0,
                0.0..=1_000_000.0,
                1,
                "",
            )
            .changed()
            {
                if let Some(v) = sim.0.vehicle_mut(id) {
                    for w in v.spec.wheels.iter_mut() {
                        w.max_brake = br;
                    }
                }
            }
        }

        // --- Damping (linear / angular) — one module per value so
        // each carries its own separator like every other field.
        let mut lin = current_linear_damping;
        let mut ang = current_angular_damping;
        let mut damping_changed = false;
        if drag_value(ui, "linear damping", &mut lin, 0.05, 0.0..=50.0, 2, "").changed() {
            damping_changed = true;
        }
        if drag_value(ui, "angular damping", &mut ang, 0.05, 0.0..=50.0, 2, "").changed() {
            damping_changed = true;
        }
        if damping_changed {
            if let Some(v) = sim.0.vehicle_mut(id) {
                v.spec.chassis.linear_damping = lin;
                v.spec.chassis.angular_damping = ang;
            }
        }
    });

    // Each helper decides whether it renders at all (Work / Power /
    // Container sections early-return when the selected vehicle has
    // nothing of that kind). PaneBuilder handles the inter-section
    // gap so an unrendered section leaves no orphan space above it.
    work_section(pane, sim, id, accent);
    power_section(pane, sim, id, accent);
    container_section(pane, sim, id, accent);
    transform_section(pane, sim, id, accent);
}

fn work_section(
    pane: &mut PaneBuilder,
    sim: &mut GearboxSim,
    id: VehicleId,
    accent: egui::Color32,
) {
    let Some(state) = sim.0.vehicle(id) else {
        return;
    };
    if state.spec.power.sources.is_empty() {
        return;
    }
    let mut work = state.spec.power.work;
    let mut resistance = state.spec.power.work_resistance;

    pane.section("prop_work", "Work", false, |ui| {
        if toggle(ui, "work", &mut work, accent).changed() {
            if let Some(v) = sim.0.vehicle_mut(id) {
                v.spec.power.work = work;
            }
        }
        if pretty_slider(ui, "resistance", &mut resistance, 0.0..=1.0, 2, "", accent).changed() {
            if let Some(v) = sim.0.vehicle_mut(id) {
                v.spec.power.work_resistance = resistance;
            }
        }
    });
}

fn power_section(
    pane: &mut PaneBuilder,
    sim: &mut GearboxSim,
    id: VehicleId,
    accent: egui::Color32,
) {
    struct Snap {
        turned_on: bool,
        primary: usize,
        entries: Vec<(String, f64, f64)>,
    }
    let snap: Snap = {
        let Some(state) = sim.0.vehicle(id) else {
            return;
        };
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

    pane.section("prop_power", "Power", false, |ui| {
        // TURN ON
        let mut turned_on = snap.turned_on;
        if toggle(ui, "turn on", &mut turned_on, accent).changed() {
            if let Some(v) = sim.0.vehicle_mut(id) {
                v.spec.power.turned_on = turned_on;
            }
        }

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
                &format!(
                    "{} · {:.0} / {:.0}",
                    label, current_snapshot, capacity_snapshot
                ),
            );
            ui.add_space(space::TIGHT);
            let mut capacity = *capacity_snapshot;
            if pretty_slider(ui, "capacity", &mut capacity, 10.0..=5000.0, 0, "", accent).changed()
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
    pane: &mut PaneBuilder,
    sim: &mut GearboxSim,
    id: VehicleId,
    accent: egui::Color32,
) {
    let snaps: Vec<ContainerSnap> = {
        let Some(state) = sim.0.vehicle(id) else {
            return;
        };
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

    pane.section("prop_container", "Container", false, |ui| {
        for (idx, s) in snaps.iter().enumerate() {
            if idx > 0 {
                ui.add_space(space::BLOCK);
            }
            // FILL bar — stacked-pane progressbar module, same language
            // as the Inspector.
            let frac = if s.capacity > 0.0 {
                (s.amount / s.capacity).clamp(0.0, 1.0) as f32
            } else {
                0.0
            };
            let fill_text = format!("{:.0} / {:.0}", s.amount, s.capacity);
            pretty_progressbar_text(ui, &format!("fill · {}", idx + 1), frac, &fill_text, accent);

            // CAPACITY slider — purpose visible without an inline
            // `.text(...)` eating the width.
            let mut capacity = s.capacity;
            if pretty_slider(ui, "capacity", &mut capacity, 1.0..=5000.0, 0, "", accent).changed() {
                if let Some(v) = sim.0.vehicle_mut(id) {
                    if let Some(c) = v.spec.containers.get_mut(idx) {
                        c.set_capacity(capacity);
                    }
                }
            }

            // +/- / empty — inline cluster, with a shared trailing
            // separator so the row still reads as a module.
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
            row_separator(ui);

            // RATE slider — auto-fill rate as % of capacity per sec.
            let mut rate_pct = s.fill_rate_frac * 100.0;
            if pretty_slider(ui, "rate (%/s)", &mut rate_pct, 0.0..=5.0, 1, "", accent).changed() {
                if let Some(v) = sim.0.vehicle_mut(id) {
                    if let Some(c) = v.spec.containers.get_mut(idx) {
                        c.fill_rate_frac = (rate_pct / 100.0).clamp(0.0, 0.05);
                    }
                }
            }
        }
    });
}

fn transform_section(
    pane: &mut PaneBuilder,
    sim: &mut GearboxSim,
    id: VehicleId,
    accent: egui::Color32,
) {
    pane.section("prop_transform", "Transform", false, |ui| {
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

        subsection(
            ui,
            "prop_tr_position",
            "Position",
            Some("drag, double-click to type"),
            accent,
            true,
            |ui| {
                changed |= axis_drag(ui, "X", AXIS_X, &mut px, 0.05, " m", 3).changed();
                changed |= axis_drag(ui, "Y", AXIS_Y, &mut py, 0.05, " m", 3).changed();
                changed |= axis_drag(ui, "Z", AXIS_Z, &mut pz, 0.05, " m", 3).changed();
            },
        );

        subsection(
            ui,
            "prop_tr_rotation",
            "Rotation",
            Some("Euler XYZ, degrees"),
            accent,
            true,
            |ui| {
                changed |= axis_drag(ui, "X", AXIS_X, &mut rx, 1.0, "°", 3).changed();
                changed |= axis_drag(ui, "Y", AXIS_Y, &mut ry, 1.0, "°", 3).changed();
                changed |= axis_drag(ui, "Z", AXIS_Z, &mut rz, 1.0, "°", 3).changed();
            },
        );

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
