use super::*;
use crate::physics::{MollaBackend, backend::*};
use gearbox_fields::{FieldProfile, WheelResponse};

fn layout() -> FieldLayout {
    serde_json::from_str(r#"{"default":"test", "default_friction":0.9,
        "fields":[{"name":"slippery", "profile":"test", "min":[-2,-2], "max":[0,2], "friction":0}]}"#).unwrap()
}

fn fixture() -> (App, [BodyId; 2], ColliderId) {
    let mut app = App::new();
    let entity = app.world_mut().spawn_empty().id();
    let mut backend = MollaBackend::default();
    backend.set_gravity(DVec3::ZERO);
    let collider = backend
        .insert_collider(
            ColliderDesc::new(Shape::Cuboid {
                half_extents: DVec3::new(2.0, 0.1, 2.0),
            })
            .translation(DVec3::new(0.0, -0.1, 0.0))
            .friction(0.6),
        )
        .unwrap();
    backend.register_wheel_ground(collider, None).unwrap();
    let wheels = [-1.0, 1.0].map(|x| {
        let pose = Pose::from_translation(DVec3::new(x, 0.45, 0.0));
        let parent = backend.insert_body(BodyDesc::fixed().pose(pose));
        let wheel = backend.insert_body(BodyDesc::dynamic().pose(pose));
        backend
            .insert_collider(ColliderDesc::new(Shape::Ball { radius: 0.5 }).parent(wheel))
            .unwrap();
        let joint = backend.insert_joint(
            parent,
            wheel,
            JointDesc::new(
                JointKind::Revolute { axis: -DVec3::Z },
                Pose::IDENTITY,
                Pose::IDENTITY,
            ),
        );
        backend
            .configure_wheel(WheelForceDesc {
                body: wheel,
                joint,
                local_hub: DVec3::ZERO,
                forward: DVec3::X,
                radius: 0.5,
                supported_mass: 100.0,
                tyre: Some(PressureTyreDesc::reference(0.3)),
            })
            .unwrap();
        wheel
    });
    let mut profiles = FieldProfiles::default();
    profiles.register(FieldProfile {
        name: "test",
        layers: vec![],
        ground: |_, _, _, _, _| panic!("headless material factory"),
        wheel_response: WheelResponse {
            recovery_seconds: 1.0,
            bend: 0.0,
            darkening: 0.0,
            footprint_length: 0.3,
            tread: false,
        },
        tread: Vec4::ZERO,
        surface_tint: Vec4::ONE,
        soft_border: 0.0,
    });
    app.insert_resource(PhysicsWorld::with_backend(Box::new(backend)))
        .insert_resource(ProceduralTerrain {
            site: 0,
            entity,
            safety_floor: collider,
            grid: Arc::new(HeightGrid::sample(4.0, 0.1, |_, _| 0.0)),
            fine_cell: 0.1,
            tiles: HashMap::default(),
            complete: true,
            heightmap: None,
        })
        // The ground under the wheels is streamed in chunks; the grid is
        // published onto whichever are laid, so the fixture lays one.
        .insert_resource(super::super::GroundColliders(
            [((0usize, IVec2::ZERO), collider)].into_iter().collect(),
        ))
        .insert_resource(profiles)
        .insert_resource(layout())
        .add_systems(Update, publish);
    (app, wheels, collider)
}

fn step(app: &mut App, wheels: [BodyId; 2]) -> [WheelForceOutput; 2] {
    let mut physics = app.world_mut().resource_mut::<PhysicsWorld>();
    physics.step();
    wheels.map(|wheel| physics.wheel_output(wheel).unwrap())
}

#[test]
fn authored_field_friction_reaches_real_pressure_tyre_forces() {
    let (mut app, wheels, ground) = fixture();
    let uniform = step(&mut app, wheels);
    assert!(uniform.iter().all(|out| out.grip_force > 100.0));
    app.update();
    let [low, high] = step(&mut app, wheels);
    assert!(low.in_contact && high.in_contact && low.normal_force > 100.0);
    assert_eq!(low.grip_force, 0.0);
    assert!(high.grip_force > uniform[1].grip_force * 1.4);
    assert!(
        app.world()
            .resource::<PhysicsWorld>()
            .contacts_with(ground)
            .is_empty()
    );
    app.update();
    let physics = app.world().resource::<PhysicsWorld>();
    assert_eq!(
        physics.wheel_output(wheels[1]).unwrap().normal_force,
        high.normal_force
    );
    assert_eq!(
        physics.wheel_output(wheels[1]).unwrap().grip_force,
        high.grip_force
    );
}

#[test]
fn invalid_layout_retains_last_good_friction_and_pressure_targets() {
    let (mut app, wheels, _) = fixture();
    app.update();
    let before = step(&mut app, wheels);
    app.world_mut()
        .resource_mut::<PhysicsWorld>()
        .set_wheel_pressures(&[(wheels[0], 220_000.0)])
        .unwrap();
    app.world_mut().resource_mut::<FieldLayout>().fields[0].friction = Some(f64::NAN);
    app.update();
    let out = step(&mut app, wheels);
    assert_eq!(out[0].grip_force, 0.0);
    assert_eq!(out[1].grip_force, before[1].grip_force);
    assert_eq!(out[0].pressure.unwrap().target_pressure_pa, 220_000.0);
    {
        let mut layout = app.world_mut().resource_mut::<FieldLayout>();
        layout.fields[0].friction = Some(0.9);
        layout.fields[0].max[0] = 0.5;
    }
    app.update();
    assert_eq!(step(&mut app, wheels)[0].grip_force, 0.0);
    app.world_mut().resource_mut::<FieldLayout>().fields[0].max[0] = 0.0;
    app.world_mut().resource_mut::<FieldLayout>().fields[0].friction = Some(0.9);
    app.update();
    assert!(step(&mut app, wheels)[0].grip_force > 100.0);
}

#[test]
fn deleting_authored_values_or_terrain_restores_collider_friction() {
    let (mut app, wheels, _) = fixture();
    app.update();
    let initial = step(&mut app, wheels);
    assert_eq!(initial[0].grip_force, 0.0);
    {
        let mut layout = app.world_mut().resource_mut::<FieldLayout>();
        layout.default_friction = None;
        layout.fields[0].friction = None;
    }
    app.update();
    let reset = step(&mut app, wheels);
    assert!(reset[0].grip_force > 100.0);
    assert!((reset[0].grip_force - reset[1].grip_force).abs() < 1e-6);
    app.insert_resource(layout());
    app.update();
    assert_eq!(step(&mut app, wheels)[0].grip_force, 0.0);
    app.world_mut().remove_resource::<ProceduralTerrain>();
    app.update();
    let removed = step(&mut app, wheels);
    assert!((removed[0].grip_force - reset[0].grip_force).abs() < 1e-6);
}

#[test]
fn removing_authoring_resources_does_not_allow_live_region_changes() {
    for remove_profiles in [false, true] {
        let (mut app, wheels, _) = fixture();
        app.update();
        assert_eq!(step(&mut app, wheels)[0].grip_force, 0.0);
        let profiles = if remove_profiles {
            app.world_mut().remove_resource::<FieldProfiles>()
        } else {
            app.world_mut().remove_resource::<FieldLayout>();
            None
        };
        app.update();
        let fallback = step(&mut app, wheels);
        assert!(fallback[0].grip_force > 100.0);
        let mut changed = layout();
        changed.fields[0].max[0] = 0.5;
        app.insert_resource(changed);
        if let Some(profiles) = profiles {
            app.insert_resource(profiles);
        }
        app.update();
        assert_eq!(step(&mut app, wheels)[0].grip_force, fallback[0].grip_force);
        app.insert_resource(layout());
        app.update();
        assert_eq!(step(&mut app, wheels)[0].grip_force, 0.0);
    }
}

#[test]
fn unrelated_spawns_do_not_republish_or_reset_tyre_output() {
    let (mut app, wheels, _) = fixture();
    app.update();
    let before = step(&mut app, wheels);
    app.world_mut().spawn_empty();
    *app.world_mut().resource_mut::<FieldLayout>() = layout();
    for _ in 0..4 {
        app.update();
    }
    let physics = app.world().resource::<PhysicsWorld>();
    for (wheel, before) in wheels.into_iter().zip(before) {
        let after = physics.wheel_output(wheel).unwrap();
        assert_eq!(after.normal_force, before.normal_force);
        assert_eq!(after.grip_force, before.grip_force);
        assert_eq!(
            after.pressure.unwrap().pressure_pa,
            before.pressure.unwrap().pressure_pa
        );
    }
}

#[test]
fn replacement_terrain_recovers_after_invalid_layout_and_preserves_pressure() {
    let (mut app, wheels, old_ground) = fixture();
    app.update();
    step(&mut app, wheels);
    let entity = app.world_mut().spawn_empty().id();
    let ground = {
        let mut physics = app.world_mut().resource_mut::<PhysicsWorld>();
        physics.remove_collider(old_ground, true);
        let ground = physics
            .insert_collider(
                ColliderDesc::new(Shape::Cuboid {
                    half_extents: DVec3::new(2.0, 0.1, 2.0),
                })
                .translation(DVec3::new(0.0, -0.1, 0.0))
                .friction(0.6),
            )
            .unwrap();
        physics.register_wheel_ground(ground, None).unwrap();
        physics
            .set_wheel_pressures(&[(wheels[0], 220_000.0)])
            .unwrap();
        ground
    };
    {
        let mut terrain = app.world_mut().resource_mut::<ProceduralTerrain>();
        terrain.entity = entity;
        terrain.grid = Arc::new(HeightGrid::sample(4.0, 0.2, |_, _| 0.0));
        app.world_mut().resource_mut::<super::super::GroundColliders>().0 =
            [((0usize, IVec2::ZERO), ground)].into_iter().collect();
    }
    {
        let mut layout = app.world_mut().resource_mut::<FieldLayout>();
        layout.fields[0].max[0] = 0.25;
        layout.fields[0].friction = Some(-1.0);
    }
    app.update();
    assert!(step(&mut app, wheels)[0].grip_force > 100.0);
    app.world_mut().resource_mut::<FieldLayout>().fields[0].friction = Some(0.0);
    app.update();
    let out = step(&mut app, wheels);
    assert_eq!(out[0].grip_force, 0.0);
    assert!(out[1].grip_force > 100.0);
    assert_eq!(out[0].pressure.unwrap().target_pressure_pa, 220_000.0);
}

#[test]
fn inherited_friction_updates_when_collider_material_changes() {
    let (mut app, wheels, ground) = fixture();
    app.world_mut()
        .resource_mut::<FieldLayout>()
        .default_friction = None;
    app.update();
    let before = step(&mut app, wheels);
    app.world_mut()
        .resource_mut::<PhysicsWorld>()
        .collider_mut(ground)
        .unwrap()
        .set_friction(1.2);
    app.update();
    let after = step(&mut app, wheels);
    assert_eq!(after[0].grip_force, 0.0);
    assert!((after[1].grip_force / before[1].grip_force - 2.0).abs() < 1e-6);
    app.world_mut().resource_mut::<ProceduralTerrain>().grid =
        Arc::new(HeightGrid::sample(4.0, 0.2, |_, _| 0.0));
    app.update();
    let regridded = step(&mut app, wheels);
    assert_eq!(regridded[0].grip_force, 0.0);
    assert!((regridded[1].grip_force - after[1].grip_force).abs() < 1e-6);
}
