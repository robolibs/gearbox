//! Machine controllers: what the loaded USDs declared, and the live states.

use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::pane::PaneBody;
use mara_core::pod::Pod;

use super::{PaneCtx, cid, pid, select_list_clicked};
use crate::controller::{ControllerInventory, ControllerStates};
use crate::host::PANE_CONTROLLERS as P;
use crate::viewer::commands::HostCommand;

pub fn show(body: &mut PaneBody<'_, '_>, world: &mut World, ctx: &PaneCtx) {
    let accent = ctx.accent;
    let inventory = world.resource::<ControllerInventory>();
    let states = world.resource::<ControllerStates>();
    let controller_count: usize = inventory
        .machines
        .iter()
        .map(|machine| machine.controllers.len())
        .sum();
    body.add_normal(
        cid(P, "summary"),
        "Discovered",
        "options",
        vec![
            Pod::new(pid(P, "summary", 0))
                .with_readout("machines", inventory.machines.len().to_string())
                .with_readout("controllers", controller_count.to_string())
                .with_readout("live states", states.states.len().to_string()),
        ],
    );
    let machines_id = cid(P, "machines");
    let roots: Vec<Option<Entity>> = inventory.machines.iter().map(|m| m.scene_root).collect();
    if !inventory.machines.is_empty() {
        let rows: Vec<String> = inventory
            .machines
            .iter()
            .map(|machine| machine.id.clone())
            .collect();
        let trailing: Vec<String> = inventory
            .machines
            .iter()
            .map(|machine| format!("{} controller(s)", machine.controllers.len()))
            .collect();
        body.add_normal(
            machines_id,
            "Machines",
            "cube",
            vec![Pod::new(pid(P, "machines", 0)).with_select_list(rows, Some(trailing), accent)],
        );
    }
    let responses = body.render();
    if let Some(index) = select_list_clicked(&responses, machines_id, 0)
        && let Some(Some(root)) = roots.get(index)
    {
        ctx.send(HostCommand::SelectRoot(Some(*root)));
    }
}
