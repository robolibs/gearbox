use crate::physics::PhysicsWorld;
use crate::physics::backend::{BodyId, DVec3};

pub(super) struct WheelSupport {
    pub normal: DVec3,
    pub grip_force_n: f64,
}

/// Support normal and friction budget from tyre output or solved wheel contacts.
pub(super) fn wheel_support(
    physics: &PhysicsWorld,
    chassis: BodyId,
    wheel: BodyId,
) -> WheelSupport {
    let up = physics
        .body(chassis)
        .map(|body| body.rotation() * DVec3::Z)
        .unwrap_or(DVec3::Y);
    let mut support = DVec3::ZERO;
    let mut grip_impulse = 0.0;
    if let Some(output) = physics.wheel_output(wheel) {
        return WheelSupport {
            normal: if output.in_contact { output.normal } else { up },
            grip_force_n: output.grip_force,
        };
    }
    if let Some(body) = physics.body(wheel) {
        for collider in body.colliders() {
            for manifold in physics.contacts_with(collider) {
                if !manifold.active {
                    continue;
                }
                // Toward the wheel, whichever side of the pair it is on.
                let normal = manifold.normal
                    * if manifold.collider1 == collider {
                        -1.0
                    } else {
                        1.0
                    };
                if normal.dot(up) <= 0.1 {
                    continue;
                }
                for point in manifold.points.iter().filter(|p| p.solved) {
                    let impulse = point.impulse.max(0.0);
                    support += normal * impulse;
                    grip_impulse += point.friction.max(0.0) * impulse;
                }
            }
        }
    }
    WheelSupport {
        normal: if support.length_squared() > 1e-12 {
            support.normalize()
        } else {
            up
        },
        grip_force_n: grip_impulse / physics.dt().max(1e-6),
    }
}
