//! Multi-USD loading: CLI args + 📂 ribbon button → `Assets<UsdScene>` →
//! mounted USD scene with `LoadedAsset` marker. The marker also tags the
//! root for the UI's pick-and-gizmo wiring.

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use bevy::transform::TransformSystems;
use gearbox_api::{
    GearboxBus, MachineDeleteQueue, MachineLoadQueue, Props, SceneEvent, SceneObject, SceneObjects,
    clear_scope, event_kind, object_kind,
};
use crate::physics::backend::{DQuat, DVec3, Pose};
use usd_bevy::{UsdScene, UsdSceneRoot, UsdSceneState};

use crate::controller::{ControllerInventory, discover_machines_from_usd, log_discovered_machines};
use crate::world::terrain_height_m;

/// Tag on every `SceneRoot` the loader spawns. Carries the source
/// path so the tree / inspector can show where it came from.
#[derive(Component, Debug, Clone)]
pub struct LoadedAsset {
    pub path: PathBuf,
    pub label: String,
}

/// Companion to `LoadedAsset`: the `Handle<UsdAsset>` that produced
/// the spawned scene. The viewer panels look the asset up via
/// `Assets<UsdAsset>::get(handle)` for stage metadata, variants,
/// cameras, etc.
#[derive(Component, Debug, Clone)]
pub struct UsdAssetHandle(pub Handle<UsdScene>);

/// Push a path here to load + spawn it next frame. The 📂 button
/// (and CLI seeding) both write to this queue.
#[derive(Resource, Default)]
pub struct LoadQueue(pub Vec<PathBuf>);

/// Tracking entry per in-flight or already-spawned load.
struct InflightLoad {
    root: Entity,
    path: PathBuf,
    label: String,
    transform: Transform,
    machine_id: Option<String>,
    activate_physics_after_sync: bool,
    spawned: bool,
    tyres: gearbox_api::tyres::SavedTyres,
}

#[derive(Resource, Default)]
struct Inflight(Vec<InflightLoad>);

#[derive(Resource, Default)]
struct RejectedLoads(Vec<Entity>);

#[derive(Component, Debug, Clone, Copy)]
struct MachinePhysicsSyncPending {
    activate_after_sync: bool,
    frames_waited: u32,
}

#[derive(Resource, Debug, Clone, Copy)]
struct PhysicsActivationPending;

pub struct LoadPlugin {
    pub cli_paths: Vec<PathBuf>,
}

impl Plugin for LoadPlugin {
    fn build(&self, app: &mut App) {
        let cli = self.cli_paths.clone();
        app.init_resource::<LoadQueue>()
            .init_resource::<Inflight>()
            .init_resource::<RejectedLoads>()
            .add_systems(Last, remove_rejected_loads)
            .add_systems(Startup, move |mut q: ResMut<LoadQueue>| {
                q.0.extend(cli.clone());
            })
            .add_systems(
                Update,
                (
                    clear_runtime_usd_loads_on_reset_system,
                    drain_load_queue,
                    drain_machine_load_queue,
                    drain_machine_delete_queue,
                    spawn_when_loaded,
                    refresh_scene_objects,
                ),
            )
            .add_systems(
                PostUpdate,
                (
                    activate_physics_after_machine_transforms_propagate
                        .after(TransformSystems::Propagate),
                    sync_pending_machine_physics_to_scene_transforms
                        .after(TransformSystems::Propagate)
                        .after(activate_physics_after_machine_transforms_propagate),
                ),
            );
    }
}

fn remove_rejected_loads(world: &mut World) {
    let roots = std::mem::take(&mut world.resource_mut::<RejectedLoads>().0);
    for root in roots {
        let mut entities = vec![root];
        let mut index = 0;
        while index < entities.len() {
            if let Some(children) = world.get::<Children>(entities[index]) {
                entities.extend(children.iter());
            }
            index += 1;
        }
        let mut physics = world.resource_mut::<crate::physics::PhysicsWorld>();
        for entity in entities {
            physics.remove_entity_body(entity);
            physics.remove_entity_collider(entity, false);
        }
        if let Ok(entity) = world.get_entity_mut(root) {
            entity.despawn();
        }
    }
}

fn activate_physics_after_machine_transforms_propagate(
    mut commands: Commands,
    pending_activation: Option<Res<PhysicsActivationPending>>,
    pending_machines: Query<Entity, With<MachinePhysicsSyncPending>>,
    mut physics_active: ResMut<gearbox_api::PhysicsActive>,
) {
    if pending_activation.is_none() || !pending_machines.is_empty() {
        return;
    }
    if !physics_active.0 {
        physics_active.0 = true;
        info!("gearbox-load: enabling physics after aligned machine transforms propagated");
    }
    commands.remove_resource::<PhysicsActivationPending>();
}

fn clear_runtime_usd_loads_on_reset_system(
    messages: Option<MessageReader<gearbox_api::SimResetRequest>>,
    mut commands: Commands,
    mut inflight: ResMut<Inflight>,
    mut controller_inventory: ResMut<ControllerInventory>,
    mut physics: ResMut<crate::physics::PhysicsWorld>,
    loaded_roots: Query<Entity, With<LoadedAsset>>,
    children_q: Query<&Children>,
    mut values: ResMut<crate::services::LinkValues>,
    mut seeded: ResMut<crate::services::SeededLinkValues>,
) {
    let Some(mut messages) = messages else { return };
    let clears = messages
        .read()
        .any(|m| matches!(m.scope, clear_scope::ALL | clear_scope::MACHINES));
    if !clears {
        return;
    }

    inflight.0.clear();
    for machine in &controller_inventory.machines {
        values.0.retain(|(id, _, _), _| id != &machine.id);
        seeded.0.remove(&machine.id);
    }
    controller_inventory.machines.clear();

    let physics = physics.as_mut();
    let mut cleared = 0usize;
    for root in loaded_roots.iter() {
        remove_loaded_usd_physics(root, physics, &children_q);
        commands.entity(root).despawn();
        cleared += 1;
    }
    if cleared > 0 {
        info!("gearbox-load: cleared {cleared} runtime USD load(s)");
    }
}

fn remove_loaded_usd_physics(
    root: Entity,
    physics: &mut crate::physics::PhysicsWorld,
    children_q: &Query<&Children>,
) {
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        physics.remove_entity_body(entity);
        physics.remove_entity_collider(entity, false);
        if let Ok(children) = children_q.get(entity) {
            stack.extend(children.iter());
        }
    }
}

fn drain_load_queue(
    mut commands: Commands,
    mut scenes: ResMut<Assets<UsdScene>>,
    mut queue: ResMut<LoadQueue>,
    mut inflight: ResMut<Inflight>,
) {
    if queue.0.is_empty() {
        return;
    }
    for abs in queue.0.drain(..) {
        let label = abs
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| abs.to_string_lossy().into_owned());
        let n = inflight.0.len();
        // Stagger mounts on a 4 m grid so multiple loads don't pile.
        let cols = ((n as f32 + 1.0).sqrt().ceil() as usize).max(1);
        let r = n / cols;
        let c = n % cols;
        let half = (cols as f32 - 1.0) * 0.5;
        let x = (c as f32 - half) * 4.0;
        let z = r as f32 * 4.0 - 2.0;
        let mount = Vec3::new(x, terrain_height_m(x, z), z);
        let _ = queue_usd_load(
            &mut commands,
            &mut scenes,
            &mut inflight,
            abs,
            label,
            Transform::from_translation(mount),
            None,
            false,
            Vec::new(),
            Default::default(),
        );
    }
}

/// The scene root exists from the start: usd_bevy projects the stage under
/// it on the next frame, and `spawn_when_loaded` finishes the job when the
/// root reports `UsdSceneState::Ready`. The bytes go straight into
/// `Assets<UsdScene>`: the asset-server loader would parse the stage once
/// more just to probe it, which for a large usdz costs more than the read.
#[allow(clippy::too_many_arguments)]
fn queue_usd_load(
    commands: &mut Commands,
    scenes: &mut Assets<UsdScene>,
    inflight: &mut Inflight,
    path: PathBuf,
    label: String,
    transform: Transform,
    machine_id: Option<String>,
    activate_physics_after_sync: bool,
    variants: Vec<(String, String, String)>,
    tyres: gearbox_api::tyres::SavedTyres,
) -> Result<(), String> {
    let read_started = std::time::Instant::now();
    let source = std::fs::read(&path)
        .and_then(|bytes| usd_bevy::UsdSource::new(&path, bytes));
    let source = match source {
        Ok(source) => source,
        Err(err) => {
            error!("gearbox-load: {label} cannot read {}: {err}", path.display());
            return Err(format!("cannot read {}: {err}", path.display()));
        }
    };
    let handle = scenes.add(UsdScene {
        source,
        textures: default(),
    });
    info!(
        "gearbox-load: read {} in {:?}",
        path.display(),
        read_started.elapsed()
    );
    info!(
        "Load USD: {label} → translation={:?} yaw={:.1}°",
        transform.translation,
        transform.rotation.to_euler(EulerRot::YXZ).0.to_degrees(),
    );
    let root = commands
        .spawn((
            Name::new(label.clone()),
            transform,
            Visibility::default(),
            LoadedAsset {
                path: path.clone(),
                label: label.clone(),
            },
            UsdAssetHandle(handle.clone()),
            UsdSceneRoot(handle),
            usd_bevy::instance::UsdInstanceOverrides { variants, ..default() },
        ))
        .id();
    inflight.0.push(InflightLoad {
        root,
        path,
        label,
        transform,
        machine_id,
        activate_physics_after_sync,
        spawned: false,
        tyres,
    });
    Ok(())
}

fn snap_grounded_machine_to_terrain(transform: &mut Transform) {
    // Runtime machine load requests use y=0 to mean "put this machine on the
    // ground". Once the world became a real heightfield, leaving y=0 made
    // tractors float above valleys or spawn half-buried inside hills. Treat
    // nonzero y as an explicit caller-provided vertical offset.
    if transform.translation.y.abs() < 0.001 {
        transform.translation.y =
            terrain_height_m(transform.translation.x, transform.translation.z);
    }
}

fn resolve_spawn_path(raw: &str) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        return path;
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let direct = cwd.join(&path);
    if direct.exists() {
        return direct;
    }

    let from_assets = default_asset_root().join(&path);
    if from_assets.exists() {
        return from_assets;
    }

    direct
}

pub fn default_asset_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets")
}

fn spawn_when_loaded(
    mut commands: Commands,
    mut inflight: ResMut<Inflight>,
    mut controller_inventory: ResMut<ControllerInventory>,
    states: Query<&UsdSceneState>,
    mut bus: Option<ResMut<GearboxBus>>,
    mut pending_static: ResMut<crate::attach::PendingStaticAttachments>,
    timings: Option<Res<usd_bevy::asset::UsdSceneTimings>>,
    instances: Option<NonSend<usd_bevy::instance::UsdInstances>>,
    mut values: ResMut<crate::services::LinkValues>,
    mut seeded: ResMut<crate::services::SeededLinkValues>,
    mut rejected: ResMut<RejectedLoads>,
) {
    for entry in inflight.0.iter_mut() {
        if entry.spawned {
            continue;
        }
        match states.get(entry.root) {
            Ok(UsdSceneState::Ready) => {}
            Ok(UsdSceneState::Failed(err)) => {
                error!("gearbox-load: {} failed to load: {err}", entry.label);
                if !entry.tyres.is_empty() {
                    if let Some(bus) = bus.as_deref_mut() {
                        bus.publish_event(SceneEvent::new(event_kind::MACHINE_REJECTED, &entry.label).with_prop("reason", &err.to_string()));
                    }
                    rejected.0.push(entry.root);
                }
                entry.spawned = true;
                continue;
            }
            _ => continue,
        }
        let scene_root = entry.root;
        if let Some(t) = timings.as_deref() {
            info!(
                "gearbox-load: {} ready; usd_bevy so far: open {:?} overrides {:?} validation {:?} textures {:?} projection {:?} ({} attempts, {} failed)",
                entry.label, t.open, t.overrides, t.validation, t.textures, t.projection, t.attempts, t.failures
            );
        }
        let discovery_started = std::time::Instant::now();
        // The projected stage is already open; scanning it beats parsing
        // the file again.
        let projected = instances.as_deref().and_then(|i| i.stage(scene_root));
        let scanned = match projected {
            Some(stage) => crate::controller::discover_machines_from_stage(stage),
            None => discover_machines_from_usd(&entry.path),
        };
        let mut discovered_machines = match scanned {
            Ok(machines) => Some(machines),
            Err(err) => {
                warn!(
                    "gearbox-control: failed to scan {} for machine controllers: {err}",
                    entry.path.display()
                );
                None
            }
        };
        if !entry.tyres.is_empty() {
            let result = discovered_machines.as_mut().ok_or_else(|| "machine discovery failed".to_owned())
                .and_then(|machines| restore_tyre_configuration(machines, &entry.tyres));
            if let Err(reason) = result {
                error!("gearbox-load: {} pressure restore rejected: {reason}", entry.label);
                if let Some(bus) = bus.as_deref_mut() {
                    bus.publish_event(SceneEvent::new(event_kind::MACHINE_REJECTED, &entry.label).with_prop("reason", &reason));
                }
                rejected.0.push(entry.root);
                entry.spawned = true;
                continue;
            }
        }
        let is_machine_asset = discovered_machines
            .as_ref()
            .is_some_and(|machines| !machines.is_empty());
        if is_machine_asset {
            commands
                .entity(scene_root)
                .insert(MachinePhysicsSyncPending {
                    activate_after_sync: entry.activate_physics_after_sync,
                    frames_waited: 0,
                });
        }
        entry.spawned = true;
        info!(
            "gearbox-load: {} machine discovery took {:?}",
            entry.label,
            discovery_started.elapsed()
        );
        let attachments_started = std::time::Instant::now();
        let attachments = match projected {
            Some(stage) => crate::controller::discover_static_attachments_from_stage(stage),
            None => crate::controller::discover_static_attachments_from_usd(&entry.path),
        };
        for (hitch, coupler) in attachments {
            pending_static.0.push(crate::attach::StaticAttachment {
                scene_root,
                hitch_prim: hitch,
                coupler_prim: coupler,
                frames_waited: 0,
            });
        }
        info!(
            "Spawned {} at {:?} (static attachment scan {:?})",
            entry.label,
            entry.transform.translation,
            attachments_started.elapsed()
        );
        if let Some(bus) = bus.as_deref_mut() {
            let t = entry.transform.translation;
            let mut event = SceneEvent::new(event_kind::LOADED, &entry.label)
                .at(t.x, t.y, t.z)
                .with_prop("path", &entry.path.to_string_lossy());
            if let Some(machine_id) = entry.machine_id.as_deref() {
                event = event.with_prop("machine_id", machine_id);
            }
            bus.publish_event(event);
        }

        if let Some(mut machines) = discovered_machines.take() {
            if let Some(machine_id) = entry.machine_id.as_deref() {
                apply_runtime_machine_id(&mut machines, machine_id);
            }
            if !entry.tyres.is_empty() {
                for machine in &machines {
                    values.0.retain(|(id, _, _), _| id != &machine.id);
                    seeded.0.remove(&machine.id);
                    for (link, pressure) in &entry.tyres {
                        values.set(&machine.id, link, "tyre_pressure_bar", pressure.applied_bar);
                        values.set(&machine.id, link, "tyre_target_pressure_bar", pressure.target_bar);
                    }
                }
            }
            log_discovered_machines(&entry.label, &machines);
            if !machines.is_empty() {
                controller_inventory.push_loaded_asset(
                    scene_root,
                    entry.label.clone(),
                    entry.path.to_string_lossy(),
                    machines,
                );
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn benchmark_align_machine(app: &mut App, root: Entity) {
    app.world_mut().entity_mut(root).insert(MachinePhysicsSyncPending {
        frames_waited: 0,
        activate_after_sync: false,
    });
    let mut schedule = bevy::ecs::schedule::Schedule::default();
    schedule.add_systems(sync_pending_machine_physics_to_scene_transforms);
    schedule.run(app.world_mut());
}

fn sync_pending_machine_physics_to_scene_transforms(
    mut commands: Commands,
    mut pending: Query<(
        Entity,
        &mut Transform,
        &mut MachinePhysicsSyncPending,
        Option<&Name>,
    )>,
    children: Query<&Children>,
    globals: Query<&GlobalTransform>,
    names: Query<&Name>,
    mut physics: ResMut<crate::physics::PhysicsWorld>,
) {
    for (root, mut root_transform, mut pending, name) in pending.iter_mut() {
        let descendants = collect_descendants(root, &children);
        let body_entities = descendants
            .into_iter()
            .filter_map(|entity| {
                physics
                    .entity_to_body
                    .get(&entity)
                    .copied()
                    .map(|h| (entity, h))
            })
            .collect::<Vec<_>>();

        if body_entities.is_empty() {
            pending.frames_waited += 1;
            if pending.frames_waited == 120 {
                warn!(
                    "gearbox-load: waiting for physics bodies before terrain-aligning {}",
                    name.map(|n| n.as_str()).unwrap_or("<unnamed machine>")
                );
            }
            continue;
        }

        let mut synced = 0usize;
        for (entity, handle) in body_entities {
            let Ok(gt) = globals.get(entity) else {
                continue;
            };
            let Some(body) = physics.body_mut(handle) else {
                continue;
            };
            let transform = gt.compute_transform();
            if !transform.translation.is_finite() || !transform.rotation.is_finite() {
                warn!(
                    "gearbox-load: {} has a non-finite projected transform for {:?}: {:?}",
                    name.map(|n| n.as_str()).unwrap_or("machine"),
                    names.get(entity).map(|n| n.as_str()).unwrap_or("?"),
                    transform
                );
                continue;
            }
            body.set_position(
                Pose {
                    translation: DVec3::new(
                        transform.translation.x as f64,
                        transform.translation.y as f64,
                        transform.translation.z as f64,
                    ),
                    rotation: DQuat::from_xyzw(
                        transform.rotation.x as f64,
                        transform.rotation.y as f64,
                        transform.rotation.z as f64,
                        transform.rotation.w as f64,
                    ),
                },
                true,
            );
            body.set_linvel(DVec3::ZERO, true);
            body.set_angvel(DVec3::ZERO, true);
            synced += 1;
        }

        if synced == 0 {
            continue;
        }

        propagate_body_positions_to_colliders(physics.as_mut());

        let collider_entities = collect_descendants(root, &children)
            .into_iter()
            .filter_map(|entity| {
                physics
                    .entity_to_collider
                    .get(&entity)
                    .copied()
                    .map(|handle| (entity, handle))
            })
            .collect::<Vec<_>>();
        if collider_entities.is_empty() {
            pending.frames_waited += 1;
            if pending.frames_waited == 120 {
                warn!(
                    "gearbox-load: waiting for physics colliders before terrain-aligning {}",
                    name.map(|n| n.as_str()).unwrap_or("<unnamed machine>")
                );
            }
            continue;
        }
        if let Some(delta_y) = terrain_contact_alignment_delta(&physics, &collider_entities, &names)
        {
            root_transform.translation.y += delta_y as f32;
            for handle in physics
                .entity_to_body
                .iter()
                .filter(|(entity, _)| is_descendant_or_self(root, **entity, &children))
                .map(|(_, handle)| *handle)
                .collect::<Vec<_>>()
            {
                if let Some(body) = physics.body_mut(handle) {
                    let mut pose = body.position();
                    pose.translation.y += delta_y;
                    body.set_position(pose, true);
                    body.set_linvel(DVec3::ZERO, true);
                    body.set_angvel(DVec3::ZERO, true);
                }
            }
            propagate_body_positions_to_colliders(physics.as_mut());
            info!(
                "gearbox-load: terrain contact adjusted {} by {delta_y:+.3} m",
                name.map(|n| n.as_str()).unwrap_or("<unnamed machine>")
            );
        }

        commands.entity(root).remove::<MachinePhysicsSyncPending>();
        if pending.activate_after_sync {
            commands.insert_resource(PhysicsActivationPending);
            info!(
                "gearbox-load: terrain-aligned {} physics bodies for {}; enabling physics after transform propagation",
                synced,
                name.map(|n| n.as_str()).unwrap_or("<unnamed machine>")
            );
        } else {
            info!(
                "gearbox-load: terrain-aligned {} physics bodies for {}",
                synced,
                name.map(|n| n.as_str()).unwrap_or("<unnamed machine>")
            );
        }
    }
}

fn propagate_body_positions_to_colliders(physics: &mut crate::physics::PhysicsWorld) {
    physics.sync_collider_positions();
}

fn terrain_contact_alignment_delta(
    physics: &crate::physics::PhysicsWorld,
    collider_entities: &[(Entity, crate::physics::backend::ColliderId)],
    names: &Query<&Name>,
) -> Option<f64> {
    let mut tire_handles = Vec::new();
    let mut fallback_handles = Vec::new();
    for (entity, handle) in collider_entities {
        fallback_handles.push(*handle);
        let lower = names
            .get(*entity)
            .map(|name| name.as_str().to_ascii_lowercase())
            .unwrap_or_default();
        if lower.contains("tire") || lower.contains("tyre") || lower.contains("wheel") {
            tire_handles.push(*handle);
        }
    }

    let handles = if tire_handles.is_empty() {
        &fallback_handles
    } else {
        &tire_handles
    };

    let mut min_clearance = f64::INFINITY;
    for handle in handles {
        let Some(collider) = physics.collider(*handle) else {
            continue;
        };
        let aabb = collider.aabb();
        let ground =
            max_terrain_height_under_aabb(aabb.mins.x, aabb.maxs.x, aabb.mins.z, aabb.maxs.z);
        let clearance = aabb.mins.y - ground;
        min_clearance = min_clearance.min(clearance);
    }

    if !min_clearance.is_finite() {
        return None;
    }

    // Leave a tiny positive clearance so the first physics tick settles the
    // tyres onto the heightfield instead of starting with interpenetration.
    let desired_clearance = 0.03;
    let delta = desired_clearance - min_clearance;
    if delta.abs() > 0.002 {
        Some(delta)
    } else {
        None
    }
}

fn max_terrain_height_under_aabb(min_x: f64, max_x: f64, min_z: f64, max_z: f64) -> f64 {
    let samples = [
        ((min_x + max_x) * 0.5, (min_z + max_z) * 0.5),
        (min_x, min_z),
        (min_x, max_z),
        (max_x, min_z),
        (max_x, max_z),
    ];
    samples
        .into_iter()
        .map(|(x, z)| terrain_height_m(x as f32, z as f32) as f64)
        .fold(f64::NEG_INFINITY, f64::max)
}

fn is_descendant_or_self(root: Entity, candidate: Entity, children: &Query<&Children>) -> bool {
    if root == candidate {
        return true;
    }
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        let Ok(kids) = children.get(entity) else {
            continue;
        };
        for child in kids.iter() {
            if child == candidate {
                return true;
            }
            stack.push(child);
        }
    }
    false
}

fn collect_descendants(root: Entity, children: &Query<&Children>) -> Vec<Entity> {
    let mut out = vec![root];
    let mut cursor = 0usize;
    while cursor < out.len() {
        let entity = out[cursor];
        cursor += 1;
        if let Ok(kids) = children.get(entity) {
            out.extend(kids.iter());
        }
    }
    out
}

fn apply_runtime_machine_id(
    machines: &mut [crate::controller::MachineInstanceSpec],
    machine_id: &str,
) {
    // One machine takes the id as is; several in one asset (a yard with a
    // tractor and a trailer) get `<machine_id>_<id>` each.
    let several = machines.len() > 1;
    for machine in machines {
        let id = if several {
            format!("{machine_id}_{}", machine.id)
        } else {
            machine_id.to_string()
        };
        machine.id = id.clone();
        for controller in &mut machine.controllers {
            controller.machine_id = id.clone();
        }
    }
}

fn restore_tyre_configuration(
    machines: &mut [crate::controller::MachineInstanceSpec],
    tyres: &gearbox_api::tyres::SavedTyres,
) -> Result<(), String> {
    gearbox_api::tyres::validate_saved(tyres)?;
    if machines.len() != 1 { return Err("tyre snapshots require a single-machine asset".into()); }
    let machine = &mut machines[0];
    for (name, pressure) in tyres {
        let link = machine.links.get(name).ok_or_else(|| format!("saved tyre link {name} is missing"))?;
        if link.role != crate::links::LinkRole::Wheel {
            return Err(format!("saved tyre {name} is not a wheel link"));
        }
        let bound = |name: &str, default| link.values.iter().find(|(key, _)| key == name).map_or(default, |(_, value)| *value);
        let low = bound("tyre_min_pressure_bar", 0.5);
        let high = bound("tyre_max_pressure_bar", 4.0);
        if !low.is_finite() || !high.is_finite() || low <= 0.0 || low > high
            || ![pressure.applied_bar, pressure.target_bar].iter().all(|p| (low..=high).contains(p))
        {
            return Err(format!("saved tyre {name} must be within {low}..={high} gauge bar"));
        }
    }
    for link in &mut machine.links.links {
        if let Some(pressure) = tyres.get(&link.name) {
            link.values.retain(|(key, _)| key != "tyre_pressure_bar" && key != "tyre_target_pressure_bar");
            link.values.push(("tyre_pressure_bar".into(), pressure.applied_bar));
            link.values.push(("tyre_target_pressure_bar".into(), pressure.target_bar));
        }
    }
    Ok(())
}

/// Machine-category loads that arrived over the bus.
fn drain_machine_load_queue(
    mut commands: Commands,
    mut scenes: ResMut<Assets<UsdScene>>,
    mut queue: ResMut<MachineLoadQueue>,
    mut inflight: ResMut<Inflight>,
    mut physics_active: ResMut<gearbox_api::PhysicsActive>,
    inventory: Res<ControllerInventory>,
    loaded: Query<&LoadedAsset>,
    physics: Res<crate::physics::PhysicsWorld>,
    mut pending: Query<&mut MachinePhysicsSyncPending>,
    mut bus: Option<ResMut<GearboxBus>>,
) {
    for req in queue.0.drain(..) {
        if req.remove() || req.delete() {
            warn!(
                "gearbox-load: machine unload `{}` reached the load queue",
                req.id()
            );
            continue;
        }
        let Some(usd_path) = req.path() else {
            warn!("gearbox-load: machine load `{}` has no path", req.id());
            continue;
        };
        let mut reject = |reason: &str| {
            warn!("gearbox-load: {}: {reason}", req.id());
            if let Some(bus) = bus.as_deref_mut() {
                bus.publish_event(SceneEvent::new(event_kind::MACHINE_REJECTED, &req.id()).with_prop("reason", reason));
            }
        };
        let tyres: gearbox_api::tyres::SavedTyres = match req.props().get("tyre_snapshot") {
            Some(json) => match serde_json::from_str(&json) {
                Ok(tyres) => tyres,
                Err(error) => { reject(&format!("invalid tyre snapshot: {error}")); continue; }
            },
            None => Default::default(),
        };
        if let Err(error) = gearbox_api::tyres::validate_saved(&tyres) {
            reject(&error); continue;
        }
        if !tyres.is_empty() && req.machine_id().is_none_or(|id| id.trim().is_empty()) {
            reject("pressure snapshot requires an explicit machine id"); continue;
        }
        if !tyres.is_empty() && !physics.uses_wheel_forces() {
            reject("this backend does not support pressure snapshots"); continue;
        }
        if !tyres.is_empty() && (inventory.machines.iter().any(|m| Some(m.id.clone()) == req.machine_id())
            || inflight.0.iter().any(|e| e.machine_id == req.machine_id() && (!e.spawned || loaded.get(e.root).is_ok())))
        {
            reject("snapshot restore requires a fresh machine id"); continue;
        }
        let keep_paused = !tyres.is_empty() || req.props().get("start_paused").as_deref() == Some("true");
        if keep_paused {
            commands.remove_resource::<PhysicsActivationPending>();
            for entry in &mut inflight.0 { entry.activate_physics_after_sync = false; }
            for mut machine in &mut pending { machine.activate_after_sync = false; }
        }
        if physics_active.0 {
            physics_active.0 = false;
            info!("gearbox-load: pausing physics until new machine USD is aligned to terrain");
        }
        let path = resolve_spawn_path(&usd_path);
        let props = req.props();
        let machine_id = req.machine_id();
        let label = props
            .get("label")
            .or_else(|| machine_id.clone())
            .unwrap_or_else(|| {
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| usd_path.clone())
            });
        let mut transform = Transform {
            translation: Vec3::new(req.x, req.y, req.z),
            rotation: Quat::from_rotation_y(req.yaw_deg.to_radians()),
            ..default()
        };
        snap_grounded_machine_to_terrain(&mut transform);
        if let Err(reason) = queue_usd_load(
            &mut commands,
            &mut scenes,
            &mut inflight,
            path,
            label,
            transform,
            machine_id,
            !keep_paused,
            req.variants(),
            tyres,
        ) {
            reject(&reason);
        }
    }
}

/// `clear usd ID` and machine loads with `remove`/`delete`: the id is the
/// machine id or the asset label. Physics goes first, then the scene root;
/// the agent follows once the inventory entry is gone.
fn drain_machine_delete_queue(
    mut commands: Commands,
    mut queue: ResMut<MachineDeleteQueue>,
    mut inventory: ResMut<ControllerInventory>,
    mut physics: ResMut<crate::physics::PhysicsWorld>,
    mut bus: Option<ResMut<GearboxBus>>,
    loaded: Query<(Entity, &LoadedAsset)>,
    children_q: Query<&Children>,
    mut values: ResMut<crate::services::LinkValues>,
    mut seeded: ResMut<crate::services::SeededLinkValues>,
) {
    if queue.0.is_empty() {
        return;
    }
    for id in queue.0.drain(..) {
        let root = inventory
            .machines
            .iter()
            .find(|m| m.id == id || m.controllers.iter().any(|c| c.machine_id == id))
            .and_then(|m| m.scene_root)
            .or_else(|| {
                loaded
                    .iter()
                    .find(|(_, asset)| asset.label == id)
                    .map(|(entity, _)| entity)
            });
        let Some(root) = root else {
            warn!("gearbox-load: nothing loaded is called `{id}`");
            continue;
        };
        remove_loaded_usd_physics(root, physics.as_mut(), &children_q);
        for machine in inventory.machines.iter().filter(|m| m.scene_root == Some(root)) {
            values.0.retain(|(id, _, _), _| id != &machine.id);
            seeded.0.remove(&machine.id);
        }
        inventory.machines.retain(|m| m.scene_root != Some(root));
        commands.entity(root).try_despawn();
        info!("gearbox-load: unloaded machine `{id}`");
        if let Some(bus) = bus.as_mut() {
            bus.publish_event(SceneEvent::new(event_kind::REMOVED, &id));
        }
    }
}

/// Report every loaded asset root for `/gearbox/scene/list`.
fn refresh_scene_objects(
    loaded: Query<(Entity, &LoadedAsset, &GlobalTransform)>,
    inventory: Res<ControllerInventory>,
    mut objects: ResMut<SceneObjects>,
    overrides: Query<&usd_bevy::instance::UsdInstanceOverrides>,
    attachments: Res<crate::attach::Attachments>,
    physics: Res<crate::physics::PhysicsWorld>,
) {
    let mut out = Vec::new();
    for (entity, asset, transform) in loaded.iter() {
        let machine = inventory
            .machines
            .iter()
            .find(|m| m.scene_root == Some(entity));
        let t = transform.translation();
        let (_, yaw, _) = transform.rotation().to_euler(EulerRot::YXZ);
        let mut props = Props::from_pairs(&[
            ("id", asset.label.as_str()),
            ("path", &asset.path.to_string_lossy()),
        ]);
        let kind = match machine {
            Some(m) => {
                props.set("machine_id", &m.id);
                if physics.uses_wheel_forces()
                    && inventory.machines.iter().filter(|m| m.scene_root == Some(entity)).count() == 1
                    && !attachments.0.iter().any(|a| a.master_id == m.id || a.slave_id == m.id)
                    && let Ok(opinions) = overrides.get(entity)
                    && opinions.attributes.is_empty()
                {
                    props.set("configuration_snapshot", "1");
                    if let Ok(variants) = serde_json::to_string(&opinions.variants) {
                        props.set("snapshot_variants", &variants);
                    }
                }
                if let Some(kind) = &m.kind {
                    props.set("kind", kind);
                }
                object_kind::MACHINE
            }
            None if asset.label.to_ascii_lowercase().contains("terrain") => object_kind::TERRAIN,
            None => object_kind::PROP,
        };
        out.push(SceneObject {
            x: t.x,
            y: t.y,
            z: t.z,
            yaw_deg: yaw.to_degrees(),
            kind,
            props: props.into_bytes(),
        });
    }
    objects.machines = out;
}

#[cfg(test)]
mod pressure_snapshot_tests {
    use super::*;
    use gearbox_api::tyres::{SavedTyrePressure, SavedTyres};

    #[test]
    fn rejected_load_cleanup_follows_projection_commands_and_removes_physics() {
        let mut app = App::new();
        app.insert_resource(crate::physics::PhysicsWorld::with_backend(Box::new(crate::physics::MollaBackend::default())))
            .init_resource::<RejectedLoads>()
            .add_systems(Last, remove_rejected_loads);
        let root = app.world_mut().spawn_empty().id();
        let child = app.world_mut().spawn(ChildOf(root)).id();
        let body = {
            let mut physics = app.world_mut().resource_mut::<crate::physics::PhysicsWorld>();
            let body = physics.insert_body(crate::physics::backend::BodyDesc::fixed());
            physics.entity_to_body.insert(child, body);
            body
        };
        app.add_systems(Update, move |mut commands: Commands, mut rejected: ResMut<RejectedLoads>| {
            rejected.0.push(root);
            commands.entity(child).insert(Visibility::Visible);
        });
        app.update();
        assert!(app.world().get_entity(root).is_err());
        assert!(app.world().get_entity(child).is_err());
        let physics = app.world().resource::<crate::physics::PhysicsWorld>();
        assert!(physics.body(body).is_none());
        assert!(physics.entity_to_body.is_empty());
    }

    #[test]
    fn restore_validates_every_tyre_before_modifying_discovered_properties() {
        let path = default_asset_root().join("tractor.usd");
        let mut machines = discover_machines_from_usd(&path).unwrap();
        let wheels: Vec<_> = machines[0].links.links.iter().filter(|l| l.role == crate::links::LinkRole::Wheel).map(|l| l.name.clone()).collect();
        assert!(wheels.len() >= 2);
        let saved: SavedTyres = wheels.iter().map(|name| (name.clone(), SavedTyrePressure { applied_bar: 1.3, target_bar: 2.2 })).collect();
        let before = machines[0].links.links.clone();
        for bad in [-1.0, 4.1, f64::NAN] {
            let mut invalid = saved.clone();
            invalid.get_mut(&wheels[1]).unwrap().target_bar = bad;
            assert!(restore_tyre_configuration(&mut machines, &invalid).is_err());
            assert_eq!(machines[0].links.links, before);
        }
        let mut invalid = saved.clone();
        invalid.insert("absent".into(), SavedTyrePressure { applied_bar: 1.3, target_bar: 2.2 });
        assert!(restore_tyre_configuration(&mut machines, &invalid).is_err());
        assert_eq!(machines[0].links.links, before);
        restore_tyre_configuration(&mut machines, &saved).unwrap();
        for name in wheels {
            let values = &machines[0].links.get(&name).unwrap().values;
            assert!(values.contains(&("tyre_pressure_bar".into(), 1.3)));
            assert!(values.contains(&("tyre_target_pressure_bar".into(), 2.2)));
        }
    }
}
