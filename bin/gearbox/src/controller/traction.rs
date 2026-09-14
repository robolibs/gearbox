use crate::physics::PhysicsWorld;
use rapier3d::prelude::{RigidBodyHandle, Vector};

pub(super) struct WheelSupport {
    pub normal: Vector,
    pub grip_force_n: f64,
}

/// Support normal and friction budget from the last solved wheel contacts.
pub(super) fn wheel_support(
    physics: &PhysicsWorld,
    chassis: RigidBodyHandle,
    wheel: RigidBodyHandle,
) -> WheelSupport {
    let up = physics
        .bodies
        .get(chassis)
        .map(|body| body.rotation() * Vector::Z)
        .unwrap_or(Vector::Y);
    let mut support = Vector::ZERO;
    let mut grip_impulse = 0.0;
    if let Some(body) = physics.bodies.get(wheel) {
        for &collider in body.colliders() {
            for pair in physics.narrow_phase.contact_pairs_with(collider) {
                if !pair.has_any_active_contact() {
                    continue;
                }
                for manifold in &pair.manifolds {
                    let normal = manifold.data.normal
                        * if pair.collider1 == collider {
                            -1.0
                        } else {
                            1.0
                        };
                    if normal.dot(up) <= 0.1 {
                        continue;
                    }
                    for contact in &manifold.data.solver_contacts {
                        if let Some(point) = manifold.points.get(contact.contact_id[0] as usize) {
                            let impulse = point.data.impulse.max(0.0);
                            support += normal * impulse;
                            grip_impulse += contact.friction.max(0.0) * impulse;
                        }
                    }
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
        grip_force_n: grip_impulse / physics.integration_parameters.dt.max(1e-6),
    }
}
