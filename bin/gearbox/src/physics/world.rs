//! `PhysicsWorld` — the one resource that owns the simulation. It holds the
//! Molla world behind [`PhysicsBackend`] and gearbox's own bookkeeping:
//! which entity is which body, which body pairs must not touch, the
//! fixed-step clock, hitch captures.
//!
//! It derefs to the backend, so `physics.body(id)` reads straight through.
//! **f64 throughout;** conversion happens at the Bevy boundary
//! (`Transform`/`Vec3`/`Quat` are `f32`).

use std::collections::{HashMap, HashSet};
use std::ops::{Deref, DerefMut};

use bevy::prelude::*;

use super::backend::{
    BodyId, ColliderId, DeviceCommand, DeviceLimits, DeviceSetting, JointAxis, JointId, MotorModel, MotorOutput,
    PhysicsBackend, Pose, SolverSettings,
};
use super::drive_motor::DriveMotor;
use super::molla::MollaBackend;

/// All physics state for the loaded scene. Exactly one of these in the world.
#[derive(Resource)]
pub struct PhysicsWorld {
    pub backend: Box<dyn PhysicsBackend>,
    /// Bevy entity → body. Lets writeback look up which body to copy into
    /// the entity's Transform each tick.
    pub entity_to_body: HashMap<Entity, BodyId>,
    pub(super) published_transforms: HashMap<Entity, super::writeback::PublishedTransform>,
    /// Bevy entity → collider.
    pub entity_to_collider: HashMap<Entity, ColliderId>,
    /// USD joint prim entity → joint.
    pub entity_to_joint: HashMap<Entity, JointId>,
    /// USD joint → its authored `PhysicsDriveAPI`, kept while controllers
    /// rewrite the joint's motor every step.
    pub authored_drives: HashMap<JointId, AuthoredDrive>,
    /// Motor devices that move their joint's authored drive.
    drive_motors: HashMap<JointId, DriveMotor>,
    /// Body pairs whose contacts are dropped (`PhysicsFilteredPairsAPI`),
    /// stored in both orders.
    pub filtered_pairs: HashSet<(BodyId, BodyId)>,
    pub attachment_filtered_pairs: HashSet<(BodyId, BodyId)>,
    hitch_captures: HashMap<JointId, HitchCapture>,
    hitch_sliders: HashMap<JointId, HitchSlider>,
    joint_frictions: HashMap<JointId, JointFriction>,
    /// Entities whose bodies the last step disabled for non-finite state.
    pub quarantined: Vec<Entity>,
    /// Fixed physics rate; the frame's real time is spent in steps of it.
    pub step_hz: f64,
    /// Real time not yet simulated.
    pub accumulator: f64,
    /// Steps this frame runs, planned in `First` so controllers can scale
    /// per-frame impulses by the time they cover.
    pub pending_steps: u32,
    /// Simulated seconds advanced by every step so far; never rewinds.
    pub simulated_seconds: f64,
}

/// The drive a USD joint authors on its free axis, in SI units: stiffness per
/// radian or metre, damping per radian or metre per second, force or torque.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AuthoredDrive {
    pub stiffness: f64,
    pub damping: f64,
    pub max_force: Option<f64>,
    /// `drive:type` is `force`; otherwise the gains scale with inertia.
    pub force: bool,
    /// The authored targets, radians or metres and per second.
    pub target_position: Option<f64>,
    pub target_velocity: Option<f64>,
}

/// Default physics rate; `GEARBOX_PHYSICS_HZ` overrides it.
const DEFAULT_STEP_HZ: f64 = 120.0;
/// Most steps one frame may run; below that frame rate physics slows
/// down instead of spiralling.
const MAX_STEPS_PER_FRAME: u32 = 12;

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::with_backend(Box::new(MollaBackend::default()))
    }
}

impl Deref for PhysicsWorld {
    type Target = dyn PhysicsBackend;
    fn deref(&self) -> &Self::Target {
        &*self.backend
    }
}

impl DerefMut for PhysicsWorld {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.backend
    }
}

impl PhysicsWorld {
    /// Run gearbox on `backend`. This is the seam a second engine plugs into.
    pub fn with_backend(mut backend: Box<dyn PhysicsBackend>) -> Self {
        let step_hz = std::env::var("GEARBOX_PHYSICS_HZ")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|hz| *hz >= 30.0)
            .unwrap_or(DEFAULT_STEP_HZ);
        // Soft constraint joints converge harder per tick; matters for
        // vehicles whose front-axle joints are not in reduced coordinates.
        backend.set_settings(SolverSettings {
            dt: 1.0 / step_hz,
            solver_iterations: 16,
            internal_iterations: 4,
        });
        backend.set_gravity(glam::DVec3::new(0.0, -9.81, 0.0));
        info!("gearbox-physics: backend `{}` at {step_hz} Hz", backend.name());
        Self {
            backend,
            entity_to_body: HashMap::new(),
            published_transforms: HashMap::new(),
            entity_to_collider: HashMap::new(),
            entity_to_joint: HashMap::new(),
            authored_drives: HashMap::new(),
            drive_motors: HashMap::new(),
            filtered_pairs: HashSet::new(),
            attachment_filtered_pairs: HashSet::new(),
            hitch_captures: HashMap::new(),
            hitch_sliders: HashMap::new(),
            joint_frictions: HashMap::new(),
            quarantined: Vec::new(),
            step_hz,
            accumulator: 0.0,
            pending_steps: 0,
            simulated_seconds: 0.0,
        }
    }

    /// Seconds one step advances.
    pub fn dt(&self) -> f64 {
        self.backend.settings().dt
    }

    /// One integration step with the current gravity and settings.
    pub fn step(&mut self) {
        self.quarantine_non_finite();
        self.advance_hitch_captures();
        self.advance_hitch_sliders();
        self.apply_joint_frictions();
        self.advance_drive_motors();
        let (authored, attached) = (&self.filtered_pairs, &self.attachment_filtered_pairs);
        self.backend
            .step(&|a, b| authored.contains(&(a, b)) || attached.contains(&(a, b)));
        self.observe_drive_motors();
        self.simulated_seconds += self.dt();
        for body in self.backend.quarantined_bodies() {
            if let Some(entity) = self.backend.body(body).and_then(|body| body.entity()) {
                if !self.quarantined.contains(&entity) {
                    self.quarantined.push(entity);
                }
            }
        }
    }

    /// Puts a motor device on `joint`. On a joint with an authored drive the
    /// motor moves that drive; elsewhere it is the backend's own.
    pub fn insert_motor(&mut self, joint: JointId, limits: DeviceLimits, max_force: f64) -> Result<(), String> {
        let coordinate = self.backend.joint(joint).and_then(|j| {
            [JointAxis::AngX, JointAxis::LinX].into_iter().find_map(|axis| j.motor_position(axis).map(|q| (axis, q)))
        });
        match (self.authored_drives.get(&joint), coordinate) {
            (Some(drive), Some((axis, position))) => {
                let motor = DriveMotor::new(axis, *drive, limits, max_force, position)?;
                self.drive_motors.insert(joint, motor);
                Ok(())
            }
            _ => self.backend.insert_motor(joint, limits, max_force),
        }
    }

    pub fn remove_motor(&mut self, joint: JointId) {
        if self.drive_motors.remove(&joint).is_none() {
            self.backend.remove_motor(joint);
        }
    }

    pub fn has_motor(&self, joint: JointId) -> bool {
        self.drive_motors.contains_key(&joint) || self.backend.has_motor(joint)
    }

    pub fn command_motor(&mut self, joint: JointId, command: DeviceCommand) -> Result<(), String> {
        match self.drive_motors.get_mut(&joint) {
            Some(motor) => motor.command(command),
            None => self.backend.command_motor(joint, command),
        }
    }

    pub fn configure_motor(&mut self, joint: JointId, setting: DeviceSetting) -> Result<(), String> {
        match self.drive_motors.get_mut(&joint) {
            Some(motor) => motor.configure(setting),
            None => self.backend.configure_motor(joint, setting),
        }
    }

    pub fn motor_output(&self, joint: JointId) -> Option<MotorOutput> {
        match self.drive_motors.get(&joint) {
            Some(motor) => Some(motor.output(self.backend.motor_output(joint))),
            None => self.backend.motor_output(joint),
        }
    }

    fn advance_drive_motors(&mut self) {
        let dt = self.dt();
        let backend = &mut self.backend;
        self.drive_motors.retain(|id, motor| {
            let changed = motor.advance(dt);
            let Some(joint) = backend.joint_mut(*id, changed) else {
                return false;
            };
            motor.write(joint);
            true
        });
    }

    fn observe_drive_motors(&mut self) {
        let dt = self.dt();
        for (id, motor) in &mut self.drive_motors {
            if let Some(position) = self.backend.joint(*id).and_then(|j| j.motor_position(motor.axis)) {
                motor.observe(position, dt);
            }
        }
    }

    /// A body whose pose or velocity went non-finite would take the broad
    /// phase down with an index panic; disable it and say which entity it
    /// was instead.
    fn quarantine_non_finite(&mut self) {
        for id in self.backend.bodies() {
            let Some(body) = self.backend.body_mut(id) else {
                continue;
            };
            let pose = body.position();
            let finite = pose.translation.is_finite()
                && pose.rotation.is_finite()
                && body.linvel().is_finite()
                && body.angvel().is_finite();
            if finite || !body.is_enabled() {
                continue;
            }
            body.set_enabled(false);
            if let Some(entity) = body.entity() {
                self.quarantined.push(entity);
            }
        }
    }

    /// Drop every contact between two bodies, in either order.
    pub fn filter_pair(&mut self, a: BodyId, b: BodyId) {
        self.filtered_pairs.insert((a, b));
        self.filtered_pairs.insert((b, a));
    }

    /// Remove an entity's body (with its colliders and joints) and forget it.
    pub fn remove_entity_body(&mut self, entity: Entity) {
        self.published_transforms.remove(&entity);
        if let Some(id) = self.entity_to_body.remove(&entity) {
            self.backend.remove_body(id);
        }
    }

    /// Remove an entity's collider and forget it.
    pub fn remove_entity_collider(&mut self, entity: Entity, wake: bool) {
        if let Some(id) = self.entity_to_collider.remove(&entity) {
            self.backend.remove_collider(id, wake);
        }
    }
}

/// A captured hitch joint starts this soft, so nothing jumps.
const CAPTURE_HZ: f64 = 8.0;

struct HitchCapture {
    start: Pose,
    target: Pose,
    elapsed: f64,
    duration: f64,
    hold: HitchHold,
}

/// A hitch joint carried by one body at a point that rides on another: a
/// hydraulic top link's barrel holds the load at its rod's pin.
struct HitchSlider {
    carrier: BodyId,
    rod: BodyId,
    end: glam::DVec3,
}

/// Coulomb friction on a joint's free axis: a capped hold at an anchor that
/// follows the joint once it slides past it. The cap is a fixed torque plus,
/// for a rolling part, `coefficient × ground load × radius`.
struct JointFriction {
    axis: JointAxis,
    torque: f64,
    rolling: Option<(f64, f64)>,
    anchor: Option<f64>,
}

/// The ground's solved normal load on a body (N): its contacts whose normal
/// points up.
fn ground_load(backend: &dyn PhysicsBackend, body: BodyId) -> f64 {
    let Some(colliders) = backend.body(body).map(|b| b.colliders()) else {
        return 0.0;
    };
    let mut impulse = 0.0;
    for collider in colliders {
        for manifold in backend.contacts_with(collider).into_iter().filter(|m| m.active) {
            let normal = manifold.normal * if manifold.collider1 == collider { -1.0 } else { 1.0 };
            if normal.y > 0.1 {
                impulse += manifold.points.iter().filter(|p| p.solved).map(|p| p.impulse.max(0.0) * normal.y).sum::<f64>();
            }
        }
    }
    impulse / backend.settings().dt.max(1e-6)
}

/// How far a sticking joint gives before it slides (rad or m).
const FRICTION_STICK: f64 = 0.01;
/// Slip rate at which a joint meets its full friction (rad/s or m/s).
const FRICTION_RATE: f64 = 0.05;

/// How a captured joint holds once its frames meet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum HitchHold {
    /// Exactly, without compliance: a tree joint.
    Rigid,
    /// Compliant at this natural frequency (Hz): a loop joint, which
    /// cannot be rigid.
    Soft(f64),
}

impl PhysicsWorld {
    /// Move a hitch's second local frame to its authored anchor at fixed-step speed.
    pub(crate) fn capture_hitch(&mut self, joint: JointId, target: Pose, hold: HitchHold) {
        let Some(start) = self.backend.joint(joint).map(|j| j.frame2()) else {
            return;
        };
        let distance = start.translation.distance(target.translation);
        let angle = start.rotation.angle_between(target.rotation);
        let duration = (1.5 * (distance / 0.2).max(angle / 0.2)).max(0.5);
        self.hitch_captures.insert(
            joint,
            HitchCapture { start, target, elapsed: 0.0, duration, hold },
        );
    }

    fn advance_hitch_captures(&mut self) {
        let dt = self.dt();
        let backend = &mut self.backend;
        self.hitch_captures.retain(|id, capture| {
            let Some(bodies) = backend.joint_bodies(*id) else {
                return false;
            };
            let Some(joint) = backend.joint_mut(*id, true) else {
                return false;
            };
            capture.elapsed = (capture.elapsed + dt).min(capture.duration);
            let t = capture.elapsed / capture.duration;
            let s = t * t * (3.0 - 2.0 * t);
            joint.set_frame2(Pose {
                translation: capture
                    .start
                    .translation
                    .lerp(capture.target.translation, s),
                rotation: capture.start.rotation.slerp(capture.target.rotation, s),
            });
            match (capture.hold, t < 1.0) {
                (HitchHold::Rigid, false) => joint.set_rigid(),
                (HitchHold::Rigid, true) => joint.set_softness(CAPTURE_HZ + (30.0 - CAPTURE_HZ) * s, 1.0),
                (HitchHold::Soft(hz), _) => joint.set_softness(CAPTURE_HZ + (hz - CAPTURE_HZ) * s, 1.0),
            }
            for body in [bodies.0, bodies.1] {
                if let Some(body) = backend.body_mut(body) {
                    body.wake_up(true);
                }
            }
            t < 1.0
        });
    }

    /// Hold a joint's free axis against up to `torque` of load, plus
    /// `rolling` (coefficient, radius m) times the ground load on the part it
    /// turns, then let it turn against that: the drag of a bearing and the soil.
    pub(crate) fn add_joint_friction(&mut self, joint: JointId, axis: JointAxis, torque: f64, rolling: Option<(f64, f64)>) {
        self.joint_frictions.insert(joint, JointFriction { axis, torque, rolling, anchor: None });
    }

    /// A joint a motor device or an authored drive turns is left to it.
    fn apply_joint_frictions(&mut self) {
        let (backend, drives) = (&mut self.backend, &self.authored_drives);
        self.joint_frictions.retain(|id, friction| {
            let (Some(position), Some((_, part))) =
                (backend.joint(*id).map(|j| j.motor_position(friction.axis)), backend.joint_bodies(*id))
            else {
                return false;
            };
            if backend.has_motor(*id) || drives.contains_key(id) {
                return true;
            }
            let Some(position) = position else {
                return true;
            };
            let torque = friction.torque
                + friction.rolling.map_or(0.0, |(coefficient, radius)| coefficient * ground_load(&**backend, part) * radius);
            let anchor = friction.anchor.get_or_insert(position);
            *anchor = position + (*anchor - position).clamp(-FRICTION_STICK, FRICTION_STICK);
            if torque <= 0.0 {
                *anchor = position;
            }
            if let Some(joint) = backend.joint_mut(*id, false) {
                joint.set_motor_model(friction.axis, MotorModel::Force);
                joint.set_motor_position(friction.axis, *anchor, torque / FRICTION_STICK, torque / FRICTION_RATE);
                joint.set_motor_max_force(friction.axis, torque);
            }
            true
        });
    }

    /// Keep a hitch joint's first frame on `carrier` at the point `end` of
    /// `rod`, wherever the rod has slid.
    pub(crate) fn slide_hitch(&mut self, joint: JointId, carrier: BodyId, rod: BodyId, end: glam::DVec3) {
        self.hitch_sliders.insert(joint, HitchSlider { carrier, rod, end });
    }

    fn advance_hitch_sliders(&mut self) {
        let backend = &mut self.backend;
        self.hitch_sliders.retain(|id, slider| {
            let poses = backend.body(slider.carrier).map(|b| b.position()).zip(backend.body(slider.rod).map(|b| b.position()));
            let (Some((carrier, rod)), Some(frame)) = (poses, backend.joint(*id).map(|j| j.frame1())) else {
                return false;
            };
            let end = carrier.rotation.inverse() * (rod.translation + rod.rotation * slider.end - carrier.translation);
            if end.distance(frame.translation) > 1e-6 {
                if let Some(joint) = backend.joint_mut(*id, true) {
                    joint.set_frame1(Pose { translation: end, rotation: frame.rotation });
                }
            }
            true
        });
    }
}

pub use gearbox_api::PhysicsActive;

/// Turn the frame's real time into a whole number of fixed steps.
pub fn plan_physics_steps(
    time: Res<Time>,
    active: Res<PhysicsActive>,
    mut world: ResMut<PhysicsWorld>,
) {
    if !active.0 {
        world.accumulator = 0.0;
        world.pending_steps = 0;
        return;
    }
    let dt = 1.0 / world.step_hz;
    world.accumulator += time.delta_secs_f64().min(0.25);
    let steps = ((world.accumulator / dt) as u32).min(MAX_STEPS_PER_FRAME);
    world.accumulator -= steps as f64 * dt;
    if steps == MAX_STEPS_PER_FRAME {
        world.accumulator = world.accumulator.min(dt);
    }
    world.pending_steps = steps;
}

/// Run the steps `plan_physics_steps` planned for this frame, and log the
/// cost per step every 10 s.
pub fn step_physics(
    active: Res<PhysicsActive>,
    time: Res<Time>,
    mut world: ResMut<PhysicsWorld>,
    mut stats: Local<(f64, u32, f64)>,
) {
    if !active.0 {
        return;
    }
    let start = std::time::Instant::now();
    for _ in 0..world.pending_steps {
        world.step();
    }
    stats.0 += start.elapsed().as_secs_f64();
    stats.1 += world.pending_steps;
    let now = time.elapsed_secs_f64();
    if now >= stats.2 {
        if stats.1 > 0 {
            info!(
                "gearbox-physics: {:.2} ms per step, {} steps in the last 10 s",
                1000.0 * stats.0 / stats.1 as f64,
                stats.1
            );
        }
        *stats = (0.0, 0, now + 10.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::backend::{BodyDesc, ColliderDesc, JointDesc, JointKind, Shape};
    use glam::DVec3;

    /// A 4.2 kg arm 1 m out on a horizontal hinge: 41 N·m of gravity.
    fn arm(friction: f64) -> (PhysicsWorld, JointId) {
        let mut world = PhysicsWorld::default();
        let mut body = |desc: BodyDesc, at: DVec3| {
            let id = world.insert_body(desc.pose(Pose::from_translation(at)));
            world.insert_collider(ColliderDesc::new(Shape::Ball { radius: 0.1 }).density(1000.0).parent(id)).unwrap();
            id
        };
        let (post, arm) = (body(BodyDesc::fixed(), DVec3::ZERO), body(BodyDesc::dynamic(), DVec3::X));
        let hinge = JointDesc::new(JointKind::Revolute { axis: DVec3::Z }, Pose::IDENTITY, Pose::from_translation(DVec3::NEG_X));
        let joint = world.backend.insert_joint(post, arm, hinge);
        world.add_joint_friction(joint, JointAxis::AngX, friction, None);
        (world, joint)
    }

    fn swing(friction: f64) -> f64 {
        let (mut world, joint) = arm(friction);
        for _ in 0..240 {
            world.step();
        }
        world.joint(joint).unwrap().motor_position(JointAxis::AngX).unwrap().abs()
    }

    #[test]
    fn joint_friction_holds_below_its_torque_and_slides_above() {
        assert!(swing(60.0) < 0.02, "{}", swing(60.0));
        assert!(swing(20.0) > 0.3, "{}", swing(20.0));
    }

    /// The ground load a rolling part's friction scales with is its weight
    /// when it rests on the ground.
    #[test]
    fn a_resting_ball_carries_its_weight_on_the_ground() {
        let mut world = PhysicsWorld::default();
        let ground = world.insert_body(BodyDesc::fixed());
        world.insert_collider(ColliderDesc::new(Shape::Cuboid { half_extents: DVec3::new(5.0, 0.5, 5.0) }).parent(ground)).unwrap();
        let ball = world.insert_body(BodyDesc::dynamic().pose(Pose::from_translation(DVec3::new(0.0, 0.8, 0.0))));
        world.insert_collider(ColliderDesc::new(Shape::Ball { radius: 0.3 }).density(1000.0).parent(ball)).unwrap();
        for _ in 0..240 {
            world.step();
        }
        let weight = world.body(ball).unwrap().mass() * 9.81;
        let load = ground_load(&*world.backend, ball);
        assert!((load - weight).abs() < 0.05 * weight, "load {load} weight {weight}");
    }
}
