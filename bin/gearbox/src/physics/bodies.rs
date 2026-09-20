//! `UsdRigidBody` + `UsdMass` → bodies in `PhysicsWorld`.
//!
//! Mass priority follows the UsdPhysics spec:
//! 1. Explicit `physics:mass` → mass on the body itself, so a
//!    reduced-coordinate solver has a valid inertia tensor before colliders
//!    attach (a dynamic body with zero inertia panics mid-step).
//! 2. `physics:density` → falls through to the collider's own
//!    mass-from-density.
//! 3. None authored → a tiny safety mass so the body can take a step before
//!    a collider materialises.

use super::backend::{BodyDesc, BodyKind, Inertia, MassProps, Pose};
use super::markers::{UsdMass, UsdRigidBody};
use bevy::prelude::*;
use glam::DVec3;

use super::convert::{quat_to_d, vec3_to_d};
use super::world::PhysicsWorld;

/// Marker on entities whose `UsdRigidBody` is already in `PhysicsWorld`.
/// Lets the system stay idempotent.
#[derive(Component)]
pub(crate) struct BodyAttached;

pub fn convert_rigid_bodies(
    mut commands: Commands,
    mut world: ResMut<PhysicsWorld>,
    bodies: Query<
        (
            Entity,
            &UsdRigidBody,
            Option<&UsdMass>,
            Option<&GlobalTransform>,
        ),
        (Added<UsdRigidBody>, Without<BodyAttached>),
    >,
    parents: Query<&ChildOf>,
    sites: Query<&crate::globe::Site>,
) {
    for (entity, rb, mass, gt) in &bodies {
        // The transform is in the site's frame; the body goes in the site's region.
        let region = gearbox_globe::physics_offset(crate::globe::site_of(entity, &parents, &sites));
        let pose = match gt {
            Some(g) => {
                let t = g.compute_transform();
                let mut translation = vec3_to_d(t.translation);
                translation.x += region.x;
                Pose::new(translation, quat_to_d(t.rotation))
            }
            None => Pose::IDENTITY,
        };
        let kind = if !rb.enabled {
            BodyKind::Fixed
        } else if rb.kinematic {
            BodyKind::Kinematic
        } else {
            BodyKind::Dynamic
        };
        let authored_mass = mass.and_then(|m| m.mass).map(|m| m as f64);
        let center_of_mass = mass.and_then(|m| m.center_of_mass).map(vec3_to_d);
        let diagonal_inertia = mass.and_then(|m| m.diagonal_inertia).map(vec3_to_d);

        let mut desc = BodyDesc::new(kind).pose(pose).entity(entity);
        desc.linvel = vec3_to_d(rb.velocity);
        desc.angvel = vec3_to_d(rb.angular_velocity);
        desc.sleeping = rb.starts_asleep;
        // Real robot joints have gearbox and bearing friction. Without
        // damping a hanging articulation chain oscillates forever; USD has
        // no body-damping schema, so these defaults stand in without
        // freezing drive wheels.
        if kind == BodyKind::Dynamic {
            desc.linear_damping = 0.1;
            desc.angular_damping = 0.5;
        }
        desc.additional_mass = match authored_mass {
            Some(mass_kg) => Some(MassProps {
                local_com: center_of_mass.unwrap_or(DVec3::ZERO),
                mass: mass_kg,
                inertia: Inertia::Principal(
                    diagonal_inertia.unwrap_or(DVec3::splat(0.4 * mass_kg * 0.01)),
                ),
            }),
            None if kind == BodyKind::Dynamic => Some(MassProps {
                local_com: DVec3::ZERO,
                mass: 0.001,
                inertia: Inertia::Principal(DVec3::splat(0.0001)),
            }),
            None => None,
        };

        if std::env::var_os("GEARBOX_PHYSICS_LOG").is_some() {
            info!(
                "gearbox-physics: body {entity:?} kinematic {} mass {:?} com {:?} inertia {:?} at {:?}",
                rb.kinematic, authored_mass, center_of_mass, diagonal_inertia, pose.translation
            );
        }
        let handle = world.insert_body(desc);
        world.entity_to_body.insert(entity, handle);
        commands.entity(entity).insert(BodyAttached);
    }
}
