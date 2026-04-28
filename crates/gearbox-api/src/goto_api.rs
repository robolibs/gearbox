//! **Pluggable** "go to point" API — accept a target pose on
//! `<robot_name>_<instance>/goto`, drive the vehicle there using the
//! [`ondrive`] motion-control library, publish progress on
//! `<robot_name>_<instance>/goto_status`.
//!
//! Same delete-it-later pattern as [`crate::vehicle_api`]:
//!
//!   1. delete this file,
//!   2. drop `pub mod goto_api;` and the `goto_api::*` re-exports
//!      in `lib.rs`,
//!   3. drop `app.add_plugins(GotoApiPlugin)` in `main.rs`,
//!   4. drop the `ondrive` dep from `gearbox-api/Cargo.toml`.
//!
//! ## Topics (per vehicle)
//!
//! | direction | key                      | payload          |
//! |-----------|--------------------------|------------------|
//! | sub       | `<prefix>/goto`          | `GotoCommand`    |
//! | pub       | `<prefix>/goto_status`   | `GotoStatusWire` |
//!
//! ## Coordinates
//!
//! Gearbox is **Y-up**. `GotoCommand.x` and `GotoCommand.z` are the
//! gearbox world coordinates the Inspector shows (X = lateral,
//! Z = longitudinal). `yaw_deg` is the same heading the
//! `<prefix>/odom` topic + Inspector report (0° = facing +Z, 90° =
//! facing +X).
//!
//! Ondrive itself works in a Z-up planar frame, so this module
//! shims gearbox (X, Z) → ondrive (X, Y) and gearbox heading →
//! ondrive yaw before handing off to the controller.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use zenoh::Wait;

use crate::wire::{decode, encode};

#[cfg(feature = "bevy")]
use bevy::prelude::*;

#[cfg(feature = "bevy")]
use gearbox_viz::GearboxSim;

#[cfg(feature = "bevy")]
use datapod::{spatial::Euler, Point, Pose, Quaternion};

#[cfg(feature = "bevy")]
use ondrive::{
    point::CarrotFollower, types::SteeringType, Controller, ControllerConfig, Goal,
    RobotConstraints, RobotState, VelocityCommand,
};

// ─── Wire types ────────────────────────────────────────────────────

/// Send to `<prefix>/goto` to start a navigation. Posting `cancel:
/// true` aborts the active goal without sending a new one.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct GotoCommand {
    /// Target X in gearbox world coordinates (metres).
    pub x: f64,
    /// Target Z in gearbox world coordinates (metres) — gearbox's
    /// "longitudinal" world axis.
    pub z: f64,
    /// Target yaw in degrees, gearbox heading convention (0° = +Z,
    /// 90° = +X).
    pub yaw_deg: f64,
    /// Position tolerance (metres). 0 ⇒ default 0.4 m.
    #[serde(default)]
    pub tolerance: f64,
    /// Yaw tolerance (degrees). 0 ⇒ default 8°.
    #[serde(default)]
    pub yaw_tolerance_deg: f64,
    /// Cap the linear velocity at this speed (m/s). 0 ⇒ vehicle's
    /// `spec.max_speed`.
    #[serde(default)]
    pub max_speed: f64,
    /// If true, drop the active goal without setting a new one.
    #[serde(default)]
    pub cancel: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GotoStatusWire {
    pub active: bool,
    pub reached: bool,
    pub distance_to_goal: f64,
    pub heading_error: f64,
    pub mode: String,
}

// ─── Broker ────────────────────────────────────────────────────────

/// One per vehicle: the active goal + its dedicated `CarrotFollower`
/// instance (so internal PID state isn't shared across vehicles).
#[cfg(feature = "bevy")]
struct ActiveGoal {
    cmd: GotoCommand,
    follower: CarrotFollower,
}

pub struct GotoBroker {
    session: Arc<zenoh::Session>,
    /// Pending `GotoCommand` per vehicle — the apply system hands
    /// these to the controller and clears the slot.
    inbox: Arc<Mutex<HashMap<u32, GotoCommand>>>,
    subscribers: HashMap<u32, zenoh::pubsub::Subscriber<()>>,
}

impl GotoBroker {
    pub fn new(session: Arc<zenoh::Session>) -> Self {
        Self {
            session,
            inbox: Arc::new(Mutex::new(HashMap::new())),
            subscribers: HashMap::new(),
        }
    }

    pub fn register(&mut self, vehicle_id: u32, robot_name: &str) {
        if self.subscribers.contains_key(&vehicle_id) {
            return;
        }
        let key = format!("{}_{}/goto", robot_name, vehicle_id);
        let inbox = Arc::clone(&self.inbox);
        let result = self
            .session
            .declare_subscriber(key)
            .callback(move |sample| {
                let bytes = sample.payload().to_bytes();
                match decode::<GotoCommand>(bytes.as_ref()) {
                    Ok(cmd) => {
                        if let Ok(mut q) = inbox.lock() {
                            q.insert(vehicle_id, cmd);
                        }
                    }
                    Err(e) => {
                        eprintln!("gearbox-api: bad goto for vehicle {vehicle_id}: {e}");
                    }
                }
            })
            .wait();
        if let Ok(sub) = result {
            self.subscribers.insert(vehicle_id, sub);
        }
    }

    pub fn unregister(&mut self, vehicle_id: u32) {
        self.subscribers.remove(&vehicle_id);
        if let Ok(mut q) = self.inbox.lock() {
            q.remove(&vehicle_id);
        }
    }

    /// Drain the inbox of newly-arrived goto commands. (Cancellations
    /// are also drained — the apply system handles them.)
    pub fn drain_inbox(&self) -> HashMap<u32, GotoCommand> {
        match self.inbox.lock() {
            Ok(mut q) => std::mem::take(&mut *q),
            Err(_) => HashMap::new(),
        }
    }

    pub fn publish_status(&self, vehicle_id: u32, robot_name: &str, status: &GotoStatusWire) {
        let Ok(bytes) = encode(status) else { return };
        let key = format!("{}_{}/goto_status", robot_name, vehicle_id);
        if let Err(e) = self.session.put(key, bytes).wait() {
            eprintln!(
                "gearbox-api: goto_status publish failed for {robot_name}_{vehicle_id}: {e}"
            );
        }
    }
}

// ─── Bevy plugin ───────────────────────────────────────────────────

#[cfg(feature = "bevy")]
#[derive(Resource)]
pub struct GotoApiSession {
    pub broker: Mutex<GotoBroker>,
    /// Active follower + goal per vehicle. Cleared when the
    /// controller reports the goal as reached, or the user posts
    /// a `cancel`.
    active: Mutex<HashMap<u32, ActiveGoal>>,
}

#[cfg(feature = "bevy")]
pub struct GotoApiPlugin;

#[cfg(feature = "bevy")]
impl Plugin for GotoApiPlugin {
    fn build(&self, app: &mut App) {
        match zenoh::open(zenoh::Config::default()).wait() {
            Ok(session) => {
                let session = Arc::new(session);
                app.insert_resource(GotoApiSession {
                    broker: Mutex::new(GotoBroker::new(session)),
                    active: Mutex::new(HashMap::new()),
                });
                // Same scheduling as `vehicle_api`: write controls
                // AFTER WASD (so we override its zero-write) and
                // BEFORE the sim step.
                app.add_systems(
                    Update,
                    (sync_goto_topics_system, drive_goto_system)
                        .chain()
                        .after(gearbox_viz::input::wasd_input_system)
                        .before(gearbox_viz::step::step_sim_system),
                );
                // Status pubs after the step + visual goal marker.
                app.add_systems(
                    PostUpdate,
                    (publish_goto_status_system, update_goal_markers_system),
                );
                info!("gearbox-api: goto API ready (goto / goto_status)");
            }
            Err(e) => {
                warn!("gearbox-api: goto broker open failed ({e}); goto API disabled");
            }
        }
    }
}

#[cfg(feature = "bevy")]
fn sync_goto_topics_system(sim: Res<GearboxSim>, api: Option<Res<GotoApiSession>>) {
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
fn drive_goto_system(
    mut sim: ResMut<GearboxSim>,
    api: Option<Res<GotoApiSession>>,
    time: Res<Time>,
) {
    let Some(api) = api else { return };
    let Ok(broker) = api.broker.lock() else { return };
    let Ok(mut active) = api.active.lock() else { return };

    // 1. Ingest new commands. `cancel: true` removes the goal;
    //    everything else replaces / installs one.
    for (vehicle_id, cmd) in broker.drain_inbox() {
        if cmd.cancel {
            active.remove(&vehicle_id);
            continue;
        }
        let mut follower = CarrotFollower::new();
        follower.set_config(controller_config_for(&cmd));
        active.insert(vehicle_id, ActiveGoal { cmd, follower });
    }

    // 2. For every active goal: read the vehicle's current state,
    //    run the ondrive controller, write the result as a
    //    ControlInput.
    let dt = time.delta_secs_f64().max(1e-3);
    let mut completed: Vec<u32> = Vec::new();

    for (vehicle_id, goal_state) in active.iter_mut() {
        let id = gearbox_physics::VehicleId(*vehicle_id);
        let Some(spec_state) = sim.0.vehicle(id) else {
            completed.push(*vehicle_id);
            continue;
        };
        let constraints = constraints_for(spec_state, &goal_state.cmd);
        let robot_state = robot_state_from_sim(&sim.0, id);
        let goal = goal_from_cmd(&goal_state.cmd);

        let cmd = goal_state
            .follower
            .compute_control(&robot_state, &goal, &constraints, dt, None);
        let ctrl = velocity_to_control(cmd.clone(), &constraints);
        sim.0.set_control(id, ctrl);

        if goal_state.follower.get_status().goal_reached {
            // Hand off a stop the next frame and drop the goal.
            sim.0.set_control(id, gearbox_physics::ControlInput::default());
            completed.push(*vehicle_id);
        }
    }

    for id in completed {
        active.remove(&id);
    }
}

#[cfg(feature = "bevy")]
fn publish_goto_status_system(
    sim: Res<GearboxSim>,
    api: Option<Res<GotoApiSession>>,
) {
    let Some(api) = api else { return };
    let Ok(broker) = api.broker.lock() else { return };
    let Ok(active) = api.active.lock() else { return };

    for (id, state) in sim.0.vehicles() {
        let name = &state.spec.name;
        let status = if let Some(goal) = active.get(&id.0) {
            let s = goal.follower.get_status();
            GotoStatusWire {
                active: true,
                reached: s.goal_reached,
                distance_to_goal: s.distance_to_goal,
                heading_error: s.heading_error,
                mode: s.mode,
            }
        } else {
            GotoStatusWire::default()
        };
        broker.publish_status(id.0, name, &status);
    }
}

// ─── Per-vehicle config helpers ────────────────────────────────────

#[cfg(feature = "bevy")]
fn controller_config_for(cmd: &GotoCommand) -> ControllerConfig {
    let tol = if cmd.tolerance > 0.0 { cmd.tolerance } else { 1.5 };
    // Default yaw tolerance is `2π` — i.e. don't require any
    // specific final orientation, just position. Without this an
    // Ackermann tractor that "hits" the point at the wrong heading
    // would keep wandering trying to align. Caller can opt in with
    // an explicit `yaw_tolerance_deg` if they care about final
    // heading.
    let yaw_tol = if cmd.yaw_tolerance_deg > 0.0 {
        cmd.yaw_tolerance_deg.to_radians()
    } else {
        std::f64::consts::TAU
    };
    // Lower gains than the ondrive defaults — the tractor is heavy
    // (~3.8 t) and Ackermann, so high kp_linear sends it past the
    // goal before it can turn. kp_angular kept moderate too — large
    // values cause the wheels to chatter when |heading_err| ≈ π
    // (the natural dead-zone of point-tracking controllers).
    ControllerConfig {
        kp_linear: 0.5,
        kp_angular: 1.2,
        goal_tolerance: tol,
        angular_tolerance: yaw_tol,
        ..Default::default()
    }
}

#[cfg(feature = "bevy")]
fn constraints_for(
    state: &gearbox_physics::vehicle::VehicleState,
    cmd: &GotoCommand,
) -> RobotConstraints {
    // Goto navigation drives at ~half the vehicle's racing-top
    // speed by default. Heavy ground vehicles overshoot point
    // targets at full speed; the user can opt in to faster runs
    // with `cmd.max_speed`.
    let max_speed = if cmd.max_speed > 0.0 {
        cmd.max_speed
    } else {
        (state.spec.max_speed * 0.5).clamp(0.5, 3.0)
    };
    let max_steer = state
        .spec
        .wheels
        .iter()
        .map(|w| w.max_steer_rad)
        .fold(0.0_f64, f64::max);
    let wheelbase = wheelbase_of(state);
    let steering_type = match state.spec.drive_mode {
        gearbox_physics::DriveMode::Differential => SteeringType::Differential,
        gearbox_physics::DriveMode::Omni => SteeringType::Holonomic,
        // Drone falls through to Ackermann — the controller's planar
        // control still produces sensible (vx, yaw) commands.
        _ => SteeringType::Ackermann,
    };
    RobotConstraints {
        steering_type,
        wheelbase: wheelbase.max(0.5),
        max_linear_velocity: max_speed.max(0.5),
        min_linear_velocity: -max_speed.max(0.5),
        max_angular_velocity: 1.5,
        max_steering_angle: max_steer.max(0.3),
        ..RobotConstraints::default()
    }
}

#[cfg(feature = "bevy")]
fn wheelbase_of(state: &gearbox_physics::vehicle::VehicleState) -> f64 {
    let mut z_min = f64::INFINITY;
    let mut z_max = f64::NEG_INFINITY;
    for w in &state.spec.wheels {
        z_min = z_min.min(w.chassis_connection.z);
        z_max = z_max.max(w.chassis_connection.z);
    }
    if z_max <= z_min {
        1.0
    } else {
        z_max - z_min
    }
}

// ─── Coord shim — gearbox (Y-up) ↔ ondrive (Z-up planar) ──────────

#[cfg(feature = "bevy")]
fn robot_state_from_sim(sim: &gearbox_physics::Sim, id: gearbox_physics::VehicleId) -> RobotState {
    let pose = sim.vehicle_pose(id);
    let lv = sim.vehicle_linvel(id);
    let av = sim.vehicle_angvel(id);
    // Project gearbox world (X-up, Y-up, Z-forward) onto a planar
    // navigation frame:  ondrive_x = gearbox_z, ondrive_y = gearbox_x.
    // Heading stays the same scalar — gearbox's `vehicle_heading`
    // already measures CCW yaw in the (X, Z) plane.
    let heading_rad = sim.vehicle_heading(id).to_radians();
    let yaw_quat = Quaternion::from_euler(Euler::new(0.0, 0.0, heading_rad));
    RobotState {
        pose: Pose {
            point: Point::new(pose.point.z, pose.point.x, 0.0),
            rotation: yaw_quat,
        },
        velocity: ondrive::Velocity {
            linear: (lv.vx * lv.vx + lv.vz * lv.vz).sqrt(),
            angular: av.vy,
            lateral: 0.0,
        },
        timestamp: 0.0,
        allow_reverse: true,
        turn_first: false,
        allow_move: true,
        has_trailer: false,
        trailer_pose: Pose::default(),
    }
}

#[cfg(feature = "bevy")]
fn goal_from_cmd(cmd: &GotoCommand) -> Goal {
    let yaw_rad = cmd.yaw_deg.to_radians();
    Goal {
        target_pose: Pose {
            // Same projection — user gave us gearbox (X, Z), shim to
            // ondrive (Z-as-X, X-as-Y).
            point: Point::new(cmd.z, cmd.x, 0.0),
            rotation: Quaternion::from_euler(Euler::new(0.0, 0.0, yaw_rad)),
        },
        target_velocity: None,
        tolerance_position: if cmd.tolerance > 0.0 { cmd.tolerance } else { 0.4 },
        tolerance_orientation: if cmd.yaw_tolerance_deg > 0.0 {
            cmd.yaw_tolerance_deg.to_radians()
        } else {
            8.0_f64.to_radians()
        },
    }
}

// ─── Visual goal marker ────────────────────────────────────────────

#[cfg(feature = "bevy")]
#[derive(Component)]
struct GoalMarker {
    vehicle_id: u32,
}

/// Spawn / move / despawn a small red marker at every active
/// goal so the user can see where the controller is heading. The
/// marker's `y` is hardcoded to lift it off the ground a bit; we
/// don't have terrain queries here so it just hovers above the
/// flat tangent plane.
#[cfg(feature = "bevy")]
fn update_goal_markers_system(
    mut commands: Commands,
    api: Option<Res<GotoApiSession>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut existing: Query<(Entity, &GoalMarker, &mut Transform)>,
) {
    let Some(api) = api else { return };
    let Ok(active) = api.active.lock() else { return };

    let mut needs_marker: std::collections::HashSet<u32> = active.keys().copied().collect();

    for (entity, marker, mut tr) in &mut existing {
        if let Some(goal) = active.get(&marker.vehicle_id) {
            tr.translation = bevy::math::Vec3::new(
                goal.cmd.x as f32,
                0.8,
                goal.cmd.z as f32,
            );
            needs_marker.remove(&marker.vehicle_id);
        } else {
            commands.entity(entity).despawn();
        }
    }

    for vehicle_id in needs_marker {
        let Some(goal) = active.get(&vehicle_id) else { continue };
        let mesh = meshes.add(bevy::prelude::Cuboid::new(0.8, 1.6, 0.8));
        let mat = materials.add(bevy::prelude::StandardMaterial {
            base_color: bevy::prelude::Color::srgb(1.0, 0.12, 0.12),
            emissive: bevy::color::LinearRgba::rgb(0.5, 0.05, 0.05),
            unlit: false,
            perceptual_roughness: 0.6,
            ..bevy::prelude::default()
        });
        commands.spawn((
            Name::new(format!("GotoMarker[{}]", vehicle_id)),
            GoalMarker { vehicle_id },
            bevy::prelude::Transform::from_xyz(
                goal.cmd.x as f32,
                0.8,
                goal.cmd.z as f32,
            ),
            bevy::prelude::Mesh3d(mesh),
            bevy::prelude::MeshMaterial3d(mat),
        ));
    }
}

/// Map ondrive's `VelocityCommand` into the `ControlInput` shape
/// gearbox's drive controllers consume. Same conventions as
/// `vehicle_api::twist_to_control` so the two driver paths stay
/// interchangeable.
#[cfg(feature = "bevy")]
fn velocity_to_control(
    cmd: VelocityCommand,
    constraints: &RobotConstraints,
) -> gearbox_physics::ControlInput {
    if !cmd.valid {
        return gearbox_physics::ControlInput::default();
    }
    let max_lin = constraints.max_linear_velocity.max(0.5);
    let max_ang = constraints.max_angular_velocity.max(0.5);
    let throttle = (cmd.linear_velocity / max_lin).clamp(-1.0, 1.0);
    let yaw_rate_norm = (cmd.angular_velocity / max_ang).clamp(-1.0, 1.0);
    // Sign convention: ondrive's `angular_velocity` is the body-
    // frame yaw rate around +Z, positive = CCW = turn LEFT.
    // Gearbox `ControlInput::steer` is also "+ = turn left" (see
    // `wasd_input_system` — A maps to +steer). They line up
    // directly — no negation. Earlier code negated and the tractor
    // steered the wrong way every time.
    gearbox_physics::ControlInput {
        throttle,
        brake: 0.0,
        steer: yaw_rate_norm,
        yaw: yaw_rate_norm,
        lift: 0.0,
    }
}
