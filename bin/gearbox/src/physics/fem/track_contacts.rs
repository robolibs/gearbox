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
    #[serde(default)]
    pub top_half_width: Option<f64>,
    pub friction: f64,
}

impl TrackContactShape {
    fn gpu_descriptor(&self, body: u32, body_count: usize) -> Result<SoftRigidShapeGpu> {
        let kind = match self.kind.as_str() {
            "box" => 1,
            "cylinder" => 2,
            "trapezoid" => 3,
            _ => return Err(invalid("unsupported contact primitive")),
        };
        let norm = self.rotation.iter().map(|v| v * v).sum::<f64>();
        if self.name.is_empty()
            || self.friction < 0.0
            || (kind == 3) != self.top_half_width.is_some()
            || self
                .position
                .iter()
                .chain(&self.rotation)
                .chain(&self.dimensions)
                .chain(self.top_half_width.iter())
                .chain(std::iter::once(&self.friction))
                .any(|v| !v.is_finite() || !(*v as f32).is_finite())
            || (norm - 1.0).abs() > 1e-6
        {
            return Err(invalid("invalid primitive descriptor"));
        }
        let shape = SoftRigidShapeGpu {
            position: [
                self.position[0] as f32,
                self.position[1] as f32,
                self.position[2] as f32,
                self.top_half_width.unwrap_or(0.0) as f32,
            ],
            rotation: self.rotation.map(|v| v as f32),
            data: [
                self.dimensions[0] as f32,
                self.dimensions[1] as f32,
                self.dimensions[2] as f32,
                self.friction as f32,
            ],
            ids: [kind, body, 0, 0],
        };
        shape.validate(body_count)?;
        Ok(shape)
    }
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
                if !unique.insert((shape.body.clone(), shape.name.clone())) {
                    return Err(invalid("duplicate primitive"));
                }
                let descriptor =
                    shape.gpu_descriptor(body.index() as u32, rigid.model.body_count)?;
                covered.insert(entity);
                names.push(format!("{}:{}", shape.body, shape.name));
                shapes.push(descriptor);
            }
            if covered.len() != allowed.len() {
                return Err(invalid("wheel is missing FEM contact geometry"));
            }
        }
        Ok(Self { shapes, names })
    }
}

#[cfg(test)]
mod tests {
    use super::TrackContactShape;

    fn shape(kind: &str) -> TrackContactShape {
        serde_json::from_value(serde_json::json!({"name":"tooth", "body":"/wheel",
            "kind":kind, "position":[0,0,0], "rotation":[0,0,0,1],
            "dimensions":[0.025,0.013,0.013], "friction":0.6}))
        .unwrap()
    }

    #[test]
    fn trapezoid_requires_explicit_valid_top_width() {
        let mut tooth = shape("trapezoid");
        assert!(tooth.gpu_descriptor(0, 1).is_err());
        tooth.top_half_width = Some(0.005);
        let gpu = tooth.gpu_descriptor(0, 1).unwrap();
        assert_eq!(gpu.ids, [3, 0, 0, 0]);
        assert_eq!(gpu.position[3], 0.005);
        assert_eq!(gpu.data, [0.025, 0.013, 0.013, 0.6]);
        for value in [0.0, -0.1, 1e-30, 1e30, f64::NAN, f64::INFINITY] {
            tooth.top_half_width = Some(value);
            assert!(tooth.gpu_descriptor(0, 1).is_err());
        }
    }

    #[test]
    fn legacy_shapes_reject_ignored_profile_metadata() {
        for kind in ["box", "cylinder"] {
            let mut primitive = shape(kind);
            assert!(primitive.gpu_descriptor(0, 1).is_ok());
            primitive.top_half_width = Some(0.005);
            assert!(primitive.gpu_descriptor(0, 1).is_err());
        }
        assert!(shape("unknown").gpu_descriptor(0, 1).is_err());
        assert!(shape("box").gpu_descriptor(1, 1).is_err());
        let mut primitive = shape("box");
        primitive.friction = -1e-50;
        assert!(primitive.gpu_descriptor(0, 1).is_err());
    }
}
