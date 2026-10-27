//! Prim entities by stage path, with their parents and joint markers, kept
//! current by observers. Control code resolves machine parts through it, so
//! a controller tick runs the same in a world that holds no prim entities.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use bevy::ecs::lifecycle::{Discard, Insert};
use bevy::prelude::*;
use usd_bevy::UsdPrimRef;

use crate::physics::markers::UsdPhysicsJoint;

/// A clone shares the tables until the original next changes.
#[derive(Resource, Clone, Default)]
pub struct PrimIndex(Arc<Tables>);

#[derive(Clone, Default)]
struct Tables {
    paths: HashMap<Entity, String>,
    entities: HashMap<String, Vec<Entity>>,
    parents: HashMap<Entity, Entity>,
    joints: HashMap<Entity, UsdPhysicsJoint>,
    found: Found,
}

/// Answers by stage root and path, until the tables next change.
#[derive(Default)]
struct Found(Mutex<HashMap<Entity, HashMap<Box<str>, Answer>>>);

impl Clone for Found {
    fn clone(&self) -> Self {
        Self::default()
    }
}

/// The first prim at a path under a root, and the first of them that is a joint.
#[derive(Clone, Copy)]
struct Answer {
    prim: Option<Entity>,
    joint: Option<Entity>,
}

impl PrimIndex {
    /// The entity of the prim at `path` in the stage under `root`.
    pub fn find(&self, root: Entity, path: &str) -> Option<Entity> {
        self.answer(root, path).prim
    }

    /// The joint prim at `path` in the stage under `root`, with its marker.
    pub fn find_joint(&self, root: Entity, path: &str) -> Option<(Entity, &UsdPhysicsJoint)> {
        let entity = self.answer(root, path).joint?;
        Some((entity, self.0.joints.get(&entity)?))
    }

    fn answer(&self, root: Entity, path: &str) -> Answer {
        let mut found = self.0.found.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(answer) = found.get(&root).and_then(|paths| paths.get(path)) {
            return *answer;
        }
        let mut answer = Answer { prim: None, joint: None };
        for &entity in self.0.entities.get(path).into_iter().flatten() {
            if !self.is_descendant_of(entity, root) {
                continue;
            }
            answer.prim.get_or_insert(entity);
            if self.0.joints.contains_key(&entity) {
                answer.joint = Some(entity);
                break;
            }
        }
        found.entry(root).or_default().insert(path.into(), answer);
        answer
    }

    /// The tables to change, answers forgotten.
    fn tables(&mut self) -> &mut Tables {
        let tables = Arc::make_mut(&mut self.0);
        tables.found.0.get_mut().unwrap_or_else(PoisonError::into_inner).clear();
        tables
    }

    /// Every joint prim: its entity, path and marker.
    pub fn joints(&self) -> impl Iterator<Item = (Entity, &str, &UsdPhysicsJoint)> {
        self.0
            .joints
            .iter()
            .filter_map(|(entity, joint)| Some((*entity, self.0.paths.get(entity)?.as_str(), joint)))
    }

    /// Keep `world`'s index current from now on.
    pub fn track(world: &mut World) {
        world.init_resource::<PrimIndex>();
        world.add_observer(index_prim);
        world.add_observer(forget_prim);
        world.add_observer(index_parent);
        world.add_observer(forget_parent);
        world.add_observer(index_joint);
        world.add_observer(forget_joint);
    }

    pub fn is_descendant_of(&self, entity: Entity, root: Entity) -> bool {
        let mut current = entity;
        for _ in 0..64 {
            if current == root {
                return true;
            }
            let Some(parent) = self.0.parents.get(&current) else {
                return false;
            };
            current = *parent;
        }
        false
    }
}

pub struct PrimIndexPlugin;

impl Plugin for PrimIndexPlugin {
    fn build(&self, app: &mut App) {
        PrimIndex::track(app.world_mut());
    }
}

fn index_prim(insert: On<Insert, UsdPrimRef>, prims: Query<(&UsdPrimRef, Option<&ChildOf>)>, mut index: ResMut<PrimIndex>) {
    let Ok((prim, parent)) = prims.get(insert.entity) else {
        return;
    };
    let tables = index.tables();
    tables.paths.insert(insert.entity, prim.path.clone());
    tables.entities.entry(prim.path.clone()).or_default().push(insert.entity);
    if let Some(parent) = parent {
        tables.parents.insert(insert.entity, parent.parent());
    }
}

fn forget_prim(discard: On<Discard, UsdPrimRef>, mut index: ResMut<PrimIndex>) {
    let tables = index.tables();
    tables.parents.remove(&discard.entity);
    let Some(path) = tables.paths.remove(&discard.entity) else {
        return;
    };
    if let Some(entities) = tables.entities.get_mut(&path) {
        entities.retain(|entity| *entity != discard.entity);
        if entities.is_empty() {
            tables.entities.remove(&path);
        }
    }
}

fn index_parent(insert: On<Insert, ChildOf>, prims: Query<&ChildOf, With<UsdPrimRef>>, mut index: ResMut<PrimIndex>) {
    if let Ok(parent) = prims.get(insert.entity) {
        index.tables().parents.insert(insert.entity, parent.parent());
    }
}

fn forget_parent(discard: On<Discard, ChildOf>, prims: Query<(), With<UsdPrimRef>>, mut index: ResMut<PrimIndex>) {
    if prims.contains(discard.entity) {
        index.tables().parents.remove(&discard.entity);
    }
}

fn index_joint(insert: On<Insert, UsdPhysicsJoint>, joints: Query<&UsdPhysicsJoint>, mut index: ResMut<PrimIndex>) {
    if let Ok(joint) = joints.get(insert.entity) {
        index.tables().joints.insert(insert.entity, joint.clone());
    }
}

fn forget_joint(discard: On<Discard, UsdPhysicsJoint>, mut index: ResMut<PrimIndex>) {
    index.tables().joints.remove(&discard.entity);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(PrimIndexPlugin);
        app
    }

    #[test]
    fn a_path_resolves_only_under_its_own_stage() {
        let mut app = app();
        let world = app.world_mut();
        let (a, b) = (world.spawn_empty().id(), world.spawn_empty().id());
        let in_a = world.spawn((UsdPrimRef::new("/Rover/Chassis"), ChildOf(a))).id();
        let in_b = world.spawn((UsdPrimRef::new("/Rover/Chassis"), ChildOf(b))).id();
        let index = world.resource::<PrimIndex>();
        assert_eq!(index.find(a, "/Rover/Chassis"), Some(in_a));
        assert_eq!(index.find(b, "/Rover/Chassis"), Some(in_b));
    }

    #[test]
    fn despawned_and_reparented_prims_leave_the_index() {
        let mut app = app();
        let world = app.world_mut();
        let (a, b) = (world.spawn_empty().id(), world.spawn_empty().id());
        let link = world.spawn((UsdPrimRef::new("/Arm/Link"), ChildOf(a))).id();
        let joint = world.spawn((UsdPrimRef::new("/Arm/Joint"), ChildOf(a), UsdPhysicsJoint::default())).id();
        let unjointed = world.spawn((UsdPrimRef::new("/Arm/Pin"), ChildOf(a), UsdPhysicsJoint::default())).id();
        world.entity_mut(link).insert(ChildOf(b));
        world.despawn(joint);
        world.entity_mut(unjointed).remove::<UsdPhysicsJoint>();
        let index = world.resource::<PrimIndex>();
        assert_eq!(index.find(a, "/Arm/Link"), None);
        assert_eq!(index.find(b, "/Arm/Link"), Some(link));
        assert!(index.find_joint(a, "/Arm/Joint").is_none());
        assert_eq!(index.find(a, "/Arm/Pin"), Some(unjointed));
        assert!(index.find_joint(a, "/Arm/Pin").is_none());
        assert_eq!(index.joints().count(), 0);
    }

    #[test]
    fn answers_follow_prims_that_arrive_move_and_leave_after_a_lookup() {
        let mut app = app();
        let world = app.world_mut();
        let (a, b) = (world.spawn_empty().id(), world.spawn_empty().id());
        assert_eq!(world.resource::<PrimIndex>().find(a, "/Arm/Link"), None);
        assert!(world.resource::<PrimIndex>().find_joint(a, "/Arm/Joint").is_none());
        let link = world.spawn((UsdPrimRef::new("/Arm/Link"), ChildOf(a))).id();
        let joint = world.spawn((UsdPrimRef::new("/Arm/Joint"), ChildOf(a), UsdPhysicsJoint::default())).id();
        let index = world.resource::<PrimIndex>();
        assert_eq!(index.find(a, "/Arm/Link"), Some(link));
        assert_eq!(index.find_joint(a, "/Arm/Joint").map(|(entity, _)| entity), Some(joint));
        world.entity_mut(link).insert(ChildOf(b));
        world.despawn(joint);
        let index = world.resource::<PrimIndex>();
        assert_eq!(index.find(a, "/Arm/Link"), None);
        assert_eq!(index.find(b, "/Arm/Link"), Some(link));
        assert_eq!(index.find(a, "/Arm/Joint"), None);
        assert!(index.find_joint(a, "/Arm/Joint").is_none());
    }

    #[test]
    fn a_reinserted_joint_marker_replaces_the_indexed_one() {
        let mut app = app();
        let world = app.world_mut();
        let root = world.spawn_empty().id();
        let body = world.spawn_empty().id();
        let joint = world.spawn((UsdPrimRef::new("/Arm/Joint"), ChildOf(root), UsdPhysicsJoint::default())).id();
        world.entity_mut(joint).insert(UsdPhysicsJoint { body1: Some(body), ..default() });
        let index = world.resource::<PrimIndex>();
        assert_eq!(index.find_joint(root, "/Arm/Joint").map(|(_, j)| j.body1), Some(Some(body)));
    }

    #[test]
    fn a_snapshot_keeps_its_tables_while_the_original_changes() {
        let mut app = app();
        let world = app.world_mut();
        let root = world.spawn_empty().id();
        let wheel = world.spawn((UsdPrimRef::new("/Rover/Wheel"), ChildOf(root))).id();
        let snapshot = world.resource::<PrimIndex>().clone();
        world.despawn(wheel);
        assert_eq!(snapshot.find(root, "/Rover/Wheel"), Some(wheel));
        assert_eq!(world.resource::<PrimIndex>().find(root, "/Rover/Wheel"), None);
    }
}
