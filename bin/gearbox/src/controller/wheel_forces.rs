use super::*;
use crate::physics::backend::{JointAxes, JointId, WheelForceDesc};

#[derive(Clone, Copy, PartialEq)]
pub(super) struct Registration {
    joint: JointId,
    radius: f64,
    hub: DVec3,
    mass: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::backend::{
        BodyDesc, ColliderDesc, DQuat, Inertia, JointDesc, JointKind, MassProps,
    };

    #[test]
    fn scene_wheels_feed_traction_and_tracks_from_tyre_output() {
        let mut app = App::new();
        let root = app.world_mut().spawn_empty().id();
        let chassis_entity = app
            .world_mut()
            .spawn((UsdPrimRef::new("/machine/chassis"), ChildOf(root)))
            .id();
        let wheel_entity = app
            .world_mut()
            .spawn((UsdPrimRef::new("/machine/wheel"), ChildOf(root)))
            .id();
        let mut physics = crate::physics::PhysicsWorld::default();
        physics.set_gravity(DVec3::ZERO);
        let force_backend = physics.uses_wheel_forces();
        let rotation = DQuat::from_mat3(&glam::DMat3::from_cols(-DVec3::Z, -DVec3::X, DVec3::Y));
        let mut desc = BodyDesc::dynamic().pose(Pose::new(DVec3::Y * 3.49, rotation));
        desc.additional_mass = Some(MassProps {
            mass: 20.0,
            local_com: DVec3::ZERO,
            inertia: Inertia::Principal(DVec3::splat(2.0)),
        });
        let fixed = physics.insert_body(BodyDesc::fixed());
        let chassis = physics.insert_body(desc.clone().entity(chassis_entity));
        let wheel = physics.insert_body(desc.entity(wheel_entity));
        physics.entity_to_body.insert(chassis_entity, chassis);
        physics.entity_to_body.insert(wheel_entity, wheel);
        physics.insert_joint(
            fixed,
            chassis,
            JointDesc::new(
                JointKind::Prismatic { axis: DVec3::X },
                Pose::from_translation(DVec3::Y * 3.49),
                Pose::new(DVec3::ZERO, rotation.conjugate()),
            ),
        );
        physics.insert_joint(
            chassis,
            wheel,
            JointDesc::new(
                JointKind::Revolute { axis: DVec3::X },
                Pose::IDENTITY,
                Pose::IDENTITY,
            ),
        );
        physics
            .insert_collider(
                ColliderDesc::new(Shape::Cylinder {
                    half_height: 0.1,
                    radius: 0.5,
                })
                .parent(wheel)
                .pose(Pose::new(
                    DVec3::ZERO,
                    DQuat::from_rotation_arc(DVec3::Y, DVec3::X),
                )),
            )
            .unwrap();
        let ground = physics
            .insert_collider(
                ColliderDesc::new(Shape::Cuboid {
                    half_extents: DVec3::new(10.0, 0.1, 10.0),
                })
                .translation(DVec3::Y * 2.9),
            )
            .unwrap();
        physics.register_wheel_ground(ground, None).unwrap();
        let links = crate::links::LinkTree {
            links: vec![crate::links::LinkSpec {
                name: "wheel".into(),
                prim_path: "/machine/wheel".into(),
                role: crate::links::LinkRole::Wheel,
                parent: None,
                joint_prim: None,
                body_prim: Some("/machine/wheel".into()),
                static_offset: None,
                coupling: None,
                element: None,
                values: Vec::new(),
            }],
            ..Default::default()
        };
        let machine = MachineInstanceSpec {
            scene_root: Some(root),
            asset_label: "test".into(),
            source_path: String::new(),
            prim_path: "/machine".into(),
            id: "test".into(),
            kind: None,
            interface_version: None,
            id_policy: String::new(),
            up_axis: None,
            body: Some("/machine/chassis".into()),
            visuals: Vec::new(),
            colliders: Vec::new(),
            sensors: Vec::new(),
            powered_wheel_joints: Vec::new(),
            passive_wheel_joints: Vec::new(),
            steering_joints: Vec::new(),
            brake_joints: Vec::new(),
            suspension_joints: Vec::new(),
            tool_joints: Vec::new(),
            controllers: Vec::new(),
            links,
            grants: Vec::new(),
        };
        let mut runtime = ControllerRuntimeState::default();
        runtime
            .machine_bodies
            .insert("test".into(), vec![chassis, wheel]);
        runtime.machine_wheels.insert("test".into(), vec![wheel]);
        app.insert_resource(physics)
            .insert_resource(runtime)
            .insert_resource(ControllerInventory {
                machines: vec![machine],
            })
            .insert_resource(gearbox_api::PhysicsActive(true))
            .init_resource::<gearbox_fields::WheelContacts>()
            .init_resource::<crate::services::LinkValues>()
            .add_systems(
                Update,
                (
                    sync_machine_wheel_forces,
                    |mut world: ResMut<crate::physics::PhysicsWorld>| world.step(),
                    record_wheel_tracks,
                )
                    .chain(),
            );
        app.update();
        let physics = app.world().resource::<crate::physics::PhysicsWorld>();
        if force_backend {
            let output = physics.wheel_output(wheel).unwrap();
            assert!(output.in_contact && output.normal_force > 0.0);
            assert!(physics.contacts_with(ground).is_empty());
            let support = crate::controller::traction::wheel_support(physics, chassis, wheel);
            assert!((support.grip_force_n - output.grip_force).abs() < 1e-8);
            assert!((support.normal - output.normal).length() < 1e-8);
            let tracks = app.world().resource::<gearbox_fields::WheelContacts>();
            assert_eq!(tracks.contacts.len(), 1);
            assert!(tracks.contacts[0].position.y > 2.9);
            assert_eq!(
                app.world().resource::<crate::services::LinkValues>().get(
                    "test",
                    "wheel",
                    "normal_force"
                ),
                Some(output.normal_force)
            );
            app.world_mut()
                .resource_mut::<crate::physics::PhysicsWorld>()
                .collider_mut(ground)
                .unwrap()
                .set_enabled(false);
            app.world_mut()
                .resource_mut::<gearbox_fields::WheelContacts>()
                .contacts
                .clear();
            app.update();
            assert!(
                app.world()
                    .resource::<gearbox_fields::WheelContacts>()
                    .contacts
                    .is_empty()
            );
        } else {
            assert!(physics.wheel_output(wheel).is_none());
        }
    }
}

pub(super) fn sync_machine_wheel_forces(
    inventory: Res<ControllerInventory>,
    runtime: Res<ControllerRuntimeState>,
    prims: Query<(Entity, &UsdPrimRef)>,
    parents: Query<&ChildOf>,
    mut physics: ResMut<crate::physics::PhysicsWorld>,
    mut registered: Local<HashMap<BodyId, Registration>>,
) {
    if !physics.uses_wheel_forces() {
        return;
    }
    registered.retain(|body, _| physics.contains_body(*body));
    for machine in &inventory.machines {
        let Some(root) = machine.scene_root else {
            continue;
        };
        let Some(wheels) = runtime.machine_wheels.get(&machine.id) else {
            continue;
        };
        let Some(chassis) = machine
            .body
            .as_deref()
            .and_then(|path| find_prim_entity(root, path, &prims, &parents))
            .and_then(|entity| physics.entity_to_body.get(&entity).copied())
        else {
            continue;
        };
        let Some(forward) = physics.body(chassis).and_then(body_forward_vector) else {
            continue;
        };
        let mass: f64 = runtime
            .machine_bodies
            .get(&machine.id)
            .into_iter()
            .flatten()
            .filter_map(|id| physics.body(*id))
            .map(|b| b.mass())
            .sum();
        for &wheel in wheels {
            let Some((_, _, radius)) = body_tyre_geometry(&physics, wheel) else {
                continue;
            };
            let joint = physics.joints().into_iter().find(|id| {
                physics
                    .joint_bodies(*id)
                    .is_some_and(|(a, b)| a == wheel || b == wheel)
                    && physics
                        .joint(*id)
                        .is_some_and(|j| j.locked_axes() == JointAxes::ALL.without(JointAxis::AngX))
            });
            let Some(joint) = joint else { continue };
            let Some(body) = physics.body(wheel) else {
                continue;
            };
            let hub = body
                .colliders()
                .into_iter()
                .filter_map(|id| physics.collider(id))
                .filter(|c| !c.is_sensor())
                .max_by(|a, b| {
                    a.local_aabb()
                        .half_extents()
                        .max_element()
                        .total_cmp(&b.local_aabb().half_extents().max_element())
                })
                .map(|c| {
                    let bounds = c.local_aabb();
                    let center = (bounds.mins + bounds.maxs) * 0.5;
                    c.position_wrt_parent()
                        .map_or(center, |p| p.translation + p.rotation * center)
                })
                .unwrap_or(DVec3::ZERO);
            let mass = (mass / wheels.len().max(1) as f64)
                .max(body.mass())
                .max(1.0);
            let registration = Registration {
                joint,
                radius,
                hub,
                mass,
            };
            if registered.get(&wheel) == Some(&registration)
                && physics.wheel_output(wheel).is_some()
            {
                continue;
            }
            match physics.configure_wheel(WheelForceDesc {
                body: wheel,
                joint,
                local_hub: hub,
                forward,
                radius,
                supported_mass: mass,
            }) {
                Ok(()) => {
                    registered.insert(wheel, registration);
                }
                Err(error) => warn!(
                    "gearbox-control: wheel force registration rejected for {wheel:?}: {error}"
                ),
            }
        }
    }
}
