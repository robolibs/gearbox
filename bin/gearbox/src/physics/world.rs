//! `PhysicsWorld` — the single resource that owns every Rapier set
//! and the integration pipeline. Bevy talks to Rapier exclusively
//! through this type; the rest of the adapter just populates it.
//!
//! **f64 throughout.** Rapier's `Real` is `f64` because we depend on
//! `rapier3d-f64`. Conversion happens at the Bevy boundary
//! (`Transform`/`Vec3`/`Quat` are `f32`).

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use rapier3d::prelude::*;

/// All Rapier state for the loaded USD scene. Exactly one of these
/// in the world.
#[derive(Resource)]
pub struct PhysicsWorld {
    pub gravity: Vector,
    pub integration_parameters: IntegrationParameters,
    pub physics_pipeline: PhysicsPipeline,
    pub islands: IslandManager,
    pub broad_phase: BroadPhaseBvh,
    pub narrow_phase: NarrowPhase,
    pub bodies: RigidBodySet,
    pub colliders: ColliderSet,
    pub impulse_joints: ImpulseJointSet,
    pub multibody_joints: MultibodyJointSet,
    pub ccd_solver: CCDSolver,
    /// Bevy entity → Rapier rigid-body handle. Lets writeback look up
    /// which body to copy into the entity's Transform each tick.
    pub entity_to_body: HashMap<Entity, RigidBodyHandle>,
    /// Bevy entity → Rapier collider handle.
    pub entity_to_collider: HashMap<Entity, ColliderHandle>,
    /// Body pairs whose contacts are dropped (`PhysicsFilteredPairsAPI`),
    /// stored in both orders.
    pub filtered_pairs: HashSet<(RigidBodyHandle, RigidBodyHandle)>,
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
        let mut integration_parameters = IntegrationParameters::default();
        // Soft `ImpulseJoint` constraints converge harder per tick;
        // matters for vehicles whose front-axle joints fall back to
        // impulse when the multibody solver can't take all of them.
        integration_parameters.num_solver_iterations = 16;
        integration_parameters.num_internal_pgs_iterations = 4;
        let step_hz = std::env::var("GEARBOX_PHYSICS_HZ")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|hz| *hz >= 30.0)
            .unwrap_or(DEFAULT_STEP_HZ);
        integration_parameters.dt = 1.0 / step_hz;
        Self {
            gravity: Vector::new(0.0, -9.81, 0.0),
            integration_parameters,
            physics_pipeline: PhysicsPipeline::new(),
            islands: IslandManager::new(),
            broad_phase: BroadPhaseBvh::new(),
            narrow_phase: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd_solver: CCDSolver::new(),
            entity_to_body: HashMap::new(),
            entity_to_collider: HashMap::new(),
            filtered_pairs: HashSet::new(),
            quarantined: Vec::new(),
            step_hz,
            accumulator: 0.0,
            pending_steps: 0,
        }
    }
}

impl PhysicsWorld {
    /// One Rapier integration step using the resource's current
    /// gravity + parameters. No event handlers.
    pub fn step(&mut self) {
        self.quarantine_non_finite();
        self.physics_pipeline.step(
            self.gravity,
            &self.integration_parameters,
            &mut self.islands,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd_solver,
            &PairFilter(&self.filtered_pairs),
            &(),
        );
    }

    /// A body whose pose or velocity went non-finite would take the broad
    /// phase down with an index panic; disable it and say which entity it
    /// was instead.
    fn quarantine_non_finite(&mut self) {
        let mut bad: Vec<RigidBodyHandle> = Vec::new();
        for (handle, body) in self.bodies.iter() {
            let pose = body.position();
            let finite = pose.translation.is_finite()
                && pose.rotation.is_finite()
                && body.linvel().is_finite()
                && body.angvel().is_finite();
            if !finite && body.is_enabled() {
                bad.push(handle);
            }
        }
        for handle in bad {
            let Some(body) = self.bodies.get_mut(handle) else {
                continue;
            };
            body.set_enabled(false);
            self.quarantined
                .push(Entity::from_bits(body.user_data as u64));
        }
    }

    /// Drop every contact between two bodies, in either order.
    pub fn filter_pair(&mut self, a: RigidBodyHandle, b: RigidBodyHandle) {
        self.filtered_pairs.insert((a, b));
        self.filtered_pairs.insert((b, a));
    }
}

/// Contact filter backed by `PhysicsWorld::filtered_pairs`.
struct PairFilter<'a>(&'a HashSet<(RigidBodyHandle, RigidBodyHandle)>);

impl PhysicsHooks for PairFilter<'_> {
    fn filter_contact_pair(&self, context: &PairFilterContext) -> Option<SolverFlags> {
        match (context.rigid_body1, context.rigid_body2) {
            (Some(a), Some(b)) if self.0.contains(&(a, b)) => None,
            _ => Some(SolverFlags::COMPUTE_IMPULSES),
        }
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
