use super::*;
use rapier3d::prelude::GenericJoint;

struct SteeredWheel {
    pair: (RigidBodyHandle, RigidBodyHandle),
    center: Vector,
    sign: f64,
    lower: f64,
    upper: f64,
    authored_drive: bool,
}

pub(super) struct Turn {
    pivot_y: f64,
    curvature: f64,
    wheels: Vec<SteeredWheel>,
}

impl Turn {
    pub(super) fn configure_servos(
        &self,
        physics: &mut crate::physics::PhysicsWorld,
        cap: Option<f64>,
    ) {
        for wheel in self.wheels.iter().filter(|wheel| !wheel.authored_drive) {
            let target = wheel.sign * angle(self.curvature, self.pivot_y, wheel.center);
            if let Some(handle) = multibody_joint_handle(physics, wheel.pair)
                && let Some((body, id)) = physics.multibody_joints.get_mut(handle)
                && let Some(link) = body.link_mut(id)
            {
                configure_fallback_steer_motor(&mut link.joint.data, target, cap);
            }
            for (_, joint) in physics.impulse_joints.iter_mut() {
                if rigid_body_pair_matches(wheel.pair, joint.body1, joint.body2) {
                    configure_fallback_steer_motor(&mut joint.data, target, cap);
                }
            }
        }
    }

    pub(super) fn trace(
        &self,
        physics: &crate::physics::PhysicsWorld,
        chassis: RigidBodyHandle,
        machine: &str,
        drive: &[JointVelocityTarget],
    ) {
        let angles: Vec<_> = self
            .wheels
            .iter()
            .filter_map(|wheel| {
                let (data, parent) = joint_data(physics, wheel.pair)?;
                let child = if parent == wheel.pair.0 {
                    wheel.pair.1
                } else {
                    wheel.pair.0
                };
                let a = physics.bodies.get(parent)?.rotation() * data.local_frame1.rotation;
                let b = physics.bodies.get(child)?.rotation() * data.local_frame2.rotation;
                let actual = (a.inverse() * b).to_scaled_axis().x.to_degrees();
                Some((
                    wheel.center.x,
                    wheel.center.y,
                    (wheel.sign * angle(self.curvature, self.pivot_y, wheel.center)).to_degrees(),
                    actual,
                ))
            })
            .collect();
        let speeds: Vec<_> = drive
            .iter()
            .filter_map(|target| {
                let wheel = wheel_body_of(physics, chassis, target.pair)?;
                let body = physics.bodies.get(wheel)?;
                let parent = if wheel == target.pair.0 {
                    target.pair.1
                } else {
                    target.pair.0
                };
                let (axis, _, radius) = body_tyre_geometry(physics, wheel)?;
                let spin = (body.angvel() - physics.bodies.get(parent)?.angvel())
                    .dot(body.rotation() * axis);
                let center = wheel_local_center(physics, chassis, target.pair)?;
                let support = traction::wheel_support(physics, chassis, wheel);
                let p = body.translation();
                let gap =
                    p.y - radius - crate::globe::ground_height_at_physics(p.x, p.z);
                Some((
                    center.x,
                    center.y,
                    target.velocity,
                    spin,
                    support.grip_force_n,
                    gap,
                ))
            })
            .collect();
        info!(
            "gearbox-steering {machine}: pivot_y={:.3} curvature={:.5} (x,y,target_deg,actual_deg)={angles:?} wheels(x,y,target_rad_s,actual_rad_s,grip_n,approx_gap_m)={speeds:?}",
            self.pivot_y, self.curvature
        );
    }

    pub(super) fn targets(&self) -> Vec<JointPositionTarget> {
        self.wheels
            .iter()
            .map(|wheel| JointPositionTarget {
                pair: wheel.pair,
                position: wheel.sign * angle(self.curvature, self.pivot_y, wheel.center),
            })
            .collect()
    }

    pub(super) fn speed(&self, linear: f64, center: Vector) -> f64 {
        let (forward, lateral) = rolling_vector(self.curvature, self.pivot_y, center);
        linear * forward.hypot(lateral)
    }
}

fn rolling_vector(curvature: f64, pivot_y: f64, center: Vector) -> (f64, f64) {
    (1.0 - curvature * center.x, curvature * (pivot_y - center.y))
}

fn angle(curvature: f64, pivot_y: f64, center: Vector) -> f64 {
    let (forward, lateral) = rolling_vector(curvature, pivot_y, center);
    lateral.atan2(forward)
}

fn limit_curvature(requested: f64, pivot_y: f64, wheels: &[SteeredWheel]) -> f64 {
    let fits = |curvature| {
        wheels.iter().all(|wheel| {
            let target = wheel.sign * angle(curvature, pivot_y, wheel.center);
            target >= wheel.lower && target <= wheel.upper
        })
    };
    if fits(requested) {
        return requested;
    }
    let (mut low, mut high) = (0.0, 1.0);
    for _ in 0..40 {
        let mid = (low + high) * 0.5;
        if fits(requested * mid) {
            low = mid;
        } else {
            high = mid;
        }
    }
    requested * low
}

fn joint_data(
    physics: &crate::physics::PhysicsWorld,
    pair: (RigidBodyHandle, RigidBodyHandle),
) -> Option<(&GenericJoint, RigidBodyHandle)> {
    if let Some(handle) = multibody_joint_handle(physics, pair)
        && let Some((body, id)) = physics.multibody_joints.get(handle)
        && let Some(link) = body.link(id)
    {
        return Some((&link.joint.data, pair.0));
    }
    physics.impulse_joints.iter().find_map(|(_, joint)| {
        rigid_body_pair_matches(pair, joint.body1, joint.body2)
            .then_some((&joint.data, joint.body1))
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn solve(
    scene_root: Entity,
    controller: &ControllerSpec,
    machine: &MachineInstanceSpec,
    joints: &Query<(
        Entity,
        &UsdPrimRef,
        &crate::physics::markers::UsdPhysicsJoint,
    )>,
    parents: &Query<&ChildOf>,
    physics: &crate::physics::PhysicsWorld,
    chassis: RigidBodyHandle,
    cmd: CmdVel,
    steering_input: Option<f32>,
) -> Option<Turn> {
    let geometry = controller
        .steering_geometry
        .as_deref()
        .unwrap_or("ackermann");
    if !matches!(
        geometry,
        "ackermann" | "six_wheel_ackermann" | "rear_counter_half" | "counter_steer_half" | "oxbo"
    ) {
        return None;
    }
    let chassis_body = physics.bodies.get(chassis)?;
    let inverse = chassis_body.rotation().inverse();
    let max_angle = (controller.max_steer_deg.unwrap_or(45.0) as f64)
        .abs()
        .clamp(0.0, 85.0)
        .to_radians();
    let mut wheels: Vec<SteeredWheel> = Vec::new();
    for path in controller
        .steer_left_joint
        .iter()
        .chain(controller.steer_right_joint.iter())
        .chain(controller.steer_joints.iter())
        .chain(machine.steering_joints.iter())
    {
        let pair = joint_pair(scene_root, path, joints, parents, physics)?;
        let authored_drive = joints.iter().any(|(entity, prim, joint)| {
            prim.path == *path
                && is_descendant_of(entity, scene_root, parents)
                && !joint.drives.is_empty()
        });
        if wheels
            .iter()
            .any(|wheel| rigid_body_pair_matches(wheel.pair, pair.0, pair.1))
        {
            continue;
        }
        let (data, parent) = joint_data(physics, pair)?;
        let parent_body = physics.bodies.get(parent)?;
        let axis = inverse * (parent_body.rotation() * (data.local_frame1.rotation * Vector::X));
        if axis.z.abs() < 0.5 {
            return None;
        }
        let anchor =
            parent_body.translation() + parent_body.rotation() * data.local_frame1.translation;
        let center = inverse * (anchor - chassis_body.translation());
        let (lower, upper) = data
            .limits(JointAxis::AngX)
            .map(|limits| (limits.min.max(-max_angle), limits.max.min(max_angle)))
            .unwrap_or((-max_angle, max_angle));
        if lower > 0.0 || upper < 0.0 {
            return None;
        }
        wheels.push(SteeredWheel {
            pair,
            center,
            sign: axis.z.signum(),
            lower,
            upper,
            authored_drive,
        });
    }
    if wheels.is_empty() {
        return None;
    }
    let tires = tire_joint_pairs(scene_root, controller, machine, joints, parents, physics);
    let mut fixed = Vec::new();
    for pair in &tires {
        let steered = wheels.iter().any(|wheel| {
            [wheel.pair.0, wheel.pair.1]
                .into_iter()
                .any(|knuckle| knuckle != chassis && (knuckle == pair.0 || knuckle == pair.1))
        });
        if !steered {
            fixed.push(wheel_local_center(physics, chassis, *pair)?.y);
        }
    }
    let pivot_y = if !fixed.is_empty() {
        fixed.iter().sum::<f64>() / fixed.len() as f64
    } else {
        let front = wheels
            .iter()
            .map(|wheel| wheel.center.y)
            .fold(f64::INFINITY, f64::min);
        let rear = wheels
            .iter()
            .map(|wheel| wheel.center.y)
            .fold(f64::NEG_INFINITY, f64::max);
        let front_gain = controller.front_steer_multiplier.unwrap_or(1.0) as f64;
        let rear_gain = controller
            .rear_steer_multiplier
            .unwrap_or(if geometry == "ackermann" { 0.0 } else { -0.5 })
            as f64;
        if (rear - front).abs() < 0.1 || (front_gain - rear_gain).abs() < 1e-6 {
            return None;
        }
        (front_gain * rear - rear_gain * front) / (front_gain - rear_gain)
    };
    let requested = if let Some(input) = steering_input {
        let input = (input as f64).clamp(-1.0, 1.0);
        input.abs() * limit_curvature(1000.0 * input.signum(), pivot_y, &wheels)
    } else if cmd.linear_mps.abs() > 0.01 {
        cmd.angular_rps as f64 / cmd.linear_mps as f64
    } else {
        0.0
    };
    let curvature = limit_curvature(requested, pivot_y, &wheels);
    Some(Turn {
        pivot_y,
        curvature,
        wheels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wheel(x: f64, y: f64, limit: f64) -> SteeredWheel {
        SteeredWheel {
            pair: (RigidBodyHandle::invalid(), RigidBodyHandle::invalid()),
            center: Vector::new(x, y, 0.0),
            sign: 1.0,
            lower: -limit.to_radians(),
            upper: limit.to_radians(),
            authored_drive: false,
        }
    }

    #[test]
    fn oxbo_axles_share_one_turn_and_respect_limits() {
        let wheels = vec![
            wheel(1.215, -2.5, 16.0),
            wheel(-1.215, -2.5, 16.0),
            wheel(1.2, 2.36, 29.0),
            wheel(-1.2, 2.36, 29.0),
        ];
        let pivot_y = -0.9;
        for requested in [-10.0, -0.05, 0.0, 0.05, 10.0] {
            let curvature = limit_curvature(requested, pivot_y, &wheels);
            for wheel in &wheels {
                let a = angle(curvature, pivot_y, wheel.center);
                assert!(a >= wheel.lower - 1e-9 && a <= wheel.upper + 1e-9);
                let (forward, lateral) = rolling_vector(curvature, pivot_y, wheel.center);
                assert!((lateral * a.cos() - forward * a.sin()).abs() < 1e-9);
            }
        }
        let turn = Turn {
            pivot_y,
            curvature: 0.1,
            wheels,
        };
        let targets = turn.targets();
        assert!(targets[0].position > targets[1].position);
        assert!(targets[2].position < targets[3].position);
        assert!(targets[2].position < 0.0);
        for y in [-2.5, -0.9, 2.36] {
            assert!(
                turn.speed(2.0, Vector::new(-1.2, y, 0.0))
                    > turn.speed(2.0, Vector::new(1.2, y, 0.0))
            );
            assert_eq!(
                turn.speed(-2.0, Vector::new(1.2, y, 0.0)),
                -turn.speed(2.0, Vector::new(1.2, y, 0.0))
            );
        }
    }

    #[test]
    fn front_steering_mirrors_and_scales_all_targets_at_lock() {
        let wheels = vec![wheel(0.8, -2.4, 35.0), wheel(-0.8, -2.4, 35.0)];
        let left = limit_curvature(1000.0, 0.0, &wheels);
        let right = limit_curvature(-1000.0, 0.0, &wheels);
        assert!((left + right).abs() < 1e-9);
        assert!((angle(left, 0.0, wheels[0].center).to_degrees() - 35.0).abs() < 1e-6);
        assert!(angle(left, 0.0, wheels[1].center).to_degrees() < 35.0);
        assert_eq!(angle(0.0, 0.0, wheels[0].center), 0.0);
        let mut reversed = wheel(0.8, -2.4, 35.0);
        reversed.sign = -1.0;
        reversed.lower = -10.0_f64.to_radians();
        let curvature = limit_curvature(1000.0, 0.0, &[reversed]);
        assert!((angle(curvature, 0.0, wheels[0].center).to_degrees() - 10.0).abs() < 1e-6);
    }

    #[test]
    fn small_stick_input_does_not_saturate() {
        let wheels = vec![
            wheel(1.215, -2.5, 16.0),
            wheel(-1.215, -2.5, 16.0),
            wheel(1.2, 2.36, 29.0),
            wheel(-1.2, 2.36, 29.0),
        ];
        let full = limit_curvature(1000.0, -0.9, &wheels);
        for wheel in &wheels {
            assert!(angle(0.1 * full, -0.9, wheel.center).abs() < 4.0_f64.to_radians());
        }
    }
}
