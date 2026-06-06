//! The core sim — owns the rapier physics world and all vehicles.
//!
//! Vehicles are physically simulated: each wheel is a real rigid body
//! (a light hub carries suspension + steering, the wheel body spins on
//! the hub) and traction comes from the tyre collider's actual contact
//! with the ground. See [`crate::vehicle_physics`] for the rig layout.
//!
//! The public API speaks [`datapod`] spatial types (f64). The rapier
//! layer is an implementation detail, bridged through [`crate::convert`].

use std::collections::HashMap;

use datapod::{Geo, Pose, Size, Velocity};
use rapier3d::prelude::*;

use gearbox_core::planet::Planet;
use gearbox_core::{ControlInput, PartKind, VehicleId, VehicleSpec};

use crate::convert::{
    point_to_vec3, pose_to_rpose, rpose_to_pose, size_to_half_extents, size_to_vec3,
    vec3_to_velocity,
};
use crate::drive::{self, DriveContext};
use crate::vehicle::VehicleState;
use crate::vehicle_physics::{
    BodyProxy, PhysicsHandles, WheelCommand, WheelHandles, WheelSnapshot, WheelsProxy,
};
use crate::world;

/// Stiffness of the steering position-motor (force-based).
const STEER_STIFFNESS: f64 = 1.0e6;
/// Damping of the steering position-motor.
const STEER_DAMPING: f64 = 3.0e4;
/// Force cap on the steering motor.
const STEER_MAX_FORCE: f64 = 1.0e7;
/// Damping of the per-wheel brake (velocity-0) motor.
const BRAKE_DAMPING: f64 = 1.0e5;
/// Gravity magnitude used for suspension-spring auto-tuning.
const GRAVITY: f64 = 9.81;

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
        // A few extra solver iterations keep the wheel/suspension joint
        // chains stable under heavy load at a 60 Hz step.
        let integration = IntegrationParameters {
            num_solver_iterations: 8,
            ..IntegrationParameters::default()
        };

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
            integration,
            gravity: Vector::new(0.0, -GRAVITY, 0.0),
            planet: Planet::default(),
            unlimited_power: false,
            vehicles: HashMap::new(),
            next_id: 0,
        }
    }

    /// Add a large flat ground collider centered at the origin.
    pub fn add_ground_plane(&mut self, half_size: f64) {
        self.colliders
            .insert(world::build_ground_collider(half_size));
    }

    /// Add a sphere collider representing the planet. The sphere is placed
    /// so its tangent (top) touches `y = 0`, matching the coordinate
    /// convention used by the visual planet mesh.
    pub fn add_planet_collider(&mut self, radius: f64) {
        let centre = Vec3::new(0.0, -radius, 0.0);
        self.colliders
            .insert(world::build_planet_collider(centre, radius));
    }

    /// Add a static box obstacle at the given pose.
    pub fn add_box_obstacle(&mut self, pose: Pose, size: Size) {
        self.colliders.insert(world::build_box_collider(
            size_to_vec3(size),
            pose_to_rpose(pose),
        ));
    }

    /// Spawn a vehicle from its declarative spec at the given pose.
    pub fn spawn_vehicle(&mut self, spec: VehicleSpec, pose: Pose) -> VehicleId {
        let id = VehicleId(self.next_id);
        self.next_id += 1;
        // Non-zero per-vehicle tag stamped onto every vehicle collider's
        // `user_data`, so `SameVehicleFilter` can reject same-vehicle
        // contacts (wheels are separate bodies now).
        let tag = world::vehicle_tag(id);

        let chassis = &spec.chassis;
        let half_extents = size_to_half_extents(chassis.size);
        let chassis_pose = pose_to_rpose(pose);

        // --- Chassis body + collider ------------------------------------
        let body = RigidBodyBuilder::dynamic()
            .pose(chassis_pose)
            .linear_damping(chassis.linear_damping)
            .angular_damping(chassis.angular_damping)
            .ccd_enabled(chassis.ccd)
            // Keep vehicles perma-awake — see the power/control loops.
            .can_sleep(false)
            .build();
        let body_handle = self.bodies.insert(body);

        // Drive mass + COM offset + inertia from explicit MassProperties.
        // `insert_with_parent` recomputes the body's mprops from its
        // colliders, so setting them on the collider is what sticks.
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
            .collision_groups(world::vehicle_groups())
            .active_hooks(ActiveHooks::FILTER_CONTACT_PAIRS)
            .user_data(tag)
            .build();
        self.colliders
            .insert_with_parent(collider, body_handle, &mut self.bodies);

        // --- Body-part colliders ----------------------------------------
        // Karosserie + tanks are solid bodywork; hitches are visual only.
        // mass = 0 so they don't perturb the explicit chassis mprops.
        for part in &spec.parts {
            if matches!(part.kind, PartKind::Hitch) {
                continue;
            }
            let hx = part.size.x * 0.5;
            let hy = part.size.y * 0.5;
            let hz = part.size.z * 0.5;
            let part_collider = ColliderBuilder::cuboid(hx, hy, hz)
                .translation(point_to_vec3(part.position))
                .mass(0.0)
                .friction(0.5)
                .restitution(0.0)
                .collision_groups(world::vehicle_groups())
                .active_hooks(ActiveHooks::FILTER_CONTACT_PAIRS)
                .user_data(tag)
                .build();
            self.colliders
                .insert_with_parent(part_collider, body_handle, &mut self.bodies);
        }

        // --- Suspension auto-tuning -------------------------------------
        // The chassis is the sprung mass; share it across the wheels and
        // size each spring to carry its share at ~25 % static compression.
        let n_wheels = spec.wheels.len().max(1) as f64;
        let sprung_share = chassis.mass / n_wheels;
        let static_load = sprung_share * GRAVITY;

        // --- Wheels: hub + wheel bodies, suspension + spin joints -------
        let mut wheels = Vec::with_capacity(spec.wheels.len());
        for w in &spec.wheels {
            let c = point_to_vec3(w.chassis_connection);
            let d = point_to_vec3(w.suspension_dir).normalize(); // suspension / kingpin
            let a = point_to_vec3(w.axle_dir).normalize(); // axle / spin axis
            let l = w.suspension_rest_length;
            // Wheel centre at full suspension extension, chassis-local.
            let center_local = c + d * l;
            let center_world = chassis_pose.translation + chassis_pose.rotation * center_local;
            // Hub + wheel start chassis-aligned and co-located at the
            // wheel centre, so joint B (the spin revolute) is untwisted.
            let start_pose = Pose3::from_parts(center_world, chassis_pose.rotation);

            // Hub: no collider, small explicit mass properties so it has
            // a defined (tiny) rotational inertia for the steering DOF.
            let hub_inertia = (0.4 * w.hub_mass * 0.01).max(1.0e-3);
            let hub = RigidBodyBuilder::dynamic()
                .pose(start_pose)
                .can_sleep(false)
                .additional_mass_properties(MassProperties::new(
                    Vec3::ZERO,
                    w.hub_mass,
                    Vec3::splat(hub_inertia),
                ))
                .build();
            let hub_handle = self.bodies.insert(hub);

            // Wheel body — carries the tyre collider.
            let wheel_rb = RigidBodyBuilder::dynamic()
                .pose(start_pose)
                .can_sleep(false)
                .build();
            let wheel_handle = self.bodies.insert(wheel_rb);

            // Round cylinder — far steadier on a flat plane than a bare
            // cylinder edge. Sized so the rolling radius stays `w.radius`.
            let q_axle = Rot3::from_rotation_arc(Vec3::Y, a);
            let border = (w.radius * 0.2).min(w.width * 0.45).max(0.01);
            let tyre = ColliderBuilder::round_cylinder(
                (w.width * 0.5 - border).max(0.01),
                (w.radius - border).max(0.01),
                border,
            )
            .position(Pose3::from_parts(Vec3::ZERO, q_axle))
            .mass(w.mass)
            .friction(w.tire_friction)
            .restitution(0.0)
            .collision_groups(world::vehicle_groups())
            .active_hooks(ActiveHooks::FILTER_CONTACT_PAIRS)
            .user_data(tag)
            .build();
            self.colliders
                .insert_with_parent(tyre, wheel_handle, &mut self.bodies);

            // Suspension spring sized for this wheel's rest length.
            let static_comp = (l * 0.25).max(0.02);
            let susp_k = static_load / static_comp;
            let susp_c = 2.0 * 0.7 * (susp_k * sprung_share).sqrt();
            let susp_cap = (static_load * 8.0).max(1.0);
            let lin_lo = l * 0.15;
            let lin_hi = l * 1.9;

            // Joint A: chassis ↔ hub. Prismatic suspension along `d`;
            // steered wheels additionally free the `d`-twist (kingpin)
            // and drive it with a stiff position motor.
            let joint_a: GenericJoint = if w.steered {
                let locked = JointAxesMask::LIN_Y
                    | JointAxesMask::LIN_Z
                    | JointAxesMask::ANG_Y
                    | JointAxesMask::ANG_Z;
                GenericJointBuilder::new(locked)
                    .local_axis1(d)
                    .local_axis2(d)
                    .local_anchor1(c)
                    .local_anchor2(Vec3::ZERO)
                    .limits(JointAxis::LinX, [lin_lo, lin_hi])
                    .motor_model(JointAxis::LinX, MotorModel::ForceBased)
                    .motor_position(JointAxis::LinX, l, susp_k, susp_c)
                    .motor_max_force(JointAxis::LinX, susp_cap)
                    .motor_model(JointAxis::AngX, MotorModel::ForceBased)
                    .motor_position(JointAxis::AngX, 0.0, STEER_STIFFNESS, STEER_DAMPING)
                    .motor_max_force(JointAxis::AngX, STEER_MAX_FORCE)
                    .build()
            } else {
                PrismaticJointBuilder::new(d)
                    .local_anchor1(c)
                    .local_anchor2(Vec3::ZERO)
                    .limits([lin_lo, lin_hi])
                    .motor_model(MotorModel::ForceBased)
                    .motor_position(l, susp_k, susp_c)
                    .motor_max_force(susp_cap)
                    .build()
                    .into()
            };
            let joint_a_handle = self
                .impulse_joints
                .insert(body_handle, hub_handle, joint_a, true);

            // Joint B: hub ↔ wheel. Revolute spin axis; the motor is
            // used only for braking (velocity-0, capped per tick).
            let joint_b = RevoluteJointBuilder::new(a)
                .local_anchor1(Vec3::ZERO)
                .local_anchor2(Vec3::ZERO)
                .motor_model(MotorModel::ForceBased)
                .motor_velocity(0.0, 0.0)
                .motor_max_force(0.0)
                .build();
            let joint_b_handle =
                self.impulse_joints
                    .insert(hub_handle, wheel_handle, joint_b, true);

            wheels.push(WheelHandles {
                hub: hub_handle,
                wheel: wheel_handle,
                joint_a: joint_a_handle,
                joint_b: joint_b_handle,
                steered: w.steered,
                radius: w.radius,
                axle_local: a,
                center_local,
                last_steering: 0.0,
                spin_angle: 0.0,
            });
        }

        self.vehicles.insert(
            id,
            VehicleState {
                spec,
                control: ControlInput::default(),
                handles: PhysicsHandles {
                    body: body_handle,
                    wheels,
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

    /// Remove a vehicle from the sim. Drops the chassis, every hub +
    /// wheel body, and their joints + colliders. The Bevy entity
    /// carrying the visuals is despawned separately by the caller.
    pub fn despawn_vehicle(&mut self, id: VehicleId) {
        let Some(state) = self.vehicles.remove(&id) else {
            return;
        };
        for wh in &state.handles.wheels {
            for h in [wh.wheel, wh.hub] {
                self.bodies.remove(
                    h,
                    &mut self.islands,
                    &mut self.colliders,
                    &mut self.impulse_joints,
                    &mut self.multibody_joints,
                    true,
                );
            }
        }
        self.bodies.remove(
            state.handles.body,
            &mut self.islands,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            true,
        );
    }

    /// Despawn every vehicle currently registered. Convenience for
    /// "scene reset" — leaves the static world (ground / planet /
    /// box obstacles) intact.
    pub fn despawn_all_vehicles(&mut self) {
        let ids: Vec<VehicleId> = self.vehicles.keys().copied().collect();
        for id in ids {
            self.despawn_vehicle(id);
        }
    }

    /// Teleport a vehicle to the given pose, zeroing its velocities.
    /// The hub + wheel bodies are snapped back to their rest position
    /// under the chassis so the suspension joints don't explode.
    pub fn set_vehicle_pose(&mut self, id: VehicleId, pose: Pose) {
        let Some(v) = self.vehicles.get(&id) else {
            return;
        };
        let handle = v.handles.body;
        if let Some(rb) = self.bodies.get_mut(handle) {
            rb.set_position(pose_to_rpose(pose), true);
            rb.set_linvel(Vec3::ZERO, true);
            rb.set_angvel(Vec3::ZERO, true);
        }
        self.snap_wheels_to_chassis(id);
    }

    /// Snap a vehicle's hub + wheel bodies back to their rest pose
    /// relative to the current chassis pose, zeroing their velocities.
    fn snap_wheels_to_chassis(&mut self, id: VehicleId) {
        let Some(v) = self.vehicles.get(&id) else {
            return;
        };
        let Some(chassis) = self.bodies.get(v.handles.body) else {
            return;
        };
        let cpos = *chassis.position();
        let targets: Vec<(RigidBodyHandle, RigidBodyHandle, Pose3)> = v
            .handles
            .wheels
            .iter()
            .map(|wh| {
                let centre = cpos.translation + cpos.rotation * wh.center_local;
                (wh.hub, wh.wheel, Pose3::from_parts(centre, cpos.rotation))
            })
            .collect();
        for (hub, wheel, pose) in targets {
            for h in [hub, wheel] {
                if let Some(rb) = self.bodies.get_mut(h) {
                    rb.set_position(pose, true);
                    rb.set_linvel(Vec3::ZERO, true);
                    rb.set_angvel(Vec3::ZERO, true);
                }
            }
        }
    }

    pub fn control(&self, id: VehicleId) -> ControlInput {
        self.vehicles
            .get(&id)
            .map(|v| v.control)
            .unwrap_or_default()
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
    pub fn vehicle_heading(&self, id: VehicleId) -> f64 {
        let pose = self.vehicle_pose(id);

        // Vehicle forward vector in world frame (rotate local +Z).
        let (mut fx, mut fy, mut fz) = (0.0_f64, 0.0_f64, 1.0_f64);
        pose.rotation.rotate_vector(&mut fx, &mut fy, &mut fz);
        let _ = fy;

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

    /// Angular velocity (rad/s) of the vehicle's chassis body.
    pub fn vehicle_angvel(&self, id: VehicleId) -> Velocity {
        self.vehicles
            .get(&id)
            .and_then(|v| self.bodies.get(v.handles.body))
            .map(|rb| vec3_to_velocity(rb.angvel()))
            .unwrap_or_default()
    }

    /// World-space pose of a wheel, oriented so the wheel's local Y axis
    /// aligns with the axle (matches Bevy's `Cylinder` primitive, which
    /// extrudes along +Y). With physical wheels this is simply the wheel
    /// body's pose plus the constant axle-alignment rotation.
    pub fn wheel_pose(&self, id: VehicleId, wheel: usize) -> Pose {
        let Some(v) = self.vehicles.get(&id) else {
            return Pose::default();
        };
        let Some(wh) = v.handles.wheels.get(wheel) else {
            return Pose::default();
        };
        let Some(wrb) = self.bodies.get(wh.wheel) else {
            return Pose::default();
        };
        let q_axle = Rot3::from_rotation_arc(Vec3::Y, wh.axle_local);
        rpose_to_pose(Pose3::from_parts(
            wrb.translation(),
            *wrb.rotation() * q_axle,
        ))
    }

    /// Cumulative spin angle of a wheel (radians, increasing as the
    /// wheel rolls forward). Used by visualisers that drive a wheel
    /// mesh's local rotation around the axle (USD scenes).
    pub fn wheel_spin_angle(&self, id: VehicleId, wheel: usize) -> f64 {
        self.vehicles
            .get(&id)
            .and_then(|v| v.handles.wheels.get(wheel))
            .map(|wh| wh.spin_angle)
            .unwrap_or(0.0)
    }

    /// Steering angle of a wheel (radians, around the kingpin axis).
    pub fn wheel_steering_angle(&self, id: VehicleId, wheel: usize) -> f64 {
        self.vehicles
            .get(&id)
            .and_then(|v| v.handles.wheels.get(wheel))
            .map(|wh| wh.last_steering)
            .unwrap_or(0.0)
    }

    pub fn vehicles(&self) -> impl Iterator<Item = (VehicleId, &VehicleState)> {
        self.vehicles.iter().map(|(id, v)| (*id, v))
    }

    pub fn vehicle(&self, id: VehicleId) -> Option<&VehicleState> {
        self.vehicles.get(&id)
    }

    /// Mutable access to a vehicle's state.
    pub fn vehicle_mut(&mut self, id: VehicleId) -> Option<&mut VehicleState> {
        self.vehicles.get_mut(&id)
    }

    /// Set a vehicle's total mass, updating the rapier rigid body's
    /// mass properties in-place so the physics reflects the change on
    /// the next tick.
    pub fn set_vehicle_mass(&mut self, id: VehicleId, mass: f64) {
        let Some(state) = self.vehicles.get_mut(&id) else {
            return;
        };
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

    /// Re-seat each vehicle's wheels under its chassis without advancing
    /// physics. Used when the editor drags a chassis while the sim clock
    /// is paused — without this the wheels would lag the dragged body.
    pub fn refresh_kinematics(&mut self) {
        let ids: Vec<VehicleId> = self.vehicles.keys().copied().collect();
        for id in ids {
            self.snap_wheels_to_chassis(id);
        }
    }

    /// Advance the simulation by `dt` seconds.
    pub fn step(&mut self, dt: f64) {
        self.integration.dt = dt;

        // 1. Power reservoir drain + container auto-fill. Horizontal
        //    speed only — the vertical (suspension) component would
        //    otherwise count a parked vehicle as "moving".
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

        // 2. Drive controllers → per-wheel commands → joints + wheels.
        //    Each controller sees the physics only through the narrow
        //    `BodyProxy` (chassis) + `WheelsProxy` (wheel snapshots +
        //    commands). The commands are applied to the joints / wheel
        //    bodies afterwards, which keeps the chassis borrow disjoint
        //    from the wheel-body borrow.
        for v in self.vehicles.values_mut() {
            let controller = drive::controller_for(v.spec.drive_mode);

            // Snapshot each wheel: last-commanded steering + the
            // suspension spring load (a proxy for the normal force,
            // read off the prismatic motor's last impulse).
            let mut snap: Vec<WheelSnapshot> = Vec::with_capacity(v.handles.wheels.len());
            for wh in &v.handles.wheels {
                let normal_force = self
                    .impulse_joints
                    .get(wh.joint_a)
                    .map(|j| j.data.motors[JointAxis::LinX as usize].impulse.abs() / dt)
                    .unwrap_or(0.0);
                snap.push(WheelSnapshot {
                    steering: wh.last_steering,
                    normal_force,
                });
            }
            let mut cmd = vec![WheelCommand::default(); v.handles.wheels.len()];

            // Run the controller.
            {
                let VehicleState {
                    spec,
                    control,
                    handles,
                } = v;
                let Some(rb) = self.bodies.get_mut(handles.body) else {
                    continue;
                };
                let mut ctx = DriveContext {
                    dt,
                    gravity: self.gravity,
                    spec,
                    control: *control,
                    body: BodyProxy::new(rb),
                    wheels: WheelsProxy::new(&snap, &mut cmd),
                };
                controller.apply(&mut ctx);
            }

            // Apply the commands.
            for (i, wh) in v.handles.wheels.iter_mut().enumerate() {
                let c = cmd[i];

                // Steering — stiff position motor on joint A's kingpin DOF.
                if wh.steered {
                    if let Some(j) = self.impulse_joints.get_mut(wh.joint_a, true) {
                        j.data.set_motor_position(
                            JointAxis::AngX,
                            c.steering,
                            STEER_STIFFNESS,
                            STEER_DAMPING,
                        );
                    }
                    wh.last_steering = c.steering;
                }

                // Brake — velocity-0 motor on joint B, capped at the
                // commanded brake torque (0 ⇒ motor inert).
                if let Some(j) = self.impulse_joints.get_mut(wh.joint_b, true) {
                    j.data
                        .set_motor_velocity(JointAxis::AngX, 0.0, BRAKE_DAMPING);
                    j.data
                        .set_motor_max_force(JointAxis::AngX, c.brake.max(0.0));
                }

                // Drive — engine torque about the axle on the wheel
                // body. Real contact friction decides grip vs slip.
                if let Some(wrb) = self.bodies.get_mut(wh.wheel) {
                    let world_axle = (*wrb.rotation()) * wh.axle_local;
                    if c.engine_force.abs() > 1.0e-9 {
                        wrb.add_torque(world_axle * (c.engine_force * wh.radius), true);
                    }
                    // Accumulate the rolling angle for visualisers.
                    wh.spin_angle += wrb.angvel().dot(world_axle) * dt;
                }
            }
        }

        // 3. Step the physics world. `SameVehicleFilter` rejects
        //    contacts between two colliders of the same vehicle.
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
            &world::SameVehicleFilter,
            &(),
        );
    }
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
