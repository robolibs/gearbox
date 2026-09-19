//! `PhysicsWorld` — the one resource that owns the simulation. It holds a
//! [`PhysicsBackend`] behind a box and the bookkeeping that is gearbox's
//! own whatever engine runs: which entity is which body, which body pairs
//! must not touch, the fixed-step clock, hitch captures.
//!
//! It derefs to the backend, so `physics.body(id)` reads straight through.
//! **f64 throughout;** conversion happens at the Bevy boundary
//! (`Transform`/`Vec3`/`Quat` are `f32`).

use std::collections::{HashMap, HashSet};
use std::ops::{Deref, DerefMut};

use bevy::prelude::*;

use super::backend::{BodyId, ColliderId, JointId, PhysicsBackend, Pose, SolverSettings};
use super::molla::MollaBackend;
use super::rapier::RapierBackend;

/// All physics state for the loaded scene. Exactly one of these in the world.
#[derive(Resource)]
pub struct PhysicsWorld {
    pub backend: Box<dyn PhysicsBackend>,
    /// Bevy entity → body. Lets writeback look up which body to copy into
    /// the entity's Transform each tick.
    pub entity_to_body: HashMap<Entity, BodyId>,
    /// Bevy entity → collider.
    pub entity_to_collider: HashMap<Entity, ColliderId>,
    /// Body pairs whose contacts are dropped (`PhysicsFilteredPairsAPI`),
    /// stored in both orders.
    pub filtered_pairs: HashSet<(BodyId, BodyId)>,
    pub attachment_filtered_pairs: HashSet<(BodyId, BodyId)>,
    hitch_captures: HashMap<JointId, HitchCapture>,
    /// Entities whose bodies the last step disabled for non-finite state.
    pub quarantined: Vec<Entity>,
    /// Fixed physics rate; the frame's real time is spent in steps of it.
    pub step_hz: f64,
    /// Real time not yet simulated.
    pub accumulator: f64,
    /// Steps this frame runs, planned in `First` so controllers can scale
    /// per-frame impulses by the time they cover.
    pub pending_steps: u32,
}

/// Default physics rate; `GEARBOX_PHYSICS_HZ` overrides it.
const DEFAULT_STEP_HZ: f64 = 120.0;
/// Most steps one frame may run; below that frame rate physics slows
/// down instead of spiralling.
const MAX_STEPS_PER_FRAME: u32 = 12;

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::with_backend(backend_by_name(std::env::var("GEARBOX_PHYSICS").ok().as_deref()))
    }
}

/// The engine `GEARBOX_PHYSICS` names; a second backend gets an arm here.
fn backend_by_name(name: Option<&str>) -> Box<dyn PhysicsBackend> {
    match name {
        None | Some("rapier") => Box::new(RapierBackend::default()),
        Some("molla") => Box::new(MollaBackend::default()),
        Some(other) => {
            warn!("gearbox-physics: no backend `{other}`; running on rapier");
            Box::new(RapierBackend::default())
        }
    }
}

#[test]
fn named_backends_select_the_requested_engine() {
    assert_eq!(backend_by_name(Some("molla")).name(), "molla");
    assert_eq!(backend_by_name(Some("rapier")).name(), "rapier");
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
            entity_to_collider: HashMap::new(),
            filtered_pairs: HashSet::new(),
            attachment_filtered_pairs: HashSet::new(),
            hitch_captures: HashMap::new(),
            quarantined: Vec::new(),
            step_hz,
            accumulator: 0.0,
            pending_steps: 0,
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
        let (authored, attached) = (&self.filtered_pairs, &self.attachment_filtered_pairs);
        self.backend
            .step(&|a, b| authored.contains(&(a, b)) || attached.contains(&(a, b)));
        for body in self.backend.quarantined_bodies() {
            if let Some(entity) = self.backend.body(body).and_then(|body| body.entity()) {
                if !self.quarantined.contains(&entity) {
                    self.quarantined.push(entity);
                }
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

struct HitchCapture {
    start: Pose,
    target: Pose,
    elapsed: f64,
    duration: f64,
}

impl PhysicsWorld {
    /// Move a hitch's second local frame to its authored anchor at fixed-step speed.
    pub(crate) fn capture_hitch(&mut self, joint: JointId, target: Pose) {
        let Some(start) = self.backend.joint(joint).map(|j| j.frame2()) else {
            return;
        };
        let distance = start.translation.distance(target.translation);
        let angle = start.rotation.angle_between(target.rotation);
        let duration = (1.5 * (distance / 0.2).max(angle / 0.2)).max(0.5);
        self.hitch_captures.insert(
            joint,
            HitchCapture { start, target, elapsed: 0.0, duration },
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
            joint.set_softness(8.0 + 22.0 * s, 1.0);
            for body in [bodies.0, bodies.1] {
                if let Some(body) = backend.body_mut(body) {
                    body.wake_up(true);
                }
            }
            t < 1.0
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
