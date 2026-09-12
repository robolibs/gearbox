//! Rapier physics on top of usd_bevy's projected stage.
//!
//! usd_bevy holds the composed stage live and projects prims into entities,
//! but simulates nothing. This module owns the Rapier f64 world
//! (`PhysicsWorld`), reads the UsdPhysics opinions of every projected stage
//! itself (`reader`, `attach`) into marker components (`markers`), converts
//! those into Rapier bodies, colliders and joints (`bodies`, `colliders`,
//! `joints`, `rapier`), steps the world and writes poses back into
//! `Transform`s. Values are SI; quaternions are Bevy order; `lower > upper`
//! on a limit locks the DOF.

mod attach;
mod bodies;
mod colliders;
mod convert;
mod debug;
mod joints;
pub mod markers;
mod rapier;
pub mod reader;
mod scene;
mod world;
mod writeback;

use std::collections::HashMap;

use bevy::prelude::*;
use usd_bevy::instance::UsdInstances;
use usd_bevy::{UsdSceneRoot, UsdSceneState};

pub use debug::ColliderDebugEnabled;
pub use gearbox_api::PhysicsActive;
pub use world::PhysicsWorld;

/// Wires the Rapier world, every marker → Rapier conversion system and the
/// writeback path. Adds `PhysicsWorld` and `ColliderDebugEnabled`;
/// `PhysicsActive` is gearbox-api's clock switch.
pub struct RapierAdapterPlugin;

impl Plugin for RapierAdapterPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PhysicsWorld>()
            .init_resource::<PhysicsActive>()
            .init_resource::<ColliderDebugEnabled>()
            .add_systems(
                Update,
                (
                    attach_projected_stages,
                    scene::sync_gravity_from_usd_scene,
                    bodies::convert_rigid_bodies,
                    colliders::convert_colliders,
                    colliders::apply_physics_materials,
                    colliders::apply_collision_filters,
                    joints::convert_joints,
                    sync_bodies_to_transforms_on_resume,
                    world::step_physics,
                    report_quarantined,
                )
                    .chain(),
            )
            .add_systems(
                PostUpdate,
                writeback::writeback_transforms.run_if(physics_is_active),
            )
            .add_systems(Last, debug::draw_collider_gizmos);
    }
}

fn physics_is_active(active: Res<PhysicsActive>) -> bool {
    active.0
}

/// Name the bodies the step quarantined, with the transform the projection
/// gave their entities, so a NaN can be traced to its prim.
fn report_quarantined(
    mut world: ResMut<PhysicsWorld>,
    prims: Query<(Option<&usd_bevy::UsdPrimRef>, Option<&GlobalTransform>)>,
) {
    for entity in world.quarantined.drain(..) {
        let (path, gt) = prims
            .get(entity)
            .map(|(p, gt)| {
                (
                    p.map(|p| p.path.clone()).unwrap_or_default(),
                    gt.map(|g| g.translation()),
                )
            })
            .unwrap_or_default();
        error!(
            "gearbox-physics: body of {entity:?} `{path}` went non-finite (entity translation {gt:?}); disabled"
        );
    }
}

/// A scene root whose prims already carry their physics markers.
#[derive(Component)]
struct PhysicsAttached;

/// Once a `UsdSceneRoot` is projected, walk its stage and put the UsdPhysics
/// markers on the projected prim entities, then resolve the relationships
/// (joint bodies, filtered pairs, materials) against the prim map.
fn attach_projected_stages(world: &mut World) {
    let ready: Vec<Entity> = world
        .query_filtered::<(Entity, &UsdSceneState), (With<UsdSceneRoot>, Without<PhysicsAttached>)>(
        )
        .iter(world)
        .filter(|(_, state)| **state == UsdSceneState::Ready)
        .map(|(entity, _)| entity)
        .collect();
    if ready.is_empty() {
        return;
    }
    let Some(instances) = world.remove_non_send::<UsdInstances>() else {
        return;
    };
    for root in ready {
        let Some(stage) = instances.stage(root) else {
            continue;
        };
        let mut prims: Vec<(String, Entity)> = Vec::new();
        let _ = stage.traverse(Default::default(), |path: &openusd::sdf::Path| {
            if let Some(entity) = instances.entity(root, path.as_str()) {
                prims.push((path.as_str().to_string(), entity));
            }
        });
        let meta = attach::read_stage_meta(stage);
        let mut pending = attach::PendingPhysics::default();
        let mut by_path: HashMap<String, Entity> = HashMap::with_capacity(prims.len());
        for (path, entity) in &prims {
            by_path.insert(path.clone(), *entity);
            if path == "/" {
                continue;
            }
            let Ok(sdf) = openusd::sdf::path(path) else {
                continue;
            };
            if let Some(name) = sdf.name()
                && world.get::<Name>(*entity).is_none()
            {
                world
                    .entity_mut(*entity)
                    .insert(Name::new(name.to_string()));
            }
            attach::attach_physics_to_prim(stage, &sdf, *entity, world, &mut pending, &meta);
        }
        attach::resolve_pending_physics(world, &pending, &by_path);
        attach::populate_articulation_joints(world, &pending.articulation_roots);
        world.entity_mut(root).insert(PhysicsAttached);
        info!(
            "gearbox-physics: attached markers to {} prim(s) of {root:?}",
            prims.len()
        );
    }
    world.insert_non_send_resource(instances);
}

/// On the OFF→ON edge of `PhysicsActive`, sync every body's pose to its
/// entity's current `GlobalTransform` so a gizmo drag while paused is not
/// undone by the first writeback.
fn sync_bodies_to_transforms_on_resume(
    active: Res<PhysicsActive>,
    mut prev_active: Local<bool>,
    mut world: ResMut<PhysicsWorld>,
    transforms: Query<&GlobalTransform>,
) {
    let was = *prev_active;
    *prev_active = active.0;
    if !active.0 || was {
        return;
    }
    use rapier3d::prelude::*;
    let world = world.as_mut();
    let pairs: Vec<(Entity, RigidBodyHandle)> =
        world.entity_to_body.iter().map(|(e, h)| (*e, *h)).collect();
    for (entity, handle) in pairs {
        let Ok(gt) = transforms.get(entity) else {
            continue;
        };
        let t = gt.compute_transform();
        let Some(rb) = world.bodies.get_mut(handle) else {
            continue;
        };
        rb.set_position(
            Pose {
                translation: convert::vec3_to_d(t.translation),
                rotation: convert::quat_to_d(t.rotation),
            },
            true,
        );
        rb.set_linvel(glam::DVec3::ZERO, true);
        rb.set_angvel(glam::DVec3::ZERO, true);
    }
}
