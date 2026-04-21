//! Top-center transport bar — play/pause + speed cycler.
//!
//! Same square-button visual language as the left/right side rails,
//! but anchored to the top edge and laid out horizontally. Buttons:
//!
//!   0. Play / Pause — toggles the physics clock. Glyph and "active"
//!                     tint both flip so you can tell the running
//!                     state at a glance.
//!   1. Speed — cycles 1× → 2× → 4× → 8× → 1×. Label on the glyph
//!              shows the current multiplier; becomes accent-tinted
//!              whenever it's not 1×.

use bevy::prelude::*;
use bevy_egui::EguiContexts;

use crate::viz::{SimClock, SimSpeed};

use super::float;
use super::style::AccentColor;

pub fn transport_bar(
    mut contexts: EguiContexts,
    mut clock: ResMut<SimClock>,
    accent: Res<AccentColor>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let accent_col = accent.0;

    // Read once, apply mutations after the closures — can't
    // multi-borrow `clock` across two `FnOnce`.
    let paused = clock.paused;
    let speed = clock.speed;
    let mut toggle_play = false;
    let mut cycle_speed = false;

    let play_glyph = if paused { "▶" } else { "⏸" };
    let play_tooltip = if paused { "Play  —  resume physics" } else { "Pause  —  freeze physics" };

    float::top_button(
        "transport_play",
        ctx,
        0,
        2,
        play_glyph,
        play_tooltip,
        !paused,
        accent_col,
        || { toggle_play = true; },
    );

    let speed_glyph = speed.label();
    let speed_active = speed != SimSpeed::X1;
    float::top_button(
        "transport_speed",
        ctx,
        1,
        2,
        speed_glyph,
        "Speed  —  click to cycle 1× / 2× / 4× / 8×",
        speed_active,
        accent_col,
        || { cycle_speed = true; },
    );

    if toggle_play {
        clock.paused = !clock.paused;
    }
    if cycle_speed {
        clock.speed = clock.speed.next();
    }
}
