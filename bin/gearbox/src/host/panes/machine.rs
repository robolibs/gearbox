//! The Machines pane: every machine in the scene as a list (click selects,
//! double-click flies behind it and takes its drive, the radio pins the
//! camera to it); below, the selected machine fills with
//! one block per controller the USD authored, driving the same command
//! paths the bus uses (`UiDrive` for twists, `ServiceCommands` and
//! `LinkValues` for services and work controllers). Nothing here is fixed
//! per machine kind; the widgets follow `gearbox:controller:*:type` and the
//! link tree's element kinds and values. Hitches share one container, PTOs
//! another; everything but the machine head and the drive starts folded.

use std::collections::HashMap;

use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::container::Tab;
use mara_core::pod::{Pod, PodResponse};
use mara_core::shelf::ShelfContainer;
use mara_core::vocab::Id as MaraId;

use super::{PaneCtx, button_clicked, cid, pid, pod_response};
use crate::viewer::commands::HostCommand;
use crate::viewer::state::FollowTarget;
use crate::controller::{
    CmdVel, ControllerInventory, ControllerKey, ControllerSpec, ControllerStates,
    MachineInstanceSpec, UiDrive,
};
use crate::host::PANE_MACHINE as P;
use crate::links::LinkSpec;
use crate::services::{LinkValues, ServiceCommands, controller_joints, moved_link};
use crate::viewer::drive::{DRIVE_TYPES, MachinePanel};
use crate::viewer::systems::Selection;

const MAX_SPEED_MPS: f64 = 6.0;
const MAX_YAW_RPS: f64 = 1.5;

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

/// The machine prim's variant sets: name, current selection, options.
fn machine_variants(world: &World, root: Entity, prim: &str) -> Vec<(String, String, Vec<String>)> {
    use usd_bevy::read::variants::{variant_options, variant_selection, variant_set_names};
    let Some(stage) = world
        .get_non_send::<usd_bevy::instance::UsdInstances>()
        .and_then(|instances| instances.stage(root))
    else {
        return Vec::new();
    };
    let Ok(path) = openusd::sdf::path(prim) else {
        return Vec::new();
    };
    variant_set_names(stage, &path)
        .into_iter()
        .map(|set| {
            let selection = variant_selection(stage, &path, &set).unwrap_or_default();
            let options = variant_options(stage, &path, &set);
            (set, selection, options)
        })
        .collect()
}

fn picked_machine(world: &World) -> Option<MachineInstanceSpec> {
    let picked = world
        .resource::<Selection>()
        .0?;
    world
        .resource::<ControllerInventory>()
        .machines
        .iter()
        .find(|m| m.scene_root == Some(picked))
        .cloned()
}

/// The machine rows at the top of the pane and each row's drive keys.
struct MachineList {
    rows: Vec<(Entity, Vec<ControllerKey>)>,
}

fn drive_keys(root: Entity, machine: &MachineInstanceSpec) -> Vec<ControllerKey> {
    machine
        .controllers
        .iter()
        .filter(|c| c.enabled && DRIVE_TYPES.contains(&c.controller_type.as_str()))
        .map(|c| ControllerKey::new(root, &machine.id, &c.instance))
        .collect()
}

/// One row per machine; the trailing text says who holds its drive.
fn machine_list_pod(world: &World, ctx: &PaneCtx) -> (Pod, MachineList) {
    let inventory = world.resource::<ControllerInventory>();
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
    let list_pod = pid(P, "list", 0);
    ctx.sync_list_memory(list_pod, index_of(selection), index_of(follow));
    let pod = if machines.is_empty() {
        Pod::new(list_pod)
            .with_readout("machines", "none")
            .with_readout("hint", "spawn a machine")
    } else {
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
        Pod::new(list_pod).with_hybrid_select_list(labels, Some(trailing), ctx.accent)
    };
    let list = MachineList {
        rows: machines.iter().map(|(root, m)| (*root, drive_keys(*root, m))).collect(),
    };
    (pod, list)
}

/// Clicks on the machine list: select, fly behind and drive, follow.
fn handle_machine_list(
    responses: &HashMap<MaraId, Vec<PodResponse>>,
    list: &MachineList,
    ctx: &PaneCtx,
) {
    let Some(rows) = pod_response(responses, cid(P, "machine"), 0).and_then(|r| r.hybrid_select_lists.first())
    else {
        return;
    };
    if let Some(i) = rows.body_clicked
        && let Some((root, _)) = list.rows.get(i)
    {
        ctx.send(HostCommand::SelectRoot(Some(*root)));
    }
    if let Some(i) = rows.body_double_clicked
        && let Some((root, _)) = list.rows.get(i)
    {
        ctx.send(HostCommand::SelectRoot(Some(*root)));
        ctx.send(HostCommand::FlyToMachine(*root));
    }
    if let Some(i) = rows.radio_clicked
        && let Some((root, _)) = list.rows.get(i)
    {
        ctx.send(HostCommand::ToggleFollow(*root));
    }
}

/// One controller's pod: where it was placed and which links it drives.
struct Block {
    controller: ControllerSpec,
    key: ControllerKey,
    pod: usize,
    link: Option<LinkSpec>,
    section_links: Vec<LinkSpec>,
    function_link: Option<LinkSpec>,
    bin_link: Option<LinkSpec>,
}

/// Same-frame handoff from `container` to `apply`: rebuilding this from
/// scratch in `apply` would mean keeping two copies of the per-controller
/// switch below in sync, so it's built once and stashed here instead.
#[derive(Resource, Default)]
struct MachineBuildCache(
    Option<(
        MachineList,
        Option<(MachineInstanceSpec, Entity, usize, Vec<(String, String, Vec<String>)>, Vec<Block>)>,
    )>,
);

pub fn container(world: &mut World, ctx: &PaneCtx) -> ShelfContainer<'static> {
    let accent = ctx.accent;
    let (list_pod, list) = machine_list_pod(world, ctx);
    let mut pods = vec![list_pod];
    let Some(machine) = picked_machine(world).filter(|m| m.scene_root.is_some()) else {
        world.insert_resource(MachineBuildCache(Some((list, None))));
        let tab = Tab::new(cid(P, "machine"), "Machine", "vehicle-tractor").pods(pods);
        return ShelfContainer::tabbed(cid(P, "root"), "Machine", "vehicle-tractor", vec![tab]);
    };
    let Some(scene_root) = machine.scene_root else {
        world.insert_resource(MachineBuildCache(Some((list, None))));
        let tab = Tab::new(cid(P, "machine"), "Machine", "vehicle-tractor").pods(pods);
        return ShelfContainer::tabbed(cid(P, "root"), "Machine", "vehicle-tractor", vec![tab]);
    };

    pods.push(
        Pod::new(pid(P, "head", 0))
            .with_readout("machine", machine.id.clone())
            .with_readout("kind", machine.kind.as_deref().unwrap_or("—"))
            .with_readout("controllers", machine.controllers.len().to_string())
            .with_readout("links", machine.links.links.len().to_string()),
    );

    // Variants of the machine prim: a row per set, a button per option.
    let variants = machine_variants(world, scene_root, &machine.prim_path);
    let variants_offset = pods.len();
    if !variants.is_empty() {
        for (i, (set, selection, options)) in variants.iter().enumerate() {
            pods.push(options.iter().fold(
                Pod::new(pid(P, "variants", i)).with_readout(set.as_str(), selection.as_str()),
                |pod, option| pod.with_button(option.as_str(), accent),
            ));
        }
    }

    let has_gamepad = {
        let mut pads = world.query::<&Gamepad>();
        pads.iter(world).next().is_some()
    };
    let states = world.resource::<ControllerStates>();
    let values = world.resource::<LinkValues>();
    let ui_drive = world.resource::<UiDrive>();
    let panel = world.resource::<MachinePanel>();

    let mut blocks: Vec<Block> = Vec::new();
    // Order: drives, hitches, PTOs, the rest — all as pods in one tab.
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
        let pod_idx = pods.len();
        let link = service_link(&machine, controller);
        let short = ty.trim_start_matches("builtin:");
        let own_title = format!("{}  ·  {short}", controller.instance);
        let pod_id = pid(P, "detail", pod_idx);
        let mut pod = Pod::new(pod_id);
        let mut block = Block {
            controller: controller.clone(),
            key: key.clone(),
            pod: pod_idx,
            link: link.cloned(),
            section_links: Vec::new(),
            function_link: None,
            bin_link: None,
        };
        let title;
        if DRIVE_TYPES.contains(&ty) {
            title = own_title;
            if ty == "builtin:ackermann_cmd_vel" {
                pod = pod
                    .with_readout("drive power", controller.max_power_kw
                        .map(|p| format!("{p:.0} kW")).unwrap_or_else(|| "not authored".into()))
                    .with_readout("wheel torque ceiling", controller.max_wheel_torque_nm
                        .map(|t| format!("{:.1} kNm / wheel", t / 1000.0)).unwrap_or_else(|| "grip limited".into()));
                if let Some(limits) = states.drive_limits.get(&key) {
                    pod = pod
                        .with_readout("powered wheels", limits.driven_wheels.to_string())
                        .with_readout("loaded motor wheels", limits.supported_wheels.to_string())
                        .with_readout(if limits.parked { "holding torque budget" } else { "drive torque budget" },
                            format!("{:.1} kNm total", limits.torque_nm / 1000.0))
                        .with_readout("power limiting", if limits.power_scale < 0.999 { "active" } else { "no" });
                }
            }
            if let Some(state) = states.states.get(&key) {
                pod = pod.with_readout(
                    "state",
                    format!(
                        "{:.2} m/s · yaw {:.2} rad/s",
                        state.linear_speed_mps, state.yaw_rate_rps
                    ),
                );
            }
            if has_gamepad {
                let layer = match panel.layer {
                    gearbox_controls::Layer::Camera => "Normal · camera",
                    gearbox_controls::Layer::Vehicle if panel.deadman_active => "R1 · vehicle enabled",
                    gearbox_controls::Layer::Vehicle => "R1 · release to re-arm",
                    gearbox_controls::Layer::Machine => "L1 · reserved",
                    gearbox_controls::Layer::Inactive => "Inactive · drive stopped",
                };
                pod = pod.with_readout("pad layer", layer)
                    .with_readout("normal sticks", "right: orbit · left: strafe/move")
                    .with_readout("normal triggers", "R2: raise · L2: lower camera")
                    .with_readout("normal D-pad", "←/→ machine · ↑ follow · ↓ cinematic");
            }
            let cmd = ui_drive.commands.get(&key).copied().unwrap_or_default();
            ctx.set_slider(pod_id, 0, cmd.linear_mps as f64);
            ctx.set_slider(pod_id, 1, cmd.angular_rps as f64);
            pod = pod
                .with_readout("drive input", "Ready · hold R1 to drive selection")
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
                    let vel = link_value(values, &machine, link, "velocity").unwrap_or(0.0);
                    pod = pod.with_slider("velocity", clamp(vel, -120.0, 120.0), -120.0..=120.0, 1, " rad/s", accent);
                }
                "builtin:hydraulic_valve" => {
                    title = own_title;
                    let flow = link_value(values, &machine, link, "flow").unwrap_or(0.0);
                    pod = pod.with_slider("flow", clamp(flow, -1.0, 1.0), -1.0..=1.0, 2, "", accent);
                }
                "builtin:brake" => {
                    title = own_title;
                    let level = link_value(values, &machine, link, "level").unwrap_or(0.0);
                    pod = pod.with_slider("level", clamp(level, 0.0, 1.0), 0.0..=1.0, 2, "", accent);
                }
                "builtin:trailer_steer" => {
                    title = own_title;
                    let max = controller.max_steer_deg.unwrap_or(35.0) as f64;
                    let angle = link_value(values, &machine, link, "angle_rad")
                        .unwrap_or(0.0)
                        .to_degrees();
                    pod = pod.with_slider("angle", clamp(angle, -max, max), -max..=max, 1, "°", accent);
                }
                "builtin:section_control" => {
                    title = own_title;
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
                    pod = pod.with_readout(
                        "command",
                        controller.command_interface.as_deref().unwrap_or("—"),
                    );
                }
            }
        }
        pod = pod.with_readout("part", title);
        pods.push(pod);
        blocks.push(block);
    }

    world.insert_resource(MachineBuildCache(Some((
        list,
        Some((machine, scene_root, variants_offset, variants, blocks)),
    ))));
    let tab = Tab::new(cid(P, "machine"), "Machine", "vehicle-tractor").pods(pods);
    ShelfContainer::tabbed(cid(P, "root"), "Machine", "vehicle-tractor", vec![tab])
}

pub fn apply(responses: &HashMap<MaraId, Vec<PodResponse>>, world: &mut World, ctx: &PaneCtx) {
    let cached = world
        .get_resource_mut::<MachineBuildCache>()
        .and_then(|mut cache| cache.0.take());
    let Some((list, selected)) = cached else {
        return;
    };
    handle_machine_list(responses, &list, ctx);
    let Some((machine, scene_root, variants_offset, variants, blocks)) = selected else {
        return;
    };
    for (i, (set, selection, options)) in variants.iter().enumerate() {
        let picked = options
            .iter()
            .enumerate()
            .find(|(j, _)| button_clicked(responses, cid(P, "machine"), variants_offset + i, *j))
            .map(|(_, option)| option);
        if let Some(option) = picked.filter(|option| *option != selection) {
            ctx.send(HostCommand::SetVariant {
                root: scene_root,
                prim: machine.prim_path.clone(),
                set: set.clone(),
                selection: option.clone(),
            });
        }
    }
    for block in blocks {
        let Some(resp) = pod_response(responses, cid(P, "machine"), block.pod) else {
            continue;
        };
        let ty = block.controller.controller_type.as_str();
        if DRIVE_TYPES.contains(&ty) {
            world.resource_scope(|world, mut panel: Mut<MachinePanel>| {
                let mut ui_drive = world.resource_mut::<UiDrive>();
                let stop = button_clicked(responses, cid(P, "machine"), block.pod, 0);
                if stop {
                    panel.holding.remove(&block.key);
                    if panel.gamepad_on(&block.key) {
                        panel.gamepad = None;
                    }
                    ui_drive.stop_once(&block.key);
                } else if !panel.gamepad_on(&block.key)
                    && !panel.keyboard_on(&block.key)
                    && resp.sliders.iter().any(|s| s.changed)
                {
                    let speed = resp.sliders.first().map(|s| s.value).unwrap_or(0.0);
                    let yaw = resp.sliders.get(1).map(|s| s.value).unwrap_or(0.0);
                    panel.holding.insert(block.key.clone());
                    ui_drive.drive(&block.key, CmdVel {
                        linear_mps: speed as f32,
                        angular_rps: yaw as f32,
                    }, None);
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
