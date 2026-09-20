use super::*;

pub(super) struct BodyAccess {
    pub shared: Shared,
    pub handle: rt::BodyHandle,
    pub entity: Option<Entity>,
}

impl Body for BodyAccess {
    fn position(&self) -> Pose {
        convert::pose(
            self.shared
                .world()
                .scene
                .body_pose(self.handle)
                .expect("body handle"),
        )
    }
    fn linvel(&self) -> DVec3 {
        self.shared
            .world()
            .scene
            .body_velocity(self.handle)
            .expect("body handle")
            .linear
    }
    fn angvel(&self) -> DVec3 {
        self.shared
            .world()
            .scene
            .body_velocity(self.handle)
            .expect("body handle")
            .angular
    }
    fn mass(&self) -> f64 {
        self.shared
            .world()
            .scene
            .body_mass_properties(self.handle)
            .expect("body handle")
            .mass
    }
    fn center_of_mass(&self) -> DVec3 {
        self.shared
            .world()
            .scene
            .body_world_com(self.handle)
            .expect("body handle")
    }
    fn local_center_of_mass(&self) -> DVec3 {
        self.shared
            .world()
            .scene
            .body_mass_properties(self.handle)
            .expect("body handle")
            .local_com
    }
    fn principal_inertia(&self) -> DVec3 {
        self.shared
            .world()
            .scene
            .body_mass_properties(self.handle)
            .expect("body handle")
            .principal()
            .expect("valid inertia")
            .0
    }
    fn inertia_tensor(&self) -> DMat3 {
        self.shared
            .world()
            .scene
            .body_mass_properties(self.handle)
            .expect("body handle")
            .inertia
    }
    fn kind(&self) -> BodyKind {
        match self
            .shared
            .world()
            .scene
            .body(self.handle)
            .expect("body handle")
            .kind
        {
            rt::BodyKind::Dynamic => BodyKind::Dynamic,
            rt::BodyKind::Fixed => BodyKind::Fixed,
            rt::BodyKind::Kinematic => BodyKind::Kinematic,
        }
    }
    fn is_enabled(&self) -> bool {
        self.shared
            .world()
            .scene
            .body(self.handle)
            .expect("body handle")
            .enabled
    }
    fn is_sleeping(&self) -> bool {
        self.shared.world().is_sleeping(self.handle)
    }
    fn colliders(&self) -> Vec<ColliderId> {
        self.shared
            .world()
            .scene
            .body_colliders(self.handle)
            .map(|c| ColliderId(c.to_bits()))
            .collect()
    }
    fn entity(&self) -> Option<Entity> {
        self.entity
    }
}

impl BodyMut for BodyAccess {
    fn set_position(&mut self, pose: Pose, _wake: bool) {
        apply(
            self.shared
                .world()
                .scene
                .set_body_pose(self.handle, convert::transform(pose)),
        );
    }
    fn set_linvel(&mut self, linvel: DVec3, _wake: bool) {
        let mut world = self.shared.world();
        let mut velocity = world.scene.body_velocity(self.handle).expect("body handle");
        velocity.linear = linvel;
        apply(world.scene.set_body_velocity(self.handle, velocity));
    }
    fn set_angvel(&mut self, angvel: DVec3, _wake: bool) {
        let mut world = self.shared.world();
        let mut velocity = world.scene.body_velocity(self.handle).expect("body handle");
        velocity.angular = angvel;
        apply(world.scene.set_body_velocity(self.handle, velocity));
    }
    fn set_kind(&mut self, kind: BodyKind, _wake: bool) {
        apply(
            self.shared
                .world()
                .scene
                .set_body_kind(self.handle, convert::body_kind(kind)),
        );
    }
    fn set_enabled(&mut self, enabled: bool) {
        apply(
            self.shared
                .world()
                .scene
                .set_body_enabled(self.handle, enabled),
        );
    }
    fn enable_ccd(&mut self, enabled: bool) {
        apply(self.shared.world().scene.set_body_ccd(self.handle, enabled));
    }
    fn wake_up(&mut self, _strong: bool) {
        apply(self.shared.world().wake_body(self.handle));
    }
    fn sleep(&mut self) {
        apply(self.shared.world().sleep_body(self.handle));
    }
    fn set_linear_damping(&mut self, damping: f64) {
        let mut world = self.shared.world();
        let angular = world
            .scene
            .body(self.handle)
            .expect("body handle")
            .angular_damping;
        apply(world.scene.set_body_damping(self.handle, damping, angular));
    }
    fn set_angular_damping(&mut self, damping: f64) {
        let mut world = self.shared.world();
        let linear = world
            .scene
            .body(self.handle)
            .expect("body handle")
            .linear_damping;
        apply(world.scene.set_body_damping(self.handle, linear, damping));
    }
    fn set_additional_mass(&mut self, props: MassProps, _wake: bool) {
        apply(
            self.shared
                .world()
                .set_body_additional_mass(self.handle, convert::mass(props)),
        );
    }
    fn add_force(&mut self, force: DVec3, _wake: bool) {
        apply(
            self.shared
                .world()
                .scene
                .add_body_wrench(self.handle, SpatialVector::new(force, DVec3::ZERO)),
        );
    }
    fn add_torque(&mut self, torque: DVec3, _wake: bool) {
        apply(
            self.shared
                .world()
                .scene
                .add_body_wrench(self.handle, SpatialVector::new(DVec3::ZERO, torque)),
        );
    }
    fn reset_forces(&mut self, _wake: bool) {
        apply(self.shared.world().scene.reset_body_forces(self.handle));
    }
    fn apply_impulse(&mut self, impulse: DVec3, _wake: bool) {
        apply(
            self.shared
                .world()
                .apply_body_impulse(self.handle, SpatialVector::new(impulse, DVec3::ZERO)),
        );
    }
    fn apply_torque_impulse(&mut self, impulse: DVec3, _wake: bool) {
        apply(
            self.shared
                .world()
                .apply_body_impulse(self.handle, SpatialVector::new(DVec3::ZERO, impulse)),
        );
    }
    fn apply_impulse_at_point(&mut self, impulse: DVec3, point: DVec3, _wake: bool) {
        apply(
            self.shared
                .world()
                .apply_body_impulse_at_point(self.handle, impulse, point),
        );
    }
}
