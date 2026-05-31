//! Generic USD loader API over zenoh.
//!
//! This is not a "marker" API. It is the tool-facing way to ask the
//! running Gearbox app to load, move, update, or unload a USD asset by id.
//! A loaded USD can be a machine/robot, an asset with variants, or a plain
//! static USD. More categories (animated USD, sensor rigs, tools, etc.) can
//! be added later without changing the topic shape.
//!
//! ## Topic
//!
//! | direction | key                         | payload         |
//! |-----------|-----------------------------|-----------------|
//! | sub       | `gearbox/usd/load/<id>`     | [`UsdLoadWire`] |
//! | sub       | `gearbox/usd/delete/<id>`   | [`UsdLoadWire`] |
//!
//! `id` is a caller-chosen runtime instance id. Re-publishing the same id
//! moves that loaded USD in place when the asset is unchanged, or replaces it
//! when the asset differs. Publishing `remove: true` unloads it. Publishing to
//! `gearbox/usd/delete/<id>` permanently tombstones that exact runtime id, so
//! late async load messages for that id cannot resurrect it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use zenoh::Wait;

use crate::wire::decode;

#[cfg(feature = "bevy")]
use bevy::ecs::entity::Entities;
#[cfg(feature = "bevy")]
use bevy::prelude::*;

pub const CATEGORY_MACHINE: &str = "machine";
pub const CATEGORY_ROBOT: &str = "robot";
pub const CATEGORY_VARIANT_USD: &str = "variant_usd";
pub const CATEGORY_STATIC_USD: &str = "static_usd";

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct UsdLoadWire {
    /// Loader category. Current base categories:
    ///
    /// * `"machine"` / `"robot"` — same USD may expose controller topics.
    ///   `bin/gearbox`'s machine loader handles these.
    /// * `"variant_usd"` — ordinary USD with authored variant selections.
    /// * `"static_usd"` — ordinary static USD with no special runtime intent.
    #[serde(default)]
    pub category: String,
    /// Optional runtime machine namespace for machine/robot USD instances.
    /// Ignored for static and variant-only USDs.
    #[serde(default)]
    pub namespace: Option<String>,
    /// Gearbox world position. Runtime is Y-up: X/Z are field plane, Y is
    /// height above ground.
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub z: f64,
    /// Yaw in degrees around Gearbox runtime +Y.
    #[serde(default)]
    pub yaw_deg: f64,
    /// Drop/unload the USD instance with this id.
    #[serde(default)]
    pub remove: bool,
    /// Permanent delete for this exact runtime id. Normally set by the
    /// `gearbox/usd/delete/<id>` topic. Deleted ids are tombstoned and later
    /// same-id load messages are ignored.
    #[serde(default)]
    pub delete: bool,
    /// Optional caller/run token. If a remove with token `T` is received for
    /// an id, later stale load messages for that same id+token are ignored.
    /// A new run can use a different token and load the id again.
    #[serde(default)]
    pub nonce: String,
    /// USD asset path, usually relative to the binary's `assets/` directory.
    #[serde(default)]
    pub usd_path: Option<String>,
    /// Optional USD variant selections: each tuple is
    /// `(prim_path, variant_set_name, option_name)`.
    #[serde(default)]
    pub usd_variants: Vec<(String, String, String)>,
}

impl UsdLoadWire {
    pub fn effective_category(&self) -> &str {
        if self.category.is_empty() {
            if self.usd_variants.is_empty() {
                CATEGORY_STATIC_USD
            } else {
                CATEGORY_VARIANT_USD
            }
        } else {
            self.category.as_str()
        }
    }

    pub fn is_machine_category(&self) -> bool {
        matches!(self.effective_category(), CATEGORY_MACHINE | CATEGORY_ROBOT)
    }
}

// ─── Broker ────────────────────────────────────────────────────────

pub struct UsdLoaderBroker {
    _session: Arc<zenoh::Session>,
    /// Newly-arrived USD load/update messages keyed by runtime id. Drained
    /// each frame by the Bevy apply system.
    inbox: Arc<Mutex<HashMap<String, UsdLoadWire>>>,
    _load_subscriber: zenoh::pubsub::Subscriber<()>,
    _delete_subscriber: zenoh::pubsub::Subscriber<()>,
}

impl UsdLoaderBroker {
    pub fn open(
        session: Arc<zenoh::Session>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let inbox: Arc<Mutex<HashMap<String, UsdLoadWire>>> = Arc::new(Mutex::new(HashMap::new()));
        let inbox_cb = Arc::clone(&inbox);
        let load_subscriber = session
            .declare_subscriber("gearbox/usd/load/**")
            .callback(move |sample| {
                let key = sample.key_expr().as_str().to_string();
                let id = key
                    .strip_prefix("gearbox/usd/load/")
                    .unwrap_or(&key)
                    .to_string();
                let bytes = sample.payload().to_bytes();
                match decode::<UsdLoadWire>(bytes.as_ref()) {
                    Ok(req) => {
                        if let Ok(mut q) = inbox_cb.lock() {
                            q.insert(id, req);
                        }
                    }
                    Err(e) => eprintln!("gearbox-api: bad USD load payload for `{key}`: {e}"),
                }
            })
            .wait()?;
        let delete_inbox_cb = Arc::clone(&inbox);
        let delete_subscriber = session
            .declare_subscriber("gearbox/usd/delete/**")
            .callback(move |sample| {
                let key = sample.key_expr().as_str().to_string();
                let id = key
                    .strip_prefix("gearbox/usd/delete/")
                    .unwrap_or(&key)
                    .to_string();
                let bytes = sample.payload().to_bytes();
                let mut req = decode::<UsdLoadWire>(bytes.as_ref()).unwrap_or_default();
                req.remove = true;
                req.delete = true;
                if let Ok(mut q) = delete_inbox_cb.lock() {
                    q.insert(id, req);
                }
            })
            .wait()?;
        Ok(Self {
            _session: session,
            inbox,
            _load_subscriber: load_subscriber,
            _delete_subscriber: delete_subscriber,
        })
    }

    pub fn drain_inbox(&self) -> HashMap<String, UsdLoadWire> {
        match self.inbox.lock() {
            Ok(mut q) => std::mem::take(&mut *q),
            Err(_) => HashMap::new(),
        }
    }
}

// ─── Bevy plugin ───────────────────────────────────────────────────

#[cfg(feature = "bevy")]
#[derive(Resource)]
pub struct UsdLoaderApiSession {
    pub broker: Mutex<UsdLoaderBroker>,
    /// Map runtime id → the entity loaded for it plus the asset it is
    /// currently showing. Re-publishing an id with the same asset moves that
    /// entity in place; a different asset replaces it.
    entities: Mutex<HashMap<String, LoadedUsdEntry>>,
    /// id -> nonce for recently/authoritatively removed ids. This prevents an
    /// async/stale `load` from the same run resurrecting a harvested bale.
    tombstones: Mutex<HashMap<String, String>>,
}

/// A live loaded-USD instance owned by the loader API.
#[cfg(feature = "bevy")]
struct LoadedUsdEntry {
    entity: Entity,
    signature: LoadSignature,
}

/// Identifies *what asset* an id is showing. A re-publish that keeps the same
/// signature is an in-place move; a different signature forces a fresh spawn.
#[cfg(feature = "bevy")]
#[derive(Clone, PartialEq)]
enum LoadSignature {
    /// A USD asset, identified by path + variant selection.
    Usd {
        path: String,
        variants: Vec<(String, String, String)>,
    },
}

#[cfg(feature = "bevy")]
pub struct UsdLoaderApiPlugin;

#[cfg(feature = "bevy")]
impl Plugin for UsdLoaderApiPlugin {
    fn build(&self, app: &mut App) {
        match zenoh::open(zenoh::Config::default()).wait() {
            Ok(session) => {
                let session = Arc::new(session);
                match UsdLoaderBroker::open(session) {
                    Ok(broker) => {
                        app.insert_resource(UsdLoaderApiSession {
                            broker: Mutex::new(broker),
                            entities: Mutex::new(HashMap::new()),
                            tombstones: Mutex::new(HashMap::new()),
                        });
                        app.add_systems(Update, apply_usd_loads_system);
                        app.add_systems(Update, instantiate_pending_loaded_usd);
                        app.add_systems(Update, clear_loaded_usds_on_reset_system);
                        info!("gearbox-api: USD loader API ready (gearbox/usd/load/<id>)");
                    }
                    Err(e) => {
                        warn!(
                            "gearbox-api: USD loader subscriber open failed ({e}); loader API disabled"
                        );
                    }
                }
            }
            Err(e) => {
                warn!("gearbox-api: USD loader session open failed ({e}); loader API disabled");
            }
        }
    }
}

/// Pending scene-load handle for a non-machine USD loaded through
/// `gearbox/usd/load/<id>`.
#[cfg(feature = "bevy")]
#[derive(Component)]
pub struct PendingLoadedUsd {
    pub handle: Handle<usd_bevy::UsdAsset>,
    pub load_id: String,
}

#[cfg(feature = "bevy")]
fn apply_usd_loads_system(
    mut commands: Commands,
    api: Option<Res<UsdLoaderApiSession>>,
    asset_server: Res<bevy::asset::AssetServer>,
    asset_root: Option<Res<gearbox_viz::UsdAssetRoot>>,
    live_entities: &Entities,
    mut transforms: Query<&mut Transform>,
) {
    let Some(api) = api else { return };
    let Ok(broker) = api.broker.lock() else {
        return;
    };
    let Ok(mut entities) = api.entities.lock() else {
        return;
    };
    let Ok(mut tombstones) = api.tombstones.lock() else {
        return;
    };
    let inbox = broker.drain_inbox();
    for (id, req) in inbox {
        if req.is_machine_category() {
            // Machine/robot categories are handled by bin/gearbox's runtime
            // loader because it also has to register controller metadata.
            continue;
        }

        // The entity for an id can be despawned by other systems — most
        // importantly the world's contact-harvest, which deletes bales
        // directly. Forget any entry whose entity is already gone so the id
        // behaves like a fresh one on its next publish.
        if entities
            .get(&id)
            .is_some_and(|entry| !live_entities.contains(entry.entity))
        {
            entities.remove(&id);
        }

        if req.remove || req.delete {
            if let Some(entry) = entities.remove(&id) {
                commands.entity(entry.entity).despawn();
            }
            if req.delete {
                // Permanent exact-id delete. This is for UUID-style runtime
                // ids: once deleted, late async load messages for the same id
                // must never bring that exact USD instance back.
                tombstones.insert(id.clone(), String::new());
            } else if !req.nonce.is_empty() {
                tombstones.insert(id.clone(), req.nonce.clone());
            }
            continue;
        }

        if tombstones
            .get(&id)
            .is_some_and(|removed_nonce| removed_nonce.is_empty())
        {
            bevy::log::debug!("gearbox-api: ignoring load for deleted USD id `{id}`");
            continue;
        }

        if !req.nonce.is_empty() {
            if tombstones
                .get(&id)
                .is_some_and(|removed_nonce| removed_nonce == &req.nonce)
            {
                bevy::log::debug!(
                    "gearbox-api: ignoring stale USD load `{id}` for removed nonce `{}`",
                    req.nonce
                );
                continue;
            }
            tombstones.remove(&id);
        }

        let Some(usd_rel) = req.usd_path.as_deref() else {
            bevy::log::warn!("gearbox-api: ignoring USD load `{id}` with no `usd_path`");
            continue;
        };
        let signature = LoadSignature::Usd {
            path: usd_rel.to_string(),
            variants: req.usd_variants.clone(),
        };

        // Re-publishing an id with the *same asset* is a move, not a replace.
        // Keeping the same entity (and its `Name`) means the loaded USD never
        // blinks and `Added`-gated terrain snapping does not re-run. This is
        // what lets a script hold a stable target marker on a bale, or slide
        // it to a new bale, with no spawn/despawn churn or vertical flashing.
        if entities
            .get(&id)
            .is_some_and(|entry| entry.signature == signature)
        {
            let entity = entities[&id].entity;
            if let Ok(mut transform) = transforms.get_mut(entity) {
                transform.translation.x = req.x as f32;
                transform.translation.z = req.z as f32;
                transform.rotation = Quat::from_rotation_y((req.yaw_deg as f32).to_radians());
            }
            continue;
        }

        // New id, or the asset itself changed: drop the previous entity (if
        // any) and spawn a fresh one.
        if let Some(entry) = entities.remove(&id) {
            commands.entity(entry.entity).despawn();
        }

        let Some(root) = asset_root.as_deref() else {
            bevy::log::warn!(
                "gearbox-api: USD load `{id}` requested `{usd_rel}` but no `UsdAssetRoot` is registered"
            );
            continue;
        };
        bevy::log::info!(
            "gearbox-api: load `{id}` category={} USD `{}` variants={:?}",
            req.effective_category(),
            usd_rel,
            req.usd_variants,
        );
        let asset_file = root.0.join(usd_rel);
        let asset_path_string = asset_file.to_string_lossy().into_owned();
        let asset_parent = asset_file
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| root.0.clone());
        let parent_clone = asset_parent.clone();
        let variants: Vec<usd_bevy::VariantSelection> = req
            .usd_variants
            .iter()
            .map(|(prim_path, set_name, option)| usd_bevy::VariantSelection {
                prim_path: prim_path.clone(),
                set_name: set_name.clone(),
                option: option.clone(),
            })
            .collect();
        let asset_path: bevy::asset::AssetPath<'static> = if variants.is_empty() {
            asset_path_string.into()
        } else {
            let label = usd_bevy::variant_label(&variants);
            bevy::asset::AssetPath::from(asset_path_string).with_label(label)
        };
        let handle: Handle<usd_bevy::UsdAsset> = asset_server.load_with_settings(
            asset_path,
            move |s: &mut usd_bevy::UsdLoaderSettings| {
                s.search_paths = vec![parent_clone.clone()];
                s.variant_selections = variants.clone();
            },
        );
        let spawned = commands
            .spawn((
                Name::new(format!("UsdLoad[{}]::pending", id)),
                Transform {
                    translation: Vec3::new(req.x as f32, req.y as f32, req.z as f32),
                    rotation: Quat::from_rotation_y((req.yaw_deg as f32).to_radians()),
                    ..default()
                },
                Visibility::default(),
                PendingLoadedUsd {
                    handle,
                    load_id: id.clone(),
                },
            ))
            .id();
        entities.insert(
            id,
            LoadedUsdEntry {
                entity: spawned,
                signature,
            },
        );
    }
}

#[cfg(feature = "bevy")]
fn instantiate_pending_loaded_usd(
    mut commands: Commands,
    asset_server: Res<bevy::asset::AssetServer>,
    usd_assets: Res<bevy::asset::Assets<usd_bevy::UsdAsset>>,
    pending: Query<(Entity, &PendingLoadedUsd)>,
) {
    use bevy::asset::LoadState;
    for (entity, pend) in pending.iter() {
        match asset_server.get_load_state(&pend.handle) {
            Some(LoadState::Loaded) => {
                let Some(asset) = usd_assets.get(&pend.handle) else {
                    continue;
                };
                commands
                    .entity(entity)
                    .insert(bevy::scene::SceneRoot(asset.scene.clone()))
                    .remove::<PendingLoadedUsd>();
            }
            Some(LoadState::Failed(err)) => {
                bevy::log::error!(
                    "gearbox-api: USD load FAILED for `{}`: {}",
                    pend.load_id,
                    err
                );
                commands.entity(entity).remove::<PendingLoadedUsd>();
            }
            _ => {}
        }
    }
}

#[cfg(feature = "bevy")]
fn clear_loaded_usds_on_reset_system(
    messages: Option<MessageReader<gearbox_viz::SimResetRequest>>,
    mut commands: Commands,
    api: Option<Res<UsdLoaderApiSession>>,
) {
    let Some(mut messages) = messages else { return };
    if messages.read().count() == 0 {
        return;
    }
    let Some(api) = api else { return };
    let Ok(mut entities) = api.entities.lock() else {
        return;
    };
    if let Ok(mut tombstones) = api.tombstones.lock() {
        tombstones.clear();
    }
    for (_id, entry) in entities.drain() {
        // `try_despawn`: a contact-harvested bale may already be gone.
        commands.entity(entry.entity).try_despawn();
    }
}
