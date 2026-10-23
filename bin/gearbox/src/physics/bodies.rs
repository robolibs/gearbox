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
use super::markers::{ModelMember, UsdMass, UsdRigidBody};
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
            Option<&ModelMember>,
        ),
        (Added<UsdRigidBody>, Without<BodyAttached>),
    >,
    parents: Query<&ChildOf>,
    globals: Query<&GlobalTransform>,
    sites: Query<&crate::globe::Site>,
) {
    for (entity, rb, mass, gt, member) in &bodies {
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
        let preview = member.is_some_and(|m| m.rejected);
        let kind = if !rb.enabled || preview {
            BodyKind::Fixed
        } else if rb.kinematic {
            BodyKind::Kinematic
        } else {
            BodyKind::Dynamic
        };
        let authored_mass = mass.and_then(|m| m.mass).map(|m| m as f64);
        let body_scale = gt.map_or(1.0, |g| g.compute_transform().scale.x as f64);
        let center_of_mass = mass.and_then(|m| m.center_of_mass).map(|c| vec3_to_d(c) * body_scale);
        let diagonal_inertia = mass.and_then(|m| m.diagonal_inertia).map(vec3_to_d);
        let principal_axes = mass.and_then(|m| m.principal_axes).map(quat_to_d);

        let mut desc = BodyDesc::new(kind).pose(pose).entity(entity);
        // Authored velocities are in the space the prim's xform is in: its parent's.
        let parent_rotation = parents
            .get(entity)
            .ok()
            .and_then(|p| globals.get(p.parent()).ok())
            .map(|g| quat_to_d(g.compute_transform().rotation))
            .unwrap_or(glam::DQuat::IDENTITY);
        if !preview {
            desc.linvel = parent_rotation * vec3_to_d(rb.velocity);
            desc.angvel = parent_rotation * vec3_to_d(rb.angular_velocity);
        }
        desc.sleeping = rb.starts_asleep;
        // Body damping from the compiled model; legacy values otherwise.
        if kind == BodyKind::Dynamic {
            use crate::machine_model::{LEGACY_ANGULAR_DAMPING, LEGACY_LINEAR_DAMPING};
            (desc.linear_damping, desc.angular_damping) =
                member.and_then(|m| m.damping).unwrap_or((LEGACY_LINEAR_DAMPING, LEGACY_ANGULAR_DAMPING));
        }
        desc.additional_mass = match authored_mass {
            Some(mass_kg) => Some(MassProps {
                local_com: center_of_mass.unwrap_or(DVec3::ZERO),
                mass: mass_kg,
                inertia: inertia(
                    diagonal_inertia.unwrap_or(DVec3::splat(0.4 * mass_kg * 0.01)),
                    principal_axes,
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

/// The body-frame inertia: the principal moments turned by
/// `physics:principalAxes`, or left on the body axes when those are unturned.
fn inertia(moments: DVec3, axes: Option<glam::DQuat>) -> Inertia {
    match axes.filter(|q| q.angle_between(glam::DQuat::IDENTITY) > 1e-6) {
        Some(q) => {
            let r = glam::DMat3::from_quat(q.normalize());
            Inertia::Tensor(r * glam::DMat3::from_diagonal(moments) * r.transpose())
        }
        None => Inertia::Principal(moments),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawn_body(app: &mut App, member: Option<ModelMember>, parent_turn: Quat) -> Entity {
        let parent = app.world_mut().spawn(GlobalTransform::from(Transform::from_rotation(parent_turn))).id();
        let mut body = app.world_mut().spawn((
            UsdRigidBody { enabled: true, velocity: Vec3::X, angular_velocity: Vec3::Z, ..Default::default() },
            UsdMass { mass: Some(2.0), diagonal_inertia: Some(Vec3::splat(0.1)), ..Default::default() },
            GlobalTransform::IDENTITY,
            ChildOf(parent),
        ));
        if let Some(member) = member {
            body.insert(member);
        }
        body.id()
    }

    fn bodies_app() -> App {
        let mut app = App::new();
        app.insert_resource(PhysicsWorld::default()).add_systems(Update, convert_rigid_bodies);
        app
    }

    /// A rejected machine's bodies are a still preview: fixed, at rest.
    #[test]
    fn a_rejected_machine_body_is_a_fixed_preview() {
        let mut app = bodies_app();
        let member = ModelMember { machine_root: "/m".into(), strict: true, rejected: true, explicit_mass: true, damping: None };
        let entity = spawn_body(&mut app, Some(member), Quat::IDENTITY);
        app.update();
        let world = app.world().resource::<PhysicsWorld>();
        let body = world.body(world.entity_to_body[&entity]).unwrap();
        assert!(body.is_fixed());
        assert_eq!(body.linvel(), DVec3::ZERO);
    }

    /// Authored velocities are in the parent's space and reach the world turned.
    #[test]
    fn initial_velocities_are_turned_into_the_world() {
        let mut app = bodies_app();
        let turn = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let entity = spawn_body(&mut app, None, turn);
        app.update();
        let world = app.world().resource::<PhysicsWorld>();
        let body = world.body(world.entity_to_body[&entity]).unwrap();
        let expected = quat_to_d(turn) * DVec3::X;
        assert!((body.linvel() - expected).length() < 1e-5, "{:?} vs {expected:?}", body.linvel());
        assert!((body.angvel() - quat_to_d(turn) * DVec3::Z).length() < 1e-5, "{:?}", body.angvel());
    }

    /// A disc spinning about X, its axes turned 90° about Z, spins about Y.
    #[test]
    fn principal_axes_turn_the_moments_into_the_body_frame() {
        let moments = DVec3::new(0.5, 0.25, 0.25);
        let turned = inertia(moments, Some(glam::DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2)));
        let Inertia::Tensor(tensor) = turned else { panic!("{turned:?}") };
        assert!(tensor.abs_diff_eq(glam::DMat3::from_diagonal(DVec3::new(0.25, 0.5, 0.25)), 1e-12), "{tensor:?}");
        assert_eq!(inertia(moments, Some(glam::DQuat::IDENTITY)), Inertia::Principal(moments));
        assert_eq!(inertia(moments, None), Inertia::Principal(moments));
    }
}
