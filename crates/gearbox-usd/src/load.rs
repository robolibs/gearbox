//! Walk a USD stage and add every `PhysicsRigidBodyAPI` prim into the
//! given gearbox `Sim` as a rapier rigid body.
//!
//! Step 1 minimal: bodies only. No colliders, no joints, no
//! articulation, no world-transform composition (each prim's local
//! transform is treated as world). Mass / starts-asleep / velocity
//! decoded; everything else added later.

use std::path::Path as StdPath;
use std::str::FromStr;

use anyhow::{Context, Result};
use glam::{DQuat, DVec3};
use openusd::Stage;
use openusd::physics;
use openusd::sdf::Path as SdfPath;

use gearbox_physics::Sim;
use usd_rapier::bodies::{RigidBodyOpinion, build_rigid_body};

use crate::SceneDescriptor;

/// Open a USD file at `path`, walk every `PhysicsRigidBodyAPI` prim,
/// and insert a rapier rigid body for each into `sim.bodies`. Returns
/// a descriptor mapping prim path → handle so callers can find the
/// bodies later. Gearbox's existing world (gravity, ground, planet)
/// is untouched.
pub fn load_usd_into_sim(path: impl AsRef<StdPath>, sim: &mut Sim) -> Result<SceneDescriptor> {
    let path = path.as_ref();
    let path_str = path
        .to_str()
        .with_context(|| format!("USD path is not valid UTF-8: {path:?}"))?;

    // Use the StageBuilder so unresolvable cross-directory payloads
    // (common in Isaac Sim assets like franka.usd that reference
    // sibling-directory grippers) become warnings instead of hard
    // errors. Whatever DOES resolve still loads.
    let stage = Stage::builder()
        .on_error(|err| {
            log::warn!("gearbox-usd: composition error: {err}");
            Ok(())
        })
        .open(path_str)
        .with_context(|| format!("opening USD stage at {path_str}"))?;
    let prims = physics::find_physics_prims(&stage)
        .context("scanning stage for physics prims")?;

    log::info!(
        "gearbox-usd: {path_str} → {} rigid body prim(s), {} collider(s), {} joint(s) (only bodies handled at this stage)",
        prims.rigid_bodies.len(),
        prims.colliders.len(),
        prims.joints.len(),
    );

    let mut descriptor = SceneDescriptor::default();

    for prim_path_str in &prims.rigid_bodies {
        let prim_path = SdfPath::from_str(prim_path_str)
            .with_context(|| format!("parsing prim path {prim_path_str}"))?;

        let rb = physics::read_rigid_body(&stage, &prim_path)
            .with_context(|| format!("reading rigid body at {prim_path_str}"))?
            .unwrap_or_default();
        let mass = physics::read_mass(&stage, &prim_path)
            .with_context(|| format!("reading mass at {prim_path_str}"))?
            .unwrap_or_default();

        let (translation, rotation) = local_pose(&stage, &prim_path);

        let op = RigidBodyOpinion {
            kinematic: rb.kinematic_enabled,
            enabled: rb.rigid_body_enabled,
            starts_asleep: rb.starts_asleep,
            world_translation: translation,
            world_rotation: rotation,
            linvel: rb
                .velocity
                .map(arr3_to_dvec3)
                .unwrap_or(DVec3::ZERO),
            angvel: rb
                .angular_velocity
                .map(|v| arr3_to_dvec3(v) * (std::f64::consts::PI / 180.0))
                .unwrap_or(DVec3::ZERO),
            mass: mass.mass.map(|m| m as f64),
            center_of_mass: mass.center_of_mass.map(arr3_to_dvec3),
            diagonal_inertia: mass.diagonal_inertia.map(arr3_to_dvec3),
            principal_axes: mass.principal_axes.map(quat_wxyz_to_dquat),
        };

        let handle = build_rigid_body(&mut sim.bodies, &op, 0)
            .with_context(|| format!("inserting rigid body for {prim_path_str}"))?;
        descriptor.bodies.insert(prim_path_str.clone(), handle);
    }

    Ok(descriptor)
}

/// Read the prim's *local* TRS via `usd_schema::xform::read_transform`
/// and return it as `(translation, rotation)`. No ancestor composition
/// — step 1 assumes top-level prims. Falls back to identity when the
/// prim has no `xformOpOrder`.
fn local_pose(stage: &Stage, prim: &SdfPath) -> (DVec3, DQuat) {
    match usd_schema::xform::read_transform(stage, prim) {
        Ok(Some(t)) => (
            DVec3::new(t.translate[0] as f64, t.translate[1] as f64, t.translate[2] as f64),
            DQuat::from_xyzw(
                t.rotate[0] as f64,
                t.rotate[1] as f64,
                t.rotate[2] as f64,
                t.rotate[3] as f64,
            ),
        ),
        _ => (DVec3::ZERO, DQuat::IDENTITY),
    }
}

fn arr3_to_dvec3(a: [f32; 3]) -> DVec3 {
    DVec3::new(a[0] as f64, a[1] as f64, a[2] as f64)
}

fn quat_wxyz_to_dquat(q: [f32; 4]) -> DQuat {
    // USD authors quaternions in (w, x, y, z) order.
    DQuat::from_xyzw(q[1] as f64, q[2] as f64, q[3] as f64, q[0] as f64)
}
