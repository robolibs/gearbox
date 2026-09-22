use super::{machine::FemMachineLayout, rigid_machine::FemRigidMachine};
use crate::{controller::MachineInstanceSpec, physics::markers::UsdPhysicsJoint};
use bevy::prelude::{Entity, World};
use molla_core::{Error, Result};
use molla_solvers::fem_rigid_gpu::SoftRigidShapeGpu;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct TrackContactSpec {
    pub version: u32,
    pub calibration: String,
    pub shapes: Vec<TrackContactShape>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct TrackContactShape {
    pub name: String,
    pub body: String,
    pub kind: String,
    pub position: [f64; 3],
    pub rotation: [f64; 4],
    pub dimensions: [f64; 3],
    pub friction: f64,
}

pub(crate) struct FemTrackContacts {
    pub shapes: Vec<SoftRigidShapeGpu>,
    pub names: Vec<String>,
}

fn invalid(message: &str) -> Error {
    Error::Build(format!("FEM track contacts: {message}"))
}

impl FemTrackContacts {
    pub(crate) fn prepare(
        world: &World,
        spec: &MachineInstanceSpec,
        layout: &FemMachineLayout,
        rigid: &FemRigidMachine,
    ) -> Result<Self> {
        if spec.tracks.len() != layout.tracks.len() {
            return Err(invalid("track count mismatch"));
        }
        let mut shapes = Vec::new();
        let mut names = Vec::new();
        let mut unique = HashSet::new();
        for (track, binding) in spec.tracks.iter().zip(&layout.tracks) {
            let contacts = track
                .fem_contacts
                .as_ref()
                .ok_or_else(|| invalid("missing authored geometry"))?;
            if contacts.version != 1
                || !matches!(contacts.calibration.as_str(), "estimated" | "measured")
            {
                return Err(invalid("unsupported geometry version or calibration"));
            }
            let mut allowed = HashMap::<String, Entity>::new();
            for joint in std::iter::once(&binding.drive_joint).chain(&binding.passive_joints) {
                let body = world
                    .get::<UsdPhysicsJoint>(*joint)
                    .and_then(|j| j.body1)
                    .ok_or_else(|| invalid("wheel joint has no child"))?;
                let path = &world
                    .get::<usd_bevy::UsdPrimRef>(body)
                    .ok_or_else(|| invalid("wheel body has no prim path"))?
                    .path;
                allowed.insert(path.clone(), body);
            }
            let mut covered = HashSet::new();
            for shape in &contacts.shapes {
                let entity = *allowed
                    .get(&shape.body)
                    .ok_or_else(|| invalid("contact belongs to a foreign body"))?;
                let body = rigid
                    .bodies
                    .get(&entity)
                    .ok_or_else(|| invalid("contact body absent from articulation"))?;
                let kind = match shape.kind.as_str() {
                    "box" => 1,
                    "cylinder" => 2,
                    _ => return Err(invalid("unsupported contact primitive")),
                };
                let norm = shape.rotation.iter().map(|v| v * v).sum::<f64>();
                if shape.name.is_empty()
                    || !unique.insert((shape.body.clone(), shape.name.clone()))
                    || shape
                        .position
                        .iter()
                        .chain(&shape.rotation)
                        .chain(&shape.dimensions)
                        .chain(std::iter::once(&shape.friction))
                        .any(|v| !v.is_finite() || !(*v as f32).is_finite())
                    || (norm - 1.0).abs() > 1e-6
                    || shape.friction < 0.0
                    || shape.dimensions[0] <= 0.0
                    || shape.dimensions[1] <= 0.0
                    || (kind == 1 && shape.dimensions[2] <= 0.0)
                {
                    return Err(invalid("invalid or duplicate primitive"));
                }
                covered.insert(entity);
                names.push(format!("{}:{}", shape.body, shape.name));
                shapes.push(SoftRigidShapeGpu {
                    position: [
                        shape.position[0] as f32,
                        shape.position[1] as f32,
                        shape.position[2] as f32,
                        0.0,
                    ],
                    rotation: shape.rotation.map(|v| v as f32),
                    data: [
                        shape.dimensions[0] as f32,
                        shape.dimensions[1] as f32,
                        shape.dimensions[2] as f32,
                        shape.friction as f32,
                    ],
                    ids: [kind, body.index() as u32, 0, 0],
                });
            }
            if covered.len() != allowed.len() {
                return Err(invalid("wheel is missing FEM contact geometry"));
            }
        }
        Ok(Self { shapes, names })
    }
}
