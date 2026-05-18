//! Static world geometry (ground, obstacles), the collision-group
//! table, and the same-vehicle contact filter.
//!
//! Two collision groups share rapier's 32-slot space:
//!   - `GROUND`  — static world (ground plane, planet, obstacles)
//!   - `VEHICLE` — every vehicle collider (chassis, wheels, body parts)
//!
//! A vehicle's wheels are now separate rigid bodies, so rapier's
//! same-body contact auto-skip no longer covers chassis↔wheel /
//! wheel↔wheel pairs within one vehicle. [`SameVehicleFilter`] rejects
//! those — it tags every collider's `user_data` with the vehicle id and
//! drops a contact when both sides carry the same tag.

use rapier3d::prelude::*;

/// Static world geometry (ground plane, planet, box obstacles).
pub const GROUND: Group = Group::GROUP_1;
/// Every vehicle collider (chassis, wheels, body parts).
pub const VEHICLE: Group = Group::GROUP_2;

/// A vehicle collider: collides with the ground and with *other*
/// vehicles. Same-vehicle pairs are rejected by [`SameVehicleFilter`].
pub fn vehicle_groups() -> InteractionGroups {
    InteractionGroups::new(VEHICLE, GROUND.union(VEHICLE), InteractionTestMode::And)
}

/// Static world geometry — collides with vehicles only.
pub fn ground_groups() -> InteractionGroups {
    InteractionGroups::new(GROUND, VEHICLE, InteractionTestMode::And)
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

/// Collider `user_data` tag for vehicle `id`. `0` means "not a vehicle"
/// (world geometry, USD bodies) so the filter never matches those.
#[inline]
pub fn vehicle_tag(id: gearbox_core::VehicleId) -> u128 {
    id.0 as u128 + 1
}

/// Contact-pair filter that rejects collisions between two colliders
/// belonging to the same vehicle. Vehicle colliders carry a non-zero
/// `user_data` tag (see [`vehicle_tag`]) and the
/// `ActiveHooks::FILTER_CONTACT_PAIRS` flag; everything else is left to
/// the default group-based filtering.
pub struct SameVehicleFilter;

impl PhysicsHooks for SameVehicleFilter {
    fn filter_contact_pair(&self, ctx: &PairFilterContext) -> Option<SolverFlags> {
        let tag1 = ctx.colliders.get(ctx.collider1).map_or(0, |c| c.user_data);
        let tag2 = ctx.colliders.get(ctx.collider2).map_or(0, |c| c.user_data);
        if tag1 != 0 && tag1 == tag2 {
            None
        } else {
            Some(SolverFlags::COMPUTE_IMPULSES)
        }
    }
}
