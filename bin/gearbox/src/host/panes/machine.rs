//! The Machine pane on the right rail: click a machine and it fills with
//! one block per controller the USD authored, driving the same command
//! paths the bus uses (`UiDrive` for twists, `ServiceCommands` and
//! `LinkValues` for services and work controllers). Nothing here is fixed
//! per machine kind; the widgets follow `gearbox:controller:*:type` and the
//! link tree's element kinds and values. Hitches share one container, PTOs
//! another; everything but the machine head and the drive starts folded.

use bevy::prelude::*;
use mara::host::MaraHostCtx;
use mara::ui::mara_core;
use mara_core::pane::PaneBody;
use mara_core::pod::Pod;

use super::{PaneCtx, button_clicked, cid, pid, pod_response};
use crate::controller::{
    CmdVel, ControllerInventory, ControllerKey, ControllerSpec, ControllerStates,
    MachineInstanceSpec, UiDrive,
};
use crate::host::{PANE_MACHINE as P, RIBBON_RIGHT};
use crate::links::LinkSpec;
use crate::services::{LinkValues, ServiceCommands, controller_joints, moved_link};
use crate::viewer::drive::{DRIVE_TYPES, MachinePanel};
use crate::viewer::state::ActiveStage;
use crate::viewer::systems::Selection;

const MAX_SPEED_MPS: f64 = 6.0;
const MAX_YAW_RPS: f64 = 1.5;
const GROUP_HITCHES: &str = "hitches";
const GROUP_PTO: &str = "pto";

fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    if v.is_finite() { v.clamp(lo, hi) } else { lo }
}

/// The link a service controller's joint moves, if any.
fn service_link<'a>(machine: &'a MachineInstanceSpec, c: &'a ControllerSpec) -> Option<&'a LinkSpec> {
    controller_joints(machine, c)
        .first()
        .and_then(|prim| moved_link(&machine.links, prim))
}

fn link_value(values: &LinkValues, machine: &MachineInstanceSpec, link: Option<&LinkSpec>, name: &str) -> Option<f64> {
    link.and_then(|l| values.get(&machine.id, &l.name, name))
}

fn element_links<'a>(machine: &'a MachineInstanceSpec, kind: &str) -> Vec<&'a LinkSpec> {
    machine
        .links
        .links
        .iter()
        .filter(|l| l.element.as_ref().is_some_and(|e| e.kind == kind))
        .collect()
}

/// Which container a controller's pod lives in.
fn group_of(c: &ControllerSpec) -> String {
    match c.controller_type.as_str() {
        "builtin:hitch" => GROUP_HITCHES.to_string(),
        "builtin:pto" => GROUP_PTO.to_string(),
        _ => c.instance.clone(),
    }
}

/// The machine the pane shows: the selected root, else the active stage.
fn picked_machine(world: &World) -> Option<MachineInstanceSpec> {
    let picked = world
        .resource::<Selection>()
        .0
        .or(world.resource::<ActiveStage>().0)?;
    world
        .resource::<ControllerInventory>()
        .machines
        .iter()
        .find(|m| m.scene_root == Some(picked))
        .cloned()
}

/// Selecting a machine opens the pane, once per selection change.
pub fn auto_open(host: &MaraHostCtx<'_>, world: &mut World) {
    let selection = world.resource::<Selection>().0;
    let is_machine = selection.is_some_and(|root| {
        world
            .resource::<ControllerInventory>()
            .machines
            .iter()
            .any(|m| m.scene_root == Some(root))
    });
    let mut panel = world.resource_mut::<MachinePanel>();
    if selection != panel.last_selection {
        panel.last_selection = selection;
        if is_machine {
            host.set_rail_pane_open(RIBBON_RIGHT, P, true);
        }
    }
}

/// One controller's pod: where it was placed and which links it drives.
struct Block {
    controller: ControllerSpec,
    key: ControllerKey,
    group: String,
    pod: usize,
    link: Option<LinkSpec>,
    section_links: Vec<LinkSpec>,
    function_link: Option<LinkSpec>,
    bin_link: Option<LinkSpec>,
}

pub fn show(body: &mut PaneBody<'_, '_>, world: &mut World, ctx: &PaneCtx) {
    let accent = ctx.accent;
    let Some(machine) = picked_machine(world) else {
        body.add_normal(
            cid(P, "none"),
            "No machine",
            "cube",
            vec![
                Pod::new(pid(P, "none", 0))
                    .with_readout("selected", "nothing")
                    .with_readout("hint", "click a machine in the scene"),
            ],
        );
        return;
    };
    let Some(scene_root) = machine.scene_root else {
        return;
    };

    // Every container but the head and the drive starts folded, once per
    // machine.
    {
        let mut panel = world.resource_mut::<MachinePanel>();
        if !panel.folded.contains(&machine.id) {
            panel.folded.insert(machine.id.clone());
            let mut fold: Vec<String> = vec![GROUP_HITCHES.to_string(), GROUP_PTO.to_string()];
            fold.extend(
                machine
                    .controllers
                    .iter()
                    .filter(|c| !DRIVE_TYPES.contains(&c.controller_type.as_str()))
                    .map(group_of),
            );
            for section in fold {
                ctx.fold_container(cid(P, &section));
            }
        }
    }

    body.add_normal(
        cid(P, "head"),
        machine.id.clone(),
        "cube",
        vec![
            Pod::new(pid(P, "head", 0))
                .with_readout("kind", machine.kind.as_deref().unwrap_or("—"))
                .with_readout("controllers", machine.controllers.len().to_string())
                .with_readout("links", machine.links.links.len().to_string()),
        ],
    );

    let has_gamepad = {
        let mut pads = world.query::<&Gamepad>();
        pads.iter(world).next().is_some()
    };
    let states = world.resource::<ControllerStates>();
    let values = world.resource::<LinkValues>();
    let ui_drive = world.resource::<UiDrive>();
    let panel = world.resource::<MachinePanel>();

    let mut blocks: Vec<Block> = Vec::new();
    // Containers in order: drives, hitches, PTOs, the rest.
    let mut containers: Vec<(String, String, &'static str, Vec<Pod>)> = Vec::new();
    let mut ordered: Vec<&ControllerSpec> = machine.controllers.iter().filter(|c| c.enabled).collect();
    ordered.sort_by_key(|c| match c.controller_type.as_str() {
        t if DRIVE_TYPES.contains(&t) => 0,
        "builtin:hitch" => 1,
        "builtin:pto" => 2,
        _ => 3,
    });

    for controller in ordered {
        let key = ControllerKey::new(scene_root, &machine.id, &controller.instance);
        let ty = controller.controller_type.as_str();
        let group = group_of(controller);
        let pod_idx = containers
            .iter()
            .find(|c| c.0 == group)
            .map(|c| c.3.len())
            .unwrap_or(0);
        let link = service_link(&machine, controller);
        let short = ty.trim_start_matches("builtin:");
        let own_title = format!("{}  ·  {short}", controller.instance);
        let pod_id = pid(P, &group, pod_idx);
        let mut pod = Pod::new(pod_id);
        let mut block = Block {
            controller: controller.clone(),
            key: key.clone(),
            group: group.clone(),
            pod: pod_idx,
            link: link.cloned(),
            section_links: Vec::new(),
            function_link: None,
            bin_link: None,
        };
        let (title, icon);
        if DRIVE_TYPES.contains(&ty) {
            title = own_title;
            icon = "vehicle-tractor";
            if let Some(state) = states.states.get(&key) {
                pod = pod.with_readout(
                    "state",
                    format!(
                        "{:.2} m/s · yaw {:.2} rad/s",
                        state.linear_speed_mps, state.yaw_rate_rps
                    ),
                );
            }
            let holding = panel.holds(&key);
            let pad = panel.gamepad_on(&key);
            ctx.sync_toggles(pod_id, &[holding, pad]);
            let pad_label = if has_gamepad {
                "Gamepad"
            } else {
                "Gamepad (none connected)"
            };
            let cmd = ui_drive.0.get(&key).copied().unwrap_or_default();
            pod = pod
                .with_toggle_initial("Viewer drives", accent, holding)
                .with_toggle_initial(pad_label, accent, pad)
                .with_slider(
                    "speed",
                    clamp(cmd.linear_mps as f64, -MAX_SPEED_MPS, MAX_SPEED_MPS),
                    -MAX_SPEED_MPS..=MAX_SPEED_MPS,
                    2,
                    " m/s",
                    accent,
                )
                .with_slider(
                    "yaw rate",
                    clamp(cmd.angular_rps as f64, -MAX_YAW_RPS, MAX_YAW_RPS),
                    -MAX_YAW_RPS..=MAX_YAW_RPS,
                    2,
                    " rad/s",
                    accent,
                )
                .with_button("Stop", accent);
        } else {
            match ty {
                "builtin:hitch" | "builtin:joint_position" => {
                    title = if ty == "builtin:hitch" {
                        "Hitches".to_string()
                    } else {
                        own_title
                    };
                    icon = "arrow-up";
                    let position = link_value(values, &machine, link, "position").unwrap_or(0.0);
                    pod = pod.with_readout("controller", controller.instance.clone());
                    if let Some(l) = link {
                        pod = pod.with_readout("link", l.name.clone());
                    }
                    if let Some(r) = link_value(values, &machine, link, "range") {
                        pod = pod.with_readout("range", format!("{r:.3} rad"));
                    }
                    pod = pod.with_slider("position", clamp(position, 0.0, 1.0), 0.0..=1.0, 2, "", accent);
                }
                "builtin:pto" => {
                    title = "PTO".to_string();
                    icon = "arrow-sync";
                    let rpm = link_value(values, &machine, link, "rpm").unwrap_or(540.0);
                    let engaged = link_value(values, &machine, link, "engaged").unwrap_or(0.0) > 0.5;
                    ctx.sync_toggles(pod_id, &[engaged]);
                    pod = pod.with_readout("controller", controller.instance.clone());
                    if let Some(l) = link {
                        pod = pod.with_readout("link", l.name.clone());
                    }
                    pod = pod
                        .with_readout("rpm", format!("{:.0}", rpm.clamp(0.0, 1200.0)))
                        .with_toggle_initial("engaged", accent, engaged);
                }
                "builtin:joint_velocity" => {
                    title = own_title;
                    icon = "arrow-sync";
                    let vel = link_value(values, &machine, link, "velocity").unwrap_or(0.0);
                    pod = pod.with_slider("velocity", clamp(vel, -120.0, 120.0), -120.0..=120.0, 1, " rad/s", accent);
                }
                "builtin:hydraulic_valve" => {
                    title = own_title;
                    icon = "arrow-sort";
                    let flow = link_value(values, &machine, link, "flow").unwrap_or(0.0);
                    pod = pod.with_slider("flow", clamp(flow, -1.0, 1.0), -1.0..=1.0, 2, "", accent);
                }
                "builtin:brake" => {
                    title = own_title;
                    icon = "record-stop";
                    let level = link_value(values, &machine, link, "level").unwrap_or(0.0);
                    pod = pod.with_slider("level", clamp(level, 0.0, 1.0), 0.0..=1.0, 2, "", accent);
                }
                "builtin:trailer_steer" => {
                    title = own_title;
                    icon = "arrow-turn-right";
                    let max = controller.max_steer_deg.unwrap_or(35.0) as f64;
                    let angle = link_value(values, &machine, link, "angle_rad")
                        .unwrap_or(0.0)
                        .to_degrees();
                    pod = pod.with_slider("angle", clamp(angle, -max, max), -max..=max, 1, "°", accent);
                }
                "builtin:section_control" => {
                    title = own_title;
                    icon = "grid";
                    let target = controller
                        .target
                        .as_deref()
                        .and_then(|t| machine.links.by_prim(t));
                    let function = target
                        .filter(|l| l.element.as_ref().is_some_and(|e| e.kind == "function"))
                        .or_else(|| element_links(&machine, "function").into_iter().next());
                    if let Some(f) = function {
                        let on = values
                            .get(&machine.id, &f.name, "SectionControlState")
                            .unwrap_or(1.0)
                            > 0.5;
                        pod = pod
                            .with_readout("function", f.name.clone())
                            .with_toggle_initial("section control", accent, on);
                        let sections: Vec<&LinkSpec> = element_links(&machine, "section")
                            .into_iter()
                            .filter(|s| s.parent.as_deref() == Some(f.name.as_str()))
                            .collect();
                        let mut states_now = vec![on];
                        for s in &sections {
                            let want = values
                                .get(&machine.id, &s.name, "SetpointWorkState")
                                .unwrap_or(0.0)
                                > 0.5;
                            states_now.push(want);
                            pod = pod.with_toggle_initial(s.name.clone(), accent, want);
                        }
                        ctx.sync_toggles(pod_id, &states_now);
                        let area = values.get(&machine.id, &f.name, "TotalArea").unwrap_or(0.0);
                        pod = pod.with_readout("total area", format!("{area:.4} ha"));
                        block.section_links = sections.into_iter().cloned().collect();
                        block.function_link = Some(f.clone());
                    } else {
                        pod = pod.with_readout("function", "none in the link tree");
                    }
                }
                "builtin:rate_control" => {
                    title = own_title;
                    icon = "drop";
                    let target = controller
                        .target
                        .as_deref()
                        .and_then(|t| machine.links.by_prim(t));
                    let bin = target
                        .filter(|l| l.element.as_ref().is_some_and(|e| e.kind == "bin"))
                        .or_else(|| element_links(&machine, "bin").into_iter().next());
                    if let Some(b) = bin {
                        let set = values
                            .get(&machine.id, &b.name, "SetpointVolumePerAreaApplicationRate")
                            .unwrap_or(0.0);
                        let content = values
                            .get(&machine.id, &b.name, "ActualVolumeContent")
                            .unwrap_or(0.0);
                        let actual = values
                            .get(&machine.id, &b.name, "ActualVolumePerAreaApplicationRate")
                            .unwrap_or(0.0);
                        pod = pod
                            .with_readout("bin", b.name.clone())
                            .with_readout("content", format!("{content:.1} l"))
                            .with_readout("applying", format!("{actual:.1} l/ha"))
                            .with_slider("setpoint", clamp(set, 0.0, 600.0), 0.0..=600.0, 0, " l/ha", accent);
                        block.bin_link = Some(b.clone());
                    } else {
                        pod = pod.with_readout("bin", "none in the link tree");
                    }
                }
                _ => {
                    title = own_title;
                    icon = "options";
                    pod = pod.with_readout(
                        "command",
                        controller.command_interface.as_deref().unwrap_or("—"),
                    );
                }
            }
        }
        if let Some(c) = containers.iter_mut().find(|c| c.0 == group) {
            c.3.push(pod);
        } else {
            containers.push((group, title, icon, vec![pod]));
        }
        blocks.push(block);
    }
    for (group, title, icon, pods) in containers {
        body.add_normal(cid(P, &group), title, icon, pods);
    }

    let responses = body.render();
    for block in blocks {
        let Some(resp) = pod_response(&responses, cid(P, &block.group), block.pod) else {
            continue;
        };
        let ty = block.controller.controller_type.as_str();
        if DRIVE_TYPES.contains(&ty) {
            world.resource_scope(|world, mut panel: Mut<MachinePanel>| {
                let mut ui_drive = world.resource_mut::<UiDrive>();
                if let Some(t) = resp.toggles.first()
                    && t.changed
                {
                    panel.set_viewer_drive(&mut ui_drive, &block.key, t.on);
                }
                if let Some(t) = resp.toggles.get(1)
                    && t.changed
                {
                    panel.set_gamepad(&mut ui_drive, &block.key, t.on);
                }
                let pad_here = panel.gamepad_on(&block.key);
                if panel.holds(&block.key) && !pad_here {
                    let speed = resp.sliders.first().map(|s| s.value).unwrap_or(0.0);
                    let yaw = resp.sliders.get(1).map(|s| s.value).unwrap_or(0.0);
                    let stop = button_clicked(&responses, cid(P, &block.group), block.pod, 0);
                    if stop {
                        let pod = pid(P, &block.group, block.pod);
                        ctx.set_slider(pod, 0, 0.0);
                        ctx.set_slider(pod, 1, 0.0);
                    }
                    ui_drive.0.insert(
                        block.key.clone(),
                        if stop {
                            CmdVel::default()
                        } else {
                            CmdVel {
                                linear_mps: speed as f32,
                                angular_rps: yaw as f32,
                            }
                        },
                    );
                }
            });
            continue;
        }
        let changed = |i: usize| resp.sliders.get(i).filter(|s| s.changed).map(|s| s.value);
        let set = |world: &mut World, name: &str, v: f64| {
            set_service_value(world, &machine, &block.key, block.link.as_ref(), name, v);
        };
        match ty {
            "builtin:hitch" | "builtin:joint_position" => {
                if let Some(v) = changed(0) {
                    set(world, "position", v);
                }
            }
            "builtin:pto" => {
                if let Some(t) = resp.toggles.first()
                    && t.changed
                {
                    set(world, "engaged", t.on as u8 as f64);
                }
            }
            "builtin:joint_velocity" => {
                if let Some(v) = changed(0) {
                    set(world, "velocity", v);
                }
            }
            "builtin:hydraulic_valve" => {
                if let Some(v) = changed(0) {
                    set(world, "flow", v);
                }
            }
            "builtin:brake" => {
                if let Some(v) = changed(0) {
                    set(world, "level", v);
                }
            }
            "builtin:trailer_steer" => {
                if let Some(v) = changed(0) {
                    set(world, "angle_rad", v.to_radians());
                }
            }
            "builtin:section_control" => {
                if let Some(f) = &block.function_link {
                    let mut values = world.resource_mut::<LinkValues>();
                    if let Some(t) = resp.toggles.first()
                        && t.changed
                    {
                        values.set(&machine.id, &f.name, "SectionControlState", t.on as u8 as f64);
                    }
                    for (i, s) in block.section_links.iter().enumerate() {
                        if let Some(t) = resp.toggles.get(i + 1)
                            && t.changed
                        {
                            values.set(&machine.id, &s.name, "SetpointWorkState", t.on as u8 as f64);
                        }
                    }
                }
            }
            "builtin:rate_control" => {
                if let (Some(b), Some(v)) = (&block.bin_link, changed(0)) {
                    world.resource_mut::<LinkValues>().set(
                        &machine.id,
                        &b.name,
                        "SetpointVolumePerAreaApplicationRate",
                        v,
                    );
                }
            }
            _ => {}
        }
    }
}

/// One value written where the bus would write it: the controller's props
/// and the link its joint moves.
fn set_service_value(
    world: &mut World,
    machine: &MachineInstanceSpec,
    key: &ControllerKey,
    link: Option<&LinkSpec>,
    name: &str,
    value: f64,
) {
    world
        .resource_mut::<ServiceCommands>()
        .0
        .entry(key.clone())
        .or_default()
        .insert(name.to_string(), value.to_string());
    if let Some(l) = link {
        world
            .resource_mut::<LinkValues>()
            .set(&machine.id, &l.name, name, value);
    }
}
