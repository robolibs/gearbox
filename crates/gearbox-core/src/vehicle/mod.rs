//! Vehicle data model (pure — no physics backend).
//!
//! A [`VehicleSpec`] is a declarative description: chassis box + list of
//! raycast wheels + decorative body parts + drive mode + power sources +
//! containers. It carries no rapier handles and no live state, so it can
//! be serialised, diff'd, loaded from a file, or handed to a non-physics
//! tool without pulling the physics crate.
//!
//! The *live*, rapier-backed counterpart is
//! [`gearbox_physics::VehicleState`].

pub mod builder;
pub mod chassis;
pub mod container;
pub mod mesh;
pub mod part;
pub mod parts_lib;
pub mod power;
pub mod wheel;

pub use builder::VehicleBuilder;
pub use chassis::ChassisSpec;
pub use container::Container;
pub use mesh::MeshSource;
pub use part::{PartKind, PartSpec};
pub use power::{PowerKind, PowerSource, PowerSystem};
pub use wheel::WheelSpec;

/// Opaque identifier handed back by `gearbox_physics::Sim::spawn_vehicle`.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct VehicleId(pub u32);

/// How the vehicle is driven.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DriveMode {
    /// Ackermann steering — `steer` rotates the wheels flagged
    /// `steered: true` to a common turn-centre angle. Standard car /
    /// tractor behaviour.
    Ackermann,
    /// Differential drive — `steer` biases left-side vs right-side
    /// wheel *throttle* instead of a steering angle. Used for
    /// skid-steer / tracked robots (e.g. Husky). Wheels' `steered`
    /// flag is ignored; `driven` still applies to decide which
    /// wheels get engine force.
    Differential,
    /// Arcade drone — no wheels. Forces and torques are applied
    /// directly to the chassis rigid body:
    ///   - `throttle` → force along the drone's local forward axis,
    ///   - `steer`    → force along the drone's local right axis,
    ///   - `lift`     → vertical force (in addition to a constant
    ///                  `mass × g` hover force that cancels gravity),
    ///   - `yaw`      → torque around world +Y.
    /// Stabilising damping keeps the drone hover-steady.
    Drone,
    /// 4-wheel independent steering (4WIS / omni). Every wheel's
    /// steering angle is computed per-tick so the combined wheel
    /// velocities match:
    ///   - `throttle` → forward/back body velocity,
    ///   - `steer`    → lateral (strafe) body velocity,
    ///   - `yaw`      → rotational body rate around +Y.
    /// Used for gantry-style field robots like AGROINTELLI Robotti.
    Omni,
}

impl Default for DriveMode {
    fn default() -> Self {
        DriveMode::Ackermann
    }
}

/// A declarative vehicle: chassis box + list of raycast wheels + any
/// attached body parts (hitches, karosseries, tanks).
#[derive(Debug, Clone)]
pub struct VehicleSpec {
    pub name: String,
    pub chassis: ChassisSpec,
    pub wheels: Vec<WheelSpec>,
    /// Extra visual body parts. Parented to the chassis entity by the
    /// viz layer, so they ride the chassis pose automatically.
    pub parts: Vec<PartSpec>,
    /// How `ControlInput::steer` is interpreted.
    pub drive_mode: DriveMode,
    /// Battery / fuel reservoir(s). Drive controllers zero out engine
    /// forces when any source is depleted. Empty `sources` means the
    /// vehicle has no power gate — every control always works.
    pub power: PowerSystem,
    /// Cargo / implement containers (grain bunker, bale trailer,
    /// fertiliser hopper…). Empty `containers` hides the Container
    /// section in the Properties panel.
    pub containers: Vec<Container>,
    /// Nominal top speed in m/s for this vehicle. Not a physics cap —
    /// the physics sim has no explicit speed limiter. Exists purely
    /// as a *display* value: speed-dependent UI effects (heading-
    /// arrow intensity, debug overlays) normalize the current speed
    /// against this so a slow field robot at its own top speed reads
    /// as "flat-out" the same way a road car does. Presets tune it
    /// to match the vehicle's realistic performance.
    pub max_speed: f64,
}
