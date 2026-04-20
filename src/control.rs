//! Control inputs fed into vehicles each tick.

/// Normalized control inputs for a driven vehicle.
#[derive(Debug, Default, Copy, Clone)]
pub struct ControlInput {
    /// Longitudinal command. `-1.0`..=`1.0`. Positive means forward.
    pub throttle: f32,
    /// Brake command. `0.0`..=`1.0`.
    pub brake: f32,
    /// Steering command. `-1.0`..=`1.0`. Positive steers right.
    pub steer: f32,
}

impl ControlInput {
    pub fn throttle(t: f32) -> Self {
        Self { throttle: t.clamp(-1.0, 1.0), ..Self::default() }
    }

    pub fn clamp(self) -> Self {
        Self {
            throttle: self.throttle.clamp(-1.0, 1.0),
            brake: self.brake.clamp(0.0, 1.0),
            steer: self.steer.clamp(-1.0, 1.0),
        }
    }
}
