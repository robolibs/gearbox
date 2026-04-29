//! Per-wheel spec for a raycast-based vehicle.

use datapod::Point;

/// Declarative description of a single wheel.
#[derive(Debug, Clone)]
pub struct WheelSpec {
    /// Connection point on the chassis in chassis-local coordinates (metres).
    pub chassis_connection: Point,
    /// Suspension direction in chassis-local coordinates. Typically
    /// `(0, -1, 0)` — straight down.
    pub suspension_dir: Point,
    /// Wheel axle direction in chassis-local coordinates. Typically
    /// `(±1, 0, 0)` — sideways across the chassis.
    pub axle_dir: Point,
    pub suspension_rest_length: f64,
    pub suspension_stiffness: f64,
    pub suspension_damping: f64,
    /// Upper bound on the spring force per wheel. Rapier's default (6000 N)
    /// is enough for ~100-kg arcade cars; heavy vehicles must raise this
    /// or the suspension saturates and the chassis sinks onto its collider.
    pub max_suspension_force: f64,
    pub friction_slip: f64,
    pub radius: f64,
    pub width: f64,
    /// Whether engine torque is applied to this wheel.
    pub driven: bool,
    /// Whether steering input rotates this wheel.
    pub steered: bool,
    pub max_engine_force: f64,
    pub max_brake: f64,
    /// Maximum steering angle in radians (applied at `steer = ±1.0`).
    pub max_steer_rad: f64,
    /// Optional offset (in chassis-local coordinates) from
    /// `chassis_connection` to the STEERING PIVOT — i.e. the physical
    /// kingpin axis, when that axis is offset from the wheel hub
    /// (common on 4WIS gantry robots where the kingpin strut sits
    /// outboard of the tyre). Default `(0, 0, 0)` → wheel pivots
    /// around its own centre, which is standard car behaviour.
    ///
    /// Non-zero values are VISUAL ONLY: the wheel's raycast + engine
    /// force still act at `chassis_connection`, but the rendered
    /// wheel-centre is computed as if it were hanging off a kingpin
    /// at `chassis_connection + steering_pivot_offset`, so the wheel
    /// swings in an arc around that outboard kingpin when steering.
    pub steering_pivot_offset: Point,
    /// USD prim path that should *be* this wheel's visual. When set,
    /// `gearbox-viz` skips spawning the procedural tyre cylinder and
    /// instead detaches the prim entity from the USD scene hierarchy
    /// after instantiation, then drives its world `Transform` from
    /// `wheel_pose()` like a regular `VehicleWheel`. Use this on USD-
    /// backed presets where the asset already authors a wheel mesh
    /// (so the rendered wheel matches the asset, not a Cylinder).
    /// `None` (default) → fall back to the procedural cylinder.
    pub usd_prim_path: Option<&'static str>,
    /// Optional USD prim path of the *steering knuckle* — the parent
    /// link that rotates around the kingpin axis when steering is
    /// applied. Use on assets where the wheel and knuckle are
    /// authored as separate links (e.g. AGROINTELLI Robotti has
    /// `<knuckle>/<wheel>`: the wheel only spins, the knuckle steers).
    /// When `Some(path)`:
    ///   * the spin prim (`usd_prim_path`) only receives the rolling
    ///     rotation (around axle).
    ///   * the steer prim receives the steering rotation (around
    ///     kingpin).
    /// `None` (default) → steer + spin both applied to `usd_prim_path`,
    /// matching the simpler tractor layout.
    pub usd_steer_prim_path: Option<&'static str>,
}

impl Default for WheelSpec {
    fn default() -> Self {
        Self {
            chassis_connection: Point::default(),
            suspension_dir: Point::new(0.0, -1.0, 0.0),
            axle_dir: Point::new(1.0, 0.0, 0.0),
            suspension_rest_length: 0.25,
            suspension_stiffness: 30.0,
            suspension_damping: 4.5,
            max_suspension_force: 50_000.0,
            friction_slip: 5.0,
            radius: 0.34,
            width: 0.22,
            driven: true,
            steered: false,
            max_engine_force: 4000.0,
            max_brake: 20.0,
            max_steer_rad: 0.0,
            steering_pivot_offset: Point::new(0.0, 0.0, 0.0),
            usd_prim_path: None,
            usd_steer_prim_path: None,
        }
    }
}
