use super::*;

fn checked_plan(
    world: &mut World,
    spec: &MachineInstanceSpec,
    layout: &FemMachineLayout,
    rigid: &FemRigidMachine,
    belts: &FemTrackMeshes,
) -> FemMachineVisuals {
    let plan = FemMachineVisuals::prepare(world, spec, layout, rigid, belts).unwrap();
    assert_eq!(plan.soft.len(), 2);
    assert!(plan.rigid.len() > 100);
    for (soft, track) in plan.soft.iter().zip(&belts.tracks) {
        assert_eq!(soft.originals.len(), 57);
        assert_eq!(soft.mesh.count_vertices(), track.surface.len() * 3);
        let material = world
            .resource::<Assets<StandardMaterial>>()
            .get(&soft.material)
            .unwrap();
        assert!(material.perceptual_roughness > 0.8);
        let color = material.base_color.to_linear();
        assert!(color.red < 0.03 && color.green < 0.03 && color.blue < 0.03);
    }
    for mesh in &plan.rigid {
        assert_eq!(
            world
                .get::<MeshMaterial3d<StandardMaterial>>(mesh.entity)
                .unwrap()
                .0,
            mesh.material
        );
        let pose = rigid.state.body_q.host().unwrap()[mesh.body as usize];
        let p = pose.position;
        let q = pose.rotation;
        let body = Mat4::from_rotation_translation(
            Quat::from_xyzw(q.x as f32, q.y as f32, q.z as f32, q.w as f32),
            Vec3::new(p.x as f32, p.y as f32, p.z as f32),
        );
        let actual = Mat4::from_translation(plan.origin) * body * mesh.mesh_to_body;
        let expected = world
            .get::<GlobalTransform>(mesh.entity)
            .unwrap()
            .to_matrix();
        assert!(
            actual
                .to_cols_array()
                .iter()
                .zip(expected.to_cols_array())
                .all(|(a, b)| (a - b).abs() < 1e-5)
        );
        assert!(
            !plan
                .soft
                .iter()
                .any(|s| s.originals.iter().any(|&e| beneath(world, mesh.entity, e)))
        );
    }
    eprintln!(
        "CEOL GPU visual preparation: {} rigid meshes, {} FEM surfaces, {} replaced belt roots",
        plan.rigid.len(),
        plan.soft.len(),
        plan.soft.iter().map(|s| s.originals.len()).sum::<usize>()
    );
    plan
}

#[test]
#[ignore = "requires explicit GEARBOX_TRACK_ASSET visuals through oslo make test-fem-visuals"]
fn ceol_fem_visual_binding_preserves_materials_and_instances() {
    let (mut app, mut spec, layout) = super::super::machine_gpu_test::asset();
    let rigid = FemRigidMachine::prepare(app.world(), &layout).unwrap();
    let belts = FemTrackMeshes::prepare(app.world(), &spec, &layout, &rigid).unwrap();
    let plan = checked_plan(app.world_mut(), &spec, &layout, &rigid, &belts);
    let tread_root = plan.soft[0].originals[1];
    let tread_mesh = {
        let world = app.world_mut();
        world
            .query_filtered::<Entity, With<Mesh3d>>()
            .iter(world)
            .find(|&e| beneath(world, e, tread_root))
            .unwrap()
    };
    let material = app
        .world()
        .get::<MeshMaterial3d<StandardMaterial>>(tread_mesh)
        .unwrap()
        .clone();
    let different = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());
    app.world_mut()
        .entity_mut(tread_mesh)
        .insert(MeshMaterial3d(different));
    assert!(
        FemMachineVisuals::prepare(app.world_mut(), &spec, &layout, &rigid, &belts)
            .err()
            .unwrap()
            .contains("shared rubber material")
    );
    app.world_mut().entity_mut(tread_mesh).insert(material);
    let original = spec.tracks[0].fem_visuals.clone().unwrap();
    let carrier = spec.tracks[0].carrier.clone();
    for fault in 0..6 {
        let visual = spec.tracks[0].fem_visuals.as_mut().unwrap();
        match fault {
            0 => visual.version = 2,
            1 => visual.deformable.push(visual.deformable[0].clone()),
            2 => visual.material = carrier.clone(),
            3 => {
                visual.deformable.pop();
            }
            4 => visual.deformable[0] = carrier.clone(),
            _ => visual.deformable[0] = "/another_machine/rubber".into(),
        }
        assert!(
            FemMachineVisuals::prepare(app.world_mut(), &spec, &layout, &rigid, &belts).is_err(),
            "accepted visual fault {fault}"
        );
        spec.tracks[0].fem_visuals = Some(original.clone());
    }
    spec.tracks[0].fem_visuals = None;
    assert!(FemMachineVisuals::prepare(app.world_mut(), &spec, &layout, &rigid, &belts).is_err());
    spec.tracks[0].fem_visuals = Some(original);
    let source = usd_bevy::UsdSource::from_file(std::path::Path::new(
        &std::env::var("GEARBOX_TRACK_ASSET").unwrap(),
    ))
    .unwrap();
    let stage = source.open_stage().unwrap();
    let second_root = crate::physics::benchmark::project(&mut app, &stage);
    spec.scene_root = Some(second_root);
    *app.world_mut().get_mut::<Transform>(second_root).unwrap() =
        Transform::from_xyz(10.0, 2.0, -4.0).with_rotation(Quat::from_rotation_y(0.3));
    app.update();
    let second_layout = FemMachineLayout::inspect(app.world_mut(), &spec).unwrap();
    let second_rigid = FemRigidMachine::prepare(app.world(), &second_layout).unwrap();
    let second_belts =
        FemTrackMeshes::prepare(app.world(), &spec, &second_layout, &second_rigid).unwrap();
    let second = checked_plan(
        app.world_mut(),
        &spec,
        &second_layout,
        &second_rigid,
        &second_belts,
    );
    assert_eq!(plan.rigid.len(), second.rigid.len());
    assert!((plan.origin - second.origin).length() > 10.0);
    assert!(
        plan.rigid
            .iter()
            .all(|a| second.rigid.iter().all(|b| a.entity != b.entity))
    );
    assert!(
        plan.soft
            .iter()
            .flat_map(|s| &s.originals)
            .all(|a| second.soft.iter().all(|s| !s.originals.contains(a)))
    );
    assert!(
        FemMachineVisuals::prepare(
            app.world_mut(),
            &spec,
            &second_layout,
            &rigid,
            &second_belts
        )
        .is_err()
    );
}
