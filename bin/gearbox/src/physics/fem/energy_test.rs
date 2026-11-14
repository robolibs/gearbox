use super::*;
use std::io::Write;

#[test]
#[ignore = "CPU asset export through oslo make export-fem-energy"]
fn export_ceol_fem_energy_reference() {
    let (app, spec, layout) = machine_gpu_test::asset();
    let mut rigid = rigid_machine::FemRigidMachine::prepare(app.world(), &layout).unwrap();
    let authored_mass = rigid.model.body_mass.host().unwrap().iter().sum::<f64>();
    let belts = track_mesh::FemTrackMeshes::prepare(app.world(), &spec, &layout, &rigid).unwrap();
    let contacts =
        track_contacts::FemTrackContacts::prepare(app.world(), &spec, &layout, &rigid).unwrap();
    rigid.partition_belt_mass(&belts).unwrap();
    let model = &rigid.model;
    let soft = &belts.model;
    let gravity = model.gravity.host().unwrap();
    assert_eq!(gravity, soft.gravity.host().unwrap());
    assert_eq!(gravity.len(), 1);
    let masses = model.body_mass.host().unwrap();
    let particle_masses = soft.particle_mass.host().unwrap();
    assert!(authored_mass.is_finite() && authored_mass > 0.0);
    assert!((masses.iter().chain(particle_masses).sum::<f64>() - authored_mass).abs() < 1e-5);
    let drive_dofs = layout
        .tracks
        .iter()
        .map(|track| {
            model.joint_dof_offset.host().unwrap()[rigid.joints[&track.drive_joint].index()]
        })
        .collect::<Vec<_>>();
    let result = serde_json::json!({
        "schema":"gearbox_fem_energy_reference_v1", "frame":"island-y-up",
        "asset":std::env::var("GEARBOX_TRACK_ASSET").unwrap(),
        "molla_dependency":include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
            .lines().find(|line| line.starts_with("molla-solvers =")).unwrap(),
        "gravity":gravity[0].to_array(), "body_masses":masses,
        "body_inertia_com_columns":model.body_inertia.host().unwrap().iter()
            .map(|inertia| inertia.to_cols_array()).collect::<Vec<_>>(),
        "body_com":model.body_com.host().unwrap().iter().map(|v| v.to_array()).collect::<Vec<_>>(),
        "initial_poses":rigid.state.body_q.host().unwrap().iter().map(|q| {
            let p = q.position.to_array();
            let r = q.rotation.to_array();
            [p[0],p[1],p[2],r[0],r[1],r[2],r[3]]
        }).collect::<Vec<_>>(),
        "initial_body_linear_velocities":rigid.state.body_qd.host().unwrap().iter()
            .map(|v| v.linear.to_array()).collect::<Vec<_>>(),
        "initial_body_angular_velocities":rigid.state.body_qd.host().unwrap().iter()
            .map(|v| v.angular.to_array()).collect::<Vec<_>>(),
        "particle_masses":particle_masses,
        "initial_positions":belts.state.particle_q.host().unwrap().iter()
            .map(|v| v.to_array()).collect::<Vec<_>>(),
        "initial_particle_velocities":belts.state.particle_qd.host().unwrap().iter()
            .map(|v| v.to_array()).collect::<Vec<_>>(),
        "rest_positions":soft.initial_particle_q.host().unwrap().iter()
            .map(|v| v.to_array()).collect::<Vec<_>>(),
        "material_reference":material_reference(soft),
        "authored_shapes":contacts.shapes.iter().map(|s| serde_json::json!({
            "position":s.position, "rotation":s.rotation, "dimensions":s.data, "ids":s.ids,
        })).collect::<Vec<_>>(),
        "tet_indices":soft.tet_indices.host().unwrap(),
        "spring_a":soft.spring_a.host().unwrap(), "spring_b":soft.spring_b.host().unwrap(),
        "spring_rest_length":soft.spring_rest_length.host().unwrap(),
        "spring_compliance":soft.spring_compliance.host().unwrap(),
        "spring_tension_only":soft.spring_tension_only.host().unwrap(),
        "drive_dofs_to_effort":drive_dofs,
        "joint_types":model.joint_type.host().unwrap(),
        "joint_coord_offsets":model.joint_coord_offset.host().unwrap(),
        "joint_dof_offsets":model.joint_dof_offset.host().unwrap(),
        "joint_limit_lower":model.joint_limit_lower.host().unwrap().iter()
            .map(|v| v.is_finite().then_some(*v)).collect::<Vec<_>>(),
        "joint_limit_upper":model.joint_limit_upper.host().unwrap().iter()
            .map(|v| v.is_finite().then_some(*v)).collect::<Vec<_>>(),
        "joint_velocity_limits":model.joint_velocity_limit.host().unwrap().iter()
            .map(|v| v.is_finite().then_some(*v)).collect::<Vec<_>>(),
        "initial_joint_positions":rigid.state.joint_q.host().unwrap(),
        "joint_target_modes_before_drive_release":rigid.control.joint_target_mode.host().unwrap(),
        "joint_target_positions_before_drive_release":rigid.control.joint_target_pos.host().unwrap(),
        "joint_target_velocities_before_drive_release":rigid.control.joint_target_vel.host().unwrap(),
        "joint_motor_max_force_before_drive_release":rigid.control.joint_motor_max_force.host().unwrap(),
        "joint_target_kp_before_drive_release":rigid.control.joint_target_kp.host().unwrap(),
        "joint_target_kd_before_drive_release":rigid.control.joint_target_kd.host().unwrap(),
        "joint_force_before_drive_release":rigid.control.joint_force.host().unwrap(),
    });
    let path = std::env::var("GEARBOX_FEM_ENERGY_EXPORT").expect("GEARBOX_FEM_ENERGY_EXPORT");
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("energy reference output must not already exist");
    output
        .write_all(&serde_json::to_vec(&result).unwrap())
        .unwrap();
    eprintln!("CPU-only FEM energy reference: {path}; not dynamics acceptance");
}

pub(super) fn material_reference(model: &molla_sim::Model) -> serde_json::Value {
    serde_json::json!({
        "positions":model.initial_particle_q.host().unwrap().iter().map(|q| q.to_array()).collect::<Vec<_>>(),
        "tet_indices":model.tet_indices.host().unwrap(),
        "inverse_rest":model.tet_dm_inv.host().unwrap().iter().map(|m| m.to_cols_array()).collect::<Vec<_>>(),
        "rest_volumes":model.tet_rest_volume.host().unwrap(),
        "particle_inv_mass":model.particle_inv_mass.host().unwrap(),
        "tet_mu":model.tet_mu.host().unwrap(),
        "tet_lambda":model.tet_lambda.host().unwrap(),
        "tet_viscosity":model.tet_viscosity.host().unwrap().iter().map(|v| [v.shear,v.bulk]).collect::<Vec<_>>(),
        "tet_yield":model.tet_yield_stress.host().unwrap(),
        "elastic_only":model.tet_yield_stress.host().unwrap().iter().all(|v| *v > 1e29),
        "spring_a":model.spring_a.host().unwrap(),
        "spring_b":model.spring_b.host().unwrap(),
        "spring_rest_length":model.spring_rest_length.host().unwrap(),
        "spring_tension_only":model.spring_tension_only.host().unwrap(),
        "cable_bending_entries":model.vertex_cable_bending_entries.host().unwrap(),
    })
}
