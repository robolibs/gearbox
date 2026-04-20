//! Scene-tree panel content. Driven by `left_dock`.

use bevy::prelude::*;
use bevy_egui::egui;

use gearbox::VehicleId;

use crate::viz::{GearboxSim, PlayerControlled, VehicleBody};

use super::selection::Selection;
use super::style::{accent_color, fg_dim};

pub fn draw_content(
    ui: &mut egui::Ui,
    commands: &mut Commands,
    sim: &GearboxSim,
    bodies: &Query<(Entity, &VehicleBody, Option<&Name>, Has<PlayerControlled>)>,
    selection: &mut Selection,
) {
    if bodies.is_empty() {
        ui.label(
            egui::RichText::new("Empty — spawn something from the Spawn tab.")
                .color(fg_dim())
                .italics(),
        );
        return;
    }

    let mut rows: Vec<(Entity, VehicleId, String, bool)> = bodies
        .iter()
        .map(|(e, vb, name, is_player)| {
            let label = match name {
                Some(n) => n.as_str().to_string(),
                None => format!("Vehicle #{}", vb.id.0),
            };
            (e, vb.id, label, is_player)
        })
        .collect();
    rows.sort_by_key(|(_, id, _, _)| id.0);

    let mut give_drive_to: Option<(VehicleId, Entity)> = None;

    egui::ScrollArea::vertical().show(ui, |ui| {
        for (entity, id, label, is_player) in rows {
            let selected = selection.vehicle == Some(id);

            let text = egui::RichText::new(format!(
                "{}  {}   #{}",
                if is_player { "●" } else { "○" },
                label, id.0
            ));
            let text = if is_player {
                text.color(accent_color()).strong()
            } else if selected {
                text.color(accent_color())
            } else {
                text
            };

            let btn = ui.add_sized(
                [ui.available_width(), 28.0],
                egui::SelectableLabel::new(selected, text),
            );

            if btn.clicked() {
                selection.vehicle = Some(id);
            }
            if btn.double_clicked() && !is_player {
                give_drive_to = Some((id, entity));
            }
        }
    });

    if let Some((_new_id, new_entity)) = give_drive_to {
        let currently_player: Vec<Entity> = bodies
            .iter()
            .filter(|(_, _, _, p)| *p)
            .map(|(e, _, _, _)| e)
            .collect();
        for e in currently_player {
            commands.entity(e).remove::<PlayerControlled>();
        }
        commands.entity(new_entity).insert(PlayerControlled);
    }

    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(format!(
            "{} total  ·  double-click to drive",
            sim.0.vehicles().count()
        ))
        .small()
        .color(fg_dim()),
    );
}
