//! Inspector panel content — driven by `right_dock`.

use bevy_egui::egui;

use crate::viz::GearboxSim;

use super::selection::Selection;
use super::style::{accent_color, fg_dim, section_header};

pub fn draw_content(
    ui: &mut egui::Ui,
    sim: &GearboxSim,
    selection: &Selection,
) {
    let Some(id) = selection.vehicle else {
        ui.label(
            egui::RichText::new("Nothing selected.\nLeft-click a vehicle in the viewport or pick one from the Scene panel.")
                .color(fg_dim())
                .italics(),
        );
        return;
    };
    let Some(state) = sim.0.vehicle(id) else {
        ui.label("Selected vehicle no longer exists.");
        return;
    };
    let pose = sim.0.vehicle_pose(id);
    let linvel = sim.0.vehicle_linvel(id);
    let ctrl = sim.0.control(id);
    let speed = (linvel.vx * linvel.vx + linvel.vy * linvel.vy + linvel.vz * linvel.vz).sqrt();

    // Header card — name + id
    egui::Frame::group(ui.style())
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(&state.spec.name)
                        .heading()
                        .color(accent_color()),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(format!("#{}", id.0)).small().color(fg_dim()));
                });
            });
            ui.horizontal(|ui| {
                kv(ui, "mass",   format!("{:.0} kg", state.spec.chassis.mass));
                ui.separator();
                kv(ui, "wheels", state.spec.wheels.len().to_string());
            });
        });

    // Geographic position — lat/lon/alt on the sim's planet datum.
    let geo = sim.0.vehicle_geo(id);
    ui.add_space(8.0);
    section_header(ui, "Geo");
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("lat").color(fg_dim()).monospace());
        ui.monospace(format!("{:+16.10}°", geo.latitude));
    });
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("lon").color(fg_dim()).monospace());
        ui.monospace(format!("{:+16.10}°", geo.longitude));
    });
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("alt").color(fg_dim()).monospace());
        ui.monospace(format!("{:+12.4} m", geo.altitude));
    });

    ui.add_space(8.0);
    section_header(ui, "Pose");
    vec3_row(ui, "pos", pose.point.x as f32, pose.point.y as f32, pose.point.z as f32);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("rot").color(fg_dim()).monospace());
        ui.monospace(format!(
            "w {:+.2}  x {:+.2}  y {:+.2}  z {:+.2}",
            pose.rotation.w, pose.rotation.x, pose.rotation.y, pose.rotation.z
        ));
    });

    ui.add_space(8.0);
    section_header(ui, "Velocity");
    vec3_row(ui, "lin", linvel.vx as f32, linvel.vy as f32, linvel.vz as f32);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("|v|").color(fg_dim()).monospace());
        ui.monospace(format!("{:.2} m/s", speed));
    });

    ui.add_space(8.0);
    section_header(ui, "Control");
    bar_row(ui, "throttle", ctrl.throttle, -1.0, 1.0);
    bar_row(ui, "steer",    ctrl.steer,    -1.0, 1.0);
    bar_row(ui, "brake",    ctrl.brake,     0.0, 1.0);
}

fn kv(ui: &mut egui::Ui, k: &str, v: String) {
    ui.label(egui::RichText::new(k).small().color(fg_dim()));
    ui.label(egui::RichText::new(v).strong());
}

fn vec3_row(ui: &mut egui::Ui, label: &str, x: f32, y: f32, z: f32) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).color(fg_dim()).monospace());
        ui.monospace(format!("x {:+7.2}   y {:+7.2}   z {:+7.2}", x, y, z));
    });
}

fn bar_row(ui: &mut egui::Ui, label: &str, v: f32, min: f32, max: f32) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).color(fg_dim()).monospace());
        let frac = ((v - min) / (max - min)).clamp(0.0, 1.0);
        let progress = egui::ProgressBar::new(frac)
            .text(egui::RichText::new(format!("{:+.2}", v)).monospace())
            .fill(accent_color());
        ui.add(progress);
    });
}
