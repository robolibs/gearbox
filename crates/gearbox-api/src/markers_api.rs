//! **Pluggable** world-marker API — drop coloured cones / cubes /
//! spheres in the scene over zenoh. Used by demo scripts to scatter
//! bales (or anything else) for the goto controller to visit.
//!
//! Same delete-it-later pattern as the rest of `gearbox-api`:
//!
//!   1. delete this file,
//!   2. drop `pub mod markers_api;` + the `markers_api::*` re-exports
//!      in `lib.rs`,
//!   3. drop `app.add_plugins(MarkersApiPlugin)` in `main.rs`.
//!
//! ## Topic
//!
//! | direction | key                       | payload         |
//! |-----------|---------------------------|-----------------|
//! | sub       | `gearbox/markers/<id>`    | [`MarkerWire`]  |
//!
//! `id` is any user-chosen string. Re-publishing with the same `id`
//! moves the marker (or removes it when `remove: true`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use zenoh::Wait;

use crate::wire::decode;

#[cfg(feature = "bevy")]
use bevy::prelude::*;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MarkerWire {
    /// Gearbox world position (X = lateral, Z = longitudinal).
    pub x: f64,
    pub z: f64,
    /// Visual height in metres. 0 ⇒ default 1.0.
    #[serde(default)]
    pub height: f64,
    /// Visual radius in metres. 0 ⇒ default 0.4.
    #[serde(default)]
    pub radius: f64,
    /// `"cone" | "box" | "sphere"`. Anything else falls back to cone.
    #[serde(default)]
    pub kind: String,
    /// sRGB colour `[r, g, b]` in 0..=1. All zero ⇒ default yellow.
    #[serde(default)]
    pub color: [f32; 3],
    /// Drop the marker with this id.
    #[serde(default)]
    pub remove: bool,
    /// Optional USD asset path (relative to the binary's `assets/`
    /// directory). When set, the marker renders the loaded USD scene
    /// at `(x, ground, z)` instead of a procedural cone/box/sphere
    /// — `kind`, `radius`, and `height` are ignored. `color` is
    /// also ignored (the asset's own materials apply).
    #[serde(default)]
    pub usd_path: Option<String>,
    /// Optional USD variant selections: each tuple is
    /// `(prim_path, variant_set_name, option_name)`. Re-publishing
    /// the same marker id with a different selection swaps which
    /// authored variant is composed into the scene — the standard
    /// way to switch between e.g. a "default" and a "red" colour
    /// on the same asset without authoring two separate files.
    #[serde(default)]
    pub usd_variants: Vec<(String, String, String)>,
}

// ─── Broker ────────────────────────────────────────────────────────

pub struct MarkersBroker {
    _session: Arc<zenoh::Session>,
    /// Newly-arrived marker messages keyed by id. Drained each frame
    /// by the apply system and converted into Bevy entities.
    inbox: Arc<Mutex<HashMap<String, MarkerWire>>>,
    _subscriber: zenoh::pubsub::Subscriber<()>,
}

impl MarkersBroker {
    pub fn open(session: Arc<zenoh::Session>) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let inbox: Arc<Mutex<HashMap<String, MarkerWire>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let inbox_cb = Arc::clone(&inbox);
        let subscriber = session
            .declare_subscriber("gearbox/markers/**")
            .callback(move |sample| {
                let key = sample.key_expr().as_str().to_string();
                let id = key
                    .strip_prefix("gearbox/markers/")
                    .unwrap_or(&key)
                    .to_string();
                let bytes = sample.payload().to_bytes();
                match decode::<MarkerWire>(bytes.as_ref()) {
                    Ok(marker) => {
                        if let Ok(mut q) = inbox_cb.lock() {
                            q.insert(id, marker);
                        }
                    }
                    Err(e) => eprintln!("gearbox-api: bad marker payload for `{key}`: {e}"),
                }
            })
            .wait()?;
        Ok(Self {
            _session: session,
            inbox,
            _subscriber: subscriber,
        })
    }

    pub fn drain_inbox(&self) -> HashMap<String, MarkerWire> {
        match self.inbox.lock() {
            Ok(mut q) => std::mem::take(&mut *q),
            Err(_) => HashMap::new(),
        }
    }
}

// ─── Bevy plugin ───────────────────────────────────────────────────

#[cfg(feature = "bevy")]
#[derive(Resource)]
pub struct MarkersApiSession {
    pub broker: Mutex<MarkersBroker>,
    /// Map marker id → entity, so we can move / despawn in place
    /// when the same id is re-published.
    entities: Mutex<HashMap<String, Entity>>,
}

#[cfg(feature = "bevy")]
pub struct MarkersApiPlugin;

#[cfg(feature = "bevy")]
impl Plugin for MarkersApiPlugin {
    fn build(&self, app: &mut App) {
        match zenoh::open(zenoh::Config::default()).wait() {
            Ok(session) => {
                let session = Arc::new(session);
                match MarkersBroker::open(session) {
                    Ok(broker) => {
                        app.insert_resource(MarkersApiSession {
                            broker: Mutex::new(broker),
                            entities: Mutex::new(HashMap::new()),
                        });
                        app.add_systems(Update, apply_markers_system);
                        app.add_systems(Update, instantiate_pending_marker_usd);
                        app.add_systems(Update, clear_markers_on_reset_system);
                        info!("gearbox-api: markers API ready (gearbox/markers/<id>)");
                    }
                    Err(e) => {
                        warn!("gearbox-api: markers subscriber open failed ({e}); markers API disabled");
                    }
                }
            }
            Err(e) => {
                warn!("gearbox-api: markers session open failed ({e}); markers API disabled");
            }
        }
    }
}

/// Marker-side counterpart to `gearbox_viz`'s `PendingUsdScene`.
/// Carries an in-flight `Handle<UsdAsset>` for a marker entity that
/// asked to render a USD asset; promoted to a `SceneRoot` once the
/// asset finishes loading.
#[cfg(feature = "bevy")]
#[derive(Component)]
pub struct PendingMarkerUsd {
    pub handle: Handle<usd_bevy::UsdAsset>,
    /// Marker id — used purely for log lines so a stuck load can be
    /// traced back to the user that published it.
    pub marker_id: String,
}

#[cfg(feature = "bevy")]
fn apply_markers_system(
    mut commands: Commands,
    api: Option<Res<MarkersApiSession>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    asset_server: Res<bevy::asset::AssetServer>,
    asset_root: Option<Res<gearbox_viz::UsdAssetRoot>>,
) {
    let Some(api) = api else { return };
    let Ok(broker) = api.broker.lock() else { return };
    let Ok(mut entities) = api.entities.lock() else { return };
    let inbox = broker.drain_inbox();
    for (id, m) in inbox {
        if let Some(entity) = entities.remove(&id) {
            commands.entity(entity).despawn();
        }
        if m.remove {
            continue;
        }

        // ── USD-asset markers ─────────────────────────────────────
        if let Some(usd_rel) = m.usd_path.as_deref() {
            let Some(root) = asset_root.as_deref() else {
                bevy::log::warn!(
                    "gearbox-api: marker `{id}` requested USD `{usd_rel}` but \
                     no `UsdAssetRoot` resource is registered; skipping"
                );
                continue;
            };
            bevy::log::info!(
                "gearbox-api: marker `{id}` USD `{}` variants={:?}",
                usd_rel, m.usd_variants,
            );
            let asset_parent = std::path::Path::new(usd_rel)
                .parent()
                .map(|p| root.0.join(p))
                .unwrap_or_else(|| root.0.clone());
            let parent_clone = asset_parent.clone();
            // Convert wire variant selections into bevy_openusd's
            // shape, and encode them into the asset path's *label*.
            // Bevy's `AssetServer` keys cached `Handle<UsdAsset>` by
            // full asset path (including label) but **ignores** the
            // settings closure when forming the cache key. Two
            // `load_with_settings` calls on the same path therefore
            // collapse to one handle and the second load overwrites
            // the first asset's content — which means flipping ONE
            // bale to "red" turns every bale red, since they all
            // share the same handle. Encoding the variant in the
            // label gives us per-variant handle isolation: the local
            // bevy_openusd fork parses the label on the way in and
            // applies the same selections, so this is enough to
            // produce the right scene without also stuffing them
            // into `settings.variant_selections`.
            let variants: Vec<usd_bevy::VariantSelection> = m
                .usd_variants
                .iter()
                .map(|(prim_path, set_name, option)| {
                    usd_bevy::VariantSelection {
                        prim_path: prim_path.clone(),
                        set_name: set_name.clone(),
                        option: option.clone(),
                    }
                })
                .collect();
            let asset_path: bevy::asset::AssetPath<'static> = if variants.is_empty() {
                usd_rel.to_string().into()
            } else {
                let label = usd_bevy::variant_label(&variants);
                bevy::asset::AssetPath::from(usd_rel.to_string()).with_label(label)
            };
            let handle: Handle<usd_bevy::UsdAsset> = asset_server
                .load_with_settings(
                    asset_path,
                    move |s: &mut usd_bevy::UsdLoaderSettings| {
                        s.search_paths = vec![parent_clone.clone()];
                        // Variants come in via the path label too;
                        // we set them in settings as well so a
                        // headless build that bypasses the label
                        // path still gets the correct composition.
                        s.variant_selections = variants.clone();
                    },
                );
            let entity = commands
                .spawn((
                    Name::new(format!("Marker[{}]::usd_pending", id)),
                    Transform::from_xyz(m.x as f32, 0.0, m.z as f32),
                    Visibility::default(),
                    PendingMarkerUsd {
                        handle,
                        marker_id: id.clone(),
                    },
                ))
                .id();
            entities.insert(id, entity);
            continue;
        }

        // ── Procedural primitive markers ─────────────────────────
        let height = if m.height > 0.0 { m.height as f32 } else { 1.0 };
        let radius = if m.radius > 0.0 { m.radius as f32 } else { 0.4 };
        let color = if m.color == [0.0, 0.0, 0.0] {
            [0.95, 0.85, 0.15] // bale-yellow default
        } else {
            m.color
        };

        let mesh = match m.kind.as_str() {
            "box" | "cube" => meshes.add(Cuboid::new(radius * 2.0, height, radius * 2.0)),
            "sphere" | "ball" => meshes.add(Sphere::new(radius)),
            _ => meshes.add(Cone {
                radius,
                height,
            }),
        };
        let mat = materials.add(StandardMaterial {
            base_color: Color::srgb(color[0], color[1], color[2]),
            perceptual_roughness: 0.6,
            metallic: 0.0,
            ..default()
        });

        // Cone / Cube apex / centre at the entity origin — lift the
        // entity by half-height so the base sits on the ground.
        let entity = commands
            .spawn((
                Name::new(format!("Marker[{}]", id)),
                Transform::from_xyz(m.x as f32, height * 0.5, m.z as f32),
                Mesh3d(mesh),
                MeshMaterial3d(mat),
            ))
            .id();
        entities.insert(id, entity);
    }
}

/// Promote `PendingMarkerUsd` markers to a `SceneRoot` once the
/// underlying `UsdAsset` finishes loading. Mirrors the pattern used
/// in `gearbox_viz::spawn::instantiate_pending_usd_scenes`.
#[cfg(feature = "bevy")]
fn instantiate_pending_marker_usd(
    mut commands: Commands,
    asset_server: Res<bevy::asset::AssetServer>,
    usd_assets: Res<bevy::asset::Assets<usd_bevy::UsdAsset>>,
    pending: Query<(Entity, &PendingMarkerUsd)>,
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
                    .remove::<PendingMarkerUsd>();
            }
            Some(LoadState::Failed(err)) => {
                bevy::log::error!(
                    "gearbox-api: USD load FAILED for marker `{}`: {}",
                    pend.marker_id, err
                );
                commands.entity(entity).remove::<PendingMarkerUsd>();
            }
            _ => {}
        }
    }
}

/// On a [`gearbox_viz::SimResetRequest`] event, despawn every marker
/// entity and forget the id→entity map so the `gearbox/sim/reset`
/// scene-clear path leaves no stale markers behind.
#[cfg(feature = "bevy")]
fn clear_markers_on_reset_system(
    mut messages: MessageReader<gearbox_viz::SimResetRequest>,
    mut commands: Commands,
    api: Option<Res<MarkersApiSession>>,
) {
    if messages.read().count() == 0 {
        return;
    }
    let Some(api) = api else { return };
    let Ok(mut entities) = api.entities.lock() else { return };
    for (_id, entity) in entities.drain() {
        commands.entity(entity).despawn();
    }
}
