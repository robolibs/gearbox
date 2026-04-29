//! **Pluggable** vehicle-spawn API — drop a fresh tractor / husky /
//! robotti / drone / oxbo into the scene over zenoh, anywhere on the
//! ground plane, at any yaw.
//!
//! Same delete-it-later pattern as the rest of `gearbox-api`:
//!
//!   1. delete this file,
//!   2. drop `pub mod spawn_api;` + the `spawn_api::*` re-exports
//!      in `lib.rs`,
//!   3. drop `app.add_plugins(SpawnApiPlugin)` in `main.rs`.
//!
//! ## Topics
//!
//! | direction | key                       | payload                |
//! |-----------|---------------------------|------------------------|
//! | sub       | `gearbox/sim/spawn`       | [`SpawnVehicleWire`]   |
//! | pub       | `gearbox/sim/spawned`     | [`SpawnedVehicleWire`] |
//!
//! `preset` is the stable id from `gearbox_core::presets::registry`:
//! `"tractor" | "husky" | "robotti" | "drone" | "oxbo"`.
//!
//! After a spawn lands, the vehicle's per-instance topic prefix is
//! `<preset_name>_<id>` (e.g. `tractor_0/odom`, `tractor_0/cmd_vel`,
//! `tractor_0/goto`) — the same convention every other API in this
//! crate uses.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use zenoh::Wait;

use crate::wire::{decode, encode};

#[cfg(feature = "bevy")]
use bevy::prelude::*;

#[cfg(feature = "bevy")]
use gearbox_viz::{spawn_height_for, spawn_vehicle_visuals, GearboxSim, PlayerControlled};

#[cfg(feature = "bevy")]
use datapod::{Point, Pose, Quaternion};

#[cfg(feature = "bevy")]
use gearbox_core::presets;

// ─── Wire types ────────────────────────────────────────────────────

/// Spawn request. `y` is auto-corrected to the preset's natural
/// resting height when 0 (so callers can pass `(x, 0, z)` and the
/// vehicle drops cleanly onto the ground).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpawnVehicleWire {
    /// Preset id — one of `"tractor" | "husky" | "robotti" | "drone" | "oxbo"`.
    pub preset: String,
    /// World-frame spawn position (gearbox tangent-plane metres).
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    pub z: f64,
    /// Yaw in degrees, around world +Y. 0 = facing -Z (gearbox forward).
    #[serde(default)]
    pub yaw_deg: f64,
    /// Tag the spawned vehicle with `PlayerControlled` so WASD drives
    /// it. Off by default — most external spawns are agent-driven.
    #[serde(default)]
    pub player: bool,
}

/// Confirmation echoed back after a spawn lands. Lets the client
/// learn the assigned `id` (and hence the topic prefix
/// `<preset_name>_<id>`) for the new vehicle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpawnedVehicleWire {
    pub id: u32,
    /// Spec name (the topic-prefix root, e.g. `"tractor"`). May
    /// differ from `preset` if a preset's spec uses a renamed
    /// `VehicleBuilder::new(...)`.
    pub name: String,
    pub preset: String,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub yaw_deg: f64,
}

// ─── Broker ────────────────────────────────────────────────────────

/// Holds the pending-request queue and a handle to the zenoh session
/// so the apply system can publish `gearbox/sim/spawned` confirmations.
pub struct SpawnBroker {
    session: Arc<zenoh::Session>,
    inbox: Arc<Mutex<VecDeque<SpawnVehicleWire>>>,
    _subscriber: zenoh::pubsub::Subscriber<()>,
}

impl SpawnBroker {
    pub fn open(
        session: Arc<zenoh::Session>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let inbox: Arc<Mutex<VecDeque<SpawnVehicleWire>>> =
            Arc::new(Mutex::new(VecDeque::new()));
        let inbox_cb = Arc::clone(&inbox);
        let subscriber = session
            .declare_subscriber("gearbox/sim/spawn")
            .callback(move |sample| {
                let bytes = sample.payload().to_bytes();
                match decode::<SpawnVehicleWire>(bytes.as_ref()) {
                    Ok(req) => {
                        if let Ok(mut q) = inbox_cb.lock() {
                            q.push_back(req);
                        }
                    }
                    Err(e) => eprintln!("gearbox-api: bad spawn payload: {e}"),
                }
            })
            .wait()?;
        Ok(Self {
            session,
            inbox,
            _subscriber: subscriber,
        })
    }

    pub fn drain_inbox(&self) -> Vec<SpawnVehicleWire> {
        match self.inbox.lock() {
            Ok(mut q) => q.drain(..).collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn publish_spawned(&self, ev: &SpawnedVehicleWire) {
        let Ok(bytes) = encode(ev) else { return };
        if let Err(e) = self.session.put("gearbox/sim/spawned", bytes).wait() {
            eprintln!("gearbox-api: spawned publish failed: {e}");
        }
    }
}

// ─── Bevy plugin ───────────────────────────────────────────────────

#[cfg(feature = "bevy")]
#[derive(Resource)]
pub struct SpawnApiSession {
    pub broker: Mutex<SpawnBroker>,
}

#[cfg(feature = "bevy")]
pub struct SpawnApiPlugin;

#[cfg(feature = "bevy")]
impl Plugin for SpawnApiPlugin {
    fn build(&self, app: &mut App) {
        match zenoh::open(zenoh::Config::default()).wait() {
            Ok(session) => {
                let session = Arc::new(session);
                match SpawnBroker::open(session) {
                    Ok(broker) => {
                        app.insert_resource(SpawnApiSession {
                            broker: Mutex::new(broker),
                        });
                        app.add_systems(Update, apply_spawn_requests_system);
                        info!("gearbox-api: spawn API ready (gearbox/sim/spawn)");
                    }
                    Err(e) => warn!(
                        "gearbox-api: spawn subscriber open failed ({e}); spawn API disabled"
                    ),
                }
            }
            Err(e) => warn!(
                "gearbox-api: spawn session open failed ({e}); spawn API disabled"
            ),
        }
    }
}

#[cfg(feature = "bevy")]
fn apply_spawn_requests_system(
    mut commands: Commands,
    api: Option<Res<SpawnApiSession>>,
    mut sim: ResMut<GearboxSim>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<bevy::image::Image>>,
    asset_server: Res<bevy::asset::AssetServer>,
) {
    let Some(api) = api else { return };
    let Ok(broker) = api.broker.lock() else { return };
    let requests = broker.drain_inbox();
    for req in requests {
        let Some(spec_factory) = lookup_preset(&req.preset) else {
            warn!(
                "gearbox-api: unknown preset `{}` — known: {}",
                req.preset,
                presets::all_presets()
                    .iter()
                    .map(|p| p.id)
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            continue;
        };
        let spec = spec_factory();
        let y = if req.y == 0.0 {
            spawn_height_for(&spec)
        } else {
            req.y
        };
        // Yaw around +Y → quaternion (cos(θ/2), 0, sin(θ/2), 0).
        let half = (req.yaw_deg.to_radians()) * 0.5;
        let pose = Pose {
            point: Point::new(req.x, y, req.z),
            rotation: Quaternion::new(half.cos(), 0.0, half.sin(), 0.0),
        };
        let id = sim.0.spawn_vehicle(spec.clone(), pose);
        let chassis = spawn_vehicle_visuals(
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut images,
            &asset_server,
            id,
            &spec,
        );
        if req.player {
            commands.entity(chassis).insert(PlayerControlled);
        }
        info!(
            "gearbox-api: spawned `{}` as {}_{} at ({:.2},{:.2},{:.2}) yaw={:.1}°",
            req.preset, spec.name, id.0, req.x, y, req.z, req.yaw_deg
        );
        broker.publish_spawned(&SpawnedVehicleWire {
            id: id.0,
            name: spec.name.clone(),
            preset: req.preset.clone(),
            x: req.x,
            y,
            z: req.z,
            yaw_deg: req.yaw_deg,
        });
    }
}

#[cfg(feature = "bevy")]
fn lookup_preset(id: &str) -> Option<fn() -> gearbox_core::VehicleSpec> {
    presets::all_presets()
        .into_iter()
        .find(|p| p.id == id)
        .map(|p| p.factory)
}
