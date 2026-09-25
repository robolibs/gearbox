use super::*;
use crate::physics::backend::{
    JointAxes, JointId, PressureTyreDesc, PressureTyreOutput, WheelForceDesc, WheelForceOutput,
};
use bevy::math::Affine3A;
use bevy::mesh::VertexAttributeValues;

pub(crate) fn rubber_name(name: &str) -> bool {
    let name = name.rsplit('/').next().unwrap_or(name).to_ascii_lowercase();
    !rigid_name(&name)
        && ["tyre", "tire", "tread", "mould_line", "bkt_fl630"]
            .iter().any(|s| name.contains(s))
}

pub(crate) fn rigid_name(name: &str) -> bool {
    let name = name.rsplit('/').next().unwrap_or(name).to_ascii_lowercase();
    ["rim", "hub", "collision", "collider"].iter().any(|s| name.contains(s))
}

type TyreHierarchy<'w, 's> = Query<'w, 's, (
    Option<&'static ChildOf>, Option<&'static Transform>,
    Option<&'static UsdPrimRef>, Option<&'static Name>,
)>;

fn radial_extent(positions: &[[f32; 3]], to_body: Affine3A, hub: DVec3, axle: DVec3) -> Option<f64> {
    let axle = axle.try_normalize()?;
    positions.iter().filter_map(|p| {
        let p = DVec3::from_array(to_body.transform_point3(Vec3::from_array(*p)).to_array().map(f64::from)) - hub;
        let radius = (p - axle * p.dot(axle)).length();
        (radius.is_finite() && radius > 0.0).then_some(radius)
    }).reduce(f64::max)
}

fn reference_rubber_radius(
    body: Entity, hub: DVec3, axle: DVec3,
    meshes: &Assets<Mesh>, visuals: &Query<(Entity, &Mesh3d)>, hierarchy: &TyreHierarchy,
) -> Option<f64> {
    visuals.iter().filter_map(|(entity, handle)| {
        let mut current = entity;
        let mut to_body = Affine3A::IDENTITY;
        let mut rubber = false;
        while current != body {
            let (parent, transform, prim, name) = hierarchy.get(current).ok()?;
            if prim.is_some_and(|p| rigid_name(&p.path)) || name.is_some_and(|n| rigid_name(n.as_str())) {
                return None;
            }
            rubber |= prim.is_some_and(|p| rubber_name(&p.path)) || name.is_some_and(|n| rubber_name(n.as_str()));
            to_body = transform.map_or(Affine3A::IDENTITY, Transform::compute_affine) * to_body;
            current = parent?.parent();
        }
        if !rubber { return None; }
        let mesh = meshes.get(&handle.0)?;
        let VertexAttributeValues::Float32x3(positions) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)? else { return None };
        radial_extent(positions, to_body, hub, axle)
    }).reduce(f64::max)
}

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

fn wheel_readout(
    values: &mut crate::services::LinkValues,
    machine: &str,
    link: &str,
    wheel: WheelForceOutput,
) {
    for (name, value) in [
        ("tyre_in_contact", f64::from(wheel.in_contact)),
        ("tyre_held_support", f64::from(wheel.held_support)),
        ("tyre_normal_load_n", wheel.normal_force),
        ("tyre_grip_budget_n", wheel.grip_force),
        ("tyre_slip_ratio", wheel.slip_ratio),
        ("tyre_slip_angle_rad", wheel.slip_angle),
        ("tyre_force_world_x_n", wheel.force.x),
        ("tyre_force_world_y_n", wheel.force.y),
        ("tyre_force_world_z_n", wheel.force.z),
        ("tyre_aligning_moment_world_x_nm", wheel.aligning_moment.x),
        ("tyre_aligning_moment_world_y_nm", wheel.aligning_moment.y),
        ("tyre_aligning_moment_world_z_nm", wheel.aligning_moment.z),
    ] {
        values.set(machine, link, name, value);
    }
    if let Some(tyre) = wheel.pressure {
        pressure_readout(values, machine, link, tyre);
        for (name, value) in [
            ("tyre_rolling_moment_world_x_nm", tyre.rolling_moment.x),
            ("tyre_rolling_moment_world_y_nm", tyre.rolling_moment.y),
            ("tyre_rolling_moment_world_z_nm", tyre.rolling_moment.z),
        ] {
            values.set(machine, link, name, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::backend::{
        BodyDesc, ColliderDesc, DQuat, Inertia, JointDesc, JointKind, MassProps,
    };

    #[test]
    fn wheel_telemetry_preserves_signed_world_wrenches_and_units() {
        let mut values = crate::services::LinkValues::default();
        let output = WheelForceOutput {
            in_contact: true,
            held_support: true,
            normal_force: 1234.0,
            grip_force: 567.0,
            force: DVec3::new(-12.0, 1234.0, 34.0),
            aligning_moment: DVec3::new(1.0, -2.0, 3.0),
            slip_ratio: -0.25,
            slip_angle: -0.125,
            pressure: Some(PressureTyreOutput {
                pressure_pa: 180_000.0,
                rolling_moment: DVec3::new(-3.0, 4.0, 0.0),
                ..Default::default()
            }),
            ..Default::default()
        };
        wheel_readout(&mut values, "machine", "wheel", output);
        for (name, expected) in [
            ("tyre_in_contact", 1.0), ("tyre_held_support", 1.0),
            ("tyre_normal_load_n", 1234.0), ("tyre_grip_budget_n", 567.0),
            ("tyre_slip_ratio", -0.25), ("tyre_slip_angle_rad", -0.125),
            ("tyre_force_world_x_n", -12.0), ("tyre_force_world_y_n", 1234.0),
            ("tyre_force_world_z_n", 34.0),
            ("tyre_aligning_moment_world_x_nm", 1.0),
            ("tyre_aligning_moment_world_y_nm", -2.0),
            ("tyre_aligning_moment_world_z_nm", 3.0),
            ("tyre_pressure_bar", 1.8), ("tyre_rolling_moment_nm", 5.0),
            ("tyre_rolling_moment_world_x_nm", -3.0),
            ("tyre_rolling_moment_world_y_nm", 4.0),
            ("tyre_rolling_moment_world_z_nm", 0.0),
        ] {
            assert_eq!(values.get("machine", "wheel", name), Some(expected), "{name}");
        }
        assert!(values.of_machine("other").is_empty());
        wheel_readout(&mut values, "machine", "wheel", WheelForceOutput {
            pressure: Some(PressureTyreOutput::default()), ..Default::default()
        });
        assert!(values.of_link("machine", "wheel").values().all(|v| *v == 0.0));
    }

    #[test]
    fn reference_radius_uses_rubber_transforms_not_rims_or_axle_width() {
        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>();
        let body = app.world_mut().spawn_empty().id();
        let make_mesh = |radius: f32| {
            let mut mesh = Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::all());
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[radius, 0.0, 4.0], [0.0, -radius, -4.0], [-radius, 0.0, 4.0]]);
            mesh
        };
        let rubber = app.world_mut().resource_mut::<Assets<Mesh>>().add(make_mesh(0.7));
        let other = app.world_mut().resource_mut::<Assets<Mesh>>().add(make_mesh(2.0));
        let hub = DVec3::new(0.2, 0.3, 0.0);
        let parent = app.world_mut().spawn((
            ChildOf(body), Name::new("radial_tyre"),
            Transform::from_translation(Vec3::from_array(hub.to_array().map(|v| v as f32))).with_rotation(Quat::from_rotation_z(0.6)),
        )).id();
        app.world_mut().spawn((ChildOf(parent), Mesh3d(rubber), Transform::from_scale(Vec3::splat(1.1))));
        for name in ["rim", "tire_collider", "tyre_collision"] {
            app.world_mut().spawn((ChildOf(body), Name::new(name), Mesh3d(other.clone()), Transform::IDENTITY));
        }
        app.world_mut().spawn((Name::new("radial_tyre"), Mesh3d(other), Transform::IDENTITY));
        let mut state = bevy::ecs::system::SystemState::<(Res<Assets<Mesh>>, Query<(Entity, &Mesh3d)>, TyreHierarchy)>::new(app.world_mut());
        let (meshes, visuals, hierarchy) = state.get(app.world()).unwrap();
        let radius = reference_rubber_radius(body, hub, DVec3::Z, &meshes, &visuals, &hierarchy).unwrap();
        assert!((radius - 0.77).abs() < 1e-6, "{radius}");
    }

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
                sensor: None,
            }],
            ..Default::default()
        };
        let machine = MachineInstanceSpec {
            tracks: Vec::new(),
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
        app.init_resource::<Assets<Mesh>>();
        let mut rubber = Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::all());
        rubber.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.1, -0.55, 0.0], [-0.1, -0.55, 0.0], [0.0, 0.55, 0.0]]);
        let rubber = app.world_mut().resource_mut::<Assets<Mesh>>().add(rubber);
        app.world_mut().spawn((ChildOf(wheel_entity), Name::new("radial_tyre"), Mesh3d(rubber.clone()), Transform::IDENTITY));
        app.insert_resource(physics)
            .insert_resource(runtime)
            .insert_resource(ControllerInventory {
                machines: vec![machine],
            })
            .insert_resource(gearbox_api::PhysicsActive(true))
            .init_resource::<gearbox_fields::WheelContacts>()
            .init_resource::<crate::services::LinkValues>()
            // The tread is placed along the track by rolled distance, so the
            // recorder reads the clock.
            .init_resource::<Time>()
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
            assert!((pressure.radius - 0.55).abs() < 1e-6);
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
            app.world_mut().resource_mut::<Assets<Mesh>>().get_mut(&rubber).unwrap()
                .insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0, -0.4, 0.0]; 3]);
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
            assert_eq!(paused.pressure.unwrap().radius, pressure.radius);
            assert!((paused.pressure.unwrap().target_pressure_pa - 220_000.0).abs() < 1e-8);
            let values = app.world().resource::<crate::services::LinkValues>();
            for (name, expected) in [
                ("tyre_force_world_x_n", paused.force.x),
                ("tyre_force_world_y_n", paused.force.y),
                ("tyre_force_world_z_n", paused.force.z),
                ("tyre_normal_load_n", paused.normal_force),
                ("tyre_slip_ratio", paused.slip_ratio),
                ("tyre_slip_angle_rad", paused.slip_angle),
                ("tyre_rolling_moment_nm", paused.pressure.unwrap().rolling_moment.length()),
            ] {
                assert_eq!(values.get("test", "wheel", name), Some(expected), "{name}");
            }
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
    meshes: Option<Res<Assets<Mesh>>>,
    visuals: Query<(Entity, &Mesh3d)>,
    hierarchy: TyreHierarchy,
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
        if !machine.tracks.is_empty() { continue; }
        let Some(root) = machine.scene_root else {
            continue;
        };
        let Some(wheels) = runtime.machine_wheels.get(&machine.id) else {
            continue;
        };
        let body_paths: HashMap<_, _> = physics.entity_to_body.iter()
            .filter_map(|(entity, body)| {
                let (_, prim) = prims.get(*entity).ok()?;
                is_descendant_of(*entity, root, &parents).then_some((prim.path.as_str(), *body))
            }).collect();
        let wheel_links: HashMap<_, _> = machine.links.links.iter().rev()
            .filter_map(|link| Some((*body_paths.get(link.body_prim.as_deref()?)?, link)))
            .collect();
        let Some(chassis) = machine
            .body
            .as_deref()
            .and_then(|path| body_paths.get(path).copied())
        else {
            continue;
        };
        let Some(forward) = physics.body(chassis).and_then(body_forward_vector) else {
            continue;
        };
        let mut axle_geometry = Vec::new();
        let mut axle_metadata_valid = true;
        for &wheel in wheels {
            let link = wheel_links.get(&wheel).copied();
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
            let Some((axle, width, collider_radius)) = body_tyre_geometry(&physics, wheel) else {
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
            let radius = registered.wheels.get(&wheel).map(|r| r.radius)
                .or_else(|| reference_rubber_radius(body.entity()?, hub, axle, meshes.as_deref()?, &visuals, &hierarchy))
                .unwrap_or(collider_radius);
            let mass = (mass / wheels.len().max(1) as f64)
                .max(body.mass())
                .max(1.0);
            let link = wheel_links.get(&wheel).copied();
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
            if let Some(output) = physics.wheel_output(wheel) {
                wheel_readout(&mut values, &machine.id, &link, output);
            }
        }
    }
}

/// What the ground takes back from a machine that is merely rolling.
///
/// A tyre is not a hoop. It flattens under load, the carcass loses energy
/// flexing back out, and on ground that gives way it has to climb out of the
/// rut it just made. Both losses scale with the load the tyre carries, so the
/// retarding force is a coefficient times that load — which is what the whole
/// of `Crr` is. Nothing modelled any of it, so a machine held its commanded
/// speed to the centimetre and, let go of, coasted for ever.
const ROLL_RESIST_BASE: f64 = 0.022;
/// What the carcass adds, per unit of deflection over the free radius. A soft
/// tyre lays down more patch and loses more in it, so it takes more pulling —
/// the same flattening that makes it harder to turn.
const ROLL_RESIST_FLEX: f64 = 0.17;
/// The resistance fades in over this speed, so it dies away with the motion
/// rather than shoving a machine that has already stopped.
const ROLL_RESIST_FADE_MPS: f64 = 0.25;

/// How much of its load a tyre gives back to the ground, from how far it is
/// flattened. A tyre reporting no deflection is taken as a hard one.
fn rolling_resistance(deflection_m: f64, radius_m: f64) -> f64 {
    let flex = if radius_m > 1e-3 {
        (deflection_m / radius_m).clamp(0.0, 1.0)
    } else {
        0.0
    };
    ROLL_RESIST_BASE + ROLL_RESIST_FLEX * flex
}

/// Whether motion resistance is applied; `GEARBOX_ROLL_RESIST=1` turns it on.
///
/// Off by default, and the reason is worth keeping. Applied from out here it
/// never reaches the machine: a driven wheel is held to a speed by its own
/// motor, which cancels whatever is put on it, and the chassis is roped to the
/// ground through those same motors, so shedding its speed is fought by every
/// one of them. Measured, a loaded harvester would not pull away at all —
/// 14.3 kN of drag against a drive worth 0.9 m/s² on 18.4 t. It belongs inside
/// molla's tyre model, beside the forces it already solves: `molla-vehicle`
/// carries a `rolling_resistance` term, and nothing has ever set it.
pub(super) fn motion_resistance_wanted() -> bool {
    matches!(
        std::env::var("GEARBOX_ROLL_RESIST").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

/// Every tyre on the ground drags, and the machine is what it drags on.
///
/// The drag is worked out tyre by tyre, from the load each one carries and how
/// far it is flattened, and then put on the chassis as one retarding impulse.
/// Not on the wheels: a driven wheel is held to a speed by its motor, which
/// simply cancels anything applied to it, and an idle one has almost no torque
/// to give, so the same drag stops it dead and it skids along as a brake. On
/// the chassis it is what it should be — something the driveline works against.
pub(super) fn apply_motion_resistance(
    inventory: Res<ControllerInventory>,
    runtime: Res<ControllerRuntimeState>,
    active: Res<gearbox_api::PhysicsActive>,
    mut physics: ResMut<crate::physics::PhysicsWorld>,
) {
    if !active.0 || !physics.uses_wheel_forces() {
        return;
    }
    // Given as an impulse over the physics this frame will run, because a
    // wrench put on a body stays on it: added every frame it piles up, and a
    // few seconds of that pins the machine to the ground.
    let over = physics.dt() * f64::from(physics.pending_steps.max(1));
    if !(over > 0.0) {
        return;
    }
    for machine in &inventory.machines {
        let Some(wheels) = runtime.machine_wheels.get(&machine.id) else {
            continue;
        };
        // The heaviest body of the machine is its chassis, and everything else
        // hangs off it; nothing here needs to know which prim that was.
        let Some(chassis) = runtime
            .machine_bodies
            .get(&machine.id)
            .and_then(|bodies| {
                bodies
                    .iter()
                    .filter(|body| !wheels.contains(body))
                    .filter_map(|body| Some((*body, physics.body(*body)?.mass())))
                    .max_by(|a, b| a.1.total_cmp(&b.1))
            })
            .map(|(body, _)| body)
        else {
            continue;
        };
        // The drag slows the whole machine, so it is shed at the rate its
        // whole mass gives, not the chassis body's alone.
        let machine_mass: f64 = runtime
            .machine_bodies
            .get(&machine.id)
            .into_iter()
            .flatten()
            .filter_map(|body| physics.body(*body))
            .map(|body| body.mass())
            .sum();
        let mut drag = 0.0;
        for &wheel in wheels {
            let Some(output) = physics.wheel_output(wheel) else {
                continue;
            };
            if !output.in_contact || !(output.normal_force > 0.0) {
                continue;
            }
            let coefficient = output
                .pressure
                .map(|tyre| rolling_resistance(tyre.deflection, tyre.radius))
                .unwrap_or(ROLL_RESIST_BASE);
            drag += coefficient * output.normal_force;
        }
        if !(drag > 0.0) || !drag.is_finite() {
            continue;
        }
        let Some(body) = physics.body(chassis) else {
            continue;
        };
        // Only the travel over the ground is resisted, and it fades out with
        // the motion so a machine that has stopped is not shoved backwards.
        let travel = body.linvel();
        let travel = DVec3::new(travel.x, 0.0, travel.z);
        let speed = travel.length();
        if !(speed > 1e-4) {
            continue;
        }
        // Taken off the speed outright rather than handed to the solver as an
        // impulse: the step runs several substeps and an impulse given to it
        // is felt once per substep, which multiplied the drag by eight and
        // pinned the machine to the spot. Never more than the speed itself,
        // so resistance can only stop a machine, never reverse it.
        let mass = machine_mass.max(1.0);
        let shed =
            (drag * (speed / ROLL_RESIST_FADE_MPS).tanh() / mass * over).clamp(0.0, speed);
        if !(shed > 0.0) {
            continue;
        }
        let slowed = travel * ((speed - shed) / speed);
        if let Some(body) = physics.body_mut(chassis) {
            let full = body.linvel();
            body.set_linvel(DVec3::new(slowed.x, full.y, slowed.z), true);
        }
    }
}

#[cfg(test)]
mod resistance_tests {
    use super::*;

    // A softer tyre is flattened further and takes more pulling; that is the
    // whole of what pressure does to how a machine rolls and turns.
    #[test]
    fn a_flatter_tyre_resists_more_and_a_hard_one_still_resists() {
        let hard = rolling_resistance(0.058, 0.75);
        let soft = rolling_resistance(0.148, 0.75);
        assert!(soft > hard * 1.5, "soft {soft} against hard {hard}");
        assert!(hard > ROLL_RESIST_BASE);
        // A tyre reporting nothing still rolls against the ground.
        assert_eq!(rolling_resistance(0.0, 0.75), ROLL_RESIST_BASE);
        assert_eq!(rolling_resistance(0.1, 0.0), ROLL_RESIST_BASE);
        // However flat it goes it never becomes a brake.
        assert!(rolling_resistance(10.0, 0.75) <= ROLL_RESIST_BASE + ROLL_RESIST_FLEX);
    }
}
