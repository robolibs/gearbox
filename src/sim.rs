//! The core sim — owns the rapier physics world and all vehicles.
//!
//! The public API speaks [`datapod`] spatial types (f64). The rapier
//! glam/f32 layer is an implementation detail, bridged through
//! [`crate::convert`].

use std::collections::HashMap;

use datapod::{Geo, Pose, Size, Velocity};
use rapier3d::control::{DynamicRayCastVehicleController, WheelTuning};
use rapier3d::prelude::*;

use crate::control::ControlInput;
use crate::convert::{
    point_to_vec3, pose_to_rpose, rpose_to_pose, size_to_half_extents, size_to_vec3,
    vec3_to_velocity,
};
use crate::planet::Planet;
use crate::vehicle::{DriveMode, PartKind, VehicleId, VehicleSpec, VehicleState, WheelSpec};
use crate::world;

pub struct Sim {
    // Rapier state.
    pub bodies: RigidBodySet,
    pub colliders: ColliderSet,
    pub impulse_joints: ImpulseJointSet,
    pub multibody_joints: MultibodyJointSet,
    pub islands: IslandManager,
    pub broad_phase: BroadPhaseBvh,
    pub narrow_phase: NarrowPhase,
    pub ccd_solver: CCDSolver,
    pub pipeline: PhysicsPipeline,
    pub integration: IntegrationParameters,
    pub gravity: Vector,

    /// The "planet" this sim lives on — used for lat/lon readouts.
    pub planet: Planet,

    // Sim-owned.
    vehicles: HashMap<VehicleId, VehicleState>,
    next_id: u32,
}

impl Default for Sim {
    fn default() -> Self {
        Self::new()
    }
}

impl Sim {
    pub fn new() -> Self {
        Self {
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            islands: IslandManager::new(),
            broad_phase: BroadPhaseBvh::new(),
            narrow_phase: NarrowPhase::new(),
            ccd_solver: CCDSolver::new(),
            pipeline: PhysicsPipeline::new(),
            integration: IntegrationParameters::default(),
            gravity: Vector::new(0.0, -9.81, 0.0),
            planet: Planet::default(),
            vehicles: HashMap::new(),
            next_id: 0,
        }
    }

    /// Add a large flat ground collider centered at the origin.
    pub fn add_ground_plane(&mut self, half_size: f32) {
        self.colliders.insert(world::build_ground_collider(half_size));
    }

    /// Add a sphere collider representing the planet. The sphere is placed
    /// so its tangent (top) touches `y = 0`, matching the coordinate
    /// convention used by the visual planet mesh.
    pub fn add_planet_collider(&mut self, radius: f32) {
        let centre = Vec3::new(0.0, -radius, 0.0);
        self.colliders.insert(world::build_planet_collider(centre, radius));
    }

    /// Add a static box obstacle at the given pose.
    pub fn add_box_obstacle(&mut self, pose: Pose, size: Size) {
        self.colliders
            .insert(world::build_box_collider(size_to_vec3(size), pose_to_rpose(pose)));
    }

    /// Spawn a vehicle from its declarative spec at the given pose.
    pub fn spawn_vehicle(&mut self, spec: VehicleSpec, pose: Pose) -> VehicleId {
        let chassis = &spec.chassis;
        let half_extents = size_to_half_extents(chassis.size);

        let body = RigidBodyBuilder::dynamic()
            .pose(pose_to_rpose(pose))
            .linear_damping(chassis.linear_damping)
            .angular_damping(chassis.angular_damping)
            .ccd_enabled(chassis.ccd)
            // Rapier's vehicle controller only wakes the chassis on a
            // *positive* engine_force — which means once the tractor
            // sleeps, pressing S (negative force = reverse) would be
            // silently ignored. Keep vehicles perma-awake.
            .can_sleep(false)
            .build();
        let body_handle = self.bodies.insert(body);

        // Drive mass + COM offset + inertia from the collider's
        // MassProperties — `insert_with_parent` recomputes the body's
        // mprops from its colliders, so setting them here is what actually
        // sticks. (Using `RigidBodyBuilder::additional_mass_properties` has
        // been unreliable in practice — the collider's default density
        // keeps winning.)
        let mass_props = MassProperties::new(
            point_to_vec3(chassis.com_offset),
            chassis.mass,
            principal_inertia(chassis.mass, half_extents),
        );
        let collider = ColliderBuilder::cuboid(half_extents.x, half_extents.y, half_extents.z)
            .mass_properties(mass_props)
            .friction(0.5)
            .restitution(0.0)
            .collision_groups(world::chassis_groups())
            .build();
        self.colliders
            .insert_with_parent(collider, body_handle, &mut self.bodies);

        // Per-wheel cylinder colliders so wheel-to-wheel contact with
        // OTHER vehicles works. They live in the `WHEEL` group and
        // only interact with other `WHEEL` colliders — so they never
        // push against ground (raycast suspension handles that) and
        // the vehicle's own wheel raycasts filter them out via
        // `wheel_raycast_groups` (see the wheel update below).
        //
        // Position them at the suspension's rest-length midpoint —
        // they're rigid with the chassis body (don't bob with
        // suspension compression), which is fine for inter-vehicle
        // bumps. Mass is 0 so they don't perturb the chassis
        // MassProperties we set explicitly above.
        for w in &spec.wheels {
            let wheel_y = w.chassis_connection.y - (w.suspension_rest_length as f64) * 0.5;
            let wheel_pos = point_to_vec3(datapod::Point::new(
                w.chassis_connection.x,
                wheel_y,
                w.chassis_connection.z,
            ));
            // Cylinder default axis is +Y; rotate −π/2 around +Z so
            // the axle lies along +X.
            let axle_rot = Vec3::new(0.0, 0.0, -std::f32::consts::FRAC_PI_2);
            let wheel_collider = ColliderBuilder::cylinder(w.width * 0.5, w.radius)
                .translation(wheel_pos)
                .rotation(axle_rot)
                .mass(0.0)
                .friction(0.8)
                .restitution(0.0)
                .collision_groups(world::wheel_groups())
                .build();
            self.colliders
                .insert_with_parent(wheel_collider, body_handle, &mut self.bodies);
        }

        // Body-part colliders — karosserie (cab, hood, roof, bunker...)
        // and tanks are solid bodywork and should stop other things.
        // Hitches are visual markers, so skip them.
        //
        // All parts attach to the same rigid body as the chassis, so
        // rapier automatically skips same-body self-collision — a hood
        // sitting above the chassis won't push against it. They share
        // the CHASSIS collision group, so they collide with ground,
        // with other vehicles' chassis+parts, and with other vehicles'
        // wheels (as inter-vehicle bumpers). mass=0 keeps them from
        // perturbing the explicit chassis MassProperties above.
        for part in &spec.parts {
            if matches!(part.kind, PartKind::Hitch) {
                continue;
            }
            let hx = (part.size.x * 0.5) as f32;
            let hy = (part.size.y * 0.5) as f32;
            let hz = (part.size.z * 0.5) as f32;
            let part_collider = ColliderBuilder::cuboid(hx, hy, hz)
                .translation(point_to_vec3(part.position))
                .mass(0.0)
                .friction(0.5)
                .restitution(0.0)
                .collision_groups(world::chassis_groups())
                .build();
            self.colliders
                .insert_with_parent(part_collider, body_handle, &mut self.bodies);
        }

        let mut controller = DynamicRayCastVehicleController::new(body_handle);
        controller.index_up_axis = 1;
        controller.index_forward_axis = 2;

        for w in &spec.wheels {
            let tuning = WheelTuning {
                suspension_stiffness: w.suspension_stiffness,
                suspension_damping: w.suspension_damping,
                friction_slip: w.friction_slip,
                max_suspension_force: w.max_suspension_force,
                ..WheelTuning::default()
            };
            controller.add_wheel(
                point_to_vec3(w.chassis_connection),
                point_to_vec3(w.suspension_dir).normalize(),
                point_to_vec3(w.axle_dir).normalize(),
                w.suspension_rest_length,
                w.radius,
                &tuning,
            );
        }

        let id = VehicleId(self.next_id);
        self.next_id += 1;
        self.vehicles.insert(
            id,
            VehicleState {
                spec,
                body: body_handle,
                controller,
                control: ControlInput::default(),
            },
        );
        id
    }

    pub fn set_control(&mut self, id: VehicleId, ctrl: ControlInput) {
        if let Some(v) = self.vehicles.get_mut(&id) {
            v.control = ctrl.clamp();
        }
    }

    /// Teleport a vehicle to the given pose, zeroing its velocities.
    /// Used by the editor's drag-to-move gesture.
    pub fn set_vehicle_pose(&mut self, id: VehicleId, pose: Pose) {
        let Some(v) = self.vehicles.get(&id) else { return };
        let handle = v.body;
        if let Some(rb) = self.bodies.get_mut(handle) {
            rb.set_position(pose_to_rpose(pose), true);
            rb.set_linvel(Vec3::ZERO, true);
            rb.set_angvel(Vec3::ZERO, true);
        }
    }

    pub fn control(&self, id: VehicleId) -> ControlInput {
        self.vehicles.get(&id).map(|v| v.control).unwrap_or_default()
    }

    pub fn vehicle_pose(&self, id: VehicleId) -> Pose {
        self.vehicles
            .get(&id)
            .and_then(|v| self.bodies.get(v.body))
            .map(|rb| rpose_to_pose(*rb.position()))
            .unwrap_or_default()
    }

    /// Geographic position of the vehicle, using the sim's `Planet` datum.
    pub fn vehicle_geo(&self, id: VehicleId) -> Geo {
        self.planet.local_to_geo(self.vehicle_pose(id).point)
    }

    /// True-north heading of the vehicle, degrees in `[0, 360)` —
    /// 0 = facing north, 90 = east, 180 = south, 270 = west.
    ///
    /// Works by projecting the vehicle's local +Z (forward) onto the
    /// ENU tangent plane at its position on the planet. Uses an ENU
    /// basis computed from the datum + local ENU projection, so it's
    /// valid everywhere except exactly at the poles (where longitude
    /// is undefined).
    pub fn vehicle_heading(&self, id: VehicleId) -> f64 {
        let pose = self.vehicle_pose(id);

        // Vehicle forward vector in world frame (rotate local +Z).
        let (mut fx, mut fy, mut fz) = (0.0_f64, 0.0_f64, 1.0_f64);
        pose.rotation.rotate_vector(&mut fx, &mut fy, &mut fz);
        let _ = fy;

        // The library uses the same ENU convention as
        // `Planet::local_to_geo`: +X = east, +Y = up, +Z = north near
        // the datum. Over a few-hundred-km tangent patch those
        // basis vectors don't meaningfully bend, so "heading" reduces
        // to atan2 of the horizontal components.
        let rad = fx.atan2(fz);
        let deg = rad.to_degrees();
        if deg < 0.0 { deg + 360.0 } else { deg }
    }

    pub fn vehicle_linvel(&self, id: VehicleId) -> Velocity {
        self.vehicles
            .get(&id)
            .and_then(|v| self.bodies.get(v.body))
            .map(|rb| vec3_to_velocity(rb.linvel()))
            .unwrap_or_default()
    }

    /// World-space pose of a wheel, oriented so the wheel's local Y axis
    /// aligns with the axle (matches Bevy's `Cylinder` primitive, which
    /// extrudes along +Y).
    pub fn wheel_pose(&self, id: VehicleId, wheel: usize) -> Pose {
        let Some(v) = self.vehicles.get(&id) else { return Pose::default() };
        let Some(wh) = v.controller.wheels().get(wheel) else { return Pose::default() };

        let axle = normalize_or(wh.axle(), Vec3::X);
        let down = normalize_or(wh.suspension(), Vec3::NEG_Y);

        // When the wheel is airborne, rapier leaves `suspension_length` at
        // its default (0), which makes `wheel.center()` return the chassis
        // attach point — wheels appear embedded in the chassis. Compute the
        // fully-extended (rest-length) position ourselves in that case.
        let center = if wh.raycast_info().is_in_contact {
            wh.center()
        } else {
            wh.raycast_info().hard_point_ws + down * wh.suspension_rest_length
        };

        // "Up-in-wheel" direction: world up, orthogonalized against the axle
        // so the basis stays orthonormal even when wheel has camber.
        let up_raw = -down - axle * (-down).dot(axle);
        let up = if up_raw.length_squared() > 1e-8 {
            up_raw.normalize()
        } else {
            Vec3::Y
        };
        // Right-handed basis: X = Y × Z = axle × up.
        let forward = axle.cross(up);

        // Columns = images of local (+X, +Y, +Z). Determinant = +1.
        let basis = Mat3::from_cols(forward, axle, up);
        let basis_rot = Rot3::from_mat3(&basis);
        // Rapier's `wheel.rotation` increases when moving forward.
        // With our axle_dir convention (`-X`), applying that rotation
        // directly around `axle` spins the cylinder the wrong way
        // around — negate so the tread visually rolls with motion.
        let spin = Rot3::from_axis_angle(axle, -wh.rotation);

        rpose_to_pose(Pose3::from_parts(center, spin * basis_rot))
    }

    pub fn vehicles(&self) -> impl Iterator<Item = (VehicleId, &VehicleState)> {
        self.vehicles.iter().map(|(id, v)| (*id, v))
    }

    pub fn vehicle(&self, id: VehicleId) -> Option<&VehicleState> {
        self.vehicles.get(&id)
    }

    /// Re-run each vehicle's wheel raycasts against the current world
    /// without advancing physics. Useful when the editor edits a
    /// chassis pose while the sim clock is paused — without this the
    /// wheel positions would lag behind the dragged body and only
    /// snap into place when the user pressed Play.
    pub fn refresh_kinematics(&mut self) {
        let Sim {
            vehicles,
            bodies,
            colliders,
            broad_phase,
            narrow_phase,
            ..
        } = self;
        for v in vehicles.values_mut() {
            let filter = QueryFilter::default()
                .exclude_rigid_body(v.body)
                .groups(world::wheel_raycast_groups());
            let qpm = broad_phase.as_query_pipeline_mut(
                narrow_phase.query_dispatcher(),
                bodies,
                colliders,
                filter,
            );
            v.controller.update_vehicle(0.0, qpm);
        }
    }

    /// Advance the simulation by `dt` seconds.
    pub fn step(&mut self, dt: f32) {
        self.integration.dt = dt;

        // 1. Translate ControlInput → per-wheel engine/brake/steering
        //    for ground vehicles, or direct body forces/torques for
        //    drones.
        for v in self.vehicles.values_mut() {
            match v.spec.drive_mode {
                DriveMode::Drone => apply_drone_controls(v, &mut self.bodies, self.gravity),
                _ => apply_controls(v, &self.bodies),
            }
        }

        // 1b. Parking brake. Rapier's `wheel.brake` is applied as
        // `engine_force = -brake * copysign(forward_impulse)` — the
        // sign term flips every step once forward_impulse is near zero,
        // which shows up in the viewport as the whole vehicle
        // shimmying. Sidestep the whole mess: when the driver is
        // pressing brake AND the vehicle is already slow, zero out
        // the horizontal linvel + the angular velocity directly.
        for v in self.vehicles.values() {
            if matches!(v.spec.drive_mode, DriveMode::Drone) {
                continue;
            }
            if v.control.brake < 0.5 {
                continue;
            }
            let Some(rb) = self.bodies.get_mut(v.body) else { continue };
            let lv = rb.linvel();
            let horiz2 = lv.x * lv.x + lv.z * lv.z;
            if horiz2 < 0.5 * 0.5 {
                // < 0.5 m/s horizontal — pin it.
                let ly = lv.y;
                rb.set_linvel(Vec3::new(0.0, ly, 0.0), true);
                rb.set_angvel(Vec3::ZERO, true);
            }
        }

        // 2. Run each ground vehicle's ray-cast controller. Drones
        //    skip this — they have no wheels.
        let Sim {
            vehicles,
            bodies,
            colliders,
            broad_phase,
            narrow_phase,
            ..
        } = self;
        for v in vehicles.values_mut() {
            if matches!(v.spec.drive_mode, DriveMode::Drone) {
                continue;
            }
            // Exclude the vehicle's OWN chassis from its wheel raycasts.
            // Without this, rays starting inside the chassis collider hit
            // the chassis itself, rapier reports `suspension_length = 0`,
            // and the wheels get "stuck" against the chassis bottom.
            let filter = QueryFilter::default()
                .exclude_rigid_body(v.body)
                .groups(world::wheel_raycast_groups());
            let qpm = broad_phase.as_query_pipeline_mut(
                narrow_phase.query_dispatcher(),
                bodies,
                colliders,
                filter,
            );
            v.controller.update_vehicle(dt, qpm);
        }

        // 3. Step the physics world.
        self.pipeline.step(
            self.gravity,
            &self.integration,
            &mut self.islands,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd_solver,
            &(),
            &(),
        );
    }
}

fn normalize_or(v: Vec3, fallback: Vec3) -> Vec3 {
    let n = v.normalize_or_zero();
    if n == Vec3::ZERO { fallback } else { n }
}

fn apply_controls(v: &mut VehicleState, bodies: &RigidBodySet) {
    let ctrl = v.control;
    let specs: &[WheelSpec] = &v.spec.wheels;

    // Brake anti-shake. Rapier's copysign-based brake flips sign near
    // zero velocity, causing oscillation. Taper the per-wheel brake
    // down starting at 1.2 m/s and force it to 0 below 0.5 m/s — the
    // parking-brake pass in `step()` takes over from there.
    let speed_mag = bodies
        .get(v.body)
        .map(|rb| rb.linvel().length())
        .unwrap_or(0.0);
    let brake_gate = if speed_mag < 0.5 {
        0.0
    } else {
        ((speed_mag - 0.5) / 0.7).clamp(0.0, 1.0)
    };

    // Ackermann wheelbase: longitudinal distance from the frontmost
    // to the rearmost wheel attachment. Used to compute each steered
    // wheel's angle such that all steered wheels point at a common
    // turn centre (no tyre scrub).
    let (mut z_min, mut z_max) = (f64::INFINITY, f64::NEG_INFINITY);
    for wspec in specs {
        z_min = z_min.min(wspec.chassis_connection.z);
        z_max = z_max.max(wspec.chassis_connection.z);
    }
    let wheelbase = (z_max - z_min) as f32;

    // Collect each wheel's current suspension (normal) force from
    // rapier. These govern per-wheel grip, which in turn drives the
    // differential's per-wheel torque split below.
    let normal_forces: Vec<f32> = v
        .controller
        .wheels()
        .iter()
        .map(|w| w.wheel_suspension_force.max(0.0))
        .collect();

    // Group driven wheels by axle (bucketed by z to handle float
    // fuzz). Within an axle, we apply a weight-transfer-aware open
    // differential: each wheel's share of the axle's total engine
    // force is proportional to its share of the axle's total normal
    // force. Unloaded wheels get less torque (no more one-wheel spin);
    // loaded wheels get more. On even ground this degenerates to 50/50.
    use std::collections::BTreeMap;
    let mut axles: BTreeMap<i32, Vec<usize>> = BTreeMap::new();
    for (idx, spec) in specs.iter().enumerate() {
        if !spec.driven {
            continue;
        }
        let z_key = (spec.chassis_connection.z * 100.0).round() as i32;
        axles.entry(z_key).or_default().push(idx);
    }

    // engine_force_per_wheel[i] is what we'll actually write. Default
    // is the legacy "each wheel gets full throttle × max_engine_force".
    let mut engine_force_per_wheel: Vec<f32> = specs
        .iter()
        .map(|s| if s.driven { ctrl.throttle * s.max_engine_force } else { 0.0 })
        .collect();

    match v.spec.drive_mode {
        DriveMode::Ackermann => {
            // Weight-transfer-aware open differential within each axle.
            for (_z, wheel_indices) in &axles {
                if wheel_indices.len() < 2 {
                    continue;
                }
                let total_n: f32 = wheel_indices.iter().map(|&i| normal_forces[i]).sum();
                if total_n < 1.0 {
                    continue; // axle airborne
                }
                let axle_total: f32 = wheel_indices
                    .iter()
                    .map(|&i| ctrl.throttle * specs[i].max_engine_force)
                    .sum();
                for &idx in wheel_indices {
                    let share = normal_forces[idx] / total_n;
                    engine_force_per_wheel[idx] = axle_total * share;
                }
            }
        }
        DriveMode::Drone => {
            // Drones never get here — `step()` dispatches them to
            // `apply_drone_controls`. Leave per-wheel forces at zero
            // just in case a wheel is present on a Drone-mode spec.
        }
        DriveMode::Differential => {
            // Skid-steer: left vs right throttle instead of wheel
            // angle. Positive `steer` pivots the vehicle LEFT
            // (matches Ackermann) — right-side wheels have to run
            // faster than left-side. `+x` is the right side by our
            // lateral convention.
            //
            // `TURN_GAIN` amplifies the steer component's
            // contribution so turns are crisp at the low
            // `max_engine_force` values the Husky needs for a
            // sensible straight-line top speed. Pure turn-in-place
            // produces `TURN_GAIN × max_engine_force` per wheel,
            // well above the straight-line force budget.
            const TURN_GAIN: f32 = 6.0;
            let t = ctrl.throttle;
            let s = ctrl.steer * TURN_GAIN;
            // +X is the right side. Positive `steer` (A key) must
            // pivot the vehicle LEFT, i.e. right-side wheels push
            // backward while left-side wheels push forward.
            let left_cmd = t + s;
            let right_cmd = t - s;
            for (idx, spec) in specs.iter().enumerate() {
                if !spec.driven {
                    engine_force_per_wheel[idx] = 0.0;
                    continue;
                }
                let cmd = if spec.chassis_connection.x < 0.0 {
                    left_cmd
                } else {
                    right_cmd
                };
                engine_force_per_wheel[idx] = cmd * spec.max_engine_force;
            }
        }
    }

    let differential_mode = matches!(v.spec.drive_mode, DriveMode::Differential);
    let wheels = v.controller.wheels_mut();
    for ((w, spec), &engine_force) in
        wheels.iter_mut().zip(specs).zip(engine_force_per_wheel.iter())
    {
        w.engine_force = engine_force;
        w.brake = ctrl.brake * spec.max_brake * brake_gate;
        w.steering = if differential_mode {
            // All wheels fixed forward on skid-steer.
            0.0
        } else if spec.steered {
            ackermann_steer(
                ctrl.steer,
                spec.max_steer_rad,
                spec.chassis_connection.x as f32,
                wheelbase,
            )
        } else {
            0.0
        };
    }
}

/// Arcade drone controls with cosmetic tilt.
///
/// A real quadrotor is an inverted pendulum — differential rotor
/// thrust tilts the body, gravity then pulls it horizontally. Doing
/// that literally in rapier needs a PID attitude stabiliser to keep
/// it from immediately flipping (no pendulum stays balanced by
/// itself). Instead we cheat in the standard game-engine way:
///
///   - Horizontal translation is driven by direct **forces at the
///     centre of mass** — always stable, arcade-easy to steer.
///   - Tilt is produced by a **PD controller** that drives the
///     drone's body toward a target pitch/roll angle proportional
///     to the `throttle` / `steer` commands. Release the stick and
///     it levels out again.
///
/// Control mapping:
///   - `throttle` (W/S) → forward / backward force + nose-down/up
///     visual tilt (drone "leans into" its motion).
///   - `steer`    (A/D) → strafe force + bank-right/left tilt.
///   - `lift`     (Z/X) → extra vertical force on top of the
///     constant hover force that cancels gravity.
///   - `yaw`      (Q/E) → yaw torque around world +Y. Positive
///     `yaw` (Q) = turn LEFT.
fn apply_drone_controls(v: &mut VehicleState, bodies: &mut RigidBodySet, gravity: Vector) {
    let ctrl = v.control;
    let Some(rb) = bodies.get_mut(v.body) else { return };
    let mass = rb.mass();
    let rot = *rb.rotation();

    // World-frame basis vectors from the body's current rotation.
    let fwd_world = rot * Vec3::Z;
    let right_world = rot * Vec3::X;
    let up_body_world = rot * Vec3::Y;

    // Horizontal projections so the drone doesn't dive when tilted.
    let fwd_h = Vec3::new(fwd_world.x, 0.0, fwd_world.z).normalize_or_zero();
    let right_h = Vec3::new(right_world.x, 0.0, right_world.z).normalize_or_zero();

    // Tunables (per-mass, so scaling the drone up keeps the feel).
    const HORIZ_ACCEL: f32 = 6.0;   // m/s² at full stick
    const LIFT_ACCEL:  f32 = 10.0;  // m/s² at full lift
    const YAW_ACCEL:   f32 = 2.7;   // rad/s² at full yaw (3× the previous 0.9 — Q/E now spin briskly)
    const MAX_TILT:    f32 = 0.30;  // rad (~17°) at full stick
    // PD gains expressed in *angular-acceleration* space (rad/s² per
    // rad of error, and rad/s² per rad/s of rate). Multiplying by
    // the body's actual inertia tensor below converts them to Nm, so
    // small drones don't get torques scaled for a refrigerator.
    // Critical-ish damping at ~ω_n = 8 rad/s (stops in ~0.4 s).
    const TILT_OMEGA:  f32 = 8.0;   // natural freq
    const TILT_ZETA:   f32 = 0.9;   // damping ratio

    // --- Linear forces ---------------------------------------------
    // Hover force goes along the drone's LOCAL +Y (`up_body_world`),
    // not world +Y. Consequences: level drone hovers normally; tilted
    // drone has its "hover" thrust tilted with it (gravity wins on
    // the vertical axis, drone starts to slide); **flipped** drone
    // (upside-down) gets pushed toward the ground and crashes
    // instead of magically floating.
    let gravity_mag = -gravity.y * mass;
    let hover = up_body_world * gravity_mag;
    // Same for the lift command — altitude control is relative to
    // the drone's own up axis.
    let lift = up_body_world * ctrl.lift * mass * LIFT_ACCEL;
    let fore = fwd_h * ctrl.throttle * mass * HORIZ_ACCEL;
    let side = right_h * ctrl.steer * mass * HORIZ_ACCEL;

    rb.reset_forces(true);
    rb.reset_torques(true);
    rb.add_force(hover + lift + fore + side, true);

    // --- Tilt controller -------------------------------------------
    // Measure current pitch / roll from how the body's local +Y
    // projects into world. Small-angle OK; the PD gains compensate.
    // Positive `pitch_angle` = nose tilted forward (toward +Z).
    // Positive `roll_angle`  = tilted to the RIGHT (toward +X).
    // (The earlier `-up_body_world.x` was the bug that made D tilt
    // the drone the wrong way and eventually flip it.)
    let pitch_angle = up_body_world.z.atan2(up_body_world.y);
    let roll_angle  = up_body_world.x.atan2(up_body_world.y);

    // Desired tilts (proportional to stick input).
    let target_pitch = ctrl.throttle * MAX_TILT;
    let target_roll  = ctrl.steer * MAX_TILT;

    // Body-frame angular velocity components (pitch rate around body X,
    // roll rate around body Z).
    let angvel_world = rb.angvel();
    let angvel_local = rot.inverse() * angvel_world;

    // Desired angular acceleration in body frame (rad/s²).
    // Second-order critically-ish-damped controller: α = ω²·err − 2ζω·rate.
    let kp = TILT_OMEGA * TILT_OMEGA;
    let kd = 2.0 * TILT_ZETA * TILT_OMEGA;
    let pitch_alpha = kp * (target_pitch - pitch_angle) - kd * angvel_local.x;
    let roll_alpha  = kp * (target_roll  - roll_angle)  - kd * (-angvel_local.z);

    // Convert α → Nm by multiplying by the body's actual local
    // principal inertia. Tiny drones get tiny torques, big drones
    // get big torques — instead of the old fixed KP-in-Nm that
    // explosion-integrated on small inertias.
    let local_inertia = rb.mass_properties().local_mprops.principal_inertia();
    let pitch_torque_local = pitch_alpha * local_inertia.x;
    let roll_torque_local  = -roll_alpha * local_inertia.z;

    // Yaw torque in Nm — also scaled by inertia so `YAW_ACCEL` is
    // a true rad/s².
    let yaw_torque_world_y = -ctrl.yaw * YAW_ACCEL * local_inertia.y;

    let torque_local = Vec3::new(pitch_torque_local, 0.0, roll_torque_local);
    let torque_world = rot * torque_local + Vec3::new(0.0, yaw_torque_world_y, 0.0);
    rb.add_torque(torque_world, true);
}

/// Per-wheel Ackermann steering correction. Returns the *actual*
/// steering angle (radians) for a wheel at lateral position `wheel_x`
/// given the nominal max-steer input.
///
///   R_c    = wheelbase / tan(input × max_steer)    (turn radius at centre)
///   R_w    = R_c + wheel_x                         (radius at this wheel)
///   δ_w    = atan(wheelbase / R_w)
///
/// Inside wheel (same side as steer direction) → smaller R_w → larger δ_w.
/// Outside wheel → larger R_w → smaller δ_w. `max_steer_rad` caps the
/// result so the wheel never exceeds its mechanical limit — this
/// matters most for low-radius turns where the inside wheel would
/// otherwise demand an angle larger than the physical steering stop.
fn ackermann_steer(input: f32, max_steer: f32, wheel_x: f32, wheelbase: f32) -> f32 {
    if input.abs() < 1e-6 || max_steer.abs() < 1e-6 || wheelbase <= 1e-3 {
        return input * max_steer;
    }
    let delta_c = input * max_steer;
    let r_c = wheelbase / delta_c.tan(); // signed; same sign as delta_c
    let r_w = r_c + wheel_x;
    // If r_w flips through zero (very tight turn with wide track),
    // atan2 keeps it sane.
    let delta_w = wheelbase.atan2(r_w);
    // atan2 returns [0, π); for a right turn we want a negative
    // result, so inherit the sign of the nominal angle.
    let delta_signed = delta_w.copysign(delta_c);
    // Don't exceed the mechanical stop; preserves spec's own limit.
    delta_signed.clamp(-max_steer.abs(), max_steer.abs())
}

fn principal_inertia(mass: f32, half_extents: Vec3) -> Vec3 {
    let (x, y, z) = (
        half_extents.x * 2.0,
        half_extents.y * 2.0,
        half_extents.z * 2.0,
    );
    let factor = mass / 12.0;
    Vec3::new(
        factor * (y * y + z * z),
        factor * (x * x + z * z),
        factor * (x * x + y * y),
    )
}
