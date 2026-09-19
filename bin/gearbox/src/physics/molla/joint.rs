use super::*;

pub(super) struct JointAccess {
    pub shared: Shared,
    pub handle: rt::JointHandle,
    pub bodies: (BodyId, BodyId),
    pub softness: Option<(f64, f64)>,
}

impl JointAccess {
    fn edit_motor(&mut self, axis: JointAxis, change: impl FnOnce(&mut jc::JointMotor)) {
        let mut world = self.shared.world();
        let axis = convert::axis(axis);
        let result = world
            .scene
            .joint_axis_settings(self.handle, axis)
            .and_then(|setting| {
                let mut motor = setting.motor.unwrap_or(jc::JointMotor {
                    model: jc::MotorModel::Acceleration,
                    ..Default::default()
                });
                change(&mut motor);
                world.scene.set_joint_motor(self.handle, axis, motor)
            });
        apply(result);
    }
}

impl Joint for JointAccess {
    fn frame1(&self) -> Pose {
        convert::pose(
            self.shared
                .world()
                .scene
                .joint(self.handle)
                .expect("joint handle")
                .frame_parent,
        )
    }
    fn frame2(&self) -> Pose {
        convert::pose(
            self.shared
                .world()
                .scene
                .joint(self.handle)
                .expect("joint handle")
                .frame_child,
        )
    }
    fn locked_axes(&self) -> JointAxes {
        JointAxes(
            self.shared
                .world()
                .scene
                .joint_locked_axes(self.handle)
                .expect("joint handle")
                .0,
        )
    }
    fn limits(&self, axis: JointAxis) -> Option<[f64; 2]> {
        self.shared
            .world()
            .scene
            .joint_axis_settings(self.handle, convert::axis(axis))
            .ok()?
            .limits
    }
    fn motor(&self, axis: JointAxis) -> Option<Motor> {
        let motor = self
            .shared
            .world()
            .scene
            .joint_axis_settings(self.handle, convert::axis(axis))
            .ok()?
            .motor?;
        if motor.mode == sim::JointTargetMode::None {
            return None;
        }
        Some(Motor {
            target_position: motor.target_position,
            target_velocity: motor.target_velocity,
            stiffness: motor.stiffness,
            damping: motor.damping,
            max_force: motor.max_force,
            model: match motor.model {
                jc::MotorModel::Force => MotorModel::Force,
                jc::MotorModel::Acceleration => MotorModel::Acceleration,
            },
        })
    }
    fn contacts_enabled(&self) -> bool {
        self.shared
            .world()
            .scene
            .joint_settings(self.handle)
            .expect("joint handle")
            .contacts_enabled
    }
    fn is_enabled(&self) -> bool {
        self.shared
            .world()
            .scene
            .joint_settings(self.handle)
            .expect("joint handle")
            .enabled
    }
}

impl JointMut for JointAccess {
    fn set_frame1(&mut self, frame: Pose) {
        let mut world = self.shared.world();
        let other = world
            .scene
            .joint(self.handle)
            .expect("joint handle")
            .frame_child;
        apply(
            world
                .scene
                .set_joint_frames(self.handle, convert::transform(frame), other),
        );
    }
    fn set_frame2(&mut self, frame: Pose) {
        let mut world = self.shared.world();
        let other = world
            .scene
            .joint(self.handle)
            .expect("joint handle")
            .frame_parent;
        apply(
            world
                .scene
                .set_joint_frames(self.handle, other, convert::transform(frame)),
        );
    }
    fn set_limits(&mut self, axis: JointAxis, limits: [f64; 2]) {
        apply(self.shared.world().scene.set_joint_limits(
            self.handle,
            convert::axis(axis),
            Some(limits),
        ));
    }
    fn set_motor_model(&mut self, axis: JointAxis, model: MotorModel) {
        self.edit_motor(axis, |m| {
            m.model = match model {
                MotorModel::Force => jc::MotorModel::Force,
                MotorModel::Acceleration => jc::MotorModel::Acceleration,
            }
        });
    }
    fn set_motor_velocity(&mut self, axis: JointAxis, target: f64, damping: f64) {
        self.edit_motor(axis, |m| {
            m.mode = sim::JointTargetMode::Velocity;
            m.target_velocity = target;
            m.stiffness = 0.0;
            m.damping = damping;
        });
    }
    fn set_motor_position(&mut self, axis: JointAxis, target: f64, stiffness: f64, damping: f64) {
        self.edit_motor(axis, |m| {
            m.mode = sim::JointTargetMode::Position;
            m.target_position = target;
            m.target_velocity = 0.0;
            m.stiffness = stiffness;
            m.damping = damping;
        });
    }
    fn set_motor(
        &mut self,
        axis: JointAxis,
        target_position: f64,
        target_velocity: f64,
        stiffness: f64,
        damping: f64,
    ) {
        self.edit_motor(axis, |m| {
            m.mode = sim::JointTargetMode::PositionVelocity;
            m.target_position = target_position;
            m.target_velocity = target_velocity;
            m.stiffness = stiffness;
            m.damping = damping;
        });
    }
    fn set_motor_max_force(&mut self, axis: JointAxis, max_force: f64) {
        self.edit_motor(axis, |m| m.max_force = max_force);
    }
    fn set_softness(&mut self, natural_frequency: f64, damping_ratio: f64) {
        self.softness = Some((natural_frequency, damping_ratio));
        bevy::log::warn!(
            "molla: compliant joint capture is not implemented; this joint remains rigid"
        );
    }
    fn set_contacts_enabled(&mut self, enabled: bool) {
        let mut world = self.shared.world();
        let mut settings = world
            .scene
            .joint_settings(self.handle)
            .expect("joint handle");
        settings.contacts_enabled = enabled;
        apply(world.scene.set_joint_settings(self.handle, settings));
    }
}
