//! A control body seen through its machine's declared control frame. Drive
//! controllers read chassis axes as forward −Y, left +X, up +Z; a machine
//! whose control frame says otherwise is turned into that basis here, so
//! heading, signed speed, steering geometry and parking all follow it.

use glam::{DMat3, DQuat, DVec3};

use super::backend::{Body, BodyId, BodyKind, ColliderId, Pose};
use super::world::PhysicsWorld;

/// The chassis axes the drive controllers read, in the control frame's axes:
/// control +X (forward) is chassis −Y, control +Y (left) is chassis +X.
pub fn controller_basis() -> DQuat {
    DQuat::from_rotation_z(-std::f64::consts::FRAC_PI_2)
}

/// The turn from a body's own axes to the controllers' chassis axes when its
/// control frame sits at `control` in the body.
pub fn chassis_turn(control: DQuat) -> DQuat {
    (control * controller_basis().inverse()).normalize()
}

pub struct ChassisBody<'a> {
    body: &'a dyn Body,
    turn: DQuat,
}

impl<'a> ChassisBody<'a> {
    pub fn new(body: &'a dyn Body, turn: DQuat) -> Self {
        Self { body, turn }
    }
}

impl Body for ChassisBody<'_> {
    fn position(&self) -> Pose {
        let pose = self.body.position();
        Pose::new(pose.translation, (pose.rotation * self.turn).normalize())
    }
    fn linvel(&self) -> DVec3 {
        self.body.linvel()
    }
    fn angvel(&self) -> DVec3 {
        self.body.angvel()
    }
    fn mass(&self) -> f64 {
        self.body.mass()
    }
    fn center_of_mass(&self) -> DVec3 {
        self.body.center_of_mass()
    }
    fn local_center_of_mass(&self) -> DVec3 {
        self.turn.inverse() * self.body.local_center_of_mass()
    }
    fn principal_inertia(&self) -> DVec3 {
        self.body.principal_inertia()
    }
    fn inertia_tensor(&self) -> DMat3 {
        let r = DMat3::from_quat(self.turn);
        r.transpose() * self.body.inertia_tensor() * r
    }
    fn kind(&self) -> BodyKind {
        self.body.kind()
    }
    fn is_enabled(&self) -> bool {
        self.body.is_enabled()
    }
    fn is_sleeping(&self) -> bool {
        self.body.is_sleeping()
    }
    fn colliders(&self) -> Vec<ColliderId> {
        self.body.colliders()
    }
    fn entity(&self) -> Option<bevy::prelude::Entity> {
        self.body.entity()
    }
}

impl PhysicsWorld {
    /// A control body in the drive controllers' chassis axes.
    pub fn chassis(&self, id: BodyId) -> Option<ChassisBody<'_>> {
        Some(ChassisBody { body: self.body(id)?, turn: self.chassis_turn(id) })
    }

    /// The turn into chassis axes of a control body; identity without a control frame.
    pub fn chassis_turn(&self, id: BodyId) -> DQuat {
        self.chassis_turns.get(&id).copied().unwrap_or(DQuat::IDENTITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::backend::BodyDesc;

    /// A chassis authored with +X forward and an identity control frame
    /// heads where its +X points; with no turn, where the controllers' −Y points.
    #[test]
    fn the_control_frame_sets_the_chassis_forward() {
        let mut world = PhysicsWorld::default();
        let yaw = DQuat::from_rotation_y(0.7);
        let z_up = DQuat::from_rotation_x(-std::f64::consts::FRAC_PI_2);
        let body = world.insert_body(BodyDesc::dynamic().pose(Pose::new(DVec3::ZERO, yaw * z_up)));
        let level = |v: DVec3| DVec3::new(v.x, 0.0, v.z).normalize();
        let unturned = crate::controller::body_forward_vector(&world.chassis(body).unwrap()).unwrap();
        assert!((unturned - level(yaw * z_up * DVec3::NEG_Y)).length() < 1e-9, "{unturned:?}");
        world.chassis_turns.insert(body, chassis_turn(DQuat::IDENTITY));
        let forward = crate::controller::body_forward_vector(&world.chassis(body).unwrap()).unwrap();
        assert!((forward - level(yaw * DVec3::X)).length() < 1e-9, "{forward:?}");
        assert!(chassis_turn(controller_basis()).angle_between(DQuat::IDENTITY) < 1e-12);
    }
}
