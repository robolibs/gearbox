//! Gamepad input via `gilrs`, merged with keyboard WASD in
//! `viz/input.rs`.
//!
//! `gilrs` owns a platform-native event loop (evdev on Linux, XInput
//! on Windows, HID on macOS). We poll it each frame on the Bevy
//! `Update` schedule and cache the latest stick + trigger state into
//! the [`GamepadState`] resource — the input system reads that and
//! overlays it onto the keyboard-derived `ControlInput`.
//!
//! Mapping (standard XInput / DualShock layout):
//!
//!   | Stick / button       | ControlInput field |
//!   |----------------------|--------------------|
//!   | left stick Y         | throttle (+up)     |
//!   | left stick X         | steer (-right)     |
//!   | right stick X        | yaw (-right)       |
//!   | right stick Y        | lift (+up)         |
//!   | left trigger (LT)    | brake              |
//!   | right trigger (RT)   | throttle (+)       |
//!
//! Keyboard input remains authoritative when both are active — we
//! take the larger-magnitude value per-axis, so you can hit a key for
//! a precise value even with the stick slightly off-centre.

use bevy::prelude::*;
use gilrs::{Axis, Button, Event, EventType, GamepadId, Gilrs};

/// Owning the live `Gilrs` handle so its background thread / event
/// socket stays alive across frames. `gilrs` internally uses
/// `std::sync::mpsc::Receiver` which isn't `Sync`, so we install this
/// as a Bevy **non-send** resource (main-thread-only) — which is
/// exactly what platform input APIs want anyway.
pub struct GamepadCtx {
    /// `None` when `gilrs::Gilrs::new` failed on startup — we fall
    /// back to keyboard-only in that case.
    pub inner: Option<Gilrs>,
    /// Captured error message from the `Gilrs::new()` attempt —
    /// `None` if it succeeded. Surfaced in the Properties panel so
    /// "no controllers" vs "backend dead on arrival" is obvious at a
    /// glance (most common cause on Linux: user isn't in the `input`
    /// group, so `/dev/input/event*` can't be opened).
    pub init_error: Option<String>,
}

impl Default for GamepadCtx {
    fn default() -> Self {
        match Gilrs::new() {
            Ok(gilrs) => Self {
                inner: Some(gilrs),
                init_error: None,
            },
            Err(e) => {
                let msg = format!("{}", e);
                bevy::log::warn!(
                    "gilrs init failed: {msg}\n\
                     On Linux, add your user to the `input` group and log out/in:\n\
                     sudo usermod -aG input $USER"
                );
                Self {
                    inner: None,
                    init_error: Some(msg),
                }
            }
        }
    }
}

/// Most-recent axis + button state from the currently-selected
/// gamepad, in ControlInput-style signed normalised values. Zeroed
/// when no gamepad is connected / selected.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct GamepadState {
    pub throttle: f32,
    pub steer: f32,
    pub yaw: f32,
    pub lift: f32,
    pub brake: f32,
    #[allow(dead_code)]
    pub connected: bool,
}

/// One entry in the list of detected controllers — id + display name.
#[derive(Debug, Clone)]
pub struct GamepadInfo {
    pub id: GamepadId,
    pub name: String,
}

/// UI-facing gamepad selection state. Refreshed every frame by
/// `poll_gamepad_system`; the Properties panel reads `detected` to
/// render the chooser, and writes `selected` when the user picks one.
#[derive(Resource, Default, Debug, Clone)]
pub struct GamepadSelection {
    /// All connected controllers this frame. Stable across ticks for
    /// the duration a given gamepad stays plugged in.
    pub detected: Vec<GamepadInfo>,
    /// Chosen gamepad. `None` = auto-pick the first connected one
    /// (legacy default). `Some(id)` = use exactly this controller;
    /// falls back to auto if that id disconnects.
    pub selected: Option<GamepadId>,
    /// Surface-level copy of `GamepadCtx::init_error` — populated on
    /// the first poll. Lets the Properties panel (which only sees
    /// Bevy `Resource`s) tell the user why the gamepad backend is
    /// dead.
    pub init_error: Option<String>,
}

/// Drain pending events from gilrs and refresh [`GamepadState`].
///
/// We read the POST-drain axis/button values off the last seen
/// gamepad rather than trying to accumulate per-event deltas — gilrs
/// already maintains the authoritative state.
pub fn poll_gamepad_system(
    mut ctx: NonSendMut<GamepadCtx>,
    mut state: ResMut<GamepadState>,
    mut selection: ResMut<GamepadSelection>,
) {
    // Propagate the one-shot init error into the resource so the
    // Properties panel (which only takes `Res`) can render a hint.
    if selection.init_error.is_none() {
        selection.init_error = ctx.init_error.clone();
    }
    let Some(gilrs) = ctx.inner.as_mut() else {
        *state = GamepadState::default();
        selection.detected.clear();
        return;
    };

    // Drain events so internal state is current. We don't actually
    // act on the event stream — just on the final post-drain state.
    while let Some(Event { event, .. }) = gilrs.next_event() {
        // Could be used later for "just-pressed" triggers; currently
        // the input mapping is purely analog so ignore.
        let _ = event_is_interesting(&event);
    }

    // Refresh the detected list so the Properties panel's chooser
    // always shows the live set of controllers.
    selection.detected.clear();
    for (id, gp) in gilrs.gamepads() {
        if gp.is_connected() {
            selection.detected.push(GamepadInfo {
                id,
                name: gp.name().to_string(),
            });
        }
    }

    // Pick which controller actually drives this frame:
    //   - if the user has explicitly selected one and it's still
    //     connected, honour it;
    //   - otherwise, pick the first connected controller ("auto").
    // We look the Gamepad up by id from gilrs after deciding.
    let chosen_id = selection
        .selected
        .filter(|id| selection.detected.iter().any(|gi| gi.id == *id))
        .or_else(|| selection.detected.first().map(|gi| gi.id));

    let Some(gp_id) = chosen_id else {
        *state = GamepadState::default();
        return;
    };
    let gp = gilrs.gamepad(gp_id);

    // Per-stick circular deadzone + disc-to-square mapping.
    // See `stick_to_square` below for the "why".
    let (ls_x, ls_y) = stick_to_square(
        gp.value(Axis::LeftStickX),
        gp.value(Axis::LeftStickY),
    );
    let (rs_x, rs_y) = stick_to_square(
        gp.value(Axis::RightStickX),
        gp.value(Axis::RightStickY),
    );
    // Triggers rest near -1 on some SDL mappings, 0 on others — clamp
    // handles either.
    let lt = gp.value(Axis::LeftZ).clamp(0.0, 1.0);

    *state = GamepadState {
        throttle: ls_y,
        steer: -ls_x, // stick right (+X) pivots vehicle right (−steer)
        yaw: -rs_x,
        lift: rs_y,
        brake: lt.max(if gp.is_pressed(Button::South) { 1.0 } else { 0.0 }),
        connected: true,
    };
}

/// Merge keyboard and gamepad values — whichever has larger magnitude
/// wins per axis. Called by `viz/input.rs` to produce the final
/// `ControlInput`.
pub fn merge_axis(keyboard: f32, gamepad: f32) -> f32 {
    if gamepad.abs() > keyboard.abs() {
        gamepad
    } else {
        keyboard
    }
}

fn event_is_interesting(e: &EventType) -> bool {
    matches!(
        e,
        EventType::AxisChanged(_, _, _)
            | EventType::ButtonChanged(_, _, _)
            | EventType::ButtonPressed(_, _)
            | EventType::ButtonReleased(_, _)
    )
}

/// Map a physical analog-stick reading onto a square output region.
///
/// Why: every consumer controller has a **circular** mechanical gate,
/// which means pushing the stick diagonally clamps to `x² + y² ≤ 1`.
/// At 45° the raw reading maxes at (±0.707, ±0.707), not (±1, ±1).
/// For a game that treats the two axes as independent commands
/// (throttle vs steer, throttle vs yaw, …) this feels wrong — you
/// can't hold full throttle AND turn hard. What racing games and
/// most shooters do instead is map the disc onto a square: the
/// stick's "fraction pushed" is preserved, but each axis's scale is
/// expanded by `1 / max(|cos θ|, |sin θ|)` so full diagonal
/// deflection reaches the square's corner, i.e. (±1, ±1).
///
/// Concretely: output = `input × (r / max(|x|, |y|))` where
/// `r = √(x² + y²)`.
///
/// The function also applies a **radial deadzone**: if the stick's
/// magnitude is under `DEADZONE`, the output snaps to zero; anything
/// above is rescaled to start from 0 at the deadzone ring rather
/// than jumping up to `DEADZONE` instantly.
fn stick_to_square(raw_x: f32, raw_y: f32) -> (f32, f32) {
    const DEADZONE: f32 = 0.10;

    let r = (raw_x * raw_x + raw_y * raw_y).sqrt();
    if r < DEADZONE {
        return (0.0, 0.0);
    }

    // Rescale the magnitude so the output starts at 0 at the edge of
    // the deadzone and hits 1 at the stick's full circular deflection.
    let r_scaled = ((r - DEADZONE) / (1.0 - DEADZONE)).min(1.0);

    // Direction unit vector.
    let nx = raw_x / r;
    let ny = raw_y / r;

    // Disc → square: scale so full-diagonal push (nx = ny = √½)
    // maps to ±1 on each axis; full-cardinal push (one of nx,ny is 1)
    // is identity.
    let corner_factor = 1.0 / nx.abs().max(ny.abs());
    let mag = r_scaled * corner_factor;

    // Per-axis clamp — safety net in case the raw input overshoots
    // the unit disc (shouldn't happen on a working stick but cheap).
    (
        (nx * mag).clamp(-1.0, 1.0),
        (ny * mag).clamp(-1.0, 1.0),
    )
}
