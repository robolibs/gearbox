//! Narrow view of the physics engine that the rest of the library
//! can touch.
//!
//! The only place that actually imports `rapier3d` types for a body /
//! controller is [`PhysicsHandles`] — every other consumer (drive
//! controllers, eventually trailers, sensors) goes through
//! [`BodyProxy`] and [`WheelsProxy`]. That's what keeps the future
//! "swap rapier for something else" refactor a contained job: swap
//! the contents of this file + `sim.rs` construction code, leave the
//! rest of the crate alone.
//!
//! The proxies intentionally only expose operations the simulator
//! **currently uses**. Extend sparingly — every new method here is a
//! method any future backend will have to implement.

use rapier3d::control::{DynamicRayCastVehicleController, Wheel};
use rapier3d::prelude::*;

/// The engine-specific handles for one vehicle. `pub(crate)` — a
/// preset / UI consumer cannot construct or inspect this directly.
pub(crate) struct PhysicsHandles {
    pub body: RigidBodyHandle,
    pub controller: DynamicRayCastVehicleController,
}

// ─── Body proxy ─────────────────────────────────────────────────────
//
// Thin wrapper around `&mut RigidBody` that hands out the handful of
// getters / setters drive controllers actually need. All the math
// types crossing the boundary (`Vec3`, `Rot3`) are re-exports from
// `rapier3d::prelude`, which are really `nalgebra::{Vector3,
// UnitQuaternion}<f32>`. Future backends likely share the same math
// crate; the ONLY thing that changes is the `rb: &mut RigidBody` on
// the inside.

pub struct BodyProxy<'a> {
    rb: &'a mut RigidBody,
}

impl<'a> BodyProxy<'a> {
    pub(crate) fn new(rb: &'a mut RigidBody) -> Self {
        Self { rb }
    }

    pub fn mass(&self) -> f64 {
        self.rb.mass()
    }

    pub fn rotation(&self) -> Rot3 {
        *self.rb.rotation()
    }

    pub fn linvel(&self) -> Vec3 {
        self.rb.linvel()
    }

    pub fn angvel(&self) -> Vec3 {
        self.rb.angvel()
    }

    pub fn linvel_horizontal(&self) -> f64 {
        let lv = self.rb.linvel();
        (lv.x * lv.x + lv.z * lv.z).sqrt()
    }

    /// Local-frame principal-inertia diagonal, in kg·m². Used by the
    /// drone tilt PD to scale rad/s² gains into Nm torques.
    pub fn principal_inertia(&self) -> Vec3 {
        self.rb.mass_properties().local_mprops.principal_inertia()
    }

    pub fn add_force(&mut self, force: Vec3, wake: bool) {
        self.rb.add_force(force, wake);
    }

    pub fn add_torque(&mut self, torque: Vec3, wake: bool) {
        self.rb.add_torque(torque, wake);
    }

    pub fn reset_forces(&mut self, wake: bool) {
        self.rb.reset_forces(wake);
    }

    pub fn reset_torques(&mut self, wake: bool) {
        self.rb.reset_torques(wake);
    }
}

// ─── Wheels proxy ───────────────────────────────────────────────────
//
// The rapier ray-cast vehicle controller keeps its wheels in an
// `&[Wheel] / &mut [Wheel]` slice. Ground drive controllers read
// normal forces + the previous-tick steering angle, and write
// engine_force / brake / steering each tick. We expose exactly those
// operations.

pub struct WheelsProxy<'a> {
    controller: &'a mut DynamicRayCastVehicleController,
}

impl<'a> WheelsProxy<'a> {
    pub(crate) fn new(controller: &'a mut DynamicRayCastVehicleController) -> Self {
        Self { controller }
    }

    pub fn len(&self) -> usize {
        self.controller.wheels().len()
    }

    /// Immutable view of one wheel's drive-relevant fields.
    pub fn get(&self, idx: usize) -> Option<WheelView<'_>> {
        self.controller.wheels().get(idx).map(|w| WheelView { w })
    }

    /// Mutable view of one wheel's drive-relevant fields.
    pub fn get_mut(&mut self, idx: usize) -> Option<WheelCtrl<'_>> {
        self.controller
            .wheels_mut()
            .get_mut(idx)
            .map(|w| WheelCtrl { w })
    }

    /// Iterate a specific fixed field across all wheels — currently
    /// only used for collecting suspension normal forces. Exposed as
    /// a free-standing method so callers don't need to call `.get()`
    /// in a loop + allocate.
    pub fn normal_forces(&self) -> Vec<f64> {
        self.controller
            .wheels()
            .iter()
            .map(|w| w.wheel_suspension_force.max(0.0))
            .collect()
    }
}

/// Read-only view of a wheel — what the drive controllers need to
/// inspect (current angle, suspension normal force).
pub struct WheelView<'a> {
    w: &'a Wheel,
}

impl<'a> WheelView<'a> {
    pub fn steering(&self) -> f64 {
        self.w.steering
    }

    pub fn wheel_suspension_force(&self) -> f64 {
        self.w.wheel_suspension_force
    }
}

/// Mutable view of a wheel — engine force, brake, steering angle.
/// The raycast state on the Wheel is read-only from here; the
/// controller itself updates it during `update_vehicle`.
pub struct WheelCtrl<'a> {
    w: &'a mut Wheel,
}

impl<'a> WheelCtrl<'a> {
    pub fn steering(&self) -> f64 {
        self.w.steering
    }

    pub fn set_steering(&mut self, angle: f64) {
        self.w.steering = angle;
    }

    pub fn set_engine_force(&mut self, force: f64) {
        self.w.engine_force = force;
    }

    pub fn set_brake(&mut self, brake: f64) {
        self.w.brake = brake;
    }
}
