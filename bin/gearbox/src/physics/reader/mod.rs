//! UsdPhysics vocabulary shared by the physics builders: joint kinds, DOFs,
//! drive types and collision approximations, plus the decoded records the
//! builders take. Reading the stage is usd_bevy's job now.

pub mod tokens;
mod types;
#[allow(unused_imports)]
pub use types::{
    CollisionApprox, Dof, DriveType, JointKind, PhysicsPrims, ReadCollisionGroup,
    ReadCollisionShape, ReadDrive, ReadFilteredPairs, ReadJoint, ReadLimit, ReadMass,
    ReadPhysicsMaterial, ReadPhysicsScene, ReadRigidBody,
};
