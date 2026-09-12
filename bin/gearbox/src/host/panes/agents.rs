//! Agents: one row per controllable machine the loaded USDs declared. Click
//! selects the machine (and opens the Machine pane), double-click flies the
//! camera behind it and gives the viewer its drive, the right-edge radio
//! pins the camera to follow it. The trailing text says who holds the
//! drive; a second container lists the selected machine's controllers.

use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::pane::PaneBody;
use mara_core::pod::Pod;

use super::{PaneCtx, cid, pid, pod_response};
use crate::controller::{ControllerInventory, ControllerKey, ControllerStates, MachineInstanceSpec, UiDrive};
use crate::host::PANE_AGENTS as P;
use crate::viewer::commands::HostCommand;
use crate::viewer::drive::{DRIVE_TYPES, MachinePanel};
use crate::viewer::state::FollowTarget;
use crate::viewer::systems::Selection;

fn drive_keys(root: Entity, machine: &MachineInstanceSpec) -> Vec<ControllerKey> {
    machine
        .controllers
        .iter()
        .filter(|c| c.enabled && DRIVE_TYPES.contains(&c.controller_type.as_str()))
        .map(|c| ControllerKey::new(root, &machine.id, &c.instance))
        .collect()
}

pub fn show(body: &mut PaneBody<'_, '_>, world: &mut World, ctx: &PaneCtx) {
    let accent = ctx.accent;
    let inventory = world.resource::<ControllerInventory>();
    let states = world.resource::<ControllerStates>();
    let panel = world.resource::<MachinePanel>();
    let selection = world.resource::<Selection>().0;
    let follow = world.resource::<FollowTarget>().entity;

    let mut machines: Vec<(Entity, &MachineInstanceSpec)> = inventory
        .machines
        .iter()
        .filter_map(|m| m.scene_root.map(|root| (root, m)))
        .collect();
    machines.sort_by(|a, b| a.1.id.cmp(&b.1.id));
    let index_of =
        |entity: Option<Entity>| entity.and_then(|e| machines.iter().position(|(root, _)| *root == e));
    let list_pod = pid(P, "agents", 0);
    ctx.sync_list_memory(list_pod, index_of(selection), index_of(follow));

    let agents_id = cid(P, "agents");
    if machines.is_empty() {
        body.add_normal(
            agents_id,
            "Agents",
            "vehicle-tractor",
            vec![
                Pod::new(list_pod)
                    .with_readout("machines", "none")
                    .with_readout("hint", "spawn a machine with a controller"),
            ],
        );
        return;
    }
    let labels: Vec<String> = machines.iter().map(|(_, m)| m.id.clone()).collect();
    let trailing: Vec<String> = machines
        .iter()
        .map(|(root, m)| {
            let drives = drive_keys(*root, m);
            let holder = if drives.iter().any(|k| panel.gamepad_on(k)) {
                " · gamepad"
            } else if drives.iter().any(|k| panel.holds(k)) {
                " · viewer"
            } else {
                ""
            };
            format!("{}{holder}", m.kind.as_deref().unwrap_or("machine"))
        })
        .collect();
    body.add_normal(
        agents_id,
        "Agents",
        "vehicle-tractor",
        vec![Pod::new(list_pod).with_hybrid_select_list(labels, Some(trailing), accent)],
    );

    let picked = index_of(selection).map(|i| machines[i]);
    let mut pod = Pod::new(pid(P, "controllers", 0));
    match picked {
        Some((root, m)) => {
            for c in m.controllers.iter().filter(|c| c.enabled) {
                let key = ControllerKey::new(root, &m.id, &c.instance);
                let short = c.controller_type.trim_start_matches("builtin:");
                let value = match states.states.get(&key) {
                    Some(s) if DRIVE_TYPES.contains(&c.controller_type.as_str()) => {
                        format!("{short} · {:.1} m/s", s.linear_speed_mps)
                    }
                    _ => short.to_string(),
                };
                pod = pod.with_readout(c.instance.clone(), value);
            }
            if m.controllers.iter().all(|c| !c.enabled) {
                pod = pod.with_readout("controllers", "none enabled");
            }
        }
        None => pod = pod.with_readout("selected", "nothing"),
    }
    body.add_normal(cid(P, "controllers"), "Controllers", "options", vec![pod]);

    let rows: Vec<(Entity, Vec<ControllerKey>)> = machines
        .iter()
        .map(|(root, m)| (*root, drive_keys(*root, m)))
        .collect();
    let responses = body.render();
    let Some(list) =
        pod_response(&responses, agents_id, 0).and_then(|r| r.hybrid_select_lists.first())
    else {
        return;
    };
    if let Some(i) = list.body_clicked
        && let Some((root, _)) = rows.get(i)
    {
        tracing::debug!(target: "gearbox", "agents pane: row {i} clicked -> {root:?}");
        ctx.send(HostCommand::SelectRoot(Some(*root)));
    }
    if let Some(i) = list.body_double_clicked
        && let Some((root, drives)) = rows.get(i)
    {
        ctx.send(HostCommand::SelectRoot(Some(*root)));
        ctx.send(HostCommand::FlyToMachine(*root));
        if let Some(key) = drives.first() {
            world.resource_scope(|world, mut panel: Mut<MachinePanel>| {
                if !drives.iter().any(|k| panel.holds(k)) {
                    let mut ui_drive = world.resource_mut::<UiDrive>();
                    panel.set_viewer_drive(&mut ui_drive, key, true);
                }
            });
        }
    }
    if let Some(i) = list.radio_clicked
        && let Some((root, _)) = rows.get(i)
    {
        tracing::debug!(target: "gearbox", "agents pane: radio {i} clicked (pinned now {:?})", list.pinned);
        ctx.send(HostCommand::ToggleFollow(*root));
    }
}
