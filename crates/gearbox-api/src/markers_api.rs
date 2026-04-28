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

#[cfg(feature = "bevy")]
fn apply_markers_system(
    mut commands: Commands,
    api: Option<Res<MarkersApiSession>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
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
