//! Chassis description — the rigid body at the core of a vehicle.

use datapod::{Point, Size};

/// Declarative description of a vehicle chassis.
#[derive(Debug, Clone)]
pub struct ChassisSpec {
    /// Full-size bounding box dimensions `(width, height, length)` in metres.
    pub size: Size,
    /// Total mass in kilograms.
    pub mass: f32,
    /// Local center-of-mass offset. A negative Y lowers the COM and reduces
    /// rollover; this matters a lot for a tall tractor on raycast wheels.
    pub com_offset: Point,
    pub linear_damping: f32,
    pub angular_damping: f32,
    /// Enable continuous collision detection on the chassis — prevents
    /// tunneling through walls at high speed.
    pub ccd: bool,
}

impl Default for ChassisSpec {
    fn default() -> Self {
        Self {
            size: Size::new(1.8, 0.8, 4.0),
            mass: 1400.0,
            com_offset: Point::new(0.0, -0.2, 0.0),
            linear_damping: 0.1,
            angular_damping: 0.7,
            ccd: true,
        }
    }
}
