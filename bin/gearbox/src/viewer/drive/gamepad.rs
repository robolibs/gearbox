use bevy::prelude::*;
use gearbox_controls::{Frame, Input, Layer, Router, cycle_index};

use super::{DRIVE_TYPES, MachinePanel};
use crate::controller::{CmdVel, ControllerInventory, ControllerKey, UiDrive};
use crate::viewer::commands::{HostCommand, HostCommands};
use crate::viewer::state::{ActiveStage, ChaseCameraFly, FlyTo, FollowTarget};
use crate::viewer::systems::Selection;

#[derive(Resource, Default)]
pub(super) struct GamepadState {
    router: Router,
    device: Option<Entity>,
    selected: Option<Entity>,
    frame: Frame,
}

pub(super) fn route(
    gamepads: Query<(Entity, &Gamepad)>,
    focus: Res<super::HostInputFocus>,
    inventory: Res<ControllerInventory>,
    mut selection: ResMut<Selection>,
    mut active: ResMut<ActiveStage>,
    mut panel: ResMut<MachinePanel>,
    mut ui: ResMut<UiDrive>,
    mut state: ResMut<GamepadState>,
    mut follow: ResMut<FollowTarget>,
    mut commands: ResMut<HostCommands>,
    mut settings: ResMut<super::ControlSettings>,
) {
    let device = state
        .device
        .and_then(|entity| gamepads.get(entity).ok())
        .or_else(|| gamepads.iter().min_by_key(|(entity, _)| entity.to_bits()));
    let device_id = device.map(|(entity, _)| entity);
    if state.device != device_id {
        state.router.require_release();
        state.device = device_id;
    }
    let mut machines: Vec<_> = inventory
        .machines
        .iter()
        .filter(|m| m.scene_root.is_some())
        .collect();
    machines.sort_by(|a, b| a.id.cmp(&b.id).then(a.scene_root.cmp(&b.scene_root)));
    machines.dedup_by_key(|m| m.scene_root);
    let mut root = match selection.0 {
        Some(root) => machines
            .iter()
            .any(|m| m.scene_root == Some(root))
            .then_some(root),
        None => None,
    };
    if state.selected != root {
        state.router.require_release();
    }
    if state.frame.drive_enabled && panel.gamepad.is_none() {
        state.router.require_release();
    }
    let input = device
        .map(|(_, pad)| Input {
            connected: true,
            focused: focus.0,
            r1: pad.pressed(GamepadButton::RightTrigger),
            l1: pad.pressed(GamepadButton::LeftTrigger),
            r2: pad.get(GamepadButton::RightTrigger2).unwrap_or(0.0),
            l2: pad.get(GamepadButton::LeftTrigger2).unwrap_or(0.0),
            stop: pad.pressed(GamepadButton::South),
            left: [
                pad.get(GamepadAxis::LeftStickX).unwrap_or(0.0),
                pad.get(GamepadAxis::LeftStickY).unwrap_or(0.0),
            ],
            right: [
                pad.get(GamepadAxis::RightStickX).unwrap_or(0.0),
                pad.get(GamepadAxis::RightStickY).unwrap_or(0.0),
            ],
            previous_pressed: pad.just_pressed(GamepadButton::DPadLeft),
            next_pressed: pad.just_pressed(GamepadButton::DPadRight),
            follow_pressed: pad.just_pressed(GamepadButton::DPadUp),
            cinematic_pressed: pad.just_pressed(GamepadButton::DPadDown),
        })
        .unwrap_or_default();
    let mut frame = state.router.update(input);
    if frame.toggle_cinematic {
        settings.cinematic_transitions = !settings.cinematic_transitions;
        if let Err(error) = settings.save() {
            warn!(
                "Camera transition setting changed for this session but could not be saved: {error}"
            );
        }
        info!(
            "gearbox-pad: cinematic_transitions={}",
            settings.cinematic_transitions
        );
    }
    if !focus.0 || input.stop || input.l1 {
        for previous in panel.holding.drain() {
            ui.stop_once(&previous);
        }
    }
    if frame.select_step != 0 {
        let current = machines.iter().position(|m| m.scene_root == root);
        if let Some(index) = cycle_index(current, machines.len(), frame.select_step) {
            root = machines[index].scene_root;
            selection.0 = root;
            active.0 = root;
            if let Some(root) = root {
                commands.0.push(HostCommand::FlyToMachine(root));
                if follow.entity.is_some() {
                    follow.set(Some(root));
                }
            }
            frame.camera = Default::default();
            state.router.require_release();
        }
    }
    if frame.toggle_follow {
        let target = if follow.entity.is_some() { None } else { root };
        follow.set(target);
        info!("gearbox-pad: follow={:?}", follow.entity);
    }
    if state.selected != root {
        for previous in panel.holding.drain() {
            ui.stop_once(&previous);
        }
        if let Some(previous) = panel.gamepad.take() {
            ui.stop_once(&previous);
        }
        if follow.entity.is_some() {
            follow.set(root);
        }
        info!("gearbox-pad: selected={root:?}");
    }
    state.selected = root;
    let drive = machines
        .iter()
        .find(|machine| machine.scene_root == root)
        .and_then(|machine| {
            let controller = machine
                .controllers
                .iter()
                .find(|c| c.enabled && DRIVE_TYPES.contains(&c.controller_type.as_str()))?;
            Some((
                ControllerKey::new(root?, &machine.id, &controller.instance),
                controller.controller_type == DRIVE_TYPES[0],
            ))
        });
    if let Some(previous) = panel.gamepad.clone()
        && (!frame.drive_enabled || drive.as_ref().map(|(key, _)| key) != Some(&previous))
    {
        panel.gamepad = None;
        panel.holding.remove(&previous);
        ui.stop_once(&previous);
    }
    if frame.drive_enabled
        && let Some((key, ackermann)) = &drive
    {
        for previous in panel.holding.drain() {
            ui.stop_once(&previous);
        }
        panel.gamepad = Some(key.clone());
        ui.drive(
            key,
            CmdVel {
                linear_mps: frame.throttle * 4.0,
                angular_rps: frame.steering,
            },
            ackermann.then_some(frame.steering),
        );
    }
    let driving = frame.drive_enabled && drive.is_some();
    if panel.layer != frame.layer || panel.deadman_active != driving {
        info!(
            "gearbox-pad: layer={:?} deadman={} selected={root:?}",
            frame.layer, driving
        );
    }
    panel.layer = frame.layer;
    panel.deadman_active = driving;
    state.frame = frame;
}

pub(super) fn camera(
    time: Res<Time>,
    state: Res<GamepadState>,
    settings: Res<super::ControlSettings>,
    follow: Res<FollowTarget>,
    mut fly: ResMut<FlyTo>,
    mut machine_fly: ResMut<ChaseCameraFly>,
    mut view: ResMut<crate::viewer::camera::View>,
    mut log_at: Local<f32>,
) {
    let input = state
        .frame
        .camera
        .for_view(settings.invert_look_y, follow.entity.is_some());
    if state.frame.toggle_follow || (settings.is_changed() && !settings.cinematic_transitions) {
        fly.remaining = 0.0;
        machine_fly.target = None;
    }
    if state.frame.layer != Layer::Camera || !input.active() {
        return;
    }
    fly.remaining = 0.0;
    machine_fly.target = None;
    let dt = time.delta_secs().min(0.1) as f64;
    view.bearing_deg += input.orbit[0] as f64 * 90.0 * dt;
    view.pitch_deg += input.orbit[1] as f64 * 70.0 * dt;
    // The pad walks the view over the ground, like the keys, and at the same
    // share of how high it stands.
    let pace = (view.eye_height_m() * 0.6).clamp(2.0, 200.0) * dt;
    let look = view.bearing_deg + 180.0;
    view.walk(look, input.forward as f64 * pace);
    view.walk(look + 90.0, input.pan as f64 * pace);
    view.at.altitude = (view.at.altitude + input.lift as f64 * pace).max(0.0);
    if std::env::var_os("GEARBOX_CONTROLS_DEBUG").is_some() && time.elapsed_secs() >= *log_at {
        *log_at = time.elapsed_secs() + 1.0;
        info!(
            "gearbox-pad camera: input={input:?} bearing={:.1} pitch={:.1} distance={:.1}",
            view.bearing_deg, view.pitch_deg, view.distance_m
        );
    }
}
