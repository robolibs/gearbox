//! Stage animation playback.

use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::pane::PaneBody;
use mara_core::pod::Pod;

use super::{PaneCtx, button_clicked, cid, pid, pod_response};
use crate::host::PANE_TIMELINE as P;
use crate::viewer::state::UsdStageTime;

pub fn show(body: &mut PaneBody<'_, '_>, world: &mut World, ctx: &PaneCtx) {
    let accent = ctx.accent;
    let clock = *world.resource::<UsdStageTime>();
    let playback_id = cid(P, "playback");
    let duration = clock.duration_seconds().max(0.01);
    body.add_normal(
        playback_id,
        "Playback",
        "play",
        vec![
            Pod::new(pid(P, "playback", 0))
                .with_readout("time code", format!("{:.2}", clock.current_time_code()))
                .with_readout("duration", format!("{:.2} s", clock.duration_seconds()))
                .with_readout(
                    "range",
                    format!(
                        "{:.0} – {:.0} @ {:.0} tc/s",
                        clock.start_time_code, clock.end_time_code, clock.time_codes_per_second
                    ),
                )
                .with_button(if clock.playing { "Pause" } else { "Play" }, accent)
                .with_slider(
                    "seconds",
                    clock.seconds.clamp(0.0, duration),
                    0.0..=duration,
                    2,
                    " s",
                    accent,
                ),
        ],
    );
    let responses = body.render();
    let mut clock = world.resource_mut::<UsdStageTime>();
    if button_clicked(&responses, playback_id, 0, 0) {
        clock.playing = !clock.playing;
    }
    if let Some(resp) = pod_response(&responses, playback_id, 0)
        && let Some(slider) = resp.sliders.first()
        && slider.changed
    {
        clock.seconds = slider.value;
    }
}
