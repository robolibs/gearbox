//! Top-center transport bar — play/pause + speed cycler.
//!
//! Now driven by `bevy_frost`'s assembly API: the transport ribbon
//! is a `Centered`, `Icon`-role ribbon declared alongside the side
//! rails (see [`super::dock_ribbons`]). Buttons are rebuilt each
//! frame so their glyphs and active state can reflect the current
//! sim-clock:
//!
//!   0. Play / Pause — toggles the physics clock. Glyph flips
//!      between `▶` and `⏸`; active tint means currently running.
//!   1. Speed — cycles `1× → 2× → 4× → 8× → 1×`. Glyph shows the
//!      current multiplier; active tint means ≠ 1×.

use bevy::prelude::*;
use bevy_egui::EguiContexts;

use bevy_frost::{
    draw_assembly, RibbonCluster, RibbonDrag, RibbonItem, RibbonOpen, RibbonPlacement,
};
use gearbox_viz::{SimClock, SimSpeed};

use super::dock_ribbons::{RIBBONS, RIBBON_TRANSPORT};
use super::style::AccentColor;

const ID_PLAY: &str = "transport_play";
const ID_SPEED: &str = "transport_speed";

pub fn transport_bar(
    mut contexts: EguiContexts,
    mut clock: ResMut<SimClock>,
    accent: Res<AccentColor>,
    mut open: ResMut<RibbonOpen>,
    mut placement: ResMut<RibbonPlacement>,
    mut drag: ResMut<RibbonDrag>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let accent_col = accent.0;

    let paused = clock.paused;
    let speed = clock.speed;

    // Build items per-frame so glyphs can reflect live state. Both
    // glyphs come from `&'static str` pools, so the items remain
    // 'static-safe even though we pick them at runtime.
    let play_glyph: &'static str = if paused { "▶" } else { "⏸" };
    let play_tooltip: &'static str = if paused {
        "Play  —  resume physics"
    } else {
        "Pause  —  freeze physics"
    };
    let speed_glyph: &'static str = match speed {
        SimSpeed::X1 => "1×",
        SimSpeed::X2 => "2×",
        SimSpeed::X4 => "4×",
        SimSpeed::X8 => "8×",
    };

    let items = [
        RibbonItem {
            id: ID_PLAY,
            ribbon: RIBBON_TRANSPORT,
            cluster: RibbonCluster::Middle,
            slot: 0,
            glyph: play_glyph,
            tooltip: play_tooltip,
            child_ribbon: None,
        },
        RibbonItem {
            id: ID_SPEED,
            ribbon: RIBBON_TRANSPORT,
            cluster: RibbonCluster::Middle,
            slot: 1,
            glyph: speed_glyph,
            tooltip: "Speed  —  click to cycle 1× / 2× / 4× / 8×",
            child_ribbon: None,
        },
    ];

    // Active state is per-item: Play active when NOT paused,
    // Speed active when multiplier ≠ 1×.
    let speed_active = speed != SimSpeed::X1;
    let active = move |id: &'static str| -> bool {
        match id {
            ID_PLAY => !paused,
            ID_SPEED => speed_active,
            _ => false,
        }
    };

    let clicks = draw_assembly(
        ctx,
        accent_col,
        RIBBONS,
        &items,
        &mut open,
        &mut placement,
        &mut drag,
        active,
    );

    // Dispatch Icon clicks — Panel toggles are handled inside
    // `draw_assembly`; Icon clicks come out as events for us.
    for c in clicks {
        match c.item {
            ID_PLAY => clock.paused = !clock.paused,
            ID_SPEED => clock.speed = clock.speed.next(),
            _ => {}
        }
    }
}
