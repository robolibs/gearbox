//! The Machine pane on the right ribbon: click a machine and it fills with
//! one block per controller the USD authored, driving the same command
//! paths the bus uses (`UiDrive` for twists, `ServiceCommands` and
//! `LinkValues` for services and work controllers). Nothing here is fixed
//! per machine kind; the widgets follow `gearbox:controller:*:type` and the
//! link tree's element kinds and values. Hitches share one container, PTOs
//! another; everything but the machine head and the drive starts folded. A
//! gamepad can take a drive controller; camera follow and fly live in the
//! agent tree.

use std::collections::HashSet;

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use crate::controller::{
    CmdVel, ControllerInventory, ControllerKey, ControllerSpec, ControllerStates,
    MachineInstanceSpec, UiDrive,
};
use crate::links::LinkSpec;
use crate::services::{LinkValues, ServiceCommands, controller_joints, moved_link};
use crate::viewer::mara_ui::*;
use crate::viewer::state::ActiveStage;
use crate::viewer::ui::{
    RIB_MACHINE, RIBBON_ITEMS, RIBBON_RIGHT, RIBBONS, Selection, button_clicked, pod_response,
};

pub struct MachinePanelPlugin;

impl Plugin for MachinePanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MachinePanel>()
            .add_systems(Startup, open_from_env)
            .add_systems(Update, gamepad_drive)
            .add_systems(EguiPrimaryContextPass, draw_machine_panel);
    }
}

/// `GEARBOX_OPEN_PANEL=machine` opens the pane at start, for scripted runs.
fn open_from_env(mut open: ResMut<RibbonOpen>) {
    if std::env::var("GEARBOX_OPEN_PANEL").as_deref() == Ok("machine") {
        open.per_ribbon.insert(RIBBON_RIGHT, RIB_MACHINE);
    }
}

#[derive(Resource, Default)]
pub struct MachinePanel {
    last_selection: Option<Entity>,
    /// Drive controllers the viewer currently holds.
    holding: HashSet<ControllerKey>,
    /// The drive controller the gamepad steers, if any.
    pub gamepad: Option<ControllerKey>,
    /// Machines whose containers have been folded once.
    folded: HashSet<String>,
}

impl MachinePanel {
    pub(crate) fn holds(&self, key: &ControllerKey) -> bool {
        self.holding.contains(key)
    }

    pub(crate) fn gamepad_on(&self, key: &ControllerKey) -> bool {
        self.gamepad.as_ref() == Some(key)
    }

    /// Take or release a drive controller for the viewer's own commands.
    pub(crate) fn set_viewer_drive(
        &mut self,
        ui_drive: &mut UiDrive,
        key: &ControllerKey,
        on: bool,
    ) {
        if on {
            self.holding.insert(key.clone());
            ui_drive.0.entry(key.clone()).or_insert(CmdVel::default());
        } else {
            self.holding.remove(key);
            ui_drive.0.remove(key);
            if self.gamepad.as_ref() == Some(key) {
                self.gamepad = None;
            }
        }
    }

    /// Hand a drive controller to the gamepad, or take it back to the sliders.
    pub(crate) fn set_gamepad(&mut self, ui_drive: &mut UiDrive, key: &ControllerKey, on: bool) {
        if on {
            self.gamepad = Some(key.clone());
            self.holding.insert(key.clone());
        } else if self.gamepad.as_ref() == Some(key) {
            self.gamepad = None;
            ui_drive.0.insert(key.clone(), CmdVel::default());
        }
    }
}

pub(crate) const DRIVE_TYPES: [&str; 2] =
    ["builtin:ackermann_cmd_vel", "builtin:diff_drive_cmd_vel"];
const MAX_SPEED_MPS: f64 = 6.0;
const MAX_YAW_RPS: f64 = 1.5;
const PAD_SPEED_MPS: f32 = 4.0;
const PAD_YAW_RPS: f32 = 1.0;
const PAD_DEADZONE: f32 = 0.12;

const GROUP_HITCHES: &str = "hitches";
const GROUP_PTO: &str = "pto";

fn cid(section: &str) -> MaraId {
    MaraId::new(("gearbox", RIB_MACHINE, section.to_string()))
}

fn pid(section: &str, idx: usize) -> MaraId {
    MaraId::new(("gearbox", RIB_MACHINE, section.to_string(), idx))
}

fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    if v.is_finite() { v.clamp(lo, hi) } else { lo }
}

/// The link a service controller's joint moves, if any.
fn service_link<'a>(
    machine: &'a MachineInstanceSpec,
    c: &'a ControllerSpec,
) -> Option<&'a LinkSpec> {
    controller_joints(machine, c)
        .first()
        .and_then(|prim| moved_link(&machine.links, prim))
}

fn link_value(
    values: &LinkValues,
    machine: &MachineInstanceSpec,
    link: Option<&LinkSpec>,
    name: &str,
) -> Option<f64> {
    link.and_then(|l| values.get(&machine.id, &l.name, name))
}

/// One value written where the bus would write it: the controller's props
/// and the link its joint moves.
fn set_service_value(
    service: &mut ServiceCommands,
    values: &mut LinkValues,
    machine: &MachineInstanceSpec,
    key: &ControllerKey,
    link: Option<&LinkSpec>,
    name: &str,
    value: f64,
) {
    service
        .0
        .entry(key.clone())
        .or_default()
        .insert(name.to_string(), value.to_string());
    if let Some(l) = link {
        values.set(&machine.id, &l.name, name, value);
    }
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

/// The first connected gamepad drives the held controller while the pane's
/// switch is on: right trigger forward, left trigger back, left stick
/// steers, south button stops.
fn gamepad_drive(
    gamepads: Query<&Gamepad>,
    panel: Res<MachinePanel>,
    mut ui_drive: ResMut<UiDrive>,
) {
    let Some(key) = panel.gamepad.clone() else {
        return;
    };
    let Some(pad) = gamepads.iter().next() else {
        return;
    };
    let dead = |v: f32| if v.abs() < PAD_DEADZONE { 0.0 } else { v };
    let throttle = dead(pad.get(GamepadButton::RightTrigger2).unwrap_or(0.0))
        - dead(pad.get(GamepadButton::LeftTrigger2).unwrap_or(0.0));
    let stick_y = dead(pad.get(GamepadAxis::LeftStickY).unwrap_or(0.0));
    let forward = if throttle.abs() > 0.0 {
        throttle
    } else {
        stick_y
    };
    let steer = dead(pad.get(GamepadAxis::LeftStickX).unwrap_or(0.0));
    let cmd = if pad.pressed(GamepadButton::South) {
        CmdVel {
            linear_mps: 0.0,
            angular_rps: 0.0,
        }
    } else {
        CmdVel {
            linear_mps: forward * PAD_SPEED_MPS,
            angular_rps: -steer * PAD_YAW_RPS,
        }
    };
    ui_drive.0.insert(key, cmd);
}

/// One controller's pod: where it was placed and which links it drives.
struct Block<'a> {
    controller: &'a ControllerSpec,
    key: ControllerKey,
    group: String,
    pod: usize,
    link: Option<&'a LinkSpec>,
    section_links: Vec<&'a LinkSpec>,
    function_link: Option<&'a LinkSpec>,
    bin_link: Option<&'a LinkSpec>,
}

#[allow(clippy::too_many_arguments)]
fn draw_machine_panel(
    mut contexts: EguiContexts,
    accent: Res<AccentColor>,
    mut open: ResMut<RibbonOpen>,
    placement: Res<RibbonPlacement>,
    selection: Res<Selection>,
    active: Res<ActiveStage>,
    inventory: Res<ControllerInventory>,
    states: Res<ControllerStates>,
    mut ui_drive: ResMut<UiDrive>,
    mut service: ResMut<ServiceCommands>,
    mut values: ResMut<LinkValues>,
    mut panel: ResMut<MachinePanel>,
    gamepads: Query<&Gamepad>,
) {
    let picked = selection.0.or(active.0);
    let machine = picked.and_then(|entity| {
        inventory
            .machines
            .iter()
            .find(|m| m.scene_root == Some(entity))
    });
    if selection.0 != panel.last_selection {
        panel.last_selection = selection.0;
        if selection.0.is_some() && machine.is_some() {
            open.per_ribbon.insert(RIBBON_RIGHT, RIB_MACHINE);
        }
    }
    if !open.is_open(RIBBON_RIGHT, RIB_MACHINE) {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let accent_col: egui::Color32 = accent.0.into();
    let panel = &mut *panel;

    // Every container but the head and the drive starts folded, once per
    // machine.
    if let Some(m) = machine
        && !panel.folded.contains(&m.id)
    {
        panel.folded.insert(m.id.clone());
        let mut fold: Vec<String> = vec![GROUP_HITCHES.to_string(), GROUP_PTO.to_string()];
        fold.extend(
            m.controllers
                .iter()
                .filter(|c| !DRIVE_TYPES.contains(&c.controller_type.as_str()))
                .map(group_of),
        );
        ctx.data_mut(|d| {
            for section in fold {
                let id: egui::Id = cid(&section).into();
                d.insert_persisted(id.with("body_open"), false);
            }
        });
    }

    show_mara_pane_for_item(
        ctx,
        RIBBONS,
        RIBBON_ITEMS,
        &placement,
        RIB_MACHINE,
        "Machine",
        accent_col,
        |body| {
            let Some(machine) = machine else {
                body.add_normal(
                    cid("none"),
                    "No machine",
                    "cube",
                    vec![
                        Pod::new(pid("none", 0))
                            .with_readout("selected", "nothing")
                            .with_readout("hint", "click a machine in the scene"),
                    ],
                );
                body.render();
                return;
            };
            let Some(scene_root) = machine.scene_root else {
                return;
            };
            body.add_normal(
                cid("head"),
                machine.id.clone(),
                "cube",
                vec![
                    Pod::new(pid("head", 0))
                        .with_readout("kind", machine.kind.as_deref().unwrap_or("—"))
                        .with_readout("controllers", machine.controllers.len().to_string())
                        .with_readout("links", machine.links.links.len().to_string()),
                ],
            );

            let mut blocks: Vec<Block> = Vec::new();
            // Containers in order: drives, hitches, PTOs, the rest.
            let mut containers: Vec<(String, String, &'static str, Vec<Pod>)> = Vec::new();

            let mut ordered: Vec<&ControllerSpec> =
                machine.controllers.iter().filter(|c| c.enabled).collect();
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
                let link = service_link(machine, controller);
                let short = ty.trim_start_matches("builtin:");
                let own_title = format!("{}  ·  {short}", controller.instance);
                let mut pod = Pod::new(pid(&group, pod_idx));
                let mut block = Block {
                    controller,
                    key: key.clone(),
                    group: group.clone(),
                    pod: pod_idx,
                    link,
                    section_links: Vec::new(),
                    function_link: None,
                    bin_link: None,
                };
                let (title, icon);
                if DRIVE_TYPES.contains(&ty) {
                    title = own_title;
                    icon = "vehicle_tractor";
                    if let Some(state) = states.states.get(&key) {
                        pod = pod.with_readout(
                            "state",
                            format!(
                                "{:.2} m/s · yaw {:.2} rad/s",
                                state.linear_speed_mps, state.yaw_rate_rps
                            ),
                        );
                    }
                    let holding = panel.holding.contains(&key);
                    let pad = panel.gamepad.as_ref() == Some(&key);
                    let pad_label = if gamepads.iter().next().is_some() {
                        "Gamepad"
                    } else {
                        "Gamepad (none connected)"
                    };
                    let cmd = ui_drive.0.get(&key).copied().unwrap_or(CmdVel {
                        linear_mps: 0.0,
                        angular_rps: 0.0,
                    });
                    pod = pod
                        .with_toggle_initial("Viewer drives", accent_col, holding)
                        .with_toggle_initial(pad_label, accent_col, pad)
                        .with_slider(
                            "speed",
                            clamp(cmd.linear_mps as f64, -MAX_SPEED_MPS, MAX_SPEED_MPS),
                            -MAX_SPEED_MPS..=MAX_SPEED_MPS,
                            2,
                            " m/s",
                            accent_col,
                        )
                        .with_slider(
                            "yaw rate",
                            clamp(cmd.angular_rps as f64, -MAX_YAW_RPS, MAX_YAW_RPS),
                            -MAX_YAW_RPS..=MAX_YAW_RPS,
                            2,
                            " rad/s",
                            accent_col,
                        )
                        .with_button("Stop", accent_col);
                } else {
                    match ty {
                        "builtin:hitch" | "builtin:joint_position" => {
                            title = if ty == "builtin:hitch" {
                                "Hitches".to_string()
                            } else {
                                own_title
                            };
                            icon = "arrow_up";
                            let position =
                                link_value(&values, machine, link, "position").unwrap_or(0.0);
                            pod = pod.with_readout("controller", controller.instance.clone());
                            if let Some(l) = link {
                                pod = pod.with_readout("link", l.name.clone());
                            }
                            if let Some(r) = link_value(&values, machine, link, "range") {
                                pod = pod.with_readout("range", format!("{r:.3} rad"));
                            }
                            pod = pod.with_slider(
                                "position",
                                clamp(position, 0.0, 1.0),
                                0.0..=1.0,
                                2,
                                "",
                                accent_col,
                            );
                        }
                        "builtin:pto" => {
                            title = "PTO".to_string();
                            icon = "arrow_sync";
                            let rpm = link_value(&values, machine, link, "rpm").unwrap_or(540.0);
                            let engaged =
                                link_value(&values, machine, link, "engaged").unwrap_or(0.0) > 0.5;
                            pod = pod.with_readout("controller", controller.instance.clone());
                            if let Some(l) = link {
                                pod = pod.with_readout("link", l.name.clone());
                            }
                            pod = pod
                                .with_readout("rpm", format!("{:.0}", rpm.clamp(0.0, 1200.0)))
                                .with_toggle_initial("engaged", accent_col, engaged);
                        }
                        "builtin:joint_velocity" => {
                            title = own_title;
                            icon = "arrow_sync";
                            let vel = link_value(&values, machine, link, "velocity").unwrap_or(0.0);
                            pod = pod.with_slider(
                                "velocity",
                                clamp(vel, -120.0, 120.0),
                                -120.0..=120.0,
                                1,
                                " rad/s",
                                accent_col,
                            );
                        }
                        "builtin:hydraulic_valve" => {
                            title = own_title;
                            icon = "arrow_sort";
                            let flow = link_value(&values, machine, link, "flow").unwrap_or(0.0);
                            pod = pod.with_slider(
                                "flow",
                                clamp(flow, -1.0, 1.0),
                                -1.0..=1.0,
                                2,
                                "",
                                accent_col,
                            );
                        }
                        "builtin:brake" => {
                            title = own_title;
                            icon = "record_stop";
                            let level = link_value(&values, machine, link, "level").unwrap_or(0.0);
                            pod = pod.with_slider(
                                "level",
                                clamp(level, 0.0, 1.0),
                                0.0..=1.0,
                                2,
                                "",
                                accent_col,
                            );
                        }
                        "builtin:trailer_steer" => {
                            title = own_title;
                            icon = "arrow_turn_right";
                            let max = controller.max_steer_deg.unwrap_or(35.0) as f64;
                            let angle = link_value(&values, machine, link, "angle_rad")
                                .unwrap_or(0.0)
                                .to_degrees();
                            pod = pod.with_slider(
                                "angle",
                                clamp(angle, -max, max),
                                -max..=max,
                                1,
                                "°",
                                accent_col,
                            );
                        }
                        "builtin:section_control" => {
                            title = own_title;
                            icon = "grid";
                            let target = controller
                                .target
                                .as_deref()
                                .and_then(|t| machine.links.by_prim(t));
                            let function = target
                                .filter(|l| {
                                    l.element.as_ref().is_some_and(|e| e.kind == "function")
                                })
                                .or_else(|| element_links(machine, "function").into_iter().next());
                            if let Some(f) = function {
                                let on = values
                                    .get(&machine.id, &f.name, "SectionControlState")
                                    .unwrap_or(1.0)
                                    > 0.5;
                                pod = pod
                                    .with_readout("function", f.name.clone())
                                    .with_toggle_initial("section control", accent_col, on);
                                let sections: Vec<&LinkSpec> = element_links(machine, "section")
                                    .into_iter()
                                    .filter(|s| s.parent.as_deref() == Some(f.name.as_str()))
                                    .collect();
                                for s in &sections {
                                    let want = values
                                        .get(&machine.id, &s.name, "SetpointWorkState")
                                        .unwrap_or(0.0)
                                        > 0.5;
                                    pod = pod.with_toggle_initial(s.name.clone(), accent_col, want);
                                }
                                let area =
                                    values.get(&machine.id, &f.name, "TotalArea").unwrap_or(0.0);
                                pod = pod.with_readout("total area", format!("{area:.4} ha"));
                                block.section_links = sections;
                                block.function_link = Some(f);
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
                                .or_else(|| element_links(machine, "bin").into_iter().next());
                            if let Some(b) = bin {
                                let set = values
                                    .get(
                                        &machine.id,
                                        &b.name,
                                        "SetpointVolumePerAreaApplicationRate",
                                    )
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
                                    .with_slider(
                                        "setpoint",
                                        clamp(set, 0.0, 600.0),
                                        0.0..=600.0,
                                        0,
                                        " l/ha",
                                        accent_col,
                                    );
                                block.bin_link = Some(b);
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
                body.add_normal(cid(&group), title, icon, pods);
            }

            let responses = body.render();
            for block in blocks {
                let c = block.controller;
                let Some(resp) = pod_response(&responses, cid(&block.group), block.pod) else {
                    continue;
                };
                let ty = c.controller_type.as_str();
                if DRIVE_TYPES.contains(&ty) {
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
                    let pad_here = panel.gamepad.as_ref() == Some(&block.key);
                    if panel.holding.contains(&block.key) && !pad_here {
                        let speed = resp.sliders.first().map(|s| s.value).unwrap_or(0.0);
                        let yaw = resp.sliders.get(1).map(|s| s.value).unwrap_or(0.0);
                        let stop = button_clicked(&responses, cid(&block.group), block.pod, 0);
                        ui_drive.0.insert(
                            block.key.clone(),
                            if stop {
                                CmdVel {
                                    linear_mps: 0.0,
                                    angular_rps: 0.0,
                                }
                            } else {
                                CmdVel {
                                    linear_mps: speed as f32,
                                    angular_rps: yaw as f32,
                                }
                            },
                        );
                    }
                    continue;
                }
                let changed = |i: usize| resp.sliders.get(i).filter(|s| s.changed).map(|s| s.value);
                let mut set = |name: &str, v: f64| {
                    set_service_value(
                        &mut service,
                        &mut values,
                        machine,
                        &block.key,
                        block.link,
                        name,
                        v,
                    );
                };
                match ty {
                    "builtin:hitch" | "builtin:joint_position" => {
                        if let Some(v) = changed(0) {
                            set("position", v);
                        }
                    }
                    "builtin:pto" => {
                        if let Some(t) = resp.toggles.first()
                            && t.changed
                        {
                            set("engaged", t.on as u8 as f64);
                        }
                    }
                    "builtin:joint_velocity" => {
                        if let Some(v) = changed(0) {
                            set("velocity", v);
                        }
                    }
                    "builtin:hydraulic_valve" => {
                        if let Some(v) = changed(0) {
                            set("flow", v);
                        }
                    }
                    "builtin:brake" => {
                        if let Some(v) = changed(0) {
                            set("level", v);
                        }
                    }
                    "builtin:trailer_steer" => {
                        if let Some(v) = changed(0) {
                            set("angle_rad", v.to_radians());
                        }
                    }
                    "builtin:section_control" => {
                        if let Some(f) = block.function_link {
                            if let Some(t) = resp.toggles.first()
                                && t.changed
                            {
                                values.set(
                                    &machine.id,
                                    &f.name,
                                    "SectionControlState",
                                    t.on as u8 as f64,
                                );
                            }
                            for (i, s) in block.section_links.iter().enumerate() {
                                if let Some(t) = resp.toggles.get(i + 1)
                                    && t.changed
                                {
                                    values.set(
                                        &machine.id,
                                        &s.name,
                                        "SetpointWorkState",
                                        t.on as u8 as f64,
                                    );
                                }
                            }
                        }
                    }
                    "builtin:rate_control" => {
                        if let (Some(b), Some(v)) = (block.bin_link, changed(0)) {
                            values.set(
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
        },
    );
}
