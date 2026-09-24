use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::container::Tab;
use mara_core::pod::{Pod, PodResponse};
use mara_core::vocab::Id as MaraId;
use std::collections::HashMap;

use super::{PaneCtx, cid, pid, pod_response};
use crate::host::PANE_LOG as P;
use crate::viewer::drive::ControlSettings;
use crate::viewer::state::FollowTarget;

pub fn tab(world: &World, ctx: &PaneCtx) -> Tab {
    let invert = world.resource::<ControlSettings>().invert_look_y;
    let cinematic = world.resource::<ControlSettings>().cinematic_transitions;
    let locked = world.resource::<FollowTarget>().entity.is_some();
    let pod = pid(P, "controller", 0);
    ctx.sync_toggles(pod, &[invert, cinematic]);

    let controller_pod = Pod::new(pod)
        .with_toggle_initial("Invert right look stick up/down", ctx.accent, invert)
        .with_toggle_initial("Cinematic machine transitions", ctx.accent, cinematic)
        .with_readout(
            "camera",
            if locked {
                "Following · ↑ to release"
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
        .with_readout("settings", "Saved on this computer");

    let lines = ctx.log.tail(40);
    let events_pod = if lines.is_empty() {
        Pod::new(pid(P, "lines", 0)).with_readout("status", "no events yet")
    } else {
        Pod::new(pid(P, "lines", 0))
            .fill()
            .with_select_list(lines, None, ctx.accent)
    };

    Tab::new(cid(P, "controller"), "Controller", "joystick").pods(vec![controller_pod, events_pod])
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
