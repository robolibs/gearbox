//! Power / energy model — battery or fuel tank.
//!
//! Every vehicle has a [`PowerSystem`] with zero or more
//! [`PowerSource`] entries. For machines with **multiple** sources
//! (e.g. Robotti: battery + fuel) the drain is SEQUENTIAL: only the
//! configured `primary` source drains until it hits zero, then the
//! next source takes over, and so on. The vehicle only reports
//! "depleted" once *every* source is empty.
//!
//! A vehicle must be [turned on](PowerSystem::turned_on) before any
//! drain occurs or any controller force is applied — flipping the
//! switch off parks the machine instantly without discharging.
//!
//! Per-source drain has three inputs, summed each tick:
//!   1. **Travel drain** — per-second rate while moving. When idle
//!      (stationary) the rate drops to `IDLE_FRACTION × travel_drain`
//!      so standing still barely sips the reservoir.
//!   2. **Work drain** — per-second rate while the `work` toggle is on.
//!   3. **Resistance** — 0..1 multiplier on the work drain only
//!      (slider today; API-driven environmental variable later).
//!
//! When the active source hits zero the drive controllers zero out
//! engine / hover forces — the vehicle can coast or free-fall under
//! gravity, only the driver's inputs go dead.

/// While the vehicle is stationary we still draw a tiny "idle"
/// current — accessories, ECU, compute. This fraction of
/// `travel_drain` is charged each tick when not moving. Dropped to
/// 1 % so the parked-vs-moving gap is glaringly obvious — 100× the
/// drain rate once you actually start driving.
pub const IDLE_FRACTION: f64 = 0.01;

/// Kind of stored energy. Used purely for presentation — the sim
/// treats every source the same way mechanically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PowerKind {
    #[default]
    Battery,
    Fuel,
}

/// A single energy reservoir. Units are deliberately abstract
/// ("power units"): presets set `capacity` to whatever feels
/// comparable — big machines get more, the base drain rates scale
/// accordingly.
#[derive(Debug, Clone)]
pub struct PowerSource {
    pub kind: PowerKind,
    /// Display name shown in the UI (`"Battery"`, `"Fuel"`, …).
    pub label: String,
    /// Maximum reservoir size. Clamps `current`.
    pub capacity: f64,
    /// Currently available energy. Starts at `capacity` by convention;
    /// `refuel()` resets it.
    pub current: f64,
    /// Per-second base travel drain for this source. Applied while
    /// the vehicle is moving faster than `MOVE_SPEED_THRESHOLD`.
    pub travel_drain: f64,
    /// Per-second work drain for this source. Applied on top of the
    /// travel drain while the `work` toggle is on, multiplied by
    /// `work_resistance`.
    pub work_drain: f64,
}

impl PowerSource {
    pub fn new(kind: PowerKind, label: impl Into<String>, capacity: f64) -> Self {
        Self {
            kind,
            label: label.into(),
            capacity,
            current: capacity,
            travel_drain: 0.0,
            work_drain: 0.0,
        }
    }

    pub fn with_travel_drain(mut self, per_sec: f64) -> Self {
        self.travel_drain = per_sec;
        self
    }

    pub fn with_work_drain(mut self, per_sec: f64) -> Self {
        self.work_drain = per_sec;
        self
    }

    /// Fraction of remaining energy, 0..1. Useful for progress bars.
    pub fn fraction(&self) -> f64 {
        if self.capacity > 0.0 {
            (self.current / self.capacity).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// Is this source empty (to within a small epsilon)?
    pub fn is_depleted(&self) -> bool {
        self.current <= 1e-3
    }
}

/// Whole vehicle's power plant — a list of sources + the work toggle
/// shared across them.
#[derive(Debug, Clone)]
pub struct PowerSystem {
    pub sources: Vec<PowerSource>,
    /// Master switch. The drive controllers treat `turned_on == false`
    /// exactly like "depleted": no drain AND no force applied.
    pub turned_on: bool,
    /// Whether the operator has turned on the "work" function — PTO,
    /// implement, fan, etc. Adds `work_drain` on every active source.
    pub work: bool,
    /// 0..1 multiplier on the work drain. Driven by a UI slider today,
    /// eventually from an environmental-resistance API.
    pub work_resistance: f64,
    /// Index into `sources` of the reservoir that drains first. When
    /// the primary hits zero the next source in order takes over, and
    /// so on, until every source is empty.
    pub primary: usize,
    // --- diagnostics updated by `tick`; read by the Inspector so we
    // can actually SEE what the power system believes on every frame ---
    /// Horizontal speed (m/s) used for the last drain decision.
    pub last_horiz_speed: f64,
    /// Did `tick` classify the last frame as "moving"?
    pub last_moving: bool,
    /// Drain rate applied to the active source last tick (units/sec).
    pub last_drain_rate: f64,
}

impl Default for PowerSystem {
    fn default() -> Self {
        Self {
            sources: Vec::new(),
            // Fresh spawns are ON — flipping to off parks the machine.
            turned_on: true,
            work: false,
            work_resistance: 0.5,
            primary: 0,
            last_horiz_speed: 0.0,
            last_moving: false,
            last_drain_rate: 0.0,
        }
    }
}

impl PowerSystem {
    /// `true` when every source is empty. Controllers also treat
    /// `turned_on == false` as an engine-off state; see
    /// [`is_engine_live`].
    pub fn is_depleted(&self) -> bool {
        !self.sources.is_empty() && self.sources.iter().all(|s| s.is_depleted())
    }

    /// Does the engine have authority this tick? Controllers call
    /// this before writing any drive force. `true` when the machine
    /// is turned on AND at least one source still has energy (or
    /// there are no sources at all — power-gate disabled).
    pub fn is_engine_live(&self) -> bool {
        self.turned_on && !self.is_depleted()
    }

    /// Index of the source currently being drained (primary first,
    /// then the remaining sources in order). `None` if every source
    /// is empty.
    pub fn active_source(&self) -> Option<usize> {
        if self.sources.is_empty() {
            return None;
        }
        let primary = self.primary.min(self.sources.len().saturating_sub(1));
        if !self.sources[primary].is_depleted() {
            return Some(primary);
        }
        // Spill over to the remaining sources in their natural order.
        (0..self.sources.len()).find(|&i| i != primary && !self.sources[i].is_depleted())
    }

    /// Refill every source back to its capacity — "refuel/repower".
    pub fn refuel(&mut self) {
        for s in &mut self.sources {
            s.current = s.capacity;
        }
    }

    /// Advance by `dt` seconds given the vehicle's current world
    /// speed (m/s). No-op when the system has no sources or the
    /// master switch is off.
    ///
    /// Drain tiers:
    ///
    ///   * Stationary → just the tiny idle trickle
    ///     (`travel_drain × IDLE_FRACTION`). Work drain does NOT
    ///     apply when parked — a stationary machine sips energy no
    ///     matter whether the operator left "work" toggled on.
    ///   * Moving, work off → full `travel_drain`.
    ///   * Moving, work on → `travel_drain + work_drain × resistance`.
    ///
    /// So the three observable tiers are (still < moving < working),
    /// which is what the ground feels like in real machines.
    pub fn tick(&mut self, dt: f64, speed_mag: f64, drain_enabled: bool) {
        self.last_horiz_speed = speed_mag;
        if !self.turned_on || self.sources.is_empty() {
            self.last_moving = false;
            self.last_drain_rate = 0.0;
            return;
        }
        let moving = is_moving(speed_mag);
        self.last_moving = moving;
        let Some(idx) = self.active_source() else {
            self.last_drain_rate = 0.0;
            return;
        };
        let src = &mut self.sources[idx];
        let drain = if moving {
            let work_mult = if self.work {
                self.work_resistance.clamp(0.0, 1.0)
            } else {
                0.0
            };
            src.travel_drain + src.work_drain * work_mult
        } else {
            // Parked — only the idle trickle; work toggle doesn't
            // drain when the vehicle isn't actually doing anything.
            src.travel_drain * IDLE_FRACTION
        };
        self.last_drain_rate = drain;
        // World-level "unlimited power" sandbox mode skips the actual
        // decrement so reservoirs stay topped up while the live
        // diagnostics still show what the drain *would* be.
        if drain_enabled {
            src.current = (src.current - drain * dt).max(0.0);
        }
    }
}

/// Below this m/s we treat the vehicle as stationary. Must sit well
/// above idle suspension bounce + physics-solver residuals — the
/// previous 0.05 m/s was low enough that a parked tractor's tiny
/// vertical wobble crossed it and drained fuel at the full moving
/// rate. 0.35 m/s (≈ 1.25 km/h) is slower than a casual walk and is
/// never sustained by a genuinely-parked machine.
pub const MOVE_SPEED_THRESHOLD: f64 = 0.35;

/// Convenience predicate used by both power drain and auto-fill.
///
/// The caller MUST pass **horizontal** speed (ignoring world-Y), not
/// the raw `rb.linvel().length()` — suspension-compression bounce
/// has Y components that shouldn't count as travel.
pub fn is_moving(horizontal_speed: f64) -> bool {
    horizontal_speed > MOVE_SPEED_THRESHOLD
}
