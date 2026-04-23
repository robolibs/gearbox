//! Static world geometry (ground, obstacles) and the collision-group
//! table the rest of the sim uses.
//!
//! Three groups share rapier's 32-slot space:
//!   - `GROUND`  — static world (ground plane, planet, obstacles)
//!   - `CHASSIS` — the main vehicle body
//!   - `WHEEL`   — wheel colliders that exist ONLY for inter-vehicle
//!                 wheel-on-wheel collision; the raycast suspension
//!                 filters them out so a vehicle's own wheel ray never
//!                 hits another vehicle's wheel.

use rapier3d::prelude::*;

pub const GROUND: Group  = Group::GROUP_1;
pub const CHASSIS: Group = Group::GROUP_2;
pub const WHEEL: Group   = Group::GROUP_3;

/// Chassis: collides with ground, other chassis, and wheels.
pub fn chassis_groups() -> InteractionGroups {
    InteractionGroups::new(
        CHASSIS,
        GROUND.union(CHASSIS).union(WHEEL),
        InteractionTestMode::And,
    )
}

/// Wheel collider: inter-vehicle bumper. Collides with other wheels AND
/// with other vehicles' chassis + body parts (so e.g. a harvester's
/// cab can shove a tractor's wheel). NOT ground (raycast handles that).
/// Same-body pairs are skipped automatically by rapier, so a wheel
/// collider never pushes against its own chassis or part colliders.
pub fn wheel_groups() -> InteractionGroups {
    InteractionGroups::new(
        WHEEL,
        WHEEL.union(CHASSIS),
        InteractionTestMode::And,
    )
}

/// Static world geometry.
pub fn ground_groups() -> InteractionGroups {
    InteractionGroups::new(GROUND, CHASSIS, InteractionTestMode::And)
}

/// Raycast-ground filter for the vehicle wheels — hits ground and
/// chassis, **skips** wheel colliders.
pub fn wheel_raycast_groups() -> InteractionGroups {
    InteractionGroups::new(
        Group::ALL,
        GROUND.union(CHASSIS),
        InteractionTestMode::And,
    )
}

/// Builds a large flat ground collider centered at the origin.
pub(crate) fn build_ground_collider(half_size: f64) -> Collider {
    ColliderBuilder::cuboid(half_size, 0.1, half_size)
        .translation(Vec3::new(0.0, -0.1, 0.0))
        .friction(1.0)
        .collision_groups(ground_groups())
        .build()
}

/// Builds a ball collider representing the planet. `centre` is the centre
/// of the sphere in world coordinates.
pub(crate) fn build_planet_collider(centre: Vec3, radius: f64) -> Collider {
    ColliderBuilder::ball(radius)
        .translation(centre)
        .friction(1.0)
        .collision_groups(ground_groups())
        .build()
}

/// Builds a static box obstacle. `size` is the full (not half) extent.
pub(crate) fn build_box_collider(size: Vec3, pose: Pose) -> Collider {
    ColliderBuilder::cuboid(size.x * 0.5, size.y * 0.5, size.z * 0.5)
        .position(pose)
        .friction(1.0)
        .collision_groups(ground_groups())
        .build()
}
