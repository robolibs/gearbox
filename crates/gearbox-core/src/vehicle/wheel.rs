//! Per-wheel spec for a physically-simulated vehicle.
//!
//! Each wheel becomes a real rigid body in `gearbox-physics`: a light
//! hub carries the suspension (and steering, if steered) and the wheel
//! body spins on the hub via a revolute joint. Traction comes from the
//! tyre collider's actual contact with the ground.

use datapod::Point;

/// Declarative description of a single wheel.
#[derive(Debug, Clone)]
pub struct WheelSpec {
    /// Connection point on the chassis in chassis-local coordinates (metres).
    pub chassis_connection: Point,
    /// Suspension direction in chassis-local coordinates. Typically
    /// `(0, -1, 0)` — straight down. Also the kingpin (steering) axis.
    pub suspension_dir: Point,
    /// Wheel axle direction in chassis-local coordinates. Typically
    /// `(±1, 0, 0)` — sideways across the chassis.
    pub axle_dir: Point,
    /// Suspension rest length (metres): distance from `chassis_connection`
    /// to the wheel centre when the spring is fully extended.
    pub suspension_rest_length: f64,
    /// Wheel rigid-body mass (kg). Drives the tyre's inertia and the
    /// vehicle's unsprung mass.
    pub mass: f64,
    /// Suspension/steering hub body mass (kg). Small — the hub is an
    /// internal massless-ish carrier.
    pub hub_mass: f64,
    /// Coulomb friction coefficient of the tyre collider against the
    /// ground. ~1.0–1.4 for agricultural tyres.
    pub tire_friction: f64,
    pub radius: f64,
    pub width: f64,
    /// Whether engine torque is applied to this wheel.
    pub driven: bool,
    /// Whether steering input rotates this wheel.
    pub steered: bool,
    /// Maximum tractive force at the contact patch (N) at full throttle.
    /// Converted to wheel torque internally (`force × radius`).
    pub max_engine_force: f64,
    /// Maximum brake torque (N·m) at full brake.
    pub max_brake: f64,
    /// Maximum steering angle in radians (applied at `steer = ±1.0`).
    pub max_steer_rad: f64,
    /// Optional offset (chassis-local) from `chassis_connection` to the
    /// physical kingpin axis. Retained for asset/visual bookkeeping;
    /// the physical model currently steers about the wheel centre.
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
    ///     `None` (default) → steer + spin both applied to `usd_prim_path`,
    ///     matching the simpler tractor layout.
    pub usd_steer_prim_path: Option<&'static str>,
}

impl Default for WheelSpec {
    fn default() -> Self {
        Self {
            chassis_connection: Point::default(),
            suspension_dir: Point::new(0.0, -1.0, 0.0),
            axle_dir: Point::new(1.0, 0.0, 0.0),
            suspension_rest_length: 0.25,
            mass: 30.0,
            hub_mass: 5.0,
            tire_friction: 1.1,
            radius: 0.34,
            width: 0.22,
            driven: true,
            steered: false,
            max_engine_force: 4000.0,
            max_brake: 800.0,
            max_steer_rad: 0.0,
            steering_pivot_offset: Point::new(0.0, 0.0, 0.0),
            usd_prim_path: None,
            usd_steer_prim_path: None,
        }
    }
}
