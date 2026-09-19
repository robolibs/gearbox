//! Wheel-contact collection shared by field surface packages.

use bevy::prelude::*;
use bevy::render::extract_resource::ExtractResource;

/// One wheel on the ground this frame: where, which way it rolls, how wide.
#[derive(Clone, Copy, Debug)]
pub struct WheelContact {
    pub position: Vec3,
    pub direction: Vec2,
    pub width: f32,
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
/// it has to match `Time`'s wrap period.
pub const TRAMPLE_CLOCK_S: f32 = 3600.0;

/// Encodes a stamp: time on the wrapped clock and roll direction as an angle;
/// 0 in the angle channel means never stamped.
pub fn trample_texel(now: f32, direction: Vec2) -> [u16; 2] {
    // Floored so a stamp never lands ahead of the shader clock: a stamp in
    // the future would read as a whole clock period old and spring back up.
    let time = ((now.rem_euclid(TRAMPLE_CLOCK_S) / TRAMPLE_CLOCK_S) * 65535.0).floor() as u16;
    let angle = (direction.y.atan2(direction.x) + std::f32::consts::PI) / std::f32::consts::TAU;
    let angle = ((angle * 65534.0).round() as u16).saturating_add(1);
    [time, angle]
}

pub(super) fn begin_wheel_contacts(mut contacts: ResMut<WheelContacts>, time: Res<Time>) {
    contacts.contacts.clear();
    contacts.now = time.elapsed_secs_wrapped();
}
