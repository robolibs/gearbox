//! USD loads over the bus. Machine loads are queued for the binary's
//! machine loader; every other category is spawned here.

use std::collections::HashMap;

use bevy::ecs::entity::Entities;
use bevy::prelude::*;

use super::{GearboxBus, SimResetRequest, UsdAssetRoot};
use crate::wire::*;

/// Machine-category loads waiting for the binary's loader, which also
/// registers controller metadata.
#[derive(Resource, Default)]
pub struct MachineLoadQueue(pub Vec<UsdLoad>);

/// Ids of machines to unload, drained by the binary's loader; a delete
/// that names nothing loaded here is assumed to name a machine.
#[derive(Resource, Default)]
pub struct MachineDeleteQueue(pub Vec<String>);

/// Pending scene-load handle for a non-machine USD.
#[derive(Component)]
pub struct PendingLoadedUsd {
    pub handle: Handle<usd_bevy::UsdAsset>,
    pub load_id: String,
}

struct LoadedUsdEntry {
    entity: Entity,
    signature: (String, Vec<(String, String, String)>),
}

#[derive(Resource, Default)]
struct LoaderState {
    entities: HashMap<String, LoadedUsdEntry>,
    tombstones: HashMap<String, String>,
}

pub struct UsdLoaderPlugin;

impl Plugin for UsdLoaderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MachineLoadQueue>()
            .init_resource::<MachineDeleteQueue>()
            .init_resource::<LoaderState>()
            .add_systems(
                Update,
                (
                    serve_usd_loads,
                    instantiate_pending,
                    clear_on_reset,
                    refresh_props,
                ),
            );
    }
}

fn refresh_props(
    state: Res<LoaderState>,
    mut objects: ResMut<super::SceneObjects>,
    transforms: Query<(&GlobalTransform, &Name)>,
) {
    let mut out: Vec<SceneObject> = state
        .entities
        .iter()
        .filter_map(|(id, entry)| {
            let (transform, name) = transforms.get(entry.entity).ok()?;
            let t = transform.translation();
            let (_, yaw, _) = transform.rotation().to_euler(EulerRot::YXZ);
            let kind = if name.as_str().starts_with("UsdWorld[") {
                object_kind::TERRAIN
            } else {
                object_kind::PROP
            };
            Some(SceneObject {
                x: t.x,
                y: t.y,
                z: t.z,
                yaw_deg: yaw.to_degrees(),
                kind,
                props: Props::from_pairs(&[
                    ("id", id.as_str()),
                    ("path", &entry.signature.0),
                    ("link", "base_link"),
                ])
                .into_bytes(),
            })
        })
        .collect();
    out.sort_by_key(|o| o.props().get("id"));
    objects.props = out;
}

fn serve_usd_loads(
    mut commands: Commands,
    bus: Option<ResMut<GearboxBus>>,
    mut state: ResMut<LoaderState>,
    mut machine_queue: ResMut<MachineLoadQueue>,
    mut machine_deletes: ResMut<MachineDeleteQueue>,
    asset_server: Res<AssetServer>,
    asset_root: Option<Res<UsdAssetRoot>>,
    live: &Entities,
    mut transforms: Query<&mut Transform>,
) {
    let Some(mut bus) = bus else { return };
    let state = state.as_mut();
    let mut events = Vec::new();
    bus.host.serve_usd_delete(|req| {
        let id = req.id();
        match state.entities.remove(&id) {
            Some(entry) => {
                commands.entity(entry.entity).try_despawn();
                events.push(SceneEvent::new(event_kind::REMOVED, &id));
            }
            None => machine_deletes.0.push(id.clone()),
        }
        state.tombstones.insert(id.clone(), String::new());
        Status::ok()
    });
    bus.host.serve_usd_load(|req| {
        if req.is_machine() {
            if req.remove() || req.delete() {
                machine_deletes.0.push(req.id());
            } else {
                machine_queue.0.push(req);
            }
            return Status::ok();
        }
        let id = req.id();
        if id.is_empty() {
            return Status::err(code::USAGE, "load needs an `id`");
        }
        if state
            .entities
            .get(&id)
            .is_some_and(|entry| !live.contains(entry.entity))
        {
            state.entities.remove(&id);
        }
        if req.remove() || req.delete() {
            if let Some(entry) = state.entities.remove(&id) {
                commands.entity(entry.entity).try_despawn();
            }
            if req.delete() {
                state.tombstones.insert(id.clone(), String::new());
            } else if !req.nonce().is_empty() {
                state.tombstones.insert(id.clone(), req.nonce());
            }
            events.push(SceneEvent::new(event_kind::REMOVED, &id));
            return Status::ok();
        }
        if state.tombstones.get(&id).is_some_and(|n| n.is_empty()) {
            return Status::err(code::REFUSED, "id was deleted; pick a new id");
        }
        let nonce = req.nonce();
        if !nonce.is_empty() {
            if state.tombstones.get(&id).is_some_and(|n| n == &nonce) {
                return Status::err(code::REFUSED, "stale load for a removed nonce");
            }
            state.tombstones.remove(&id);
        }
        let Some(path) = req.path() else {
            return Status::err(code::USAGE, "load needs a `path`");
        };
        let signature = (path.clone(), req.variants());
        if state
            .entities
            .get(&id)
            .is_some_and(|entry| entry.signature == signature)
        {
            let entity = state.entities[&id].entity;
            if let Ok(mut transform) = transforms.get_mut(entity) {
                transform.translation.x = req.x;
                transform.translation.z = req.z;
                transform.rotation = Quat::from_rotation_y(req.yaw_deg.to_radians());
            }
            return Status::ok();
        }
        if let Some(entry) = state.entities.remove(&id) {
            commands.entity(entry.entity).try_despawn();
        }
        let Some(root) = asset_root.as_deref() else {
            return Status::err(code::ERROR, "no asset root registered");
        };
        let asset_file = if std::path::Path::new(&path).is_absolute() {
            std::path::PathBuf::from(&path)
        } else {
            root.0.join(&path)
        };
        let parent = asset_file
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| root.0.clone());
        let variants: Vec<usd_bevy::VariantSelection> = req
            .variants()
            .into_iter()
            .map(|(prim_path, set_name, option)| usd_bevy::VariantSelection {
                prim_path,
                set_name,
                option,
            })
            .collect();
        let asset_path_string = asset_file.to_string_lossy().into_owned();
        let asset_path: bevy::asset::AssetPath<'static> = if variants.is_empty() {
            asset_path_string.into()
        } else {
            let label = usd_bevy::variant_label(&variants);
            bevy::asset::AssetPath::from(asset_path_string).with_label(label)
        };
        let variants_for_settings = variants.clone();
        let handle: Handle<usd_bevy::UsdAsset> = asset_server.load_with_settings(
            asset_path,
            move |s: &mut usd_bevy::UsdLoaderSettings| {
                s.search_paths = vec![parent.clone()];
                s.variant_selections = variants_for_settings.clone();
            },
        );
        let is_world = matches!(req.category, category::WORLD | category::TERRAIN);
        let name = if is_world {
            format!("UsdWorld[{id}]::pending")
        } else {
            format!("UsdLoad[{id}]::pending")
        };
        info!(
            "gearbox-api: load `{id}` category={} USD `{path}`",
            category::name(req.category)
        );
        let entity = commands
            .spawn((
                Name::new(name),
                Transform {
                    translation: Vec3::new(req.x, req.y, req.z),
                    rotation: Quat::from_rotation_y(req.yaw_deg.to_radians()),
                    ..default()
                },
                Visibility::default(),
                PendingLoadedUsd {
                    handle,
                    load_id: id.clone(),
                },
            ))
            .id();
        state
            .entities
            .insert(id, LoadedUsdEntry { entity, signature });
        Status::ok()
    });
    for ev in events {
        bus.publish_event(ev);
    }
}

fn instantiate_pending(
    mut commands: Commands,
    bus: Option<ResMut<GearboxBus>>,
    asset_server: Res<AssetServer>,
    usd_assets: Res<Assets<usd_bevy::UsdAsset>>,
    pending: Query<(Entity, &PendingLoadedUsd, &Transform)>,
) {
    use bevy::asset::LoadState;
    let mut bus = bus;
    for (entity, pend, transform) in pending.iter() {
        match asset_server.get_load_state(&pend.handle) {
            Some(LoadState::Loaded) => {
                let Some(asset) = usd_assets.get(&pend.handle) else {
                    continue;
                };
                let spawned = asset.scene.spawn_under(&mut commands, entity);
                info!(
                    "gearbox-api: instantiated USD load `{}` ({} projected entities)",
                    pend.load_id,
                    spawned.len()
                );
                commands.entity(entity).remove::<PendingLoadedUsd>();
                if let Some(bus) = bus.as_deref_mut() {
                    let t = transform.translation;
                    bus.publish_event(
                        SceneEvent::new(event_kind::LOADED, &pend.load_id).at(t.x, t.y, t.z),
                    );
                }
            }
            Some(LoadState::Failed(err)) => {
                error!("gearbox-api: USD load FAILED for `{}`: {err}", pend.load_id);
                commands.entity(entity).remove::<PendingLoadedUsd>();
            }
            _ => {}
        }
    }
}

fn clear_on_reset(
    mut messages: MessageReader<SimResetRequest>,
    mut commands: Commands,
    mut state: ResMut<LoaderState>,
) {
    let scopes: Vec<u32> = messages.read().map(|m| m.scope).collect();
    if scopes.is_empty() {
        return;
    }
    if !scopes
        .iter()
        .any(|s| matches!(*s, clear_scope::ALL | clear_scope::PROPS))
    {
        return;
    }
    state.tombstones.clear();
    for (_id, entry) in state.entities.drain() {
        commands.entity(entry.entity).try_despawn();
    }
}
