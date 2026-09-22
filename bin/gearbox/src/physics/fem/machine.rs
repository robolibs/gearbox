use crate::controller::MachineInstanceSpec;
use crate::physics::markers::{UsdCollider, UsdJointKind, UsdPhysicsJoint, UsdRigidBody};
use bevy::prelude::*;
use std::collections::{BTreeMap, HashSet};
use usd_bevy::UsdPrimRef;

#[derive(Debug)]
pub(crate) struct TrackBinding {
    pub carrier: Entity,
    pub sprocket: Entity,
    pub drive_joint: Entity,
    pub passive_joints: Vec<Entity>,
    pub contact_patches: Vec<Entity>,
    pub treads: Vec<Entity>,
}

/// Resolved machine topology; inspecting it does not transfer physics ownership.
#[derive(Debug)]
pub(crate) struct FemMachineLayout {
    pub chassis: Entity,
    pub bodies: Vec<Entity>,
    pub tree_joints: Vec<Entity>,
    pub loop_joints: Vec<Entity>,
    pub tracks: Vec<TrackBinding>,
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

impl FemMachineLayout {
    pub(crate) fn inspect(
        world: &mut World,
        machine: &MachineInstanceSpec,
    ) -> Result<Self, String> {
        let root = machine.scene_root.ok_or("FEM machine has no scene root")?;
        let prefix = format!("{}/", machine.prim_path);
        let mut paths = BTreeMap::new();
        for (entity, prim) in world.query::<(Entity, &UsdPrimRef)>().iter(world) {
            if (prim.path == machine.prim_path || prim.path.starts_with(&prefix))
                && beneath(world, entity, root)
            {
                if paths.insert(prim.path.clone(), entity).is_some() {
                    return Err(format!("duplicate FEM prim path: {}", prim.path));
                }
            }
        }
        let resolve = |path: &str| {
            paths
                .get(path)
                .copied()
                .ok_or_else(|| format!("FEM target is missing or outside its machine: {path}"))
        };
        let chassis = resolve(
            machine
                .body
                .as_deref()
                .ok_or("FEM machine has no chassis")?,
        )?;
        let mut body_set = HashSet::new();
        let mut joints = Vec::new();
        for (&entity, path) in paths.values().zip(paths.keys()) {
            if let Some(body) = world.get::<UsdRigidBody>(entity) {
                if !body.enabled || body.kinematic {
                    return Err(format!("FEM body must be enabled and dynamic: {path}"));
                }
                body_set.insert(entity);
            }
            if let Some(joint) = world.get::<UsdPhysicsJoint>(entity) {
                if joint.joint_enabled {
                    joints.push((entity, joint));
                }
            }
        }
        if !body_set.contains(&chassis) {
            return Err("FEM chassis is not a rigid body".into());
        }
        let mut tree = Vec::new();
        let mut loop_joints = Vec::new();
        let mut children = HashSet::new();
        for (entity, joint) in joints {
            let (Some(parent), Some(child)) = (joint.body0, joint.body1) else {
                return Err("FEM tracked machine contains a world-anchored joint".into());
            };
            if parent == child || !body_set.contains(&parent) || !body_set.contains(&child) {
                return Err("FEM joint references a foreign or invalid body".into());
            }
            if joint.exclude_from_articulation {
                loop_joints.push(entity);
            } else {
                if child == chassis || !children.insert(child) {
                    return Err(
                        "FEM articulation has multiple parents or a reversed chassis joint".into(),
                    );
                }
                tree.push((entity, parent, child));
            }
        }
        let mut bodies = vec![chassis];
        let mut tree_joints = Vec::new();
        let mut visited = HashSet::from([chassis]);
        while !tree.is_empty() {
            let Some(index) = tree
                .iter()
                .position(|(_, parent, _)| visited.contains(parent))
            else {
                return Err("FEM articulation is cyclic or disconnected".into());
            };
            let (joint, _, child) = tree.remove(index);
            visited.insert(child);
            bodies.push(child);
            tree_joints.push(joint);
        }
        if visited != body_set {
            return Err("FEM machine has disconnected bodies".into());
        }
        if machine.tracks.len() != 2
            || machine.tracks[0].side * machine.tracks[1].side != -1.0
            || machine.tracks.iter().any(|t| t.side.abs() != 1.0)
            || machine.powered_wheel_joints.len() != 2
        {
            return Err("FEM machine requires one left and one right track".into());
        }
        let mut tracks = Vec::new();
        let mut owned = HashSet::new();
        let mut owned_joints = HashSet::new();
        for spec in &machine.tracks {
            let carrier = resolve(&spec.carrier)?;
            let sprocket = resolve(&spec.sprocket)?;
            if !body_set.contains(&carrier)
                || !body_set.contains(&sprocket)
                || !owned.insert(sprocket)
            {
                return Err("FEM track carrier or sprocket ownership is invalid".into());
            }
            let drive_joint = *tree_joints
                .iter()
                .find(|&&entity| {
                    let joint = world.get::<UsdPhysicsJoint>(entity).unwrap();
                    joint.body0 == Some(carrier)
                        && joint.body1 == Some(sprocket)
                        && joint.kind == UsdJointKind::Revolute
                })
                .ok_or("FEM sprocket has no carrier revolute joint")?;
            if !machine
                .powered_wheel_joints
                .iter()
                .any(|p| resolve(p) == Ok(drive_joint))
            {
                return Err("FEM drive joint is not authored as powered".into());
            }
            if !owned_joints.insert(drive_joint) {
                return Err("shared FEM drive joint".into());
            }
            let mut passive_joints = Vec::new();
            for path in &machine.passive_wheel_joints {
                let entity = resolve(path)?;
                let joint = world
                    .get::<UsdPhysicsJoint>(entity)
                    .ok_or("FEM passive wheel target is not a joint")?;
                if joint.body0 == Some(carrier) {
                    if joint.kind != UsdJointKind::Revolute
                        || !tree_joints.contains(&entity)
                        || !owned_joints.insert(entity)
                    {
                        return Err("FEM passive wheel joint is invalid".into());
                    }
                    passive_joints.push(entity);
                }
            }
            let contact_patches = spec
                .contacts
                .iter()
                .map(|p| resolve(p))
                .collect::<Result<Vec<_>, _>>()?;
            let treads = spec
                .treads
                .iter()
                .map(|p| resolve(p))
                .collect::<Result<Vec<_>, _>>()?;
            if contact_patches.is_empty() || treads.is_empty() || passive_joints.is_empty() {
                return Err("FEM track is missing contacts, treads or passive wheels".into());
            }
            for &entity in contact_patches.iter().chain(&treads) {
                if !beneath(world, entity, carrier) || !owned.insert(entity) {
                    return Err("FEM track geometry is shared or outside its carrier".into());
                }
            }
            if contact_patches
                .iter()
                .any(|&e| !world.get::<UsdCollider>(e).is_some_and(|c| c.enabled))
            {
                return Err("FEM track contact target is not an enabled collider".into());
            }
            tracks.push(TrackBinding {
                carrier,
                sprocket,
                drive_joint,
                passive_joints,
                contact_patches,
                treads,
            });
        }
        if owned_joints.len()
            != machine.powered_wheel_joints.len() + machine.passive_wheel_joints.len()
        {
            return Err("FEM machine contains unbound wheel joints".into());
        }
        Ok(Self {
            chassis,
            bodies,
            tree_joints,
            loop_joints,
            tracks,
        })
    }
}
