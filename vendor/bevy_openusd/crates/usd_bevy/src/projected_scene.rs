use std::collections::HashMap;

use bevy::ecs::entity::Entity;
use bevy::ecs::hierarchy::ChildOf;
use bevy::ecs::name::Name;
use bevy::ecs::system::Commands;
use bevy::mesh::Mesh3d;
use bevy::mesh::morph::MeshMorphWeights;
use bevy::mesh::skinning::SkinnedMesh;
use bevy::pbr::{MeshMaterial3d, StandardMaterial};
use bevy::prelude::*;

use crate::markers::{
    UsdArticulationRoot, UsdCollider, UsdCollisionFilter, UsdCollisionGroup, UsdMass,
    UsdPhysicsJoint, UsdPhysicsMaterial, UsdPhysicsScene, UsdRigidBody,
};
use crate::prim_ref::{
    UsdBlendShapeBinding, UsdDisplayName, UsdKind, UsdLocalExtent, UsdPrimRef, UsdProcedural,
    UsdPurpose, UsdSkelAnimDriver, UsdSkelRoot, UsdSpatialAudio,
};

/// Marker inserted on the external entity that owns one spawned USD scene.
#[derive(Component, Reflect, Debug, Clone, Copy, Default)]
#[reflect(Component, Default)]
pub struct UsdSceneRoot;

/// A Bevy-0.19-friendly snapshot of the ECS tree projected from a USD stage.
///
/// Bevy 0.19 replaced the old `Scene` asset / `SceneRoot` component with the
/// BSN scene system. Gearbox still wants the classic loader behaviour: load a
/// USD asset, then mount its entity tree under a caller-owned root entity.
/// This plain snapshot keeps that behaviour without depending on Bevy's old
/// scene asset type.
#[derive(Debug, Clone, Default)]
pub struct ProjectedScene {
    records: Vec<ProjectedEntity>,
    source_to_index: HashMap<Entity, usize>,
}

impl ProjectedScene {
    pub fn from_world(world: &World) -> Self {
        let entities: Vec<Entity> = world.iter_entities().map(|entity| entity.id()).collect();
        let source_to_index: HashMap<Entity, usize> = entities
            .iter()
            .enumerate()
            .map(|(idx, entity)| (*entity, idx))
            .collect();

        let records = entities
            .iter()
            .map(|entity| {
                let entity_ref = world.entity(*entity);
                let parent = entity_ref
                    .get::<ChildOf>()
                    .and_then(|parent| source_to_index.get(&parent.parent()).copied());
                ProjectedEntity {
                    parent,
                    name: entity_ref.get::<Name>().cloned(),
                    transform: entity_ref.get::<Transform>().cloned(),
                    visibility: entity_ref.get::<Visibility>().cloned(),
                    mesh: entity_ref.get::<Mesh3d>().cloned(),
                    material: entity_ref
                        .get::<MeshMaterial3d<StandardMaterial>>()
                        .cloned(),
                    directional_light: entity_ref.get::<DirectionalLight>().cloned(),
                    point_light: entity_ref.get::<PointLight>().cloned(),
                    spot_light: entity_ref.get::<SpotLight>().cloned(),
                    skinned_mesh: entity_ref.get::<SkinnedMesh>().cloned(),
                    morph_weights: entity_ref.get::<MeshMorphWeights>().cloned(),
                    usd_prim: entity_ref.get::<UsdPrimRef>().cloned(),
                    usd_local_extent: entity_ref.get::<UsdLocalExtent>().cloned(),
                    usd_kind: entity_ref.get::<UsdKind>().cloned(),
                    usd_display_name: entity_ref.get::<UsdDisplayName>().cloned(),
                    usd_purpose: entity_ref.get::<UsdPurpose>().cloned(),
                    usd_spatial_audio: entity_ref.get::<UsdSpatialAudio>().cloned(),
                    usd_procedural: entity_ref.get::<UsdProcedural>().cloned(),
                    usd_skel_root: entity_ref.get::<UsdSkelRoot>().cloned(),
                    usd_skel_anim_driver: entity_ref.get::<UsdSkelAnimDriver>().cloned(),
                    usd_blend_shape_binding: entity_ref.get::<UsdBlendShapeBinding>().cloned(),
                    usd_physics_scene: entity_ref.get::<UsdPhysicsScene>().cloned(),
                    usd_rigid_body: entity_ref.get::<UsdRigidBody>().cloned(),
                    usd_mass: entity_ref.get::<UsdMass>().cloned(),
                    usd_collider: entity_ref.get::<UsdCollider>().cloned(),
                    usd_physics_material: entity_ref.get::<UsdPhysicsMaterial>().cloned(),
                    usd_articulation_root: entity_ref.get::<UsdArticulationRoot>().cloned(),
                    usd_physics_joint: entity_ref.get::<UsdPhysicsJoint>().cloned(),
                    usd_collision_group: entity_ref.get::<UsdCollisionGroup>().cloned(),
                    usd_collision_filter: entity_ref.get::<UsdCollisionFilter>().cloned(),
                }
            })
            .collect();

        Self {
            records,
            source_to_index,
        }
    }

    /// Spawn this projected USD tree under `root`.
    ///
    /// The caller-owned `root` keeps its own `Name`, `Transform`, and metadata;
    /// every projected USD entity becomes a child below it. This matches the
    /// old `SceneRoot` mounting model Gearbox's loader/controller code expects.
    pub fn spawn_under(&self, commands: &mut Commands<'_, '_>, root: Entity) -> Vec<Entity> {
        let mut spawned = Vec::with_capacity(self.records.len());
        for _ in &self.records {
            spawned.push(commands.spawn_empty().id());
        }

        for (idx, record) in self.records.iter().enumerate() {
            let entity = spawned[idx];
            let parent = record.parent.map(|parent| spawned[parent]).unwrap_or(root);
            let mut entity_commands = commands.entity(entity);
            entity_commands.insert(ChildOf(parent));

            if let Some(value) = &record.name {
                entity_commands.insert(value.clone());
            }
            if let Some(value) = record.transform {
                entity_commands.insert(value);
            }
            if let Some(value) = record.visibility {
                entity_commands.insert(value);
            }
            if let Some(value) = &record.mesh {
                entity_commands.insert(value.clone());
            }
            if let Some(value) = &record.material {
                entity_commands.insert(value.clone());
            }
            if let Some(value) = &record.directional_light {
                entity_commands.insert(value.clone());
            }
            if let Some(value) = &record.point_light {
                entity_commands.insert(value.clone());
            }
            if let Some(value) = &record.spot_light {
                entity_commands.insert(value.clone());
            }
            if let Some(value) = &record.skinned_mesh {
                let mut value = value.clone();
                value.joints = value
                    .joints
                    .iter()
                    .filter_map(|joint| self.remap(*joint, &spawned))
                    .collect();
                entity_commands.insert(value);
            }
            if let Some(value) = &record.morph_weights {
                entity_commands.insert(match value {
                    MeshMorphWeights::Value { weights } => MeshMorphWeights::Value {
                        weights: weights.clone(),
                    },
                    MeshMorphWeights::Reference(entity) => self
                        .remap(*entity, &spawned)
                        .map(MeshMorphWeights::Reference)
                        .unwrap_or_else(|| value.clone()),
                });
            }
            if let Some(value) = &record.usd_prim {
                entity_commands.insert(value.clone());
            }
            if let Some(value) = record.usd_local_extent {
                entity_commands.insert(value);
            }
            if let Some(value) = &record.usd_kind {
                entity_commands.insert(value.clone());
            }
            if let Some(value) = &record.usd_display_name {
                entity_commands.insert(value.clone());
            }
            if let Some(value) = record.usd_purpose {
                entity_commands.insert(value);
            }
            if let Some(value) = &record.usd_spatial_audio {
                entity_commands.insert(value.clone());
            }
            if let Some(value) = &record.usd_procedural {
                entity_commands.insert(value.clone());
            }
            if let Some(value) = &record.usd_skel_root {
                entity_commands.insert(value.clone());
            }
            if let Some(value) = &record.usd_skel_anim_driver {
                let mut value = value.clone();
                value.skeleton_joint_entities = value
                    .skeleton_joint_entities
                    .iter()
                    .map(|entity| entity.and_then(|entity| self.remap(entity, &spawned)))
                    .collect();
                value.joint_entities = value
                    .joint_entities
                    .iter()
                    .map(|entity| entity.and_then(|entity| self.remap(entity, &spawned)))
                    .collect();
                entity_commands.insert(value.clone());
            }
            if let Some(value) = &record.usd_blend_shape_binding {
                entity_commands.insert(value.clone());
            }
            if let Some(value) = &record.usd_physics_scene {
                entity_commands.insert(value.clone());
            }
            if let Some(value) = &record.usd_rigid_body {
                entity_commands.insert(value.clone());
            }
            if let Some(value) = &record.usd_mass {
                entity_commands.insert(value.clone());
            }
            if let Some(value) = &record.usd_collider {
                let mut value = value.clone();
                value.physics_material = value
                    .physics_material
                    .and_then(|entity| self.remap(entity, &spawned));
                entity_commands.insert(value);
            }
            if let Some(value) = &record.usd_physics_material {
                entity_commands.insert(value.clone());
            }
            if let Some(value) = &record.usd_articulation_root {
                let mut value = value.clone();
                value.joints = value
                    .joints
                    .iter()
                    .filter_map(|joint| self.remap(*joint, &spawned))
                    .collect();
                entity_commands.insert(value);
            }
            if let Some(value) = &record.usd_physics_joint {
                let mut value = value.clone();
                value.body0 = value.body0.and_then(|entity| self.remap(entity, &spawned));
                value.body1 = value.body1.and_then(|entity| self.remap(entity, &spawned));
                entity_commands.insert(value);
            }
            if let Some(value) = &record.usd_collision_group {
                let mut value = value.clone();
                value.members = value
                    .members
                    .iter()
                    .filter_map(|entity| self.remap(*entity, &spawned))
                    .collect();
                value.filtered = value
                    .filtered
                    .iter()
                    .filter_map(|entity| self.remap(*entity, &spawned))
                    .collect();
                entity_commands.insert(value);
            }
            if let Some(value) = &record.usd_collision_filter {
                let mut value = value.clone();
                value.filtered = value
                    .filtered
                    .iter()
                    .filter_map(|entity| self.remap(*entity, &spawned))
                    .collect();
                entity_commands.insert(value);
            }
        }

        commands.entity(root).insert(UsdSceneRoot);
        spawned
    }

    fn remap(&self, source: Entity, spawned: &[Entity]) -> Option<Entity> {
        self.source_to_index
            .get(&source)
            .and_then(|idx| spawned.get(*idx))
            .copied()
    }
}

#[derive(Debug, Clone, Default)]
struct ProjectedEntity {
    parent: Option<usize>,
    name: Option<Name>,
    transform: Option<Transform>,
    visibility: Option<Visibility>,
    mesh: Option<Mesh3d>,
    material: Option<MeshMaterial3d<StandardMaterial>>,
    directional_light: Option<DirectionalLight>,
    point_light: Option<PointLight>,
    spot_light: Option<SpotLight>,
    skinned_mesh: Option<SkinnedMesh>,
    morph_weights: Option<MeshMorphWeights>,
    usd_prim: Option<UsdPrimRef>,
    usd_local_extent: Option<UsdLocalExtent>,
    usd_kind: Option<UsdKind>,
    usd_display_name: Option<UsdDisplayName>,
    usd_purpose: Option<UsdPurpose>,
    usd_spatial_audio: Option<UsdSpatialAudio>,
    usd_procedural: Option<UsdProcedural>,
    usd_skel_root: Option<UsdSkelRoot>,
    usd_skel_anim_driver: Option<UsdSkelAnimDriver>,
    usd_blend_shape_binding: Option<UsdBlendShapeBinding>,
    usd_physics_scene: Option<UsdPhysicsScene>,
    usd_rigid_body: Option<UsdRigidBody>,
    usd_mass: Option<UsdMass>,
    usd_collider: Option<UsdCollider>,
    usd_physics_material: Option<UsdPhysicsMaterial>,
    usd_articulation_root: Option<UsdArticulationRoot>,
    usd_physics_joint: Option<UsdPhysicsJoint>,
    usd_collision_group: Option<UsdCollisionGroup>,
    usd_collision_filter: Option<UsdCollisionFilter>,
}
