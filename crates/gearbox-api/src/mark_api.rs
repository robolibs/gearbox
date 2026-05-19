//! Lightweight marker API over zenoh.
//!
//! This is deliberately **not** the USD loader. It owns small debug/target
//! marker meshes directly and updates them in place by caller-chosen UUID.
//!
//! ## Topics
//!
//! ```text
//! gearbox/usd/mark/<mark_uuid>/<x>/<y>/<z>
//! gearbox/usd/mark/<mark_uuid>/delete
//! ```
//!
//! Re-publishing the same `<mark_uuid>` moves the existing marker entity.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use zenoh::Wait;

#[cfg(feature = "bevy")]
use bevy::prelude::*;

#[derive(Debug, Clone)]
enum MarkCommand {
    Set { id: String, x: f32, y: f32, z: f32 },
    Delete { id: String },
}

pub struct MarkerBroker {
    _session: Arc<zenoh::Session>,
    inbox: Arc<Mutex<VecDeque<MarkCommand>>>,
    _subscriber: zenoh::pubsub::Subscriber<()>,
}

impl MarkerBroker {
    pub fn open(
        session: Arc<zenoh::Session>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let inbox: Arc<Mutex<VecDeque<MarkCommand>>> = Arc::new(Mutex::new(VecDeque::new()));
        let inbox_cb = Arc::clone(&inbox);
        let subscriber = session
            .declare_subscriber("gearbox/usd/mark/**")
            .callback(move |sample| {
                let key = sample.key_expr().as_str();
                match parse_mark_key(key) {
                    Some(cmd) => {
                        if let Ok(mut q) = inbox_cb.lock() {
                            q.push_back(cmd);
                        }
                    }
                    None => {
                        eprintln!("gearbox-api: bad marker key `{key}`");
                    }
                }
            })
            .wait()?;
        Ok(Self {
            _session: session,
            inbox,
            _subscriber: subscriber,
        })
    }

    fn drain_inbox(&self) -> Vec<MarkCommand> {
        match self.inbox.lock() {
            Ok(mut q) => q.drain(..).collect(),
            Err(_) => Vec::new(),
        }
    }
}

fn parse_mark_key(key: &str) -> Option<MarkCommand> {
    let rest = key.strip_prefix("gearbox/usd/mark/")?;
    let parts = rest.split('/').collect::<Vec<_>>();
    match parts.as_slice() {
        [id, "delete"] | [id, "remove"] => Some(MarkCommand::Delete {
            id: (*id).to_string(),
        }),
        [id, x, y, z] => Some(MarkCommand::Set {
            id: (*id).to_string(),
            x: x.parse().ok()?,
            y: y.parse().ok()?,
            z: z.parse().ok()?,
        }),
        _ => None,
    }
}

#[cfg(feature = "bevy")]
#[derive(Resource)]
pub struct MarkerApiSession {
    broker: Mutex<MarkerBroker>,
    entities: Mutex<HashMap<String, Entity>>,
}

#[cfg(feature = "bevy")]
#[derive(Resource, Clone)]
struct MarkerAssets {
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

#[cfg(feature = "bevy")]
pub struct UsdMarkerApiPlugin;

#[cfg(feature = "bevy")]
impl Plugin for UsdMarkerApiPlugin {
    fn build(&self, app: &mut App) {
        match zenoh::open(zenoh::Config::default()).wait() {
            Ok(session) => {
                let session = Arc::new(session);
                match MarkerBroker::open(session) {
                    Ok(broker) => {
                        app.insert_resource(MarkerApiSession {
                            broker: Mutex::new(broker),
                            entities: Mutex::new(HashMap::new()),
                        });
                        app.add_systems(Startup, setup_marker_assets);
                        app.add_systems(Update, apply_marker_commands_system);
                        app.add_systems(Update, clear_markers_on_reset_system);
                        info!(
                            "gearbox-api: marker API ready \
                             (gearbox/usd/mark/<uuid>/<x>/<y>/<z>)"
                        );
                    }
                    Err(e) => warn!("gearbox-api: marker subscriber open failed ({e})"),
                }
            }
            Err(e) => warn!("gearbox-api: marker session open failed ({e})"),
        }
    }
}

#[cfg(feature = "bevy")]
fn setup_marker_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(MarkerAssets {
        mesh: meshes.add(Cuboid::new(0.64, 0.45, 0.64)),
        material: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.0, 0.0),
            emissive: Color::srgb(1.0, 0.0, 0.0).into(),
            perceptual_roughness: 0.5,
            metallic: 0.0,
            ..default()
        }),
    });
}

#[cfg(feature = "bevy")]
fn apply_marker_commands_system(
    mut commands: Commands,
    api: Option<Res<MarkerApiSession>>,
    assets: Option<Res<MarkerAssets>>,
    mut transforms: Query<&mut Transform>,
) {
    let (Some(api), Some(assets)) = (api, assets) else {
        return;
    };
    let Ok(broker) = api.broker.lock() else {
        return;
    };
    let Ok(mut entities) = api.entities.lock() else {
        return;
    };

    for cmd in broker.drain_inbox() {
        match cmd {
            MarkCommand::Set { id, x, y, z } => {
                let position = Vec3::new(x, y, z);
                if let Some(entity) = entities.get(&id).copied() {
                    if let Ok(mut tr) = transforms.get_mut(entity) {
                        tr.translation = position;
                        continue;
                    }
                    entities.remove(&id);
                }

                let entity = commands
                    .spawn((
                        Name::new(format!("UsdMarker[{id}]")),
                        Transform::from_translation(position),
                        Mesh3d(assets.mesh.clone()),
                        MeshMaterial3d(assets.material.clone()),
                    ))
                    .id();
                entities.insert(id, entity);
            }
            MarkCommand::Delete { id } => {
                if let Some(entity) = entities.remove(&id) {
                    commands.entity(entity).try_despawn();
                }
            }
        }
    }
}

#[cfg(feature = "bevy")]
fn clear_markers_on_reset_system(
    messages: Option<MessageReader<gearbox_viz::SimResetRequest>>,
    mut commands: Commands,
    api: Option<Res<MarkerApiSession>>,
) {
    let Some(mut messages) = messages else { return };
    if messages.read().count() == 0 {
        return;
    }
    let Some(api) = api else { return };
    let Ok(mut entities) = api.entities.lock() else {
        return;
    };
    for (_id, entity) in entities.drain() {
        commands.entity(entity).try_despawn();
    }
}
