//! Wheel-contact collection shared by field surface packages.

use bevy::prelude::*;
use bevy::render::extract_resource::ExtractResource;

/// One wheel on the ground this frame: where, which way it rolls, how wide.
#[derive(Clone, Copy, Debug)]
pub struct WheelContact {
    pub position: Vec3,
    pub direction: Vec2,
    pub width: f32,
    /// How hard the tyre works the ground, 0..1: faint rolling straight,
    /// strong when it spins, slides sideways or twists through a turn.
    pub scrub: f32,
    /// Metres the *machine* has rolled; it places the tread along the track.
    pub travelled: f32,
    /// Where the machine was when it had rolled that far, in the same metres as
    /// `position`. The tread is measured from here and not from the wheel,
    /// because a tractor's wheels reach the same ground at different moments —
    /// the rear one a wheelbase later — and measured from each wheel they stamp
    /// phases a wheelbase apart onto the same texels. Measured from one place
    /// that moves with the machine, every wheel writes the same number for the
    /// same ground. The arm is short, a wheelbase at most, which matters: it
    /// multiplies any wobble of the heading, and from a far-off origin a half
    /// degree of steering correction would slide the whole tread by pitches.
    pub anchor: Vec2,
    /// The line this wheel runs down, as a place on it, with the frame-to-frame
    /// jitter of the contact taken out of it sideways. What the tread is
    /// measured across from, and it has to be steady: the contact itself moves
    /// a centimetre or two each frame as the physics settles, every texel keeps
    /// whichever frame last touched it, and since the bars are placed from the
    /// distance off the tyre's middle, the whole pattern steps sideways along
    /// the line where one frame's stamp meets the next's.
    pub centreline: Vec2,
    /// Physical rolling-direction footprint length; None uses the field default.
    pub length: Option<f32>,
}

impl WheelContact {
    pub fn footprint_length(&self, fallback: f32) -> f32 {
        self.length
            .filter(|v| v.is_finite() && *v >= 0.0)
            .unwrap_or(fallback)
    }
}

#[test]
fn explicit_wheel_footprint_overrides_field_default() {
    let mut wheel = WheelContact {
        position: Vec3::ZERO,
        direction: Vec2::X,
        width: 0.4,
        scrub: 0.0,
        travelled: 0.0,
        anchor: Vec2::ZERO,
        centreline: Vec2::ZERO,
        length: None,
    };
    assert_eq!(wheel.footprint_length(0.3), 0.3);
    for length in [0.0, 0.2, 0.6] {
        wheel.length = Some(length);
        assert_eq!(wheel.footprint_length(0.3), length);
    }
    wheel.length = Some(f32::NAN);
    assert_eq!(wheel.footprint_length(0.3), 0.3);
}

/// Wheel contacts collected by the controllers during one frame, stamped
/// into the trample map by the render world.
#[derive(Resource, ExtractResource, Clone, Default)]
pub struct WheelContacts {
    pub contacts: Vec<WheelContact>,
    /// `Time::elapsed_secs_wrapped` this frame, the clock the stamps carry.
    pub now: f32,
}

/// Wheel contacts are stamped into a wrapped-time clock with this period;
/// it has to match `Time`'s wrap period. It is also the ceiling on how long any
/// mark can be read for: past it a stamp wraps and an old mark reads as a fresh
/// one. `SCAR_SECONDS` in `shaders/interaction.wgsl`, the slow clock a wheel's
/// bare scar greens over on, is held under this with room to spare for that
/// reason — a scar wanting longer than an hour needs a map of its own, not a
/// larger number here.
pub const TRAMPLE_CLOCK_S: f32 = 3600.0;

/// Encodes a stamp: time on the wrapped clock, then one channel packing the
/// roll direction (8 bits), where across the tyre the texel lies (4 bits,
/// -1..1) and the scrub (4 bits, never 0, so 0 means never stamped).
pub fn trample_texel(now: f32, direction: Vec2, across: f32, scrub: f32) -> [u16; 2] {
    // Floored so a stamp never lands ahead of the shader clock: a stamp in
    // the future would read as a whole clock period old and spring back up.
    let time = ((now.rem_euclid(TRAMPLE_CLOCK_S) / TRAMPLE_CLOCK_S) * 65535.0).floor() as u16;
    let angle = (direction.y.atan2(direction.x) + std::f32::consts::PI) / std::f32::consts::TAU;
    let angle = (angle * 255.0).round() as u16 & 255;
    let across = ((across.clamp(-1.0, 1.0) * 0.5 + 0.5) * 15.0).round() as u16;
    let scrub = ((scrub.clamp(0.0, 1.0) * 15.0).round() as u16).max(1);
    [time, angle << 8 | across << 4 | scrub]
}

pub(super) fn begin_wheel_contacts(mut contacts: ResMut<WheelContacts>, time: Res<Time>) {
    contacts.contacts.clear();
    contacts.now = time.elapsed_secs_wrapped();
}

/// Tread coordinates are stored in these steps; a lug pitch is 48 of the
/// first, and the rolled distance wraps on a whole number of pitches.
pub const TREAD_ALONG_STEP_M: f32 = 0.004;
pub const TREAD_ACROSS_STEP_M: f32 = 0.005;
const TREAD_ALONG_WRAP: f32 = 65520.0;

/// The two extra channels of a tread-printing map: how far the wheel had
/// rolled at this texel, then its signed offset from the tyre's centreline
/// (high byte, biased by 128) with the tyre's half width (low byte).
pub fn tread_texel(along_m: f32, across_m: f32, half_width_m: f32) -> [u16; 2] {
    let along = (along_m / TREAD_ALONG_STEP_M).round().rem_euclid(TREAD_ALONG_WRAP) as u16;
    let across = ((across_m / TREAD_ACROSS_STEP_M).round() + 128.0).clamp(0.0, 255.0) as u16;
    let half_width = (half_width_m / TREAD_ACROSS_STEP_M).round().clamp(1.0, 255.0) as u16;
    [along, across << 8 | half_width]
}
