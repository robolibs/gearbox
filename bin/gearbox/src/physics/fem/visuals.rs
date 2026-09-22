use super::{
    machine::FemMachineLayout, rigid_machine::FemRigidMachine, track_mesh::FemTrackMeshes,
};
use crate::controller::MachineInstanceSpec;
use bevy::prelude::*;
use std::collections::{HashMap, HashSet};
use usd_bevy::UsdPrimRef;

#[cfg(test)]
#[path = "visuals_test.rs"]
mod tests;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct TrackVisualSpec {
    pub version: u32,
    pub deformable: Vec<String>,
    pub material: String,
}

pub(crate) struct RigidVisual {
    pub entity: Entity,
    pub body: u32,
    pub mesh_to_body: Mat4,
    pub material: Handle<StandardMaterial>,
}

pub(crate) struct SoftVisual {
    pub originals: Vec<Entity>,
    pub material: Handle<StandardMaterial>,
    pub mesh: Mesh,
}

/// Validated visual bindings prepared before physics ownership or material replacement.
pub(crate) struct FemMachineVisuals {
    pub origin: Vec3,
    pub rigid: Vec<RigidVisual>,
    pub soft: Vec<SoftVisual>,
}

fn beneath(world: &World, mut entity: Entity, root: Entity) -> bool {
    loop {
        if entity == root {
            return true;
        }
        let Some(parent) = world.get::<ChildOf>(entity) else {
            return false;
        };
        entity = parent.parent();
    }
}

impl FemMachineVisuals {
    pub(crate) fn prepare(
        world: &mut World,
        spec: &MachineInstanceSpec,
        layout: &FemMachineLayout,
        rigid: &FemRigidMachine,
        belts: &FemTrackMeshes,
    ) -> Result<Self, String> {
        let root = spec.scene_root.ok_or("FEM visual scene root missing")?;
        if spec.tracks.len() != belts.tracks.len() || spec.tracks.len() != layout.tracks.len() {
            return Err("FEM visual track count mismatch".into());
        }
        let mut paths = HashMap::new();
        for (entity, prim) in world.query::<(Entity, &UsdPrimRef)>().iter(world) {
            if beneath(world, entity, root)
                && (prim.path == spec.prim_path
                    || prim.path.starts_with(&format!("{}/", spec.prim_path)))
                && paths.insert(prim.path.clone(), entity).is_some()
            {
                return Err("duplicate FEM visual prim path".into());
            }
        }
        let resolve = |path: &str| {
            paths
                .get(path)
                .copied()
                .ok_or_else(|| format!("missing FEM visual: {path}"))
        };
        let machine = resolve(&spec.prim_path)?;
        let meshes = world
            .query_filtered::<Entity, With<Mesh3d>>()
            .iter(world)
            .filter(|&e| beneath(world, e, machine))
            .collect::<Vec<_>>();
        let materials = world.resource::<Assets<StandardMaterial>>();
        let source_material = |entity| -> Result<Handle<StandardMaterial>, String> {
            let handle = &world
                .get::<MeshMaterial3d<StandardMaterial>>(entity)
                .ok_or("FEM visual has no StandardMaterial")?
                .0;
            if !materials.contains(handle.id()) {
                return Err("FEM visual material is not loaded".into());
            }
            Ok(handle.clone())
        };
        let reference = belts
            .state
            .particle_q
            .host()
            .map_err(|e| e.to_string())?
            .iter()
            .map(|p| [p.x as f32, p.y as f32, p.z as f32])
            .collect::<Vec<_>>();
        let material_points = belts
            .model
            .initial_particle_q
            .host()
            .map_err(|e| e.to_string())?;
        let mut soft = Vec::new();
        let mut hidden = HashSet::new();
        for ((authored, binding), track) in
            spec.tracks.iter().zip(&layout.tracks).zip(&belts.tracks)
        {
            let visual = authored
                .fem_visuals
                .as_ref()
                .ok_or("missing explicit FEM visual targets")?;
            if visual.version != 1
                || visual.deformable.is_empty()
                || track.carrier != binding.carrier
            {
                return Err("invalid FEM visual version or carrier".into());
            }
            let originals = visual
                .deformable
                .iter()
                .map(|p| resolve(p))
                .collect::<Result<Vec<_>, _>>()?;
            for &entity in &originals {
                if !beneath(world, entity, binding.carrier)
                    || layout
                        .bodies
                        .iter()
                        .any(|&body| beneath(world, body, entity))
                    || binding
                        .contact_patches
                        .iter()
                        .any(|&patch| beneath(world, patch, entity))
                    || !hidden.insert(entity)
                    || !meshes.iter().any(|&mesh| beneath(world, mesh, entity))
                    || originals
                        .iter()
                        .any(|&other| other != entity && beneath(world, entity, other))
                {
                    return Err("shared, empty, overlapping or nonvisual FEM target".into());
                }
            }
            if binding
                .treads
                .iter()
                .any(|&tread| !originals.iter().any(|&e| beneath(world, tread, e)))
            {
                return Err("FEM visuals omit an animated tread".into());
            }
            let material_root = resolve(&visual.material)?;
            if !originals.contains(&material_root) {
                return Err("FEM rubber material must belong to a replaced visual".into());
            }
            let sources = meshes
                .iter()
                .copied()
                .filter(|&e| beneath(world, e, material_root))
                .map(source_material)
                .collect::<Result<Vec<_>, _>>()?;
            let material = sources
                .first()
                .ok_or("FEM rubber material target is empty")?
                .clone();
            if sources.iter().any(|m| *m != material) {
                return Err("FEM rubber material target is ambiguous".into());
            }
            for &mesh in meshes
                .iter()
                .filter(|&&mesh| originals.iter().any(|&e| beneath(world, mesh, e)))
            {
                if source_material(mesh)? != material {
                    return Err("FEM deformable visuals require a shared rubber material".into());
                }
            }
            let mut uvs = vec![[0.0; 2]; reference.len()];
            for &node in &track.nodes {
                let p = material_points[node as usize];
                uvs[node as usize] = [
                    (p.z.atan2(p.y).rem_euclid(std::f64::consts::TAU) / std::f64::consts::TAU)
                        as f32,
                    p.x as f32,
                ];
            }
            soft.push(SoftVisual {
                originals,
                material,
                mesh: super::surface::surface_mesh(&reference, &uvs, &track.surface)?,
            });
        }
        let origin = Vec3::new(
            rigid.origin.x as f32,
            rigid.origin.y as f32,
            rigid.origin.z as f32,
        );
        let poses = rigid.state.body_q.host().map_err(|e| e.to_string())?;
        let mut visuals = Vec::new();
        for entity in meshes {
            if hidden.iter().any(|&root| beneath(world, entity, root)) {
                continue;
            }
            let mut owner = entity;
            let body = loop {
                if let Some(body) = rigid.bodies.get(&owner) {
                    break body.index();
                }
                owner = world
                    .get::<ChildOf>(owner)
                    .ok_or("FEM mesh has no owning rigid body")?
                    .parent();
                if owner == root {
                    return Err("FEM mesh has no owning rigid body".into());
                }
            };
            let pose = poses[body];
            let p = pose.position;
            let q = pose.rotation;
            let body_to_island = Mat4::from_rotation_translation(
                Quat::from_xyzw(q.x as f32, q.y as f32, q.z as f32, q.w as f32),
                Vec3::new(p.x as f32, p.y as f32, p.z as f32),
            );
            let world_from_mesh = world
                .get::<GlobalTransform>(entity)
                .ok_or("FEM visual transform missing")?
                .to_matrix();
            let mesh_to_body =
                body_to_island.inverse() * Mat4::from_translation(-origin) * world_from_mesh;
            if !mesh_to_body.is_finite() || mesh_to_body.determinant().abs() < 1e-12 {
                return Err("FEM rigid visual transform is singular or nonfinite".into());
            }
            visuals.push(RigidVisual {
                entity,
                body: body as u32,
                mesh_to_body,
                material: source_material(entity)?,
            });
        }
        if visuals.is_empty() {
            return Err("FEM machine contains no rigid visual meshes".into());
        }
        Ok(Self {
            origin,
            rigid: visuals,
            soft,
        })
    }
}
