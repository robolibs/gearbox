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
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        let mut integration_parameters = IntegrationParameters::default();
        // Soft `ImpulseJoint` constraints converge harder per tick;
        // matters for vehicles whose front-axle joints fall back to
        // impulse when the multibody solver can't take all of them.
        integration_parameters.num_solver_iterations = 16;
        integration_parameters.num_internal_pgs_iterations = 4;
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

/// Step the world once per frame when `PhysicsActive(true)`.
pub fn step_physics(active: Res<PhysicsActive>, mut world: ResMut<PhysicsWorld>) {
    if active.0 {
        world.step();
    }
}
