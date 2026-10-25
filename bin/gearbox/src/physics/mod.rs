//! Rigid-body physics on top of usd_bevy's projected stage.
//!
//! usd_bevy holds the composed stage live and projects prims into entities,
//! but simulates nothing. This module owns the f64 world (`PhysicsWorld`),
//! reads the UsdPhysics opinions of every projected stage itself (`reader`,
//! `attach`) into marker components (`markers`), converts those into bodies,
//! colliders and joints (`bodies`, `colliders`, `joints`), steps the world
//! and writes poses back into `Transform`s. Values are SI; quaternions are
//! Bevy order; `lower > upper` on a limit locks the DOF.
//!
//! Molla is the engine: `molla` implements `backend::PhysicsBackend`, the
//! interface the rest of gearbox steps and reads it through.

pub(crate) mod attach;
#[cfg(test)]
pub(crate) mod benchmark;
pub mod backend;
mod bodies;
pub mod chassis;
mod colliders;
#[cfg(test)]
mod commit_tests;
pub(crate) mod commit;
mod convert;
mod debug;
mod drive_motor;
pub(crate) mod fem;
mod joints;
pub mod markers;
pub(crate) mod molla;
#[cfg(test)]
pub(crate) use molla::MollaBackend;
pub mod reader;
mod scene;
#[cfg(test)]
mod tracks;
mod thread;
mod world;
mod writeback;

use std::collections::{HashMap, HashSet};

use bevy::math::Affine3A;
use bevy::prelude::*;
use usd_bevy::instance::UsdInstances;
use usd_bevy::{UsdSceneInstance, UsdSceneRoot, UsdSceneState};

use convert::{quat_from_d, vec3_from_d};
use markers::ModelMember;

pub use debug::ColliderDebugEnabled;
pub use gearbox_api::PhysicsActive;
pub use thread::PhysicsThreadPlugin;
pub(crate) use thread::{Merge, TickResourcesExt};
pub use world::{AuthoredDrive, ControlTick, PhysicsWorld, step_physics};
#[cfg(test)]
pub use world::plan_physics_steps;
pub(crate) use world::HitchHold;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PhysicsWriteback;

/// Wires the physics world, every marker → backend conversion system and
/// the writeback path. Adds `PhysicsWorld` and `ColliderDebugEnabled`;
/// `PhysicsActive` is gearbox-api's clock switch.
pub struct PhysicsPlugin;

impl Plugin for PhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(crate::prim_index::PrimIndexPlugin);
        app.init_schedule(world::ControlTick);
        app.init_resource::<PhysicsWorld>()
            .init_resource::<PhysicsActive>()
            .init_resource::<ColliderDebugEnabled>()
            // After the clock moves: planned before it, a frame's steps would be the
            // last frame's time, and uneven frames would show as jerks.
            .add_systems(
                First,
                world::plan_physics_steps.after(bevy::time::TimeSystems).run_if(stepped_on_the_frame),
            )
            .add_systems(
                Update,
                (
                    attach_projected_stages,
                    refresh_swapped_stages,
                    scene::sync_gravity_from_usd_scene,
                    bodies::convert_rigid_bodies,
                    colliders::convert_colliders,
                    colliders::apply_physics_materials,
                    colliders::apply_collision_filters,
                    joints::convert_joints,
                    sync_bodies_to_transforms_on_resume,
                    world::step_physics.run_if(stepped_on_the_frame),
                    report_quarantined,
                )
                    .chain(),
            )
            .add_systems(
                PostUpdate,
                (
                    writeback::writeback_transforms
                        .in_set(PhysicsWriteback)
                        .before(bevy::transform::TransformSystems::Propagate),
                    debug::draw_collider_gizmos,
                ),
            );
        let backend = app.world().resource::<PhysicsWorld>().name();
        app.insert_resource(PhysicsBackendName(backend));
        app.add_systems(Update, fem::advance_islands.after(world::step_physics));
        app.add_systems(Startup, fem::render_health::install);
        if std::env::var_os("GEARBOX_TF_DEBUG").is_some() {
            app.add_systems(
                PostUpdate,
                debug::report_transform_alignment
                    .run_if(physics_is_active)
                    .after(bevy::transform::TransformSystems::Propagate),
            );
        }
    }
}


fn physics_is_active(active: Res<PhysicsActive>) -> bool {
    active.0
}

/// The frame steps the physics itself unless the physics thread does.
fn stepped_on_the_frame(thread: Option<Res<thread::PhysicsThread>>) -> bool {
    thread.is_none()
}

/// The engine's name, readable while the physics thread holds the world.
#[derive(Resource)]
pub struct PhysicsBackendName(pub &'static str);

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
        let prims: Vec<(String, Entity)> = instances
            .prims(root)
            .map(|(path, entity)| (path.to_string(), entity))
            .collect();
        let meta = attach::read_stage_meta(stage);
        let mut pending = attach::PendingPhysics::default();
        let mut by_path: HashMap<String, Entity> = HashMap::with_capacity(prims.len());
        for (path, entity) in &prims {
            by_path.insert(path.clone(), *entity);
            if path == "/" {
                continue;
            }
            if let Some(name) = path.rsplit('/').next()
                && world.get::<Name>(*entity).is_none()
            {
                world
                    .entity_mut(*entity)
                    .insert(Name::new(name.to_string()));
            }
            attach::attach_physics_to_entity(world, *entity, &mut pending, &meta);
            attach::attach_joint_softness(world, *entity, stage, path);
            world.entity_mut(*entity).insert(PhysicsScanned);
        }
        attach::resolve_pending_physics(world, &pending, &by_path);
        attach::populate_articulation_joints(world, &pending.articulation_roots);
        let model = compile_loaded_model(world, stage, root);
        mark_model_members(world, &model, &by_path);
        world.entity_mut(root).insert((PhysicsAttached, LoadedModel(model)));
        info!(
            "gearbox-physics: attached markers to {} prim(s) of {root:?}",
            prims.len()
        );
    }
    world.insert_non_send(instances);
}

/// The compiled machines of a loaded stage; activation, discovery and the
/// API endpoints read this one result.
#[derive(Component, Clone)]
pub struct LoadedModel(pub std::sync::Arc<crate::machine_model::CompileResult>);

/// Generation counter shared by every compiled load.
static NEXT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub(crate) fn next_generation() -> u64 {
    NEXT_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn compile_loaded_model(world: &World, stage: &openusd::usd::Stage, root: Entity) -> std::sync::Arc<crate::machine_model::CompileResult> {
    let options = crate::machine_model::CompileOptions {
        selection: crate::machine_model::Selection::Asset { entry: None },
        load_instance: format!("{root}"),
        generation: next_generation(),
        meters_per_unit_override: std::env::var("BEVY_OPENUSD_METERS_PER_UNIT").ok().and_then(|s| s.parse().ok()),
        physics_rate_hz: world.get_resource::<PhysicsWorld>().map_or(120.0, |w| w.step_hz),
    };
    let result = crate::machine_model::compile(stage, &options);
    for machine in &result.machines {
        for d in machine.errors() {
            warn!("gearbox-model: {} rejected: {d}", machine.root);
        }
    }
    std::sync::Arc::new(result)
}

/// Tag every body and joint entity with the machine that owns it, so body,
/// collider and joint insertion follow its validation.
fn mark_model_members(world: &mut World, model: &crate::machine_model::CompileResult, by_path: &HashMap<String, Entity>) {
    for machine in &model.machines {
        let rejected = !machine.accepted();
        for body in &machine.bodies {
            if let Some(&entity) = by_path.get(&body.path) {
                world.entity_mut(entity).insert(ModelMember {
                    machine_root: machine.root.clone(),
                    rejected,
                    damping: Some((body.linear_damping.value, body.angular_damping.value)),
                });
            }
        }
        for joint in &machine.joints {
            if let Some(&entity) = by_path.get(&joint.path) {
                world.entity_mut(entity).insert(ModelMember {
                    machine_root: machine.root.clone(),
                    rejected,
                    damping: None,
                });
            }
        }
    }
}

/// A prim entity whose physics markers were read; prims a variant swap adds
/// lack it, so only they are attached after the swap.
#[derive(Component)]
struct PhysicsScanned;

/// A variant switch was asked for on this scene root; the host sets it so
/// only requested swaps rebuild physics and controllers.
#[derive(Component)]
pub(crate) struct VariantSwapRequested;

/// A scene root whose variant swap is attached; the controller discovery
/// rereads its machines and drops the marker.
#[derive(Component)]
pub(crate) struct SwappedStage;

/// Once usd_bevy has reconciled a requested variant swap: drop the physics
/// of the prims it despawned, carry the prims it added onto the machine
/// where it stands now, and attach their physics.
fn refresh_swapped_stages(world: &mut World) {
    let swapped: Vec<Entity> = world
        .query_filtered::<(Entity, &UsdSceneState), (
            With<PhysicsAttached>,
            With<VariantSwapRequested>,
            Changed<UsdSceneInstance>,
        )>()
        .iter(world)
        .filter(|(_, state)| **state == UsdSceneState::Ready)
        .map(|(entity, _)| entity)
        .collect();
    if swapped.is_empty() {
        return;
    }
    let started = std::time::Instant::now();
    drop_dead_physics(world);
    let Some(instances) = world.remove_non_send::<UsdInstances>() else {
        return;
    };
    for root in swapped {
        world
            .entity_mut(root)
            .remove::<VariantSwapRequested>()
            .insert(SwappedStage);
        let Some(stage) = instances.stage(root) else {
            continue;
        };
        let prims: Vec<(String, Entity)> = instances
            .prims(root)
            .map(|(path, entity)| (path.to_string(), entity))
            .collect();
        let fresh: Vec<(String, Entity)> = prims
            .iter()
            .filter(|(path, entity)| path != "/" && world.get::<PhysicsScanned>(*entity).is_none())
            .cloned()
            .collect();
        let model = compile_loaded_model(world, stage, root);
        mark_model_members(world, &model, &prims.iter().cloned().collect());
        world.entity_mut(root).insert(LoadedModel(model));
        if fresh.is_empty() {
            continue;
        }
        carry_onto_machine(world, stage, &prims, &fresh);
        let by_path: HashMap<String, Entity> = prims.iter().cloned().collect();
        let meta = attach::read_stage_meta(stage);
        let mut pending = attach::PendingPhysics::default();
        for (path, entity) in &fresh {
            if let Some(name) = path.rsplit('/').next()
                && world.get::<Name>(*entity).is_none()
            {
                world
                    .entity_mut(*entity)
                    .insert(Name::new(name.to_string()));
            }
            attach::attach_physics_to_entity(world, *entity, &mut pending, &meta);
            attach::attach_joint_softness(world, *entity, stage, path);
            world.entity_mut(*entity).insert(PhysicsScanned);
        }
        attach::resolve_pending_physics(world, &pending, &by_path);
        let articulations: Vec<Entity> = prims
            .iter()
            .map(|(_, entity)| *entity)
            .filter(|entity| world.get::<markers::UsdArticulationRoot>(*entity).is_some())
            .collect();
        attach::populate_articulation_joints(world, &articulations);
        info!(
            "gearbox-physics: variant swap attached {} new prim(s) of {root:?} in {:?}",
            fresh.len(),
            started.elapsed()
        );
    }
    world.insert_non_send(instances);
}

/// Removes the bodies and colliders of entities that no longer
/// exist; a removed body takes its joints with it.
fn drop_dead_physics(world: &mut World) {
    let (bodies, colliders): (Vec<Entity>, Vec<Entity>) = {
        let physics = world.resource::<PhysicsWorld>();
        let dead = |entity: &Entity| world.get_entity(*entity).is_err();
        (
            physics.entity_to_body.keys().copied().filter(dead).collect(),
            physics.entity_to_collider.keys().copied().filter(dead).collect(),
        )
    };
    let mut physics = world.resource_mut::<PhysicsWorld>();
    for entity in bodies {
        physics.remove_entity_body(entity);
    }
    for entity in colliders {
        physics.remove_entity_collider(entity, false);
    }
}

/// Moves the top prims a swap added by the displacement of the machine's
/// heaviest body, its pose now against its authored one, so a new tool
/// appears on the machine where it stands. Prims under an existing body
/// already ride with it.
fn carry_onto_machine(
    world: &mut World,
    stage: &openusd::usd::Stage,
    prims: &[(String, Entity)],
    fresh: &[(String, Entity)],
) {
    let reference = {
        let physics = world.resource::<PhysicsWorld>();
        prims
            .iter()
            .filter_map(|(path, entity)| {
                let body = physics
                    .entity_to_body
                    .get(entity)
                    .and_then(|handle| physics.body(*handle))?;
                Some((path.clone(), *entity, body.mass(), body.position()))
            })
            .max_by(|a, b| a.2.total_cmp(&b.2))
    };
    let Some((path, entity, _, pose)) = reference else {
        return;
    };
    let Some(local) = openusd::sdf::path(&path)
        .ok()
        .and_then(|path| usd_bevy::read::xform::read_transform(stage, &path).ok().flatten())
    else {
        return;
    };
    let authored_local = Transform {
        translation: Vec3::from(local.translate),
        rotation: Quat::from_array(local.rotate),
        scale: Vec3::from(local.scale),
    };
    let parent = world.get::<ChildOf>(entity).map(ChildOf::parent);
    let authored = parent.map_or(GlobalTransform::IDENTITY, |p| chain_world(world, p)) * authored_local;
    let (_, rotation, translation) = authored.to_scale_rotation_translation();
    let now = Affine3A::from_rotation_translation(quat_from_d(pose.rotation), vec3_from_d(pose.translation));
    let displacement = now * Affine3A::from_rotation_translation(rotation, translation).inverse();
    let fresh_set: HashSet<Entity> = fresh.iter().map(|(_, entity)| *entity).collect();
    for (_, entity) in fresh {
        let parent = world.get::<ChildOf>(*entity).map(ChildOf::parent);
        if parent.is_some_and(|p| fresh_set.contains(&p)) || rides_a_body(world, *entity) {
            continue;
        }
        let moved = displacement * chain_world(world, *entity).affine();
        let parent_world = parent.map_or(GlobalTransform::IDENTITY, |p| chain_world(world, p));
        let local = parent_world.affine().inverse() * moved;
        world
            .entity_mut(*entity)
            .insert(Transform::from_matrix(Mat4::from(local)));
    }
    // Bodies are built from `GlobalTransform`, before propagation runs.
    for (_, entity) in fresh {
        let global = chain_world(world, *entity);
        world.entity_mut(*entity).insert(global);
    }
}

/// World pose composed from local transforms, for entities spawned since
/// propagation last ran.
/// "World" is the entity's site: the chain stops at the site's grid.
fn chain_world(world: &World, entity: Entity) -> GlobalTransform {
    let local = world.get::<Transform>(entity).copied().unwrap_or_default();
    match world.get::<ChildOf>(entity) {
        Some(parent) if world.get::<crate::globe::Site>(parent.parent()).is_none() => {
            chain_world(world, parent.parent()) * local
        }
        _ => GlobalTransform::from(local),
    }
}

fn rides_a_body(world: &World, entity: Entity) -> bool {
    let physics = world.resource::<PhysicsWorld>();
    let mut current = world.get::<ChildOf>(entity).map(ChildOf::parent);
    while let Some(ancestor) = current {
        if physics.entity_to_body.contains_key(&ancestor) {
            return true;
        }
        current = world.get::<ChildOf>(ancestor).map(ChildOf::parent);
    }
    false
}

/// On resume, apply transforms edited since the last physics publication.
fn sync_bodies_to_transforms_on_resume(
    active: Res<PhysicsActive>,
    mut prev_active: Local<bool>,
    mut world: ResMut<PhysicsWorld>,
    transforms: Query<&Transform>,
    parents: Query<&ChildOf>,
    sites: Query<&crate::globe::Site>,
) {
    let was = *prev_active;
    *prev_active = active.0;
    if !active.0 || was {
        return;
    }
    let poses: Vec<_> = world.entity_to_body.iter().filter_map(|(&entity, &body)| {
        let published = world.published_transforms.get(&entity)?;
        if published.body != body {
            return None;
        }
        if !transforms.contains(entity) { return None; }
        let current = crate::globe::transform_in_site(entity, &parents, &transforms, &sites);
        if current.affine().abs_diff_eq(published.global.affine(), 1e-6) {
            return None;
        }
        let t = current.compute_transform();
        let region = gearbox_globe::physics_offset(crate::globe::site_of(entity, &parents, &sites));
        let mut translation = convert::vec3_to_d(t.translation);
        translation.x += region.x;
        Some((body, backend::Pose {
            translation,
            rotation: convert::quat_to_d(t.rotation),
        }))
    }).collect();
    if let Err(error) = world.set_body_poses(&poses, true) {
        warn!("gearbox-physics: paused transform edits rejected: {error}");
    }
}
