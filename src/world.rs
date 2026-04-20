//! Static world geometry (ground, obstacles).
//!
//! For phase 1 this is just a flat ground plane. Terrain heightmaps and
//! curved roads belong in a later phase.

use rapier3d::prelude::*;

/// Builds a large flat ground collider centered at the origin.
pub(crate) fn build_ground_collider(half_size: f32) -> Collider {
    ColliderBuilder::cuboid(half_size, 0.1, half_size)
        .translation(Vec3::new(0.0, -0.1, 0.0))
        .friction(1.0)
        .build()
}

/// Builds a ball collider representing the planet. `centre` is the centre
/// of the sphere in world coordinates.
pub(crate) fn build_planet_collider(centre: Vec3, radius: f32) -> Collider {
    ColliderBuilder::ball(radius)
        .translation(centre)
        .friction(1.0)
        .build()
}

/// Builds a static box obstacle. `size` is the full (not half) extent.
pub(crate) fn build_box_collider(size: Vec3, pose: Pose) -> Collider {
    ColliderBuilder::cuboid(size.x * 0.5, size.y * 0.5, size.z * 0.5)
        .position(pose)
        .friction(1.0)
        .build()
}
