//! Effort caps, normalized endpoint mapping and controller cadence.

/// The four bounds on a DOF's force or torque; None adds no bound.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CapPolicy {
    /// The authored drive's `maxForce`; fixed for the generation.
    pub authored: Option<f64>,
    /// A declared hardware rating.
    pub hardware: Option<f64>,
    /// The owner's requested operating cap.
    pub requested: Option<f64>,
    /// The host's safety cap.
    pub safety: Option<f64>,
}

impl CapPolicy {
    /// The minimum of the applicable bounds; None when all are absent.
    pub fn effective(&self) -> Option<f64> {
        [self.authored, self.hardware, self.requested, self.safety]
            .into_iter()
            .flatten()
            .map(|v| v.max(0.0))
            .reduce(f64::min)
    }
}

/// `q(u) = q0 + u (q1 - q0)`, `u` clamped to [0, 1].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NormalizedRange {
    pub at_zero: f64,
    pub at_one: f64,
}

impl NormalizedRange {
    pub fn coordinate(&self, u: f64) -> f64 {
        let u = if u.is_finite() { u.clamp(0.0, 1.0) } else { 0.0 };
        self.at_zero + u * (self.at_one - self.at_zero)
    }
}

/// A controller's due ticks at `rate_hz` on a `physics_hz` clock, by a
/// simulation-time phase accumulator: deterministic for non-integer ratios.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cadence {
    rate_hz: f64,
    physics_hz: f64,
    phase: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CadenceError {
    NotPositive,
    AbovePhysics,
}

impl Cadence {
    pub fn new(rate_hz: f64, physics_hz: f64) -> Result<Self, CadenceError> {
        if !(rate_hz.is_finite() && rate_hz > 0.0 && physics_hz.is_finite() && physics_hz > 0.0) {
            return Err(CadenceError::NotPositive);
        }
        if rate_hz > physics_hz + 1e-9 {
            return Err(CadenceError::AbovePhysics);
        }
        Ok(Self { rate_hz, physics_hz, phase: 1.0 })
    }

    /// Advance one physics tick; whether the controller runs on it. The first
    /// tick is always due.
    pub fn tick(&mut self) -> bool {
        if self.phase >= 1.0 - 1e-9 {
            self.phase -= 1.0;
            self.phase += self.rate_hz / self.physics_hz;
            true
        } else {
            self.phase += self.rate_hz / self.physics_hz;
            false
        }
    }

    pub fn rate_hz(&self) -> f64 {
        self.rate_hz
    }

    /// Start over: the next tick is due.
    pub fn reset(&mut self) {
        self.phase = 1.0;
    }
}
