//! Control inputs fed into vehicles each tick.

/// Normalized control inputs for a driven vehicle.
///
/// Ground vehicles read `throttle` / `brake` / `steer`. Drones
/// additionally read `yaw` (rotation around vertical) and `lift`
/// (altitude). Unused fields default to `0.0`, so the same struct
/// feeds every drive mode without special casing.
#[derive(Debug, Default, Copy, Clone)]
pub struct ControlInput {
    /// Longitudinal command. `-1.0`..=`1.0`. Positive means forward.
    pub throttle: f64,
    /// Brake command. `0.0`..=`1.0`.
    pub brake: f64,
    /// Steering command. `-1.0`..=`1.0`. Positive steers right.
    pub steer: f64,
    /// Yaw command for drones. `-1.0`..=`1.0`. Positive = turn left.
    pub yaw: f64,
    /// Altitude command for drones. `-1.0`..=`1.0`. Positive = up.
    pub lift: f64,
}

impl ControlInput {
    pub fn throttle(t: f64) -> Self {
        Self { throttle: t.clamp(-1.0, 1.0), ..Self::default() }
    }

    pub fn clamp(self) -> Self {
        Self {
            throttle: self.throttle.clamp(-1.0, 1.0),
            brake: self.brake.clamp(0.0, 1.0),
            steer: self.steer.clamp(-1.0, 1.0),
            yaw: self.yaw.clamp(-1.0, 1.0),
            lift: self.lift.clamp(-1.0, 1.0),
        }
    }
}
