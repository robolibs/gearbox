//! Spawn-panel content. Rendered by `left_dock` when the Spawn tab is active.

use bevy::prelude::*;
use bevy_egui::egui;

use gearbox::{
    datapod::{Point, Pose, Quaternion},
    presets, VehicleSpec,
};

use crate::viz::{GearboxSim, PlayerControlled, VehicleBody, spawn_vehicle_visuals};
use crate::BigSpaceRoot;

use super::selection::Selection;
use super::style::{fg_dim, section_header};

pub fn draw_content(
    ui: &mut egui::Ui,
    commands: &mut Commands,
    sim: &mut GearboxSim,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    existing_bodies: &Query<Entity, With<VehicleBody>>,
    player_tagged: &Query<Entity, With<PlayerControlled>>,
    selection: &mut Selection,
    big_space_root: Entity,
) {
    let big = |label: &str, ui: &mut egui::Ui| {
        ui.add_sized(
            [ui.available_width(), 32.0],
            egui::Button::new(egui::RichText::new(label).strong()),
        )
        .clicked()
    };

    if big("＋  Tractor", ui) {
        spawn_and_select(
            presets::tractor(),
            commands, sim, meshes, materials, player_tagged, selection, big_space_root,
        );
    }
    ui.add_space(4.0);
    if big("＋  Car", ui) {
        spawn_and_select(
            presets::car(),
            commands, sim, meshes, materials, player_tagged, selection, big_space_root,
        );
    }

    ui.add_space(14.0);
    section_header(ui, "Stats");
    ui.label(format!("Vehicles: {}", existing_bodies.iter().count()));

    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(
                "LMB click   select\nLMB drag   move\nLMB+RMB  orbit\nMMB drag   pan\nWheel      zoom",
            )
            .small()
            .color(fg_dim()),
        );
        ui.separator();
        ui.label(
            egui::RichText::new("W/S throttle · A/D steer · Space brake")
                .small()
                .color(fg_dim()),
        );
    });
}

fn spawn_and_select(
    spec: VehicleSpec,
    commands: &mut Commands,
    sim: &mut GearboxSim,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    player_tagged: &Query<Entity, With<PlayerControlled>>,
    selection: &mut Selection,
    big_space_root: Entity,
) {
    let pose = Pose {
        point: Point::new(0.0, 1.4, 0.0),
        rotation: Quaternion::identity(),
    };
    let id = sim.0.spawn_vehicle(spec.clone(), pose);
    let root = spawn_vehicle_visuals(commands, meshes, materials, id, &spec, big_space_root);
    for e in player_tagged {
        commands.entity(e).remove::<PlayerControlled>();
    }
    commands.entity(root).insert(PlayerControlled);
    selection.vehicle = Some(id);
}
