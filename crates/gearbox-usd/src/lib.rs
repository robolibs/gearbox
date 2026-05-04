//! Load OpenUSD scenes into a `gearbox_physics::Sim`.
//!
//! This crate is the bridge from a USD file on disk to live rapier
//! bodies/colliders/joints inside an existing gearbox sim. The
//! gearbox world (skybox, ground, GPS, planet, gravity) is **not**
//! touched — USD content is *additive*, mounted on top of the world
//! gearbox already owns.
//!
//! Step 1 scope: walk the stage, find every `UsdPhysicsRigidBodyAPI`
//! prim, hand its decoded record + world pose to
//! `usd_rapier::bodies::build_rigid_body`, and remember the
//! `prim_path → RigidBodyHandle` mapping in a returned descriptor.
//! Colliders, joints, articulations, materials, GPS anchors, mount
//! namespacing — all later.

use std::collections::HashMap;

use rapier3d::prelude::RigidBodyHandle;

mod load;

pub use load::load_usd_into_sim;

/// What `load_usd_into_sim` produces: enough to find the rapier
/// handles for any prim that ended up in the sim, so callers can
/// drive controllers, attach sensors, or unload later.
#[derive(Debug, Default)]
pub struct SceneDescriptor {
    /// USD prim path → rapier rigid body handle.
    pub bodies: HashMap<String, RigidBodyHandle>,
}

impl SceneDescriptor {
    pub fn body(&self, prim_path: &str) -> Option<RigidBodyHandle> {
        self.bodies.get(prim_path).copied()
    }
}
