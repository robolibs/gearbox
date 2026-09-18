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
    /// Metres this wheel has rolled; it places the tread along the track.
    pub travelled: f32,
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
/// it has to match `Time`'s wrap period.
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
