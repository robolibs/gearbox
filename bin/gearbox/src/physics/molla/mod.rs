//! CPU Featherstone backend with stable-handle accessors.

mod body;
mod collider;
mod convert;
mod joint;
#[cfg(test)]
mod tests;
mod wheel;
mod track;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use bevy::prelude::Entity;
use molla_math::{SpatialVector, Transform};
use molla_sim::{self as sim, joint_control as jc, runtime as rt};
use molla_solvers::rigid_world::RigidWorld;

use super::backend::*;
use body::BodyAccess;
use collider::ColliderAccess;
use joint::JointAccess;

#[derive(Clone)]
struct Shared(Arc<Mutex<RigidWorld>>);

impl Shared {
    fn world(&self) -> MutexGuard<'_, RigidWorld> {
        self.0.lock().expect("Molla world lock poisoned")
    }
}

fn apply(result: molla_core::Result<()>) {
    if let Err(error) = result {
        bevy::log::error!("molla: edit rejected: {error}");
    }
}

pub struct MollaBackend {
    shared: Shared,
    bodies: BTreeMap<BodyId, BodyAccess>,
    colliders: BTreeMap<ColliderId, ColliderAccess>,
    joints: BTreeMap<JointId, JointAccess>,
    settings: SolverSettings,
    wheels: BTreeMap<BodyId, (JointId, f64)>,
    wheel_step_dt: f64,
}

impl Default for MollaBackend {
    fn default() -> Self {
        let mut world = RigidWorld::new().expect("Molla world initialization");
        world.set_sleep_settings(Some(molla_solvers::rigid_world::SleepSettings::default()))
            .expect("Molla sleep settings");
        Self {
            shared: Shared(Arc::new(Mutex::new(world))),
            bodies: BTreeMap::new(),
            colliders: BTreeMap::new(),
            joints: BTreeMap::new(),
            wheels: BTreeMap::new(),
            wheel_step_dt: 1.0 / 120.0,
            settings: SolverSettings {
                dt: 1.0 / 120.0,
                solver_iterations: 16,
                internal_iterations: 4,
            },
        }
    }
}

impl PhysicsBackend for MollaBackend {
    fn configure_track(&mut self, desc: TrackForceDesc) -> Result<(), String> {
        self.configure_track_force(desc)
    }

    fn set_track_speed(&mut self, sprocket: BodyId, speed: f64) -> Result<(), String> {
        let handle = self.bodies.get(&sprocket).ok_or("unknown track sprocket")?.handle;
        self.shared.world().tracks.set_speed(handle, speed).map_err(|e| e.to_string())
    }

    fn track_output(&self, sprocket: BodyId) -> Option<TrackForceOutput> {
        let handle = self.bodies.get(&sprocket)?.handle;
        let world = self.shared.world();
        let s = world.tracks.sample(&world.scene, handle)?;
        Some(TrackForceOutput {
            angular_velocity: s.angular_velocity, travel: s.travel, motor_torque: s.motor_torque,
            longitudinal_force: s.longitudinal_force, normal_load: s.normal_load, contacts: s.contacts,
        })
    }

    fn remove_track(&mut self, sprocket: BodyId) {
        if let Some(body) = self.bodies.get(&sprocket) {
            self.shared.world().tracks.remove(body.handle);
        }
    }

    fn uses_wheel_forces(&self) -> bool {
        true
    }

    fn configure_wheel(&mut self, desc: WheelForceDesc) -> Result<(), String> {
        self.configure_wheel_force(desc)
    }

    fn register_wheel_ground(
        &mut self,
        collider: ColliderId,
        friction: Option<TerrainFrictionGrid>,
    ) -> Result<(), String> {
        let handle = self
            .colliders
            .get(&collider)
            .ok_or("unknown tyre terrain")?
            .handle;
        let mut world = self.shared.world();
        let RigidWorld { scene, wheels, .. } = &mut *world;
        wheels
            .register_ground(
                scene,
                handle,
                friction.map(|grid| molla_solvers::wheel_forces::FrictionGrid {
                    origin: grid.origin,
                    cell_size: grid.cell_size,
                    cols: grid.cols,
                    rows: grid.rows,
                    values: grid.values,
                }),
            )
            .map_err(|error| error.to_string())
    }

    fn wheel_output(&self, body: BodyId) -> Option<WheelForceOutput> {
        self.wheel_force_output(body)
    }

    fn set_wheel_pressures(&mut self, targets: &[(BodyId, f64)]) -> Result<(), String> {
        let targets: Result<Vec<_>, _> = targets
            .iter()
            .map(|&(body, pressure)| {
                self.bodies
                    .get(&body)
                    .map(|body| (body.handle, pressure))
                    .ok_or_else(|| "unknown tyre body".to_string())
            })
            .collect();
        let mut world = self.shared.world();
        let RigidWorld { scene, wheels, .. } = &mut *world;
        wheels
            .set_pressures(scene, &targets?)
            .map_err(|e| e.to_string())
    }

    fn wheel_drive_sign(&self, joint: JointId) -> f64 {
        self.wheels
            .values()
            .find(|(id, _)| *id == joint)
            .map_or(1.0, |(_, sign)| *sign)
    }

    fn set_body_poses(
        &mut self,
        poses: &[(BodyId, Pose)],
        reset_velocity: bool,
    ) -> Result<(), String> {
        let poses: Result<Vec<_>, _> = poses
            .iter()
            .map(|&(id, pose)| {
                self.bodies
                    .get(&id)
                    .map(|body| (body.handle, convert::transform(pose)))
                    .ok_or_else(|| "unknown body in pose batch".to_string())
            })
            .collect();
        self.shared
            .world()
            .scene
            .set_body_poses(&poses?, reset_velocity)
            .map_err(|e| e.to_string())
    }
    fn name(&self) -> &'static str {
        "molla"
    }
    fn with_molla_scene(&self, f: &mut dyn FnMut(&rt::RigidScene)) -> bool {
        f(&self.shared.world().scene);
        true
    }
    fn molla_body_handle(&self, body: BodyId) -> Option<rt::BodyHandle> {
        self.bodies.get(&body).map(|body| body.handle)
    }
    fn gravity(&self) -> DVec3 {
        self.shared.world().scene.gravity()
    }
    fn set_gravity(&mut self, gravity: DVec3) {
        apply(self.shared.world().scene.set_gravity(gravity));
    }
    fn settings(&self) -> SolverSettings {
        self.settings
    }
    fn set_settings(&mut self, settings: SolverSettings) {
        if !settings.dt.is_finite() || settings.dt <= 0.0 {
            bevy::log::error!("molla: invalid solver dt");
            return;
        }
        self.shared.world().solver.substeps = settings.internal_iterations.clamp(1, 64) * 2;
        self.settings = settings;
    }
    fn step(&mut self, excluded: PairExcluded<'_>) {
        self.wheel_step_dt = self.settings.dt;
        let result = self.shared.world().step_filtered(self.settings.dt, |a, b| {
            a.zip(b)
                .is_some_and(|(a, b)| excluded(BodyId(a.to_bits()), BodyId(b.to_bits())))
        });
        if let Err(error) = result {
            bevy::log::error!("molla: step rejected: {error}");
        }
    }
    fn insert_body(&mut self, desc: BodyDesc) -> BodyId {
        let entity = desc.entity;
        let sleeping = desc.sleeping;
        let mut body = rt::BodyDesc {
            kind: convert::body_kind(desc.kind),
            velocity: SpatialVector::new(desc.linvel, desc.angvel),
            linear_damping: desc.linear_damping,
            angular_damping: desc.angular_damping,
            additional_mass: Some(
                desc.additional_mass
                    .map(convert::mass)
                    .unwrap_or(sim::MassProperties::ZERO),
            ),
            user_tag: entity.map_or(0, Entity::to_bits),
            ..Default::default()
        };
        body.params.pose = convert::transform(desc.pose);
        let handle = self
            .shared
            .world()
            .scene
            .insert_body(body)
            .expect("invalid Molla body description");
        if sleeping {
            apply(self.shared.world().sleep_body(handle));
        }
        let id = BodyId(handle.to_bits());
        self.bodies.insert(
            id,
            BodyAccess {
                shared: self.shared.clone(),
                handle,
                entity,
            },
        );
        id
    }
    fn quarantined_bodies(&self) -> Vec<BodyId> {
        self.shared
            .world()
            .quarantined_bodies()
            .iter()
            .map(|body| BodyId(body.to_bits()))
            .collect()
    }
    fn remove_body(&mut self, id: BodyId) {
        if let Some(body) = self.bodies.get(&id) {
            let mut world = self.shared.world();
            if let Err(error) = world.scene.remove_body(body.handle) {
                apply(Err(error));
                return;
            }
            self.wheels.retain(|wheel, (joint, _)| {
                let handle = self.bodies[wheel].handle;
                let live = world.scene.body(handle).is_some()
                    && self.joints.get(joint).is_some_and(|j| world.scene.joint(j.handle).is_some());
                if !live { world.wheels.remove(handle); }
                live
            });
            self.bodies.remove(&id);
            self.colliders
                .retain(|_, c| world.scene.collider(c.handle).is_some());
            self.joints
                .retain(|_, j| world.scene.joint(j.handle).is_some());
        }
    }
    fn body(&self, id: BodyId) -> Option<&dyn Body> {
        self.bodies.get(&id).map(|b| b as &dyn Body)
    }
    fn body_mut(&mut self, id: BodyId) -> Option<&mut dyn BodyMut> {
        self.bodies.get_mut(&id).map(|b| b as &mut dyn BodyMut)
    }
    fn bodies(&self) -> Vec<BodyId> {
        self.bodies.keys().copied().collect()
    }
    fn recompute_mass(&mut self, id: BodyId) {
        if let Some(body) = self.bodies.get(&id) {
            apply(self.shared.world().recompute_body_mass(body.handle));
        }
    }
    fn sync_collider_positions(&mut self) {}
    fn insert_collider(&mut self, desc: ColliderDesc) -> Option<ColliderId> {
        let mut collider = rt::ColliderDesc::new(convert::geometry(desc.shape).ok()?);
        collider.parent = match desc.parent {
            Some(id) => Some(self.bodies.get(&id)?.handle),
            None => None,
        };
        collider.pose = convert::transform(desc.pose);
        collider.friction = desc.friction.unwrap_or(0.5);
        collider.restitution = desc.restitution.unwrap_or(0.0);
        collider.mass = sim::ColliderMass::Density(desc.density.unwrap_or(1.0));
        collider.contact_material.friction_combine =
            convert::combine(desc.friction_combine.unwrap_or(CombineRule::Average));
        let groups = desc.groups.unwrap_or(CollisionGroups::ALL);
        collider.memberships = groups.memberships;
        collider.filter = groups.filter;
        collider.user_tag = desc.entity.map_or(0, Entity::to_bits);
        let parent = collider.parent;
        let mut world = self.shared.world();
        let handle = world.scene.insert_collider(collider).ok()?;
        if let Some(parent) = parent {
            apply(world.recompute_body_mass(parent));
        }
        let id = ColliderId(handle.to_bits());
        self.colliders.insert(
            id,
            ColliderAccess {
                shared: self.shared.clone(),
                handle,
                entity: desc.entity,
            },
        );
        Some(id)
    }
    fn remove_collider(&mut self, id: ColliderId, _wake: bool) {
        if let Some(collider) = self.colliders.get(&id) {
            let mut world = self.shared.world();
            let parent = world.scene.collider(collider.handle).and_then(|c| c.parent);
            if let Err(error) = world.scene.remove_collider(collider.handle) {
                apply(Err(error));
                return;
            }
            self.colliders.remove(&id);
            if let Some(parent) = parent {
                apply(world.recompute_body_mass(parent));
            }
        }
    }
    fn collider(&self, id: ColliderId) -> Option<&dyn Collider> {
        self.colliders.get(&id).map(|c| c as &dyn Collider)
    }
    fn collider_mut(&mut self, id: ColliderId) -> Option<&mut dyn ColliderMut> {
        self.colliders
            .get_mut(&id)
            .map(|c| c as &mut dyn ColliderMut)
    }
    fn colliders(&self) -> Vec<ColliderId> {
        self.colliders.keys().copied().collect()
    }
    fn insert_joint(&mut self, body1: BodyId, body2: BodyId, desc: JointDesc) -> JointId {
        let parent = self.bodies[&body1].handle;
        let child = self.bodies[&body2].handle;
        let (kind, axis) = match desc.kind {
            JointKind::Revolute { axis } => (sim::JointType::Revolute, axis),
            JointKind::Prismatic { axis } => (sim::JointType::Prismatic, axis),
            JointKind::Fixed => (sim::JointType::Fixed, DVec3::X),
            JointKind::Spherical => (sim::JointType::Ball, DVec3::X),
            JointKind::Generic { .. } => (sim::JointType::D6, DVec3::X),
        };
        let handle = {
            let mut world = self.shared.world();
            let runtime = rt::JointDesc {
                parent: Some(parent),
                child,
                kind,
                axis,
                frame_parent: convert::transform(desc.frame1),
                frame_child: convert::transform(desc.frame2),
            };
            let handle = if desc.loop_closure {
                let (natural_frequency, damping_ratio) = desc.softness.unwrap_or((30.0, 1.0));
                world.scene.insert_loop_joint(
                    runtime,
                    jc::JointSoftness {
                        natural_frequency,
                        damping_ratio,
                    },
                )
            } else {
                world.scene.insert_joint(runtime)
            }
            .expect("invalid Molla joint topology");
            if let JointKind::Generic { locked } = desc.kind {
                apply(
                    world
                        .scene
                        .set_joint_locked_axes(handle, jc::JointAxes(locked.0)),
                );
            }
            apply(world.scene.set_joint_settings(
                handle,
                rt::JointSettings {
                    enabled: true,
                    contacts_enabled: desc.contacts_enabled,
                },
            ));
            handle
        };
        let id = JointId(handle.to_bits());
        let mut joint = JointAccess {
            shared: self.shared.clone(),
            handle,
            bodies: (body1, body2),
            wake_on_change: true,
        };
        for (axis, limits) in desc.limits {
            joint.set_limits(axis, limits);
        }
        for motor in desc.motors {
            if let Some(model) = motor.model {
                joint.set_motor_model(motor.axis, model);
            }
            if let Some(target) = motor.target {
                match target {
                    MotorTarget::Position {
                        target,
                        stiffness,
                        damping,
                    } => joint.set_motor_position(motor.axis, target, stiffness, damping),
                    MotorTarget::Velocity { target, damping } => {
                        joint.set_motor_velocity(motor.axis, target, damping)
                    }
                }
            }
            if let Some(max_force) = motor.max_force {
                joint.set_motor_max_force(motor.axis, max_force);
            }
        }
        if let Some((frequency, ratio)) = desc.softness {
            joint.set_softness(frequency, ratio);
        }
        self.joints.insert(id, joint);
        id
    }
    fn remove_joint(&mut self, id: JointId) {
        if let Some(joint) = self.joints.get(&id) {
            let mut world = self.shared.world();
            if let Err(error) = world.scene.remove_joint(joint.handle) {
                apply(Err(error));
                return;
            }
            self.joints.remove(&id);
            self.wheels.retain(|body, (joint, _)| {
                if *joint == id { world.wheels.remove(self.bodies[body].handle); }
                *joint != id
            });
        }
    }
    fn joint(&self, id: JointId) -> Option<&dyn Joint> {
        self.joints.get(&id).map(|j| j as &dyn Joint)
    }
    fn joint_mut(&mut self, id: JointId, wake: bool) -> Option<&mut dyn JointMut> {
        self.joints.get_mut(&id).map(|joint| {
            joint.wake_on_change = wake;
            joint as &mut dyn JointMut
        })
    }
    fn joint_bodies(&self, id: JointId) -> Option<(BodyId, BodyId)> {
        self.joints.get(&id).map(|j| j.bodies)
    }
    fn joints(&self) -> Vec<JointId> {
        self.joints.keys().copied().collect()
    }
    fn joint_is_reduced(&self, id: JointId) -> bool {
        self.joints.contains_key(&id)
    }
    fn cast_ray_filtered(
        &self,
        origin: DVec3,
        direction: DVec3,
        max_distance: f64,
        include: &dyn Fn(ColliderId) -> bool,
    ) -> Option<(ColliderId, f64)> {
        self.shared
            .world()
            .cast_ray_filtered(origin, direction, max_distance, |h| include(ColliderId(h.to_bits())))
            .ok()
            .flatten()
            .map(|h| (ColliderId(h.collider.to_bits()), h.distance))
    }
    fn contacts_with(&self, collider: ColliderId) -> Vec<ContactManifold> {
        self.contacts()
            .into_iter()
            .filter(|c| c.collider1 == collider || c.collider2 == collider)
            .collect()
    }
    fn contacts(&self) -> Vec<ContactManifold> {
        self.shared
            .world()
            .contact_pairs()
            .into_iter()
            .flat_map(|pair| {
                let collider1 = ColliderId(pair.colliders[0].to_bits());
                let collider2 = ColliderId(pair.colliders[1].to_bits());
                pair.points.into_iter().map(move |p| ContactManifold {
                    collider1,
                    collider2,
                    normal: p.normal,
                    active: p.acted,
                    points: vec![ContactPoint {
                        impulse: p.normal_impulse,
                        friction: if p.acted { p.friction } else { 0.0 },
                        solved: p.acted,
                        point: p.point,
                        dist: -p.penetration,
                    }],
                })
            })
            .collect()
    }
}
