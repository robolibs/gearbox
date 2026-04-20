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
            .build();
        self.colliders
            .insert_with_parent(collider, body_handle, &mut self.bodies);

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
            apply_controls(v);
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
            let filter = QueryFilter::default().exclude_rigid_body(v.body);
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

fn apply_controls(v: &mut VehicleState) {
    let ctrl = v.control;
    let specs: &[WheelSpec] = &v.spec.wheels;
    let wheels = v.controller.wheels_mut();
    for (w, spec) in wheels.iter_mut().zip(specs) {
        w.engine_force = if spec.driven {
            ctrl.throttle * spec.max_engine_force
        } else {
            0.0
        };
        w.brake = ctrl.brake * spec.max_brake;
        w.steering = if spec.steered {
            ctrl.steer * spec.max_steer_rad
        } else {
            0.0
        };
    }
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
