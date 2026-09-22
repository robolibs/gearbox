use super::*;

fn cord_tension(extension: f64, compliance: f64) -> f64 {
    extension.max(0.0) / compliance
}

#[test]
fn cord_tension_is_zero_for_slack_elements() {
    assert_eq!(cord_tension(-0.001, 0.0001), 0.0);
    assert_eq!(cord_tension(0.0, 0.0001), 0.0);
    assert_eq!(cord_tension(0.001, 0.0001), 10.0);
}

#[test]
#[ignore = "asset generator through oslo make export-fem-mass"]
fn export_ceol_fem_mass_properties() {
    use crate::physics::fem::{
        machine::FemMachineLayout, rigid_machine::FemRigidMachine, track_mesh::FemTrackMeshes,
    };
    use molla_math::{Mat3, Vec3 as Vector};
    let input = std::env::var("GEARBOX_TRACK_ASSET").expect("GEARBOX_TRACK_ASSET");
    let output = std::env::var("GEARBOX_FEM_MASS_EXPORT").expect("GEARBOX_FEM_MASS_EXPORT");
    let source = usd_bevy::UsdSource::from_file(Path::new(&input)).unwrap();
    let stage = source.open_stage().unwrap();
    let mut spec = discover_machines_from_stage(&stage).unwrap().remove(0);
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::transform::TransformPlugin,
        bevy::asset::AssetPlugin {
            file_path: "/".into(),
            unapproved_path_mode: bevy::asset::UnapprovedPathMode::Allow,
            ..default()
        },
        usd_bevy::UsdPlugin,
    ));
    app.init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
        .init_asset::<Image>();
    app.insert_resource(PhysicsWorld::with_backend(
        Box::new(MollaBackend::default()),
    ));
    app.finish();
    app.cleanup();
    spec.scene_root = Some(crate::physics::benchmark::project(&mut app, &stage));
    let layout = FemMachineLayout::inspect(app.world_mut(), &spec).unwrap();
    let rigid = FemRigidMachine::prepare(app.world(), &layout).unwrap();
    let belts = FemTrackMeshes::prepare(app.world(), &spec, &layout, &rigid).unwrap();
    let mut result = Vec::new();
    for (authored, track) in spec.tracks.iter().zip(&belts.tracks) {
        let body = rigid.bodies[&track.carrier].index();
        let inverse = rigid.state.body_q.host().unwrap()[body].inverse();
        let mut mass = 0.0;
        let mut first = Vector::ZERO;
        let mut inertia = Mat3::ZERO;
        for &node in &track.nodes {
            let m = belts.model.particle_mass.host().unwrap()[node as usize];
            let p = inverse.transform_point(belts.state.particle_q.host().unwrap()[node as usize]);
            mass += m;
            first += m * p;
            inertia += (Mat3::IDENTITY * p.length_squared()
                - Mat3::from_cols(p * p.x, p * p.y, p * p.z))
                * m;
        }
        assert!((mass - track.mass).abs() < 1e-10);
        let geometry = authored.fem.as_ref().unwrap();
        let stride = geometry.width_stations.len() * (geometry.thickness_cells + 1);
        let indices = track
            .nodes
            .iter()
            .enumerate()
            .map(|(i, &n)| (n, i))
            .collect::<HashMap<_, _>>();
        let mut tension = vec![0.0; authored.treads.len() * geometry.segments_per_pitch];
        let mut strains = Vec::new();
        for i in 0..belts.model.spring_count {
            let a = belts.model.spring_a.host().unwrap()[i];
            let Some(&local) = indices.get(&a) else {
                continue;
            };
            let b = belts.model.spring_b.host().unwrap()[i];
            assert!(indices.contains_key(&b));
            let q = belts.state.particle_q.host().unwrap();
            let rest = belts.model.spring_rest_length.host().unwrap()[i];
            let extension = (q[b as usize] - q[a as usize]).length() - rest;
            assert_ne!(belts.model.spring_tension_only.host().unwrap()[i], 0);
            strains.push(extension / rest);
            tension[local / stride] +=
                cord_tension(extension, belts.model.spring_compliance.host().unwrap()[i]);
        }
        assert_eq!(strains.len(), tension.len() * geometry.width_stations.len());
        let range = |values: &[f64]| {
            [
                values.iter().copied().fold(f64::INFINITY, f64::min),
                values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            ]
        };
        let strain_range = range(&strains);
        let tension_range = range(&tension);
        eprintln!(
            "{} installed cord strain={strain_range:?}, span tension={tension_range:?} N; estimated constitutive data",
            authored.carrier
        );
        result.push(serde_json::json!({"carrier":authored.carrier,"fem":authored.fem,"path":authored.path,
            "combined_mass":rigid.model.body_mass.host().unwrap()[body],"belt_mass":mass,
            "belt_first_moment":first.to_array(),"belt_inertia_origin_columns":inertia.to_cols_array(),
            "installed_cord_strain_range":strain_range,"installed_cord_span_tension_range":tension_range}));
    }
    let json = serde_json::json!({"version":1,"calibration":"estimated","tracks":result});
    std::fs::write(&output, serde_json::to_vec_pretty(&json).unwrap()).unwrap();
    eprintln!("Exported FEM nodal mass properties to {output}; not a simulation acceptance test");
}
