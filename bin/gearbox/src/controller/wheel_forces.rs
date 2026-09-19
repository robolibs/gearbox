use super::*;
use crate::physics::backend::{
    JointAxes, JointId, PressureTyreDesc, PressureTyreOutput, WheelForceDesc,
};

#[derive(Default)]
pub(super) struct Registrations {
    wheels: HashMap<BodyId, Registration>,
    axles: HashMap<String, (Vec<(BodyId, Option<u16>)>, HashMap<BodyId, u16>)>,
    invalid_axles: HashSet<String>,
}

fn infer_axles(wheels: &[(BodyId, f64, Option<u16>)]) -> Result<HashMap<BodyId, u16>, String> {
    if wheels.iter().any(|w| !w.1.is_finite() || w.2 == Some(0)) {
        return Err("invalid wheel position or axle id".into());
    }
    let mut sorted = wheels.to_vec();
    sorted.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut used: HashSet<_> = sorted.iter().filter_map(|w| w.2).collect();
    let mut result = HashMap::new();
    let mut start = 0;
    while start < sorted.len() {
        let end = start
            + sorted[start..]
                .iter()
                .take_while(|w| (w.1 - sorted[start].1).abs() <= 0.25)
                .count();
        let group = &sorted[start..end];
        let ids: HashSet<_> = group.iter().filter_map(|w| w.2).collect();
        if ids.len() > 1 && group.iter().any(|w| w.2.is_none()) {
            return Err(
                "ambiguous partial axle metadata; author tyre_axle on every wheel in this row"
                    .into(),
            );
        }
        let axle = if let Some(id) = ids.iter().min() {
            *id
        } else {
            (1..=u16::MAX)
                .find(|id| !used.contains(id))
                .ok_or("too many tyre axles")?
        };
        used.insert(axle);
        for &(body, _, authored) in group {
            result.insert(body, authored.unwrap_or(axle));
        }
        start = end;
    }
    Ok(result)
}

pub(crate) fn pressure_tyres(
    machine: &MachineInstanceSpec,
    values: &crate::services::LinkValues,
) -> Result<Vec<gearbox_api::tyres::TyrePressure>, String> {
    machine
        .links
        .links
        .iter()
        .filter(|link| {
            values
                .get(&machine.id, &link.name, "tyre_target_pressure_bar")
                .is_some()
        })
        .map(|link| {
            gearbox_api::tyres::TyrePressure::read(&link.name, |name| {
                values.get(&machine.id, &link.name, name)
            })
            .ok_or_else(|| format!("incomplete tyre pressure telemetry for {}", link.name))
        })
        .collect()
}

pub(crate) fn set_pressure_group(
    machine: &MachineInstanceSpec,
    values: &mut crate::services::LinkValues,
    scope: &str,
    bar: f64,
) -> Result<(), String> {
    let links = gearbox_api::tyres::targets(&pressure_tyres(machine, values)?, scope, bar)?;
    for link in links {
        values.set(&machine.id, &link, "tyre_target_pressure_bar", bar);
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
pub(super) struct Registration {
    joint: JointId,
    radius: f64,
    hub: DVec3,
    mass: f64,
    tyre: PressureTyreDesc,
}

fn tyre_properties(link: Option<&crate::links::LinkSpec>, width: f64) -> PressureTyreDesc {
    let mut tyre = PressureTyreDesc::reference(width);
    let authored = |name: &str, fallback: f64| {
        link.and_then(|l| l.values.iter().find(|(n, _)| n == name))
            .map_or(fallback, |(_, v)| *v)
    };
    tyre.pressure_pa = authored("tyre_pressure_bar", tyre.pressure_pa / 100_000.0) * 100_000.0;
    tyre.min_pressure_pa =
        authored("tyre_min_pressure_bar", tyre.min_pressure_pa / 100_000.0) * 100_000.0;
    tyre.max_pressure_pa =
        authored("tyre_max_pressure_bar", tyre.max_pressure_pa / 100_000.0) * 100_000.0;
    tyre.pressure_rate_pa_s = authored(
        "tyre_pressure_rate_bar_s",
        tyre.pressure_rate_pa_s / 100_000.0,
    ) * 100_000.0;
    tyre.width = authored("tyre_width_m", width);
    tyre.carcass_stiffness = authored("tyre_carcass_stiffness_pa_m", tyre.carcass_stiffness);
    tyre.tread_stiffness = authored("tyre_tread_stiffness_n_m3", tyre.tread_stiffness);
    tyre.damping_ratio = authored("tyre_damping_ratio", tyre.damping_ratio);
    tyre.hysteresis_fraction = authored("tyre_hysteresis_fraction", tyre.hysteresis_fraction);
    tyre
}

fn pressure_readout(
    values: &mut crate::services::LinkValues,
    machine: &str,
    link: &str,
    tyre: PressureTyreOutput,
) {
    for (name, value) in [
        ("tyre_pressure_bar", tyre.pressure_pa / 100_000.0),
        (
            "tyre_target_pressure_bar",
            tyre.target_pressure_pa / 100_000.0,
        ),
        ("tyre_min_pressure_bar", tyre.min_pressure_pa / 100_000.0),
        ("tyre_max_pressure_bar", tyre.max_pressure_pa / 100_000.0),
        ("tyre_loaded_radius_m", tyre.loaded_radius),
        ("tyre_deflection_m", tyre.deflection),
        ("tyre_patch_length_m", tyre.patch_length),
        ("tyre_patch_width_m", tyre.patch_width),
        ("tyre_tread_area_m2", tyre.patch_area),
        ("tyre_rolling_moment_nm", tyre.rolling_moment.length()),
    ] {
        values.set(machine, link, name, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::backend::{
        BodyDesc, ColliderDesc, DQuat, Inertia, JointDesc, JointKind, MassProps,
    };

    #[test]
    fn axle_inference_orders_rows_and_respects_authored_ids() {
        let wheels = [
            (BodyId(4), -1.0, None),
            (BodyId(2), 2.05, None),
            (BodyId(3), -1.02, None),
            (BodyId(1), 2.0, None),
        ];
        let axles = infer_axles(&wheels).unwrap();
        assert_eq!(axles[&BodyId(1)], 1);
        assert_eq!(axles[&BodyId(2)], 1);
        assert_eq!(axles[&BodyId(3)], 2);
        assert_eq!(axles[&BodyId(4)], 2);
        let mut authored = wheels;
        authored[0].2 = Some(7);
        assert_eq!(infer_axles(&authored).unwrap()[&BodyId(3)], 7);
        let mut reversed = wheels;
        reversed.reverse();
        assert_eq!(infer_axles(&reversed).unwrap(), axles);
        assert!(infer_axles(&[(BodyId(1), f64::NAN, None)]).is_err());
        assert!(infer_axles(&[(BodyId(1), 0.0, Some(0))]).is_err());
        assert!(
            infer_axles(&[
                (BodyId(1), 0.0, Some(1)),
                (BodyId(2), 0.0, Some(2)),
                (BodyId(3), 0.0, None)
            ])
            .is_err()
        );
    }

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
                    |mut world: ResMut<crate::physics::PhysicsWorld>,
                     active: Res<gearbox_api::PhysicsActive>| {
                        if active.0 {
                            world.step();
                        }
                    },
                    record_wheel_tracks,
                )
                    .chain(),
            );
        app.update();
        let physics = app.world().resource::<crate::physics::PhysicsWorld>();
        if force_backend {
            assert_eq!(
                app.world().resource::<crate::services::LinkValues>().get(
                    "test",
                    "wheel",
                    "tyre_axle"
                ),
                Some(1.0)
            );
            let output = physics.wheel_output(wheel).unwrap();
            assert!(output.in_contact && output.normal_force > 0.0);
            assert!(physics.contacts_with(ground).is_empty());
            let support = crate::controller::traction::wheel_support(physics, chassis, wheel);
            assert!((support.grip_force_n - output.grip_force).abs() < 1e-8);
            assert!((support.normal - output.normal).length() < 1e-8);
            let tracks = app.world().resource::<gearbox_fields::WheelContacts>();
            assert_eq!(tracks.contacts.len(), 1);
            assert!(tracks.contacts[0].position.y > 2.9);
            let pressure = output.pressure.unwrap();
            assert_eq!(
                tracks.contacts[0].length,
                Some(pressure.patch_length as f32)
            );
            assert_eq!(tracks.contacts[0].width, pressure.patch_width as f32);
            assert_eq!(
                app.world().resource::<crate::services::LinkValues>().get(
                    "test",
                    "wheel",
                    "normal_force"
                ),
                Some(output.normal_force)
            );
            let initial_pressure = pressure.pressure_pa;
            app.world_mut()
                .resource_mut::<gearbox_api::PhysicsActive>()
                .0 = false;
            app.world_mut()
                .resource_mut::<crate::services::LinkValues>()
                .set("test", "wheel", "tyre_target_pressure_bar", 2.2);
            app.update();
            let paused = app
                .world()
                .resource::<crate::physics::PhysicsWorld>()
                .wheel_output(wheel)
                .unwrap();
            assert_eq!(paused.normal_force, output.normal_force);
            assert_eq!(paused.pressure.unwrap().pressure_pa, initial_pressure);
            assert!((paused.pressure.unwrap().target_pressure_pa - 220_000.0).abs() < 1e-8);
            app.world_mut()
                .resource_mut::<crate::services::LinkValues>()
                .set("test", "wheel", "tyre_target_pressure_bar", f64::NAN);
            app.update();
            assert_eq!(
                app.world().resource::<crate::services::LinkValues>().get(
                    "test",
                    "wheel",
                    "tyre_target_pressure_bar"
                ),
                Some(2.2)
            );
            app.world_mut()
                .resource_mut::<gearbox_api::PhysicsActive>()
                .0 = true;
            app.update();
            assert!(
                app.world()
                    .resource::<crate::physics::PhysicsWorld>()
                    .wheel_output(wheel)
                    .unwrap()
                    .pressure
                    .unwrap()
                    .pressure_pa
                    > initial_pressure
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
    mut values: ResMut<crate::services::LinkValues>,
    mut registered: Local<Registrations>,
) {
    if !physics.uses_wheel_forces() {
        return;
    }
    registered
        .wheels
        .retain(|body, _| physics.contains_body(*body));
    registered
        .axles
        .retain(|machine, _| inventory.machines.iter().any(|m| &m.id == machine));
    registered
        .invalid_axles
        .retain(|machine| inventory.machines.iter().any(|m| &m.id == machine));
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
        let mut axle_geometry = Vec::new();
        let mut axle_metadata_valid = true;
        for &wheel in wheels {
            let link = machine.links.links.iter().find(|link| {
                link.body_prim
                    .as_deref()
                    .and_then(|path| find_prim_entity(root, path, &prims, &parents))
                    .and_then(|entity| physics.entity_to_body.get(&entity).copied())
                    == Some(wheel)
            });
            let authored = link.and_then(|link| {
                link.values
                    .iter()
                    .find(|(name, _)| name == "tyre_axle")
                    .map(|(_, value)| *value)
            });
            if authored.is_some_and(|id| {
                !id.is_finite() || id < 1.0 || id > u16::MAX as f64 || id.fract() != 0.0
            }) {
                axle_metadata_valid = false;
                break;
            }
            if let Some(body) = physics.body(wheel) {
                axle_geometry.push((
                    wheel,
                    body.translation().dot(forward),
                    authored.map(|id| id as u16),
                ));
            }
        }
        if !axle_metadata_valid {
            if registered.invalid_axles.insert(machine.id.clone()) {
                warn!(
                    "gearbox-control: {}: tyre_axle must be an integer from 1 to 65535",
                    machine.id
                );
            }
            continue;
        }
        let signature: Vec<_> = axle_geometry
            .iter()
            .map(|(body, _, axle)| (*body, *axle))
            .collect();
        if registered
            .axles
            .get(&machine.id)
            .is_none_or(|(key, _)| key != &signature)
        {
            match infer_axles(&axle_geometry) {
                Ok(axles) => {
                    registered
                        .axles
                        .insert(machine.id.clone(), (signature, axles));
                }
                Err(error) => {
                    if registered.invalid_axles.insert(machine.id.clone()) {
                        warn!("gearbox-control: {}: {error}", machine.id);
                    }
                    continue;
                }
            }
        }
        registered.invalid_axles.remove(&machine.id);
        let mass: f64 = runtime
            .machine_bodies
            .get(&machine.id)
            .into_iter()
            .flatten()
            .filter_map(|id| physics.body(*id))
            .map(|b| b.mass())
            .sum();
        let mut targets = Vec::new();
        let mut readouts = Vec::new();
        for &wheel in wheels {
            let Some((_, width, radius)) = body_tyre_geometry(&physics, wheel) else {
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
            let link = machine.links.links.iter().find(|link| {
                link.body_prim
                    .as_deref()
                    .and_then(|path| find_prim_entity(root, path, &prims, &parents))
                    .and_then(|entity| physics.entity_to_body.get(&entity).copied())
                    == Some(wheel)
            });
            let tyre = tyre_properties(link, width);
            let registration = Registration {
                joint,
                radius,
                hub,
                mass,
                tyre,
            };
            if registered.wheels.get(&wheel) != Some(&registration)
                || physics.wheel_output(wheel).is_none()
            {
                match physics.configure_wheel(WheelForceDesc {
                    body: wheel,
                    joint,
                    local_hub: hub,
                    forward,
                    radius,
                    supported_mass: mass,
                    tyre: Some(tyre),
                }) {
                    Ok(()) => {
                        registered.wheels.insert(wheel, registration);
                    }
                    Err(error) => warn!(
                        "gearbox-control: wheel force registration rejected for {wheel:?}: {error}"
                    ),
                }
            }
            if let Some(link) = link
                && let Some(tyre) = physics.wheel_output(wheel).and_then(|out| out.pressure)
            {
                if let Some(axle) = registered.axles[&machine.id].1.get(&wheel) {
                    values.set(&machine.id, &link.name, "tyre_axle", f64::from(*axle));
                }
                let requested = values
                    .get(&machine.id, &link.name, "tyre_target_pressure_bar")
                    .unwrap_or(tyre.target_pressure_pa / 100_000.0)
                    * 100_000.0;
                if requested != tyre.target_pressure_pa {
                    targets.push((wheel, requested));
                }
                readouts.push((wheel, link.name.clone()));
            }
        }
        if !targets.is_empty()
            && let Err(error) = physics.set_wheel_pressures(&targets)
        {
            warn!(
                "gearbox-control: tyre pressure group rejected for {}: {error}",
                machine.id
            );
        }
        for (wheel, link) in readouts {
            if let Some(tyre) = physics.wheel_output(wheel).and_then(|out| out.pressure) {
                pressure_readout(&mut values, &machine.id, &link, tyre);
            }
        }
    }
}
