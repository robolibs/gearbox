//! The core sim — owns the rapier physics world and all vehicles.
//!
//! The public API speaks [`datapod`] spatial types (f64). The rapier
//! glam/f32 layer is an implementation detail, bridged through
//! [`crate::convert`].

use std::collections::HashMap;

use datapod::{Geo, Pose, Size, Velocity};
use rapier3d::control::{DynamicRayCastVehicleController, WheelTuning};
use rapier3d::prelude::*;

use gearbox_core::planet::Planet;
use gearbox_core::{ControlInput, PartKind, VehicleId, VehicleSpec};

use crate::convert::{
    point_to_vec3, pose_to_rpose, rpose_to_pose, size_to_half_extents, size_to_vec3,
    vec3_to_velocity,
};
use crate::drive::{self, DriveContext};
use crate::vehicle::VehicleState;
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

    /// Debug / sandbox toggle — when `true`, every vehicle's power
    /// system skips the decrement step. Drain rate + "moving" flag
    /// are still computed so the Inspector shows live diagnostics,
    /// but reservoirs never fall. World-level, not per-vehicle.
    pub unlimited_power: bool,

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
            unlimited_power: false,
            vehicles: HashMap::new(),
            next_id: 0,
        }
    }

    /// Add a large flat ground collider centered at the origin.
    pub fn add_ground_plane(&mut self, half_size: f64) {
        self.colliders.insert(world::build_ground_collider(half_size));
    }

    /// Add a sphere collider representing the planet. The sphere is placed
    /// so its tangent (top) touches `y = 0`, matching the coordinate
    /// convention used by the visual planet mesh.
    pub fn add_planet_collider(&mut self, radius: f64) {
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
        // Inertia uses `inertia_size` if provided, otherwise the
        // collider half-extents. Gantry-style machines (Robotti) need
        // the full outer bounding box here even though the collider
        // is a small central pod.
        let inertia_half_extents = chassis
            .inertia_size
            .map(size_to_half_extents)
            .unwrap_or(half_extents);
        let mass_props = MassProperties::new(
            point_to_vec3(chassis.com_offset),
            chassis.mass,
            principal_inertia(chassis.mass, inertia_half_extents),
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
            let axle_rot = Vec3::new(0.0, 0.0, -std::f64::consts::FRAC_PI_2);
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
            let hx = (part.size.x * 0.5) as f64;
            let hy = (part.size.y * 0.5) as f64;
            let hz = (part.size.z * 0.5) as f64;
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
                control: ControlInput::default(),
                handles: crate::vehicle_physics::PhysicsHandles {
                    body: body_handle,
                    controller,
                },
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
        let handle = v.handles.body;
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
            .and_then(|v| self.bodies.get(v.handles.body))
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
            .and_then(|v| self.bodies.get(v.handles.body))
            .map(|rb| vec3_to_velocity(rb.linvel()))
            .unwrap_or_default()
    }

    /// Angular velocity (rad/s) of the vehicle's chassis body. The
    /// returned struct reuses `Velocity` as an xyz triple: `vy` is the
    /// yaw rate (rotation around world +Y).
    pub fn vehicle_angvel(&self, id: VehicleId) -> Velocity {
        self.vehicles
            .get(&id)
            .and_then(|v| self.bodies.get(v.handles.body))
            .map(|rb| vec3_to_velocity(rb.angvel()))
            .unwrap_or_default()
    }

    /// World-space pose of a wheel, oriented so the wheel's local Y axis
    /// aligns with the axle (matches Bevy's `Cylinder` primitive, which
    /// extrudes along +Y).
    pub fn wheel_pose(&self, id: VehicleId, wheel: usize) -> Pose {
        let Some(v) = self.vehicles.get(&id) else { return Pose::default() };
        let Some(wh) = v.handles.controller.wheels().get(wheel) else { return Pose::default() };
        let spec = &v.spec.wheels[wheel];

        let axle = normalize_or(wh.axle(), Vec3::X);
        let down = normalize_or(wh.suspension(), Vec3::NEG_Y);

        // When the wheel is airborne, rapier leaves `suspension_length` at
        // its default (0), which makes `wheel.center()` return the chassis
        // attach point — wheels appear embedded in the chassis. Compute the
        // fully-extended (rest-length) position ourselves in that case.
        let mut center = if wh.raycast_info().is_in_contact {
            wh.center()
        } else {
            wh.raycast_info().hard_point_ws + down * wh.suspension_rest_length
        };

        // Kingpin-offset swing: if the spec says the steering pivot is
        // offset from `chassis_connection` (Robotti's cylinder struts
        // are outboard of the wheel hubs), swing the wheel around that
        // kingpin. Purely visual — physics forces are still applied at
        // `chassis_connection`, this just relocates the rendered hub.
        //
        // Geometry: kingpin in chassis-local is K = C + D, where C is
        // the chassis_connection and D = steering_pivot_offset. The
        // wheel hangs off an arm from K; at steering θ the arm rotates
        // by θ around the vertical axis at K. Rest-position offset is
        // −D (from K back to C), so at angle θ the wheel centre is
        //     W_local = K + R_y(θ) · (−D) = C + D − R_y(θ)·D.
        // We apply the delta (W_local − C) in world space to shift
        // the rapier-computed centre.
        let d_local = point_to_vec3(spec.steering_pivot_offset);
        if d_local.length_squared() > 1e-10 {
            if let Some(rb) = self.bodies.get(v.handles.body) {
                let chassis_rot = rb.position().rotation;
                let theta = wh.steering;
                let (sin_t, cos_t) = theta.sin_cos();
                // R_y(θ) · d_local (row vector form: only X and Z swap).
                let rotated = Vec3::new(
                    d_local.x * cos_t + d_local.z * sin_t,
                    d_local.y,
                    -d_local.x * sin_t + d_local.z * cos_t,
                );
                let local_delta = d_local - rotated;
                center += chassis_rot * local_delta;
            }
        }

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

    /// Mutable access to a vehicle's state. Use with care — changing
    /// collider extents at runtime won't reshape the rapier body, but
    /// edits to spec fields read each tick (wheel engine force, mass
    /// overrides, visual colour, etc.) take effect immediately.
    pub fn vehicle_mut(&mut self, id: VehicleId) -> Option<&mut VehicleState> {
        self.vehicles.get_mut(&id)
    }

    /// Set a vehicle's total mass, updating the rapier rigid body's
    /// mass properties in-place so the physics reflects the change on
    /// the next tick. Inertia is recomputed from the current inertia
    /// box (either `chassis.inertia_size` override or the collider size).
    pub fn set_vehicle_mass(&mut self, id: VehicleId, mass: f64) {
        let Some(state) = self.vehicles.get_mut(&id) else { return };
        state.spec.chassis.mass = mass.max(0.01);
        let half_extents = state
            .spec
            .chassis
            .inertia_size
            .map(size_to_half_extents)
            .unwrap_or_else(|| size_to_half_extents(state.spec.chassis.size));
        let new_inertia = principal_inertia(state.spec.chassis.mass, half_extents);
        let com = point_to_vec3(state.spec.chassis.com_offset);
        if let Some(rb) = self.bodies.get_mut(state.handles.body) {
            let props = MassProperties::new(com, state.spec.chassis.mass, new_inertia);
            rb.set_additional_mass_properties(props, true);
        }
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
                .exclude_rigid_body(v.handles.body)
                .groups(world::wheel_raycast_groups());
            let qpm = broad_phase.as_query_pipeline_mut(
                narrow_phase.query_dispatcher(),
                bodies,
                colliders,
                filter,
            );
            v.handles.controller.update_vehicle(0.0, qpm);
        }
    }

    /// Advance the simulation by `dt` seconds.
    pub fn step(&mut self, dt: f64) {
        self.integration.dt = dt;

        // 1a. Drain the power reservoir(s) + tick auto-fill on each
        //     vehicle's containers.
        //
        //     Tiers (observable behaviour):
        //       * parked              → tiny idle trickle, NO fill
        //       * moving, work off    → travel drain,       NO fill
        //       * moving, work on     → travel + work drain, auto-fill
        //
        //     The controllers themselves gate on `is_engine_live()`
        //     so a depleted / powered-off vehicle can't move.
        //
        //     We feed the power/auto-fill tick **horizontal speed
        //     only** — `linvel().length()` includes the Y component,
        //     which is non-zero on any suspension-sprung vehicle even
        //     when parked. Letting that count as "moving" is the
        //     classic-rapier gotcha that makes a stationary tractor
        //     burn fuel at full travel rate.
        let drain_enabled = !self.unlimited_power;
        for v in self.vehicles.values_mut() {
            let horiz_speed = self
                .bodies
                .get(v.handles.body)
                .map(|rb| {
                    let lv = rb.linvel();
                    (lv.x * lv.x + lv.z * lv.z).sqrt()
                })
                .unwrap_or(0.0);
            v.spec.power.tick(dt, horiz_speed, drain_enabled);
            let moving = gearbox_core::vehicle::power::is_moving(horiz_speed);
            let work_on = v.spec.power.work && v.spec.power.is_engine_live();
            for c in &mut v.spec.containers {
                c.tick_auto_fill(dt, work_on, moving);
            }
        }

        // 1b. Translate ControlInput → per-wheel engine/brake/steering
        //     for ground vehicles, or direct body forces/torques for
        //     drones. Dispatched through the DriveController trait so
        //     new modes are just new files under `vehicle/drive/`.
        //     Each controller sees the physics only through narrow
        //     `BodyProxy` / `WheelsProxy` handles — constructed here
        //     from the rapier body + wheel-controller under the hood.
        for v in self.vehicles.values_mut() {
            let controller = drive::controller_for(v.spec.drive_mode);
            // Split-borrow the vehicle: `spec` + `control` are shared
            // reads / copies; `handles.body` is a handle into a
            // separately-held `bodies` map; `handles.controller` is
            // borrowed mutably on a disjoint field. Rust's field
            // splitting makes this OK.
            let VehicleState {
                spec,
                control,
                handles,
            } = v;
            let Some(rb) = self.bodies.get_mut(handles.body) else { continue };
            let mut ctx = DriveContext {
                dt,
                gravity: self.gravity,
                spec,
                control: *control,
                body: crate::vehicle_physics::BodyProxy::new(rb),
                wheels: crate::vehicle_physics::WheelsProxy::new(&mut handles.controller),
            };
            controller.apply(&mut ctx);
        }

        // 1b. Parking brake. Rapier's `wheel.brake` is applied as
        // `engine_force = -brake * copysign(forward_impulse)` — the
        // sign term flips every step once forward_impulse is near zero,
        // which shows up in the viewport as the whole vehicle
        // shimmying. Sidestep: when the driver is pressing brake AND
        // the vehicle is already slow, zero out horizontal linvel +
        // angular velocity directly. Airborne vehicles skip this.
        for v in self.vehicles.values() {
            if drive::controller_for(v.spec.drive_mode).is_airborne() {
                continue;
            }
            if v.control.brake < 0.5 {
                continue;
            }
            let Some(rb) = self.bodies.get_mut(v.handles.body) else { continue };
            let lv = rb.linvel();
            let horiz2 = lv.x * lv.x + lv.z * lv.z;
            if horiz2 < 0.5 * 0.5 {
                let ly = lv.y;
                rb.set_linvel(Vec3::new(0.0, ly, 0.0), true);
                rb.set_angvel(Vec3::ZERO, true);
            }
        }

        // 2. Run each ground vehicle's ray-cast controller. Airborne
        //    vehicles skip this — they have no wheels.
        let Sim {
            vehicles,
            bodies,
            colliders,
            broad_phase,
            narrow_phase,
            ..
        } = self;
        for v in vehicles.values_mut() {
            if drive::controller_for(v.spec.drive_mode).is_airborne() {
                continue;
            }
            // Exclude the vehicle's OWN chassis from its wheel raycasts.
            // Without this, rays starting inside the chassis collider hit
            // the chassis itself, rapier reports `suspension_length = 0`,
            // and the wheels get "stuck" against the chassis bottom.
            let filter = QueryFilter::default()
                .exclude_rigid_body(v.handles.body)
                .groups(world::wheel_raycast_groups());
            let qpm = broad_phase.as_query_pipeline_mut(
                narrow_phase.query_dispatcher(),
                bodies,
                colliders,
                filter,
            );
            v.handles.controller.update_vehicle(dt, qpm);
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

fn principal_inertia(mass: f64, half_extents: Vec3) -> Vec3 {
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
