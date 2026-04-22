//! Pluggable drive controllers.
//!
//! Each `DriveMode` maps to a controller struct that implements
//! [`DriveController`]. The controller is what translates a
//! `ControlInput` into wheel forces (for ground vehicles) or body
//! forces + torques (for drones). Adding a new drive mode means:
//!
//!   1. Define a new unit struct implementing [`DriveController`].
//!   2. Add a variant to [`super::DriveMode`] and extend
//!      [`controller_for`] to dispatch to it.
//!
//! The dispatch is static (no heap allocation, no dynamic lookup —
//! the compiler inlines it through the `&'static dyn` ref), while the
//! controller implementations are decoupled from `sim.rs`.
//!
//! Ground vehicles and drones share the [`DriveController`] trait but
//! have different needs from the surrounding world. The [`DriveContext`]
//! bundles everything a controller might touch (bodies, dt, gravity)
//! so the trait signature stays uniform.

use rapier3d::prelude::Vec3;

use crate::control::ControlInput;
use crate::vehicle::physics::{BodyProxy, WheelsProxy};
use crate::vehicle::{DriveMode, VehicleSpec};

pub mod ackermann;
pub mod differential;
pub mod drone;
pub mod omni;
pub mod util;

pub use ackermann::AckermannController;
pub use differential::DifferentialController;
pub use drone::DroneController;
pub use omni::OmniController;

/// Everything a drive controller may need this tick.
///
/// Engine-agnostic: `body` / `wheels` are narrow proxies that hide
/// whichever physics backend is underneath. `spec` + `control` live
/// here too (rather than being passed alongside) so the caller can
/// split-borrow the physics handles on [`VehicleState`] without
/// conflicting with the shared read of `spec`.
pub struct DriveContext<'a> {
    pub dt: f32,
    pub gravity: Vec3,
    pub spec: &'a VehicleSpec,
    pub control: ControlInput,
    pub body: BodyProxy<'a>,
    pub wheels: WheelsProxy<'a>,
}

/// A strategy for turning [`ControlInput`](crate::ControlInput) into
/// vehicle motion. Implementors are unit structs — state lives on
/// [`DriveContext`]; the controller is a pure function.
pub trait DriveController: core::fmt::Debug + Send + Sync {
    /// Airborne controllers skip the wheel-raycast step and the
    /// parking-brake pass in `Sim::step`. Ground controllers return
    /// `false`; drones return `true`.
    fn is_airborne(&self) -> bool {
        false
    }

    /// Apply controls to the vehicle. Ground controllers write to
    /// `ctx.wheels`; airborne controllers write forces + torques on
    /// `ctx.body`.
    fn apply(&self, ctx: &mut DriveContext);
}

/// Return the single shared instance of the controller for the given
/// mode. Matches over the enum once at dispatch time; each arm
/// resolves to a fixed `&'static dyn` so the compiler can inline.
pub fn controller_for(mode: DriveMode) -> &'static dyn DriveController {
    match mode {
        DriveMode::Ackermann => &AckermannController,
        DriveMode::Differential => &DifferentialController,
        DriveMode::Drone => &DroneController,
        DriveMode::Omni => &OmniController,
    }
}

/// Snapshot of ground-vehicle-wide state that several controllers
/// need: brake taper, Ackermann wheelbase, Z-extent of wheel mounts,
/// and the per-wheel suspension normal forces used by open-diff splits.
///
/// Computed once per tick by ground controllers (`Ackermann`,
/// `Differential`, `Omni`). Drone doesn't use it.
pub struct GroundFrame {
    pub brake_gate: f32,
    pub wheelbase: f32,
    pub z_min: f32,
    pub z_max: f32,
    pub normal_forces: Vec<f32>,
}

impl GroundFrame {
    pub fn compute(ctx: &DriveContext<'_>) -> Self {
        let specs = &ctx.spec.wheels;

        // Brake anti-shake: rapier's copysign brake flips sign near
        // zero velocity and oscillates. Taper the brake off below
        // 1.2 m/s and force it to zero below 0.5 m/s — the parking
        // brake in `Sim::step` takes over from there.
        let speed_mag = ctx.body.linvel().length();
        let brake_gate = if speed_mag < 0.5 {
            0.0
        } else {
            ((speed_mag - 0.5) / 0.7).clamp(0.0, 1.0)
        };

        let (mut z_min, mut z_max) = (f64::INFINITY, f64::NEG_INFINITY);
        for w in specs {
            z_min = z_min.min(w.chassis_connection.z);
            z_max = z_max.max(w.chassis_connection.z);
        }
        let wheelbase = (z_max - z_min) as f32;

        let normal_forces = ctx.wheels.normal_forces();

        Self {
            brake_gate,
            wheelbase,
            z_min: z_min as f32,
            z_max: z_max as f32,
            normal_forces,
        }
    }
}
