use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::container::Tab;
use mara_core::pod::{Pod, PodResponse};
use mara_core::shelf::ShelfContainer;
use mara_core::vocab::Id as MaraId;
use std::collections::HashMap;

use super::{PaneCtx, cid, pid, pod_response};
use crate::host::PANE_LOG as P;
use crate::viewer::drive::ControlSettings;
use crate::viewer::state::FollowTarget;

pub fn container(world: &World, ctx: &PaneCtx) -> ShelfContainer<'static> {
    let invert = world.resource::<ControlSettings>().invert_look_y;
    let cinematic = world.resource::<ControlSettings>().cinematic_transitions;
    let locked = world.resource::<FollowTarget>().entity.is_some();
    let pod = pid(P, "controller", 0);
    ctx.sync_toggles(pod, &[invert, cinematic]);

    let controller = Tab::new(cid(P, "controller"), "Controller", "joystick").pods(vec![
        Pod::new(pod)
            .with_toggle_initial("Invert right look stick up/down", ctx.accent, invert)
            .with_toggle_initial("Cinematic machine transitions", ctx.accent, cinematic)
            .with_readout(
                "camera",
                if locked {
                    "Follow locked · ↑ to release"
                } else {
                    "Free · ↑ to follow selection"
                },
            )
            .with_readout(
                "transitions",
                if cinematic {
                    "Cinematic · ↓ to disable"
                } else {
                    "Keep angle / distance · ↓ for cinematic"
                },
            )
            .with_readout("settings", "Saved on this computer")
            .with_keybindings(vec![
                ("Right stick", "Orbit camera"),
                ("Left stick", "Strafe / forward-back (free camera only)"),
                ("R2 / L2", "Raise / lower camera (free camera only)"),
                ("D-pad ← / →", "Switch machine using chosen camera mode"),
                ("D-pad ↑", "Toggle camera follow"),
                ("D-pad ↓", "Toggle cinematic transitions"),
                ("Hold R1 + left stick", "Drive selected machine"),
                ("Release R1", "Stop local drive; allow external control"),
                ("Hold L1", "Machine layer · reserved for internals"),
                ("Cross / A", "Stop; release R1 to re-arm"),
            ]),
    ]);

    let lines = ctx.log.tail(40);
    let events = if lines.is_empty() {
        Pod::new(pid(P, "lines", 0)).with_readout("status", "no events yet")
    } else {
        Pod::new(pid(P, "lines", 0))
            .fill()
            .with_select_list(lines, None, ctx.accent)
    };
    let events_tab = Tab::new(cid(P, "lines"), "Events", "document").pods(vec![events]);

    let keys_tab = Tab::new(cid(P, "keys"), "Keys", "keyboard").pods(vec![
        Pod::new(pid(P, "keys", 0)).with_keybindings(vec![
            ("M", "Machines"),
            ("F", "Scene"),
            ("O", "View"),
            ("?", "Controller and keyboard"),
            ("R", "Reload"),
            ("Ctrl+K", "Command palette"),
            ("G / X", "Ground grid / axes"),
            ("Y / C", "Physics / colliders"),
            ("Esc", "Clear selection"),
        ]),
    ]);

    let camera_tab = Tab::new(cid(P, "camera"), "Camera", "camera").pods(vec![
        Pod::new(pid(P, "camera", 0)).with_keybindings(vec![
            ("Drag", "Orbit"),
            ("Middle drag", "Pan"),
            ("Shift+Middle drag", "Lift"),
            ("Wheel", "Zoom"),
            ("W A S D", "Move over the ground"),
            ("Q / E", "Down / up"),
            ("Shift", "Faster"),
            ("Click", "Select the object under the cursor"),
        ]),
    ]);

    ShelfContainer::tabbed(
        cid(P, "root"),
        "Controller and keyboard",
        "joystick",
        vec![controller, events_tab, keys_tab, camera_tab],
    )
}

pub fn apply(responses: &HashMap<MaraId, Vec<PodResponse>>, world: &mut World, _ctx: &PaneCtx) {
    if let Some(response) = pod_response(responses, cid(P, "controller"), 0) {
        let mut settings = world.resource_mut::<ControlSettings>();
        let mut changed = false;
        for (index, toggle) in response
            .toggles
            .iter()
            .enumerate()
            .filter(|(_, t)| t.changed)
        {
            match index {
                0 => settings.invert_look_y = toggle.on,
                1 => settings.cinematic_transitions = toggle.on,
                _ => continue,
            }
            changed = true;
        }
        if changed && let Err(error) = settings.save() {
            warn!("Controller settings changed for this session but could not be saved: {error}");
        }
    }
}
