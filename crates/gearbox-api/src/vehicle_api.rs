//! **Pluggable** per-vehicle zenoh topics.
//!
//! Self-contained on purpose — the user has flagged this whole
//! integration as "delete-and-substitute-later". To rip it out:
//!
//!   1. delete this file,
//!   2. drop the `pub mod vehicle_api;` + re-exports in `lib.rs`,
//!   3. drop the `app.add_plugins(VehicleApiPlugin)` line in
//!      whichever Bevy app installs the API.
//!
//! Nothing else in the crate (or in `gearbox-physics` /
//! `gearbox-viz`) references this module — the rest of the system
//! talks to `Sim` directly.
//!
//! ## Topics (per vehicle)
//!
//! Topic prefix: `<robot_name>_<instance>` where `instance` is the
//! `VehicleId.0` allocated by `Sim::spawn_vehicle`.
//!
//! | direction | key                                 | payload      |
//! |-----------|-------------------------------------|--------------|
//! | sub       | `<prefix>/cmd_vel`                  | `TwistWire`  |
//! | pub       | `<prefix>/odom`                     | `OdomWire`   |
//! | pub       | `<prefix>/fix`                      | `FixWire`    |
//!
//! Every payload is CBOR-encoded so the wire format is the same
//! shape as the rest of the gearbox-api surface.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use zenoh::Wait;

use crate::wire::{decode, encode};

#[cfg(feature = "bevy")]
use bevy::prelude::*;

#[cfg(feature = "bevy")]
use gearbox_viz::GearboxSim;

// ─── Wire types ────────────────────────────────────────────────────

/// Standard ROS-style `Twist` — linear + angular velocity, in the
/// vehicle's body frame. Units are SI (m/s and rad/s).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct TwistWire {
    pub linear: [f64; 3],
    pub angular: [f64; 3],
}

/// Odometry — current pose + body-frame twist of the vehicle's
/// chassis. Mirrors the geometry_msgs/Odometry layout: position +
/// orientation in world frame, twist in body frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OdomWire {
    /// World-frame position of the chassis (metres, gearbox local
    /// tangent-plane coordinates).
    pub position: [f64; 3],
    /// World-frame orientation of the chassis as `(x, y, z, w)`
    /// (matching nalgebra / glam convention).
    pub orientation: [f64; 4],
    /// World-frame linear velocity of the chassis CoM (m/s).
    pub linear_velocity: [f64; 3],
    /// World-frame angular velocity of the chassis (rad/s).
    pub angular_velocity: [f64; 3],
}

/// Geographic fix — lat/lon/alt computed via the sim's `Planet`
/// datum so external robots see the same coordinates the editor's
/// Inspector shows.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FixWire {
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: f64,
}

// ─── Broker ────────────────────────────────────────────────────────

/// Per-vehicle topic broker. Owns one zenoh `Session` (shared with
/// the parent [`crate::ApiBroker`]) and one subscriber per vehicle.
///
/// Publish-side topics are declared lazily via `session.put(...)`
/// each frame — cached publishers would require `'static` keys, but
/// our keys depend on runtime vehicle names + ids.
pub struct VehicleBroker {
    session: Arc<zenoh::Session>,
    /// Last `cmd_vel` received per vehicle. Latched (not drained) so
    /// the apply system can re-emit the command every frame —
    /// otherwise `wasd_input_system` writes a zero on every Bevy
    /// frame and clobbers our 10 Hz Python feed in between.
    last_cmd_vel: Arc<Mutex<HashMap<u32, TwistWire>>>,
    subscribers: HashMap<u32, zenoh::pubsub::Subscriber<()>>,
}

impl VehicleBroker {
    pub fn new(session: Arc<zenoh::Session>) -> Self {
        Self {
            session,
            last_cmd_vel: Arc::new(Mutex::new(HashMap::new())),
            subscribers: HashMap::new(),
        }
    }

    /// Declare the `cmd_vel` subscriber for a freshly-spawned
    /// vehicle. Idempotent — re-registering does nothing.
    pub fn register(&mut self, vehicle_id: u32, robot_name: &str) {
        if self.subscribers.contains_key(&vehicle_id) {
            return;
        }
        let key = format!("{}_{}/cmd_vel", robot_name, vehicle_id);
        let pending = Arc::clone(&self.last_cmd_vel);
        let result = self
            .session
            .declare_subscriber(key)
            .callback(move |sample| {
                let bytes = sample.payload().to_bytes();
                match decode::<TwistWire>(bytes.as_ref()) {
                    Ok(twist) => {
                        if let Ok(mut q) = pending.lock() {
                            q.insert(vehicle_id, twist);
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "gearbox-api: bad cmd_vel for vehicle {}: {}",
                            vehicle_id, e
                        );
                    }
                }
            })
            .wait();
        match result {
            Ok(sub) => {
                self.subscribers.insert(vehicle_id, sub);
            }
            Err(e) => {
                eprintln!(
                    "gearbox-api: cmd_vel subscribe failed for {}_{}: {}",
                    robot_name, vehicle_id, e
                );
            }
        }
    }

    /// Drop the subscriber for a despawned vehicle.
    pub fn unregister(&mut self, vehicle_id: u32) {
        self.subscribers.remove(&vehicle_id);
        if let Ok(mut q) = self.last_cmd_vel.lock() {
            q.remove(&vehicle_id);
        }
    }

    /// Snapshot the current latched `cmd_vel` for every vehicle.
    /// Caller re-applies every frame so the value persists between
    /// publishes from a slow client (e.g. Python at 10 Hz).
    pub fn snapshot_cmd_vel(&self) -> HashMap<u32, TwistWire> {
        match self.last_cmd_vel.lock() {
            Ok(q) => q.clone(),
            Err(_) => HashMap::new(),
        }
    }

    pub fn publish_odom(&self, vehicle_id: u32, robot_name: &str, odom: &OdomWire) {
        let Ok(bytes) = encode(odom) else { return };
        let key = format!("{}_{}/odom", robot_name, vehicle_id);
        if let Err(e) = self.session.put(key, bytes).wait() {
            eprintln!("gearbox-api: odom publish failed for {robot_name}_{vehicle_id}: {e}");
        }
    }

    pub fn publish_fix(&self, vehicle_id: u32, robot_name: &str, fix: &FixWire) {
        let Ok(bytes) = encode(fix) else { return };
        let key = format!("{}_{}/fix", robot_name, vehicle_id);
        if let Err(e) = self.session.put(key, bytes).wait() {
            eprintln!("gearbox-api: fix publish failed for {robot_name}_{vehicle_id}: {e}");
        }
    }
}

// ─── Bevy plugin ───────────────────────────────────────────────────

#[cfg(feature = "bevy")]
#[derive(Resource)]
pub struct VehicleApiSession {
    pub broker: Mutex<VehicleBroker>,
}

#[cfg(feature = "bevy")]
pub struct VehicleApiPlugin;

#[cfg(feature = "bevy")]
impl Plugin for VehicleApiPlugin {
    fn build(&self, app: &mut App) {
        // Open an independent zenoh session — the vehicle API stays
        // self-contained so removing this plugin is one line at the
        // app level and doesn't touch the parent `GearboxApiPlugin`.
        match zenoh::open(zenoh::Config::default()).wait() {
            Ok(session) => {
                let session = Arc::new(session);
                app.insert_resource(VehicleApiSession {
                    broker: Mutex::new(VehicleBroker::new(session)),
                });
                // Critical scheduling: `cmd_vel` writes have to land
                // AFTER `wasd_input_system` (which writes zero to
                // every PlayerControlled vehicle each frame, even
                // with no keys held) and BEFORE `step_sim_system`
                // (which consumes the control). Otherwise the WASD
                // zero-write clobbers our network command before the
                // physics step sees it, and the vehicle never moves.
                app.add_systems(
                    Update,
                    (
                        sync_vehicle_topics_system,
                        apply_cmd_vel_system,
                    )
                        .chain()
                        .after(gearbox_viz::input::wasd_input_system)
                        .before(gearbox_viz::step::step_sim_system),
                );
                // Telemetry pubs run later — after the step so we
                // publish post-physics poses, not pre-physics.
                app.add_systems(PostUpdate, publish_odom_fix_system);
                info!("gearbox-api: vehicle API ready (cmd_vel / odom / fix)");
            }
            Err(e) => {
                warn!("gearbox-api: vehicle broker open failed ({e}); vehicle API disabled");
            }
        }
    }
}

/// Discover newly-spawned / despawned vehicles each frame and keep
/// the per-vehicle subscriber set in sync.
#[cfg(feature = "bevy")]
fn sync_vehicle_topics_system(
    sim: Res<GearboxSim>,
    api: Option<Res<VehicleApiSession>>,
) {
    let Some(api) = api else { return };
    let Ok(mut broker) = api.broker.lock() else { return };

    let mut alive: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for (id, state) in sim.0.vehicles() {
        alive.insert(id.0);
        broker.register(id.0, &state.spec.name);
    }
    let to_drop: Vec<u32> = broker
        .subscribers
        .keys()
        .copied()
        .filter(|id| !alive.contains(id))
        .collect();
    for id in to_drop {
        broker.unregister(id);
    }
}

#[cfg(feature = "bevy")]
fn apply_cmd_vel_system(
    mut sim: ResMut<GearboxSim>,
    api: Option<Res<VehicleApiSession>>,
) {
    let Some(api) = api else { return };
    let Ok(broker) = api.broker.lock() else { return };
    let cmds = broker.snapshot_cmd_vel();
    for (vehicle_id, twist) in cmds {
        let id = gearbox_physics::VehicleId(vehicle_id);
        let ctrl = twist_to_control(twist);
        sim.0.set_control(id, ctrl);
    }
}

#[cfg(feature = "bevy")]
fn publish_odom_fix_system(
    sim: Res<GearboxSim>,
    api: Option<Res<VehicleApiSession>>,
) {
    let Some(api) = api else { return };
    let Ok(broker) = api.broker.lock() else { return };
    for (id, state) in sim.0.vehicles() {
        let name = &state.spec.name;
        let pose = sim.0.vehicle_pose(id);
        let lv = sim.0.vehicle_linvel(id);
        let av = sim.0.vehicle_angvel(id);
        let geo = sim.0.vehicle_geo(id);

        broker.publish_odom(
            id.0,
            name,
            &OdomWire {
                position: [pose.point.x, pose.point.y, pose.point.z],
                orientation: [
                    pose.rotation.x,
                    pose.rotation.y,
                    pose.rotation.z,
                    pose.rotation.w,
                ],
                linear_velocity: [lv.vx, lv.vy, lv.vz],
                angular_velocity: [av.vx, av.vy, av.vz],
            },
        );
        broker.publish_fix(
            id.0,
            name,
            &FixWire {
                latitude: geo.latitude,
                longitude: geo.longitude,
                altitude: geo.altitude,
            },
        );
    }
}

/// Map a body-frame `Twist` into the `ControlInput` shape gearbox's
/// drive controllers consume. Pluggable: pick a different mapping
/// here if you want lin.x to be a normalised throttle vs an absolute
/// speed setpoint.
#[cfg(feature = "bevy")]
fn twist_to_control(twist: TwistWire) -> gearbox_physics::ControlInput {
    // `linear.x` ∈ assumed m/s. Normalise against a 5 m/s reference
    // — clients that need precise speed control should target
    // throttle directly via a dedicated topic later.
    let throttle = (twist.linear[0] / 5.0).clamp(-1.0, 1.0);
    // `linear.y` doubles as drone altitude — gearbox's ControlInput
    // has a `lift` slot for exactly this.
    let lift = twist.linear[1].clamp(-1.0, 1.0);
    // `angular.z` (yaw rate, +Z = CCW = turn LEFT) maps directly
    // to gearbox `+steer = turn left` (`wasd_input_system`'s
    // A-key comment) — no sign flip.
    let steer = twist.angular[2].clamp(-1.0, 1.0);
    let yaw = twist.angular[2].clamp(-1.0, 1.0);

    gearbox_physics::ControlInput {
        throttle,
        brake: 0.0,
        steer,
        yaw,
        lift,
    }
}
