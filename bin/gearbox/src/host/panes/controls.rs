use bevy::prelude::*;
use mara::ui::mara_core::{pane::PaneBody, pod::Pod};

use super::{PaneCtx, cid, pid, pod_response};
use crate::host::PANE_LOG as P;
use crate::viewer::drive::ControlSettings;
use crate::viewer::state::FollowTarget;

pub fn show(body: &mut PaneBody<'_, '_>, world: &mut World, ctx: &PaneCtx) {
    let invert = world.resource::<ControlSettings>().invert_look_y;
    let cinematic = world.resource::<ControlSettings>().cinematic_transitions;
    let locked = world.resource::<FollowTarget>().entity.is_some();
    let pod = pid(P, "controller", 0);
    ctx.sync_toggles(pod, &[invert, cinematic]);
    body.add_normal(
        cid(P, "controller"),
        "Controller",
        "joystick",
        vec![
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
        ],
    );
    super::log::show(body, ctx);
    let responses = body.render();
    if let Some(response) = pod_response(&responses, cid(P, "controller"), 0) {
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
