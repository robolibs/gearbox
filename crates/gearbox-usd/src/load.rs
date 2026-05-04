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
use glam::{DMat4, DQuat, DVec3};
use openusd::Stage;
use openusd::physics;
use openusd::sdf::Path as SdfPath;

use gearbox_physics::Sim;
use rapier3d::prelude::ColliderBuilder;
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

    // USD-authored frame → gearbox's Y-up meters frame. The sim is
    // always Y-up (gearbox convention so gravity = -Y matches the
    // ground plane), so anything authored Z-up has to be rotated on
    // the way in. Same for `metersPerUnit != 1.0` (Pixar's reference
    // assets default to centimetres when unauthored).
    let basis = stage_to_gearbox_basis(&stage);
    log::info!(
        "gearbox-usd: stage basis (USD→Y-up,m): rot={:?}, scale={}",
        basis.rotation, basis.uniform_scale
    );

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

        let (usd_translation, usd_rotation) = local_pose(&stage, &prim_path);
        let (translation, rotation) = basis.apply_pose(usd_translation, usd_rotation);

        let op = RigidBodyOpinion {
            kinematic: rb.kinematic_enabled,
            enabled: rb.rigid_body_enabled,
            starts_asleep: rb.starts_asleep,
            world_translation: translation,
            world_rotation: rotation,
            linvel: rb
                .velocity
                .map(|v| basis.apply_vec(arr3_to_dvec3(v)))
                .unwrap_or(DVec3::ZERO),
            angvel: rb
                .angular_velocity
                .map(|v| {
                    basis.apply_axial(arr3_to_dvec3(v) * (std::f64::consts::PI / 180.0))
                })
                .unwrap_or(DVec3::ZERO),
            mass: mass.mass.map(|m| m as f64),
            center_of_mass: mass.center_of_mass.map(|c| basis.apply_vec(arr3_to_dvec3(c))),
            diagonal_inertia: mass.diagonal_inertia.map(arr3_to_dvec3),
            principal_axes: mass.principal_axes.map(quat_wxyz_to_dquat),
        };

        let handle = build_rigid_body(&mut sim.bodies, &op, 0)
            .with_context(|| format!("inserting rigid body for {prim_path_str}"))?;
        descriptor.bodies.insert(prim_path_str.clone(), handle);

        // STOPGAP: real `UsdPhysicsCollisionAPI` shape decoding is the
        // next step. For now, attach a tiny ball so dynamic bodies
        // collide with the world (otherwise they fall through the
        // ground forever and the demo "just disappears"). Sized small
        // enough not to overlap neighbouring links into separation
        // forces; large enough to register at default solver
        // sub-stepping. Safe to remove once full collider handling
        // lands in `gearbox-usd::load`.
        let placeholder = ColliderBuilder::ball(0.05).build();
        sim.colliders.insert_with_parent(placeholder, handle, &mut sim.bodies);
    }

    descriptor.basis = basis;
    Ok(descriptor)
}

/// USD-authored frame → gearbox sim frame (Y-up, metres) conversion.
/// Built once per stage, applied on every body pose / vector that
/// crosses the boundary.
#[derive(Debug, Clone, Copy)]
pub struct StageBasis {
    /// Rotation that maps USD's authored up-axis to +Y. `IDENTITY`
    /// when the stage is already Y-up.
    pub rotation: DQuat,
    /// Multiplicative scale `metersPerUnit` (1.0 when absent or
    /// authored to 1.0; Pixar's spec default is 0.01 = centimetres).
    pub uniform_scale: f64,
}

impl Default for StageBasis {
    fn default() -> Self {
        Self {
            rotation: DQuat::IDENTITY,
            uniform_scale: 1.0,
        }
    }
}

impl StageBasis {
    /// Bake the basis into a TRS pose authored in USD's frame.
    pub fn apply_pose(&self, translation: DVec3, rotation: DQuat) -> (DVec3, DQuat) {
        let trans = self.apply_vec(translation);
        let rot = self.rotation * rotation;
        (trans, rot)
    }

    /// Apply rotation + scale to a position-like vector (translation,
    /// centre of mass, linear velocity).
    pub fn apply_vec(&self, v: DVec3) -> DVec3 {
        (self.rotation * v) * self.uniform_scale
    }

    /// Apply rotation only — for axial vectors (angular velocity)
    /// where scale doesn't apply.
    pub fn apply_axial(&self, v: DVec3) -> DVec3 {
        self.rotation * v
    }

    /// 4×4 form, useful when callers need to compose with a full
    /// transform stack.
    pub fn matrix(&self) -> DMat4 {
        DMat4::from_scale_rotation_translation(
            DVec3::splat(self.uniform_scale),
            self.rotation,
            DVec3::ZERO,
        )
    }
}

fn stage_to_gearbox_basis(stage: &Stage) -> StageBasis {
    use openusd::sdf::Value;

    let up_axis = stage
        .field::<String>(SdfPath::abs_root(), "upAxis")
        .ok()
        .flatten();

    let mpu = stage
        .field::<Value>(SdfPath::abs_root(), "metersPerUnit")
        .ok()
        .flatten()
        .and_then(|v| match v {
            Value::Double(d) => Some(d),
            Value::Float(f) => Some(f as f64),
            Value::Int(i) => Some(i as f64),
            Value::Int64(i) => Some(i as f64),
            _ => None,
        })
        .unwrap_or(1.0);

    let rotation = match up_axis.as_deref() {
        Some("Z") => DQuat::from_rotation_x(-std::f64::consts::FRAC_PI_2),
        // "Y" is the spec default; "X" is exotic enough that we'd
        // rather log + leave the user to handle than guess.
        _ => DQuat::IDENTITY,
    };

    StageBasis {
        rotation,
        uniform_scale: mpu,
    }
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
