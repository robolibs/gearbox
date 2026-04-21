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
use crate::vehicle::{VehicleId, VehicleSpec, VehicleState, WheelSpec};
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
        let spin = Rot3::from_axis_angle(axle, wh.rotation);

        rpose_to_pose(Pose3::from_parts(center, spin * basis_rot))
    }

    pub fn vehicles(&self) -> impl Iterator<Item = (VehicleId, &VehicleState)> {
        self.vehicles.iter().map(|(id, v)| (*id, v))
    }

    pub fn vehicle(&self, id: VehicleId) -> Option<&VehicleState> {
        self.vehicles.get(&id)
    }

    /// Advance the simulation by `dt` seconds.
    pub fn step(&mut self, dt: f32) {
        self.integration.dt = dt;

        // 1. Translate ControlInput → per-wheel engine/brake/steering.
        for v in self.vehicles.values_mut() {
            apply_controls(v, &self.bodies);
        }

        // 1b. Parking brake. Rapier's `wheel.brake` is applied as
        // `engine_force = -brake * copysign(forward_impulse)` — the
        // sign term flips every step once forward_impulse is near zero,
        // which shows up in the viewport as the whole vehicle
        // shimmying. Sidestep the whole mess: when the driver is
        // pressing brake AND the vehicle is already slow, zero out
        // the horizontal linvel + the angular velocity directly.
        for v in self.vehicles.values() {
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

        // 2. Run each vehicle's ray-cast controller.
        let Sim {
            vehicles,
            bodies,
            colliders,
            broad_phase,
            narrow_phase,
            ..
        } = self;
        for v in vehicles.values_mut() {
            // Exclude the vehicle's OWN chassis from its wheel raycasts.
            // Without this, rays starting inside the chassis collider hit
            // the chassis itself, rapier reports `suspension_length = 0`,
            // and the wheels get "stuck" against the chassis bottom.
            // Exclude own body + restrict to GROUND|CHASSIS so our
            // ray never treats another vehicle's wheel collider as
            // ground (see `world::wheel_raycast_groups`).
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

    for (_z, wheel_indices) in &axles {
        if wheel_indices.len() < 2 {
            continue; // only one driven wheel on this axle — no split needed
        }
        let total_n: f32 = wheel_indices.iter().map(|&i| normal_forces[i]).sum();
        if total_n < 1.0 {
            continue; // axle airborne — keep legacy force
        }
        // Total axle torque = throttle × sum of wheel max forces
        // (so an axle with two 10 kN wheels still delivers 20 kN
        // combined, same as before — we're only changing the split).
        let axle_total: f32 = wheel_indices
            .iter()
            .map(|&i| ctrl.throttle * specs[i].max_engine_force)
            .sum();
        for &idx in wheel_indices {
            let share = normal_forces[idx] / total_n;
            engine_force_per_wheel[idx] = axle_total * share;
        }
    }

    let wheels = v.controller.wheels_mut();
    for ((w, spec), &engine_force) in
        wheels.iter_mut().zip(specs).zip(engine_force_per_wheel.iter())
    {
        w.engine_force = engine_force;
        w.brake = ctrl.brake * spec.max_brake * brake_gate;
        w.steering = if spec.steered {
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
