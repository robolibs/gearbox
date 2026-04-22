//! Cargo / implement containers — harvest bunkers, trailer beds, etc.
//!
//! Deliberately simple: one scalar reservoir with a capacity and a
//! current fill. The UI treats the step size (for +/- buttons) and
//! the auto-fill rate as **fractions of capacity** — set capacity
//! to 20 and +/- bumps by 1; set capacity to 1000 and +/- bumps by
//! 50. Same idea for the auto-fill rate slider (0..5% of capacity
//! per second; `0` disables auto-fill entirely).

/// The `+`/`-` buttons bump by this fraction of capacity.
pub const BUMP_FRACTION: f32 = 0.05;

/// A single container on a vehicle.
#[derive(Debug, Clone)]
pub struct Container {
    /// Generic label — the UI uses a short display string ("Cont"),
    /// but presets can still give it a semantic name for debugging /
    /// logs.
    pub label: String,
    pub amount: f32,
    pub capacity: f32,
    /// Auto-fill rate as a **fraction of capacity per second**.
    /// `0.0` disables auto-fill. Valid range `[0, 0.05]`, i.e. up to
    /// 5 %/s — a full bunker in 20 s at max setting.
    pub fill_rate_frac: f32,
}

impl Container {
    pub fn new(label: impl Into<String>, capacity: f32) -> Self {
        Self {
            label: label.into(),
            amount: 0.0,
            capacity: capacity.max(0.0),
            fill_rate_frac: 0.0,
        }
    }

    /// Fractional auto-fill rate (0..0.05) — hooked into the slider
    /// on the Properties panel. `0.0` means no auto-fill.
    pub fn with_fill_rate_frac(mut self, frac: f32) -> Self {
        self.fill_rate_frac = frac.clamp(0.0, 0.05);
        self
    }

    pub fn fraction(&self) -> f32 {
        if self.capacity > 0.0 {
            (self.amount / self.capacity).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// Step size for the `+` / `-` buttons — 5 % of capacity, floored
    /// at 1 so tiny-capacity containers still bump by whole units.
    pub fn bump_step(&self) -> f32 {
        (self.capacity * BUMP_FRACTION).max(1.0)
    }

    /// Adjust the fill by `sign × bump_step()`. Clamped to `[0, capacity]`.
    pub fn bump(&mut self, sign: f32) {
        let step = self.bump_step() * sign.signum();
        self.amount = (self.amount + step).clamp(0.0, self.capacity);
    }

    pub fn empty_out(&mut self) {
        self.amount = 0.0;
    }

    /// Set a new capacity; clamps `amount` down if necessary.
    pub fn set_capacity(&mut self, new_capacity: f32) {
        self.capacity = new_capacity.max(0.0);
        if self.amount > self.capacity {
            self.amount = self.capacity;
        }
    }

    /// Auto-fill tick. Only accrues when the slider is above zero,
    /// the work toggle is on, AND the vehicle is actually moving —
    /// three independent gates.
    pub fn tick_auto_fill(&mut self, dt: f32, work_on: bool, moving: bool) {
        if self.fill_rate_frac <= 0.0 || !work_on || !moving {
            return;
        }
        let delta = self.capacity * self.fill_rate_frac * dt;
        self.amount = (self.amount + delta).clamp(0.0, self.capacity);
    }
}
