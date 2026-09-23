use super::*;
use molla_sim::Contacts;
use molla_solvers::{FeatherstoneSolver, Solver};

#[test]
#[ignore = "authored servo stability through oslo make test-fem-gpu"]
fn authored_ceol_rigid_servo_feedback_is_stable() {
    let (app, spec, layout) = machine_gpu_test::asset();
    let joint_entity = *layout
        .tree_joints
        .iter()
        .find(|&&entity| {
            app.world()
                .get::<usd_bevy::UsdPrimRef>(entity)
                .is_some_and(|prim| prim.path.ends_with("/top_link"))
        })
        .unwrap();
    let (render_device, render_queue, _) = tests::gpu_island();
    let device = Arc::new(render_device.wgpu_device().clone());
    let queue = Arc::new((**render_queue.0).clone());
    let mut outcomes = Vec::new();
    let mut stable_reference = Vec::new();
    let directory = std::env::var_os("GEARBOX_FEM_CAPTURE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("gearbox-fem-servo"));
    std::fs::create_dir_all(&directory).unwrap();
    for (gpu, stable, subdivisions) in [
        (false, false, 1),
        (false, true, 1),
        (true, false, 1),
        (true, false, 2),
        (true, true, 1),
    ] {
        let mut rigid = rigid_machine::FemRigidMachine::prepare(app.world(), &layout).unwrap();
        let belts =
            track_mesh::FemTrackMeshes::prepare(app.world(), &spec, &layout, &rigid).unwrap();
        rigid.partition_belt_mass(&belts).unwrap();
        let joint = rigid.joints[&joint_entity].index();
        let dof = rigid.model.joint_dof_offset.host().unwrap()[joint] as usize;
        let coordinate = rigid.model.joint_coord_offset.host().unwrap()[joint] as usize;
        let kp = rigid.control.joint_target_kp.host().unwrap()[dof];
        let kd = rigid.control.joint_target_kd.host().unwrap()[dof];
        let cap = rigid.control.joint_motor_max_force.host().unwrap()[dof];
        rigid.control.joint_target_pos.host_mut().unwrap()[dof] += 1e-4;
        let mut next = rigid.model.state().unwrap();
        let mut solver = if gpu {
            rigid.model.upload_to_device(&device, &queue).unwrap();
            rigid.state.upload_to_device(&device, &queue).unwrap();
            next.upload_to_device(&device, &queue).unwrap();
            rigid.control.upload_to_device(&device, &queue).unwrap();
            FeatherstoneSolver::new_gpu_on_device(device.clone(), queue.clone(), &rigid.model)
                .unwrap()
        } else {
            FeatherstoneSolver::new_cpu()
        }
        .with_rigorous_d_solve(true)
        .with_substeps(1)
        .with_stable_motor_feedback(stable);
        let h = 1.0 / (19200.0 * subdivisions as f64);
        let mut samples = Vec::new();
        for step in 1..=96 * subdivisions {
            solver
                .step(
                    &rigid.model,
                    &rigid.state,
                    &rigid.control,
                    &Contacts::with_capacity(0),
                    h,
                    &mut next,
                )
                .unwrap();
            std::mem::swap(&mut rigid.state, &mut next);
            let (position, velocity, all_velocities) = if gpu {
                let status = machine_gpu_test::read_floats::<1>(
                    &render_device,
                    &render_queue,
                    solver.gpu_status_buffer().unwrap(),
                );
                assert_eq!(status[0][0].to_bits(), 0, "servo status at step{step}");
                let q = machine_gpu_test::read_floats::<1>(
                    &render_device,
                    &render_queue,
                    rigid.state.joint_q.device_buffer().unwrap(),
                );
                let qd = machine_gpu_test::read_floats::<1>(
                    &render_device,
                    &render_queue,
                    rigid.state.joint_qd.device_buffer().unwrap(),
                );
                (
                    q[coordinate][0] as f64,
                    qd[dof][0] as f64,
                    qd.iter().map(|v| v[0] as f64).collect::<Vec<_>>(),
                )
            } else {
                (
                    rigid.state.joint_q.host().unwrap()[coordinate],
                    rigid.state.joint_qd.host().unwrap()[dof],
                    rigid.state.joint_qd.host().unwrap().to_vec(),
                )
            };
            assert!(all_velocities.iter().all(|value| value.is_finite()));
            if stable {
                if gpu {
                    let reference: &Vec<f64> = &stable_reference[step - 1];
                    assert_eq!(all_velocities.len(), reference.len());
                    for (dof, (&actual, &expected)) in
                        all_velocities.iter().zip(reference).enumerate()
                    {
                        assert!(
                            (actual - expected).abs() < 1e-4,
                            "stable articulation step={step} dof={dof}: GPU={actual}, CPU={expected}"
                        );
                    }
                } else {
                    stable_reference.push(all_velocities);
                }
            }
            assert!(position.is_finite() && velocity.is_finite());
            samples.push([step as f64 * h, position, velocity]);
        }
        let peak = samples.iter().map(|s| s[2].abs()).fold(0.0_f64, f64::max);
        let max_position = samples.iter().map(|s| s[1].abs()).fold(0.0_f64, f64::max);
        let report = serde_json::json!({
            "gpu":gpu, "stable_feedback":stable, "subdivisions":subdivisions,
            "dt":h, "kp":kp, "kd":kd, "max_force":cap, "target_step_rad":1e-4,
            "peak_speed":peak, "max_position":max_position, "samples":samples,
            "scope":"authored rigid articulation without FEM contact or hitch ball loops",
        });
        std::fs::write(
            directory.join(format!(
                "servo_gpu{gpu}_stable{stable}_subdiv{subdivisions}.json"
            )),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        eprintln!(
            "servo gpu={gpu} stable={stable} subdivisions={subdivisions}: peak={peak}rad/s, max_position={max_position}rad"
        );
        outcomes.push((gpu, stable, subdivisions, peak, max_position, samples));
    }
    for (gpu, stable, subdivisions, speed, position, _) in &outcomes {
        if !stable && *subdivisions == 1 {
            assert!(
                *speed > 1.0,
                "explicit coarse instability control: gpu={gpu}"
            );
        } else {
            assert!(
                *speed < 0.05 && *position < 0.001,
                "authored servo is unstable: gpu={gpu}, stable={stable}, subdivisions={subdivisions}; speed={speed}, position={position}"
            );
        }
    }
    let cpu = &outcomes.iter().find(|case| !case.0 && case.1).unwrap().5;
    let gpu = &outcomes.iter().find(|case| case.0 && case.1).unwrap().5;
    assert_eq!(cpu.len(), gpu.len());
    for (reference, actual) in cpu.iter().zip(gpu) {
        assert!((reference[1] - actual[1]).abs() < 1e-6);
        assert!((reference[2] - actual[2]).abs() < 1e-5);
    }
}
