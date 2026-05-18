//! Narrow view of the physics engine that the rest of the library
//! can touch.
//!
//! The only place that imports `rapier3d` body / joint handles is
//! [`PhysicsHandles`] — every other consumer (drive controllers,
//! trailers, sensors) goes through [`BodyProxy`] and [`WheelsProxy`].
//!
//! Each vehicle is a chassis body plus, per wheel, a light **hub** body
//! and a **wheel** body:
//!
//! ```text
//! chassis ──[joint A: prismatic suspension (+ revolute steer)]── hub
//!                                  └──[joint B: revolute spin]── wheel
//! ```
//!
//! Drive controllers don't touch any of that — they read wheel
//! snapshots and write per-wheel commands ([`WheelCommand`]) through
//! [`WheelsProxy`]; `Sim::step` applies the commands to the joints and
//! wheel bodies after the controller has run (which sidesteps the
//! borrow conflict between the chassis `BodyProxy` and the wheel
//! bodies).

use rapier3d::prelude::*;

/// The engine-specific handles for one vehicle. `pub(crate)` — a
/// preset / UI consumer cannot construct or inspect this directly.
pub(crate) struct PhysicsHandles {
    /// Chassis rigid body.
    pub body: RigidBodyHandle,
    /// Per-wheel hub + wheel bodies and their joints.
    pub wheels: Vec<WheelHandles>,
}

/// Bodies + joints making up one physically-simulated wheel.
pub(crate) struct WheelHandles {
    /// Suspension/steering hub body.
    pub hub: RigidBodyHandle,
    /// Spinning wheel body (carries the tyre collider).
    pub wheel: RigidBodyHandle,
    /// chassis ↔ hub joint: prismatic suspension, plus a motorised
    /// `AngX` steer DOF when `steered`.
    pub joint_a: ImpulseJointHandle,
    /// hub ↔ wheel joint: revolute spin axis (carries drive/brake).
    pub joint_b: ImpulseJointHandle,
    pub steered: bool,
    pub radius: f64,
    /// Axle direction in the wheel body's local frame (normalised).
    pub axle_local: Vec3,
    /// Wheel-centre offset from the chassis body origin, chassis-local.
    /// Used to snap wheels back under a teleported / dragged chassis.
    pub center_local: Vec3,
    /// Last commanded steering angle (rad) — reported back to controllers.
    pub last_steering: f64,
    /// Accumulated rolling angle (rad) for visualisers.
    pub spin_angle: f64,
}

// ─── Body proxy ─────────────────────────────────────────────────────
//
// Thin wrapper around `&mut RigidBody` that hands out the handful of
// getters / setters drive controllers actually need.

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
// A drive controller sees each wheel as a read-only snapshot (the
// previous tick's steering + the live suspension load) and writes a
// command (engine force / brake / steering). The proxy is a pair of
// plain buffers — no physics borrow — so `Sim::step` can split the
// chassis `BodyProxy` from the wheel-body access cleanly.

/// What a controller can observe about a wheel this tick.
#[derive(Clone, Copy, Default)]
pub struct WheelSnapshot {
    /// Current steering angle (rad) — the previous tick's command.
    pub steering: f64,
    /// Suspension-spring load (N) — a proxy for the wheel's normal force.
    pub normal_force: f64,
}

/// What a controller asks of a wheel this tick.
#[derive(Clone, Copy, Default)]
pub struct WheelCommand {
    /// Tractive force at the contact patch (N). Converted to wheel
    /// torque (`force × radius`) when applied.
    pub engine_force: f64,
    /// Brake torque cap (N·m).
    pub brake: f64,
    /// Target steering angle (rad). Ignored on non-steered wheels.
    pub steering: f64,
}

pub struct WheelsProxy<'a> {
    snap: &'a [WheelSnapshot],
    cmd: &'a mut [WheelCommand],
}

impl<'a> WheelsProxy<'a> {
    pub(crate) fn new(snap: &'a [WheelSnapshot], cmd: &'a mut [WheelCommand]) -> Self {
        Self { snap, cmd }
    }

    pub fn len(&self) -> usize {
        self.snap.len()
    }

    pub fn is_empty(&self) -> bool {
        self.snap.is_empty()
    }

    /// Immutable view of one wheel's drive-relevant fields.
    pub fn get(&self, idx: usize) -> Option<WheelView> {
        self.snap.get(idx).map(|s| WheelView {
            steering: s.steering,
            normal_force: s.normal_force,
        })
    }

    /// Mutable command handle for one wheel.
    pub fn get_mut(&mut self, idx: usize) -> Option<WheelCtrl<'_>> {
        let steering_now = self.snap.get(idx)?.steering;
        let cmd = self.cmd.get_mut(idx)?;
        Some(WheelCtrl { steering_now, cmd })
    }

    /// Per-wheel suspension loads — used by the weight-transfer open
    /// differential. Order matches the spec's wheel order.
    pub fn normal_forces(&self) -> Vec<f64> {
        self.snap.iter().map(|s| s.normal_force).collect()
    }
}

/// Read-only view of a wheel — current steering angle + suspension load.
#[derive(Clone, Copy)]
pub struct WheelView {
    steering: f64,
    normal_force: f64,
}

impl WheelView {
    pub fn steering(&self) -> f64 {
        self.steering
    }

    pub fn wheel_suspension_force(&self) -> f64 {
        self.normal_force
    }
}

/// Mutable command handle for a wheel — engine force, brake, steering.
pub struct WheelCtrl<'a> {
    steering_now: f64,
    cmd: &'a mut WheelCommand,
}

impl WheelCtrl<'_> {
    pub fn steering(&self) -> f64 {
        self.steering_now
    }

    pub fn set_steering(&mut self, angle: f64) {
        self.cmd.steering = angle;
    }

    pub fn set_engine_force(&mut self, force: f64) {
        self.cmd.engine_force = force;
    }

    pub fn set_brake(&mut self, brake: f64) {
        self.cmd.brake = brake;
    }
}
