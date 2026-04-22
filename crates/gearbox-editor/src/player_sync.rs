//! Keep the `PlayerControlled` viz tag in lockstep with the editor
//! [`Selection`]. Lives in the editor crate (not viz) because viz
//! has no awareness of selection — that's an editor concept.

use bevy::prelude::*;

use gearbox_viz::{PlayerControlled, VehicleBody};

use crate::selection::Selection;

/// Rewrite the `PlayerControlled` tag so the selected vehicle is
/// always the one WASD drives. Nothing selected → nothing tagged →
/// WASD has no effect (remote-controlled vehicles will use a
/// different tag and won't need to be selected).
pub fn sync_player_to_selection_system(
    mut commands: Commands,
    selection: Res<Selection>,
    bodies: Query<(Entity, &VehicleBody, Has<PlayerControlled>)>,
) {
    if !selection.is_changed() {
        return;
    }
    let target_id = selection.vehicle;
    for (entity, body, is_player) in &bodies {
        let should_drive = target_id == Some(body.id);
        if should_drive && !is_player {
            commands.entity(entity).insert(PlayerControlled);
        } else if !should_drive && is_player {
            commands.entity(entity).remove::<PlayerControlled>();
        }
    }
}
