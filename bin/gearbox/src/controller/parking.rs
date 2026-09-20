use super::*;
use crate::physics::backend::JointId;

const HOLD_COMPLIANCE_RAD: f64 = 0.01;
const HOLD_DAMPING_SPEED_RAD_S: f64 = 0.25;
const MAX_HOLD_ERROR_RAD: f64 = 0.25;

#[derive(Debug, Default, Clone)]
pub(super) struct ParkingBrakes {
    holds: HashMap<JointId, Hold>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::backend::{BodyDesc, JointDesc, JointKind};
    use crate::physics::{MollaBackend, PhysicsWorld, RapierBackend};

    fn fixture(molla: bool) -> (PhysicsWorld, JointVelocityTarget, JointId) {
        let mut physics = PhysicsWorld::with_backend(if molla {
            Box::new(MollaBackend::default())
        } else {
            Box::new(RapierBackend::default())
        });
        physics.set_gravity(DVec3::ZERO);
        let parent = physics.insert_body(BodyDesc::fixed());
        let mut desc = BodyDesc::dynamic();
        desc.additional_mass = Some(MassProps {
            mass: 10.0,
            local_com: DVec3::ZERO,
            inertia: Inertia::Principal(DVec3::splat(2.0)),
        });
        let wheel = physics.insert_body(desc);
        let joint = physics.insert_joint(
            parent,
            wheel,
            JointDesc::new(
                JointKind::Revolute { axis: DVec3::Z },
                Pose::IDENTITY,
                Pose::IDENTITY,
            ),
        );
        (
            physics,
            JointVelocityTarget {
                pair: (parent, wheel),
                velocity: 4.0,
                damping: 100.0,
                max_torque: 100.0,
                force_based: true,
            },
            joint,
        )
    }

    fn tick(
        physics: &mut PhysicsWorld,
        brakes: &mut ParkingBrakes,
        target: JointVelocityTarget,
        parked: bool,
    ) {
        let holds = brakes.prepare(physics, &[target], parked);
        apply_joint_motors_with_holds(physics, &[target], &[], None, &holds);
        physics.step();
        assert!(physics.quarantined.is_empty());
    }

    #[test]
    fn hold_captures_unwrapped_angle_resists_load_and_releases() {
        let (mut physics, mut target, joint) = fixture(true);
        let mut brakes = ParkingBrakes::default();
        for _ in 0..360 {
            tick(&mut physics, &mut brakes, target, false);
        }
        let captured = physics
            .joint(joint)
            .unwrap()
            .motor_position(JointAxis::AngX)
            .unwrap();
        assert_eq!(
            physics
                .joint(joint)
                .unwrap()
                .motor_position(JointAxis::LinX),
            None
        );
        assert!(captured > std::f64::consts::TAU);
        target.velocity = 0.0;
        physics
            .body_mut(target.pair.1)
            .unwrap()
            .add_torque(DVec3::Z * 20.0, true);
        for _ in 0..600 {
            tick(&mut physics, &mut brakes, target, true);
        }
        let held = physics
            .joint(joint)
            .unwrap()
            .motor_position(JointAxis::AngX)
            .unwrap();
        assert!((held - captured).abs() < 0.01, "hold {held} != {captured}");
        assert!(physics.body(target.pair.1).unwrap().angvel().length() < 1e-5);
        let motor = physics
            .joint(joint)
            .unwrap()
            .motor(JointAxis::AngX)
            .unwrap();
        assert_eq!(motor.target_position, captured);
        assert_eq!(motor.max_force, target.max_torque);
        physics.body_mut(target.pair.1).unwrap().reset_forces(true);
        target.velocity = -2.0;
        for _ in 0..120 {
            tick(&mut physics, &mut brakes, target, false);
        }
        assert!(brakes.holds.is_empty());
        assert_eq!(
            physics
                .joint(joint)
                .unwrap()
                .motor(JointAxis::AngX)
                .unwrap()
                .stiffness,
            0.0
        );
        assert!(physics.body(target.pair.1).unwrap().angvel().z < -1.9);
        let position = physics
            .joint(joint)
            .unwrap()
            .motor_position(JointAxis::AngX)
            .unwrap();
        target.velocity = 0.0;
        brakes.apply(&mut physics, &[target], true);
        assert_eq!(brakes.holds[&joint].target, position);
        physics.remove_joint(joint);
        brakes.apply(&mut physics, &[], true);
        assert!(brakes.holds.is_empty());
    }

    #[test]
    fn overload_slips_with_bounded_reference_instead_of_locking() {
        let (mut physics, mut target, joint) = fixture(true);
        let mut brakes = ParkingBrakes::default();
        target.velocity = 0.0;
        physics
            .body_mut(target.pair.1)
            .unwrap()
            .add_torque(DVec3::Z * 150.0, true);
        for _ in 0..120 {
            tick(&mut physics, &mut brakes, target, true);
        }
        let position = physics
            .joint(joint)
            .unwrap()
            .motor_position(JointAxis::AngX)
            .unwrap();
        brakes.apply(&mut physics, &[target], true);
        assert!(position > 5.0);
        assert!((position - brakes.holds[&joint].target).abs() <= MAX_HOLD_ERROR_RAD + 1e-12);
        assert!(physics.body(target.pair.1).unwrap().angvel().z > 20.0);
        target.max_torque = 0.0;
        tick(&mut physics, &mut brakes, target, true);
        assert!(brakes.holds.is_empty());
        assert_eq!(
            physics
                .joint(joint)
                .unwrap()
                .motor(JointAxis::AngX)
                .unwrap()
                .stiffness,
            0.0
        );
    }

    #[test]
    fn unavailable_coordinate_keeps_existing_velocity_brake() {
        let (mut physics, mut target, joint) = fixture(false);
        let mut brakes = ParkingBrakes::default();
        target.velocity = 0.0;
        tick(&mut physics, &mut brakes, target, true);
        assert!(brakes.holds.is_empty());
        let motor = physics
            .joint(joint)
            .unwrap()
            .motor(JointAxis::AngX)
            .unwrap();
        assert_eq!(motor.stiffness, 0.0);
        assert_eq!(motor.damping, target.damping);
    }

    #[test]
    fn unchanged_hold_does_not_wake_and_releasing_it_resumes_rotation() {
        let (mut physics, mut target, joint) = fixture(true);
        let mut brakes = ParkingBrakes::default();
        target.velocity = 0.0;
        tick(&mut physics, &mut brakes, target, true);
        physics.body_mut(target.pair.1).unwrap().sleep();
        for _ in 0..20 {
            tick(&mut physics, &mut brakes, target, true);
            assert!(physics.body(target.pair.1).unwrap().is_sleeping());
            assert_eq!(physics.joint(joint).unwrap().motor(JointAxis::AngX).unwrap().target_position, brakes.holds[&joint].target);
        }
        target.velocity = 2.0;
        wake_vehicle_for_command(&mut physics, target.pair.1, &[target.pair],
            CmdVel { linear_mps: 2.0, angular_rps: 0.0 }, 0.0);
        tick(&mut physics, &mut brakes, target, false);
        assert!(!physics.body(target.pair.1).unwrap().is_sleeping());
        assert!(physics.body(target.pair.1).unwrap().angvel().z > 0.0);
    }
}

#[derive(Debug, Clone)]
struct Hold {
    target: f64,
    previous: f64,
}

impl ParkingBrakes {
    #[cfg(test)]
    pub(super) fn apply(
        &mut self,
        physics: &mut crate::physics::PhysicsWorld,
        targets: &[JointVelocityTarget],
        parked: bool,
    ) {
        let holds = self.prepare(physics, targets, parked);
        apply_holds(physics, &holds);
    }

    /// Capture bounded position commands for eligible wheel joints.
    pub(super) fn prepare(
        &mut self,
        physics: &crate::physics::PhysicsWorld,
        targets: &[JointVelocityTarget],
        parked: bool,
    ) -> Vec<HoldTarget> {
        let mut commands = Vec::new();
        self.holds.retain(|id, _| physics.joint(*id).is_some());
        for target in targets {
            for id in physics.joints_between(target.pair.0, target.pair.1) {
                if !parked
                    || !target.max_torque.is_finite()
                    || target.max_torque <= 0.0
                    || !target.force_based
                {
                    self.holds.remove(&id);
                    continue;
                }
                let Some(position) = physics
                    .joint(id)
                    .and_then(|joint| joint.motor_position(JointAxis::AngX))
                    .filter(|p| p.is_finite())
                else {
                    self.holds.remove(&id);
                    continue;
                };
                let hold = self.holds.entry(id).or_insert(Hold {
                    target: position,
                    previous: position,
                });
                if (position - hold.previous).abs() > std::f64::consts::PI {
                    hold.target = position;
                }
                hold.previous = position;
                hold.target = hold
                    .target
                    .clamp(position - MAX_HOLD_ERROR_RAD, position + MAX_HOLD_ERROR_RAD);
                commands.push(HoldTarget { joint: id, position: hold.target, max_torque: target.max_torque });
            }
        }
        commands
    }
}

pub(super) struct HoldTarget {
    pub joint: JointId,
    position: f64,
    max_torque: f64,
}

pub(super) fn apply_holds(physics: &mut crate::physics::PhysicsWorld, holds: &[HoldTarget]) -> bool {
    let mut applied = false;
    for hold in holds {
        if let Some(joint) = physics.joint_mut(hold.joint, false) {
            joint.set_motor_model(JointAxis::AngX, MotorModel::Force);
            joint.set_motor_position(JointAxis::AngX, hold.position,
                hold.max_torque / HOLD_COMPLIANCE_RAD, hold.max_torque / HOLD_DAMPING_SPEED_RAD_S);
            joint.set_motor_max_force(JointAxis::AngX, hold.max_torque);
            applied = true;
        }
    }
    applied
}
