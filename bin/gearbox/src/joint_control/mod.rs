//! The joint command contract: exact DOF identity, one owner per DOF, one
//! resolved actuator state per DOF per physics tick.
//!
//! ```text
//! producers -> Batch -> Arbiter::submit -> Arbiter::resolve (per tick)
//!           -> commit (physics::world) -> backend joint / device
//! ```

pub mod api;
pub mod arbiter;
pub mod policy;
pub mod runtime;
pub mod trace;
pub mod types;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod schedule_tests;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
pub(crate) mod conformance;

pub use arbiter::{Arbiter, Batch, DofSpec, DriveSpec, Grant};
pub use policy::Cadence;
pub use types::*;

use crate::machine_model::{MachineDescription, OwnerKind};

pub fn owner_of(machine_id: &str, owner: &crate::machine_model::DofOwner) -> OwnerId {
    let class = match owner.kind {
        OwnerKind::Controller => OwnerClass::Controller,
        OwnerKind::Service => OwnerClass::Service,
        OwnerKind::Device => OwnerClass::Device,
    };
    OwnerId::new(class, machine_id, &owner.owner)
}

/// The DOFs of an accepted machine as the arbiter registers them.
pub fn dof_specs(machine_id: &str, model: &MachineDescription) -> Vec<DofSpec> {
    model
        .dofs
        .iter()
        .enumerate()
        .filter(|(_, d)| model.joints[d.joint].is_physical())
        .map(|(index, d)| DofSpec {
            key: DofKey {
                machine: machine_id.to_string(),
                generation: model.generation,
                joint: model.joints[d.joint].path.clone(),
                dof: d.token,
            },
            interfaces: d.interfaces.value.clone(),
            lower: d.limit.lower,
            upper: d.limit.upper,
            drive: d.drive.as_ref().map(|drive| DriveSpec {
                stiffness: drive.stiffness.value,
                damping: drive.damping.value,
                drive: drive.drive_type.value,
                max_force: drive.max_force.value,
            }),
            hardware_cap: None,
            default_owner: model.owners.get(&index).map(|o| owner_of(machine_id, o)),
        })
        .collect()
}
