use bevy::prelude::*;
use mara::ui::mara_core::{pane::PaneBody, pod::Pod};

use super::{PaneCtx, cid, pid, pod_response};
use crate::host::PANE_LOG as P;
use crate::viewer::drive::ControlSettings;
use crate::viewer::state::FollowTarget;

pub fn show(body: &mut PaneBody<'_, '_>, world: &mut World, ctx: &PaneCtx) {
    let invert = world.resource::<ControlSettings>().invert_right_y;
    let locked = world.resource::<FollowTarget>().entity.is_some();
    let pod = pid(P, "controller", 0);
    ctx.sync_toggles(pod, &[invert]);
    body.add_normal(
        cid(P, "controller"),
        "Controller",
        "joystick",
        vec![
            Pod::new(pod)
                .with_toggle_initial("Invert right stick up/down", ctx.accent, invert)
                .with_readout(
                    "camera",
                    if locked {
                        "Follow locked · ↓ to release"
                    } else {
                        "Free · ↑ to follow selection"
                    },
                )
                .with_readout("settings", "Saved on this computer")
                .with_keybindings(vec![
                    ("Right stick", "Orbit camera"),
                    ("Left stick", "Strafe / forward-back (free camera only)"),
                    ("R2 / L2", "Raise / lower camera (free camera only)"),
                    (
                        "D-pad ← / →",
                        "Select previous / next machine and fly to it",
                    ),
                    ("D-pad ↑", "Lock follow to selected machine"),
                    ("D-pad ↓", "Release follow; free camera"),
                    ("Hold R1 + left stick", "Drive selected machine"),
                    ("Release R1", "Stop local drive; allow external control"),
                    ("Hold L1", "Machine layer · reserved for internals"),
                    ("Cross / A", "Stop; release R1 to re-arm"),
                ]),
        ],
    );
    super::log::show(body, ctx);
    let responses = body.render();
    if let Some(toggle) = pod_response(&responses, cid(P, "controller"), 0)
        .and_then(|r| r.toggles.first())
        .filter(|t| t.changed)
    {
        let mut settings = world.resource_mut::<ControlSettings>();
        settings.invert_right_y = toggle.on;
        if let Err(error) = settings.save() {
            warn!("Controller settings changed for this session but could not be saved: {error}");
        }
    }
}
