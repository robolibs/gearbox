use super::*;
use molla_math::{Quat as DQuat, Transform as Pose, Vec3 as DVec3};
use molla_sim::{Contacts, JointTargetMode};
use molla_solvers::{FeatherstoneSolver, Solver};

#[test]
#[ignore = "requires GEARBOX_TRACK_ASSET and GPU through oslo make test-fem-gpu"]
fn authored_ceol_rigid_prediction_momentum_matches_cpu() {
    rigid_prediction_momentum(false, false);
}

#[test]
#[ignore = "requires GEARBOX_TRACK_ASSET and GPU through oslo make test-fem-gpu"]
fn authored_ceol_rigid_prediction_without_stops() {
    rigid_prediction_momentum(true, false);
}

#[test]
#[ignore = "requires GEARBOX_TRACK_ASSET and GPU through oslo make test-fem-gpu"]
fn authored_ceol_rigid_prediction_without_accessory_drives() {
    rigid_prediction_momentum(false, true);
}

#[test]
#[ignore = "requires GEARBOX_TRACK_ASSET and GPU through oslo make test-fem-gpu"]
fn authored_ceol_rigid_prediction_without_stops_or_accessory_drives() {
    rigid_prediction_momentum(true, true);
}

fn rigid_prediction_momentum(remove_stops: bool, remove_drives: bool) {
    eprintln!("rigid diagnostic remove_stops={remove_stops}, remove_drives={remove_drives}");
    let (app, spec, layout) = machine_gpu_test::asset();
    let (render_device, render_queue, _) = tests::gpu_island();
    let device = Arc::new(render_device.wgpu_device().clone());
    let queue = Arc::new((**render_queue.0).clone());
    let mut outcomes = Vec::new();
    for effort in [0.0, 100.0] {
        for gpu in [false, true] {
            let mut rigid = rigid_machine::FemRigidMachine::prepare(app.world(), &layout).unwrap();
            let belts =
                track_mesh::FemTrackMeshes::prepare(app.world(), &spec, &layout, &rigid).unwrap();
            rigid.partition_belt_mass(&belts).unwrap();
            if effort == 0.0 && !gpu {
                for joint in 0..rigid.model.joint_count {
                    let dof = rigid.model.joint_dof_offset.host().unwrap()[joint] as usize;
                    let end = rigid.model.joint_dof_offset.host().unwrap()[joint + 1] as usize;
                    eprintln!(
                        "rigid joint={joint}, type={}, child={}, dof={dof}, lo={:?}, hi={:?}",
                        rigid.model.joint_type.host().unwrap()[joint],
                        rigid.model.joint_child.host().unwrap()[joint],
                        &rigid.model.joint_limit_lower.host().unwrap()[dof..end],
                        &rigid.model.joint_limit_upper.host().unwrap()[dof..end]
                    );
                }
            }
            if remove_stops {
                rigid
                    .model
                    .joint_limit_lower
                    .host_mut()
                    .unwrap()
                    .fill(f64::NEG_INFINITY);
                rigid
                    .model
                    .joint_limit_upper
                    .host_mut()
                    .unwrap()
                    .fill(f64::INFINITY);
            }
            if remove_drives {
                rigid.control.joint_target_kp.host_mut().unwrap().fill(0.0);
                rigid.control.joint_target_kd.host_mut().unwrap().fill(0.0);
            }
            let masses = rigid.model.body_mass.host().unwrap().to_vec();
            let centers = rigid.model.body_com.host().unwrap().to_vec();
            let mass = masses.iter().sum::<f64>();
            let com = |poses: &[Pose]| {
                poses
                    .iter()
                    .enumerate()
                    .map(|(i, q)| q.transform_point(centers[i]) * masses[i])
                    .sum::<DVec3>()
                    / mass
            };
            let initial = com(rigid.state.body_q.host().unwrap());
            for track in &layout.tracks {
                let joint = rigid.joints[&track.drive_joint].index();
                let dof = rigid.model.joint_dof_offset.host().unwrap()[joint] as usize;
                rigid.control.joint_target_mode.host_mut().unwrap()[dof] =
                    JointTargetMode::Effort as u32;
                rigid.control.joint_target_pos.host_mut().unwrap()[dof] = effort;
            }
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
            .with_substeps(1);
            let h = 1.0 / 19200.0;
            let contacts = Contacts::with_capacity(0);
            for step in 1..=512 {
                solver
                    .step(
                        &rigid.model,
                        &rigid.state,
                        &rigid.control,
                        &contacts,
                        h,
                        &mut next,
                    )
                    .unwrap();
                std::mem::swap(&mut rigid.state, &mut next);
                if ![1, 64, 512].contains(&step) {
                    continue;
                }
                let (poses, velocities) = if gpu {
                    let poses = machine_gpu_test::read_floats::<8>(
                        &render_device,
                        &render_queue,
                        rigid.state.body_q.device_buffer().unwrap(),
                    );
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
                    let mut oracle = rigid.model.state().unwrap();
                    if step == 1 || step == 512 {
                        eprintln!("rigid DOFs effort={effort}, gpu={gpu}, step={step}: {qd:?}");
                    }
                    for (out, value) in oracle.joint_q.host_mut().unwrap().iter_mut().zip(q) {
                        *out = value[0] as f64;
                    }
                    for (out, value) in oracle.joint_qd.host_mut().unwrap().iter_mut().zip(qd) {
                        *out = value[0] as f64;
                    }
                    molla_sim::eval_fk(&rigid.model, &mut oracle).unwrap();
                    molla_sim::eval_body_velocities(&rigid.model, &mut oracle).unwrap();
                    assert!(rigid.state.body_q.host().is_err());
                    (
                        poses
                            .iter()
                            .map(|q| Pose {
                                position: DVec3::new(q[0] as f64, q[1] as f64, q[2] as f64),
                                rotation: DQuat::from_xyzw(
                                    q[4] as f64,
                                    q[5] as f64,
                                    q[6] as f64,
                                    q[7] as f64,
                                )
                                .normalize(),
                            })
                            .collect::<Vec<_>>(),
                        oracle
                            .body_qd
                            .host()
                            .unwrap()
                            .iter()
                            .map(|v| v.linear)
                            .collect::<Vec<_>>(),
                    )
                } else {
                    if step == 1 || step == 512 {
                        eprintln!(
                            "rigid DOFs effort={effort}, gpu={gpu}, step={step}: {:?}",
                            rigid.state.joint_qd.host().unwrap()
                        );
                    }
                    (
                        rigid.state.body_q.host().unwrap().to_vec(),
                        rigid
                            .state
                            .body_qd
                            .host()
                            .unwrap()
                            .iter()
                            .map(|v| v.linear)
                            .collect::<Vec<_>>(),
                    )
                };
                let momentum = velocities
                    .iter()
                    .zip(&masses)
                    .map(|(v, m)| *v * *m)
                    .sum::<DVec3>();
                let travel = com(&poses) - initial;
                let expected_momentum = DVec3::NEG_Y * (mass * 9.81 * h * step as f64);
                let error = momentum - expected_momentum;
                assert!(momentum.is_finite() && travel.is_finite());
                eprintln!(
                    "CEOL rigid-only effort={effort}, gpu={gpu}, step={step}: momentum={momentum:?}, error={error:?}, COM travel={travel:?}; no belts, contacts or hitch loops"
                );
                if step == 512 {
                    outcomes.push((effort, gpu, error, travel));
                }
            }
        }
    }
    for &(effort, gpu, error, travel) in &outcomes {
        assert!(
            error.length() < 0.02,
            "rigid momentum failure effort={effort}, gpu={gpu}: {outcomes:?}"
        );
        assert!(
            travel.x.abs().max(travel.z.abs()) < 1e-5,
            "rigid COM failure effort={effort}, gpu={gpu}: {outcomes:?}"
        );
    }
}
