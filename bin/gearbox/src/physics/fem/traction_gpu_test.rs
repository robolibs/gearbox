use super::*;
use molla_math::{Quat as DQuat, Transform as Pose, Vec3 as DVec3};
use molla_sim::soft_belt::path::BeltPath;
use molla_solvers::fem_rigid_gpu::SoftRigidShapeGpu;

fn pose(q: [f32; 8]) -> Pose {
    Pose {
        position: DVec3::new(q[0] as f64, q[1] as f64, q[2] as f64),
        rotation: DQuat::from_xyzw(q[4] as f64, q[5] as f64, q[6] as f64, q[7] as f64).normalize(),
    }
}

fn radial(path: &BeltPath, mut point: DVec3) -> f64 {
    point.x = 0.0;
    let s = path.project(point).unwrap().0;
    let frame = path.sample(s).unwrap();
    (point - frame.position).dot(frame.outward)
}

#[derive(Debug, serde::Serialize)]
struct Outcome {
    com_travel: f64,
    chassis_travel: f64,
    retention: f64,
    penetration: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    equilibrium: Option<settling_test::Report>,
}

fn run_cases(cases: &[(f64, bool, bool)], monitor_momentum: bool) -> Vec<Outcome> {
    run_cases_with_samples(cases, monitor_momentum, false)
}

fn run_cases_with_samples(
    cases: &[(f64, bool, bool)],
    monitor_momentum: bool,
    capture_samples: bool,
) -> Vec<Outcome> {
    run_trajectory(cases, monitor_momentum, capture_samples, 512, false)
}

fn run_trajectory(
    cases: &[(f64, bool, bool)],
    monitor_momentum: bool,
    capture_samples: bool,
    steps: u64,
    monitor_settling: bool,
) -> Vec<Outcome> {
    run_trajectory_configured(cases, monitor_momentum, capture_samples, steps, monitor_settling, 128, 1.0 / 19200.0)
}

fn run_trajectory_configured(
    cases: &[(f64, bool, bool)],
    monitor_momentum: bool,
    capture_samples: bool,
    steps: u64,
    monitor_settling: bool,
    elastic_iterations: usize,
    substep: f64,
) -> Vec<Outcome> {
    assert!(steps > 0 && steps % 64 == 0);
    run_trajectory_coupled(cases, monitor_momentum, capture_samples, steps, monitor_settling,
        elastic_iterations, substep, None, None)
}

fn run_trajectory_coupled(
    cases: &[(f64, bool, bool)],
    monitor_momentum: bool,
    capture_samples: bool,
    steps: u64,
    monitor_settling: bool,
    elastic_iterations: usize,
    substep: f64,
    material_contact: Option<molla_solvers::fem_rigid_gpu::MaterialContactConfig>,
    reuse_iterations: Option<u32>,
) -> Vec<Outcome> {
    assert!(steps > 0);
    assert!(!monitor_settling || (monitor_momentum && capture_samples));
    let (app, spec, layout, contacts) = machine_gpu_test::checked_wheel_contacts();
    let material_metadata = serde_json::json!({
        "law":"finite_step_green_strain_kelvin_voigt_v1",
        "tracks":spec.tracks.iter().map(|t| {
            let material = t.fem.as_ref().unwrap();
            serde_json::json!({
                "carrier":t.carrier, "calibration":material.calibration,
                "youngs_modulus_Pa":material.youngs_modulus, "poisson_ratio":material.poisson_ratio,
                "shear_viscosity_Pa_s":material.shear_viscosity,
                "bulk_viscosity_Pa_s":material.bulk_viscosity,
            })
        }).collect::<Vec<_>>(),
    });
    let molla_dependency = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .lines().find(|line| line.starts_with("molla-solvers =")).unwrap();
    let mut solver_settings = serde_json::json!({"elastic_iterations":elastic_iterations, "substep_seconds":substep,
        "material_contact":material_contact.map(|c| serde_json::json!({"iterations":c.iterations,
            "linear_tolerance_m_s":c.linear_tolerance, "angular_tolerance_rad_s":c.angular_tolerance}))});
    if let Some(iterations) = reuse_iterations {
        assert!(material_contact.is_some());
        solver_settings["material_reuse_iterations"] = serde_json::json!(iterations);
    }
    assert!(
        spec.tracks
            .iter()
            .all(|t| t.fem.as_ref().unwrap().version == 2)
    );
    let (device, queue, _) = tests::gpu_island();
    let device_lost = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let lost_signal = device_lost.clone();
    device
        .wgpu_device()
        .set_device_lost_callback(move |reason, message| {
            eprintln!("FEM diagnostic device loss: {reason:?}: {message}");
            lost_signal.store(true, std::sync::atomic::Ordering::Release);
        });
    let mut outcomes = Vec::new();
    for &(effort, ground, teeth) in cases {
        let mut rigid = rigid_machine::FemRigidMachine::prepare(app.world(), &layout).unwrap();
        let belts =
            track_mesh::FemTrackMeshes::prepare(app.world(), &spec, &layout, &rigid).unwrap();
        rigid.partition_belt_mass(&belts).unwrap();
        let loops = rigid.ball_joints().unwrap();
        let chassis = rigid.bodies[&layout.chassis].index();
        let carrier = rigid.bodies[&layout.tracks[0].carrier].index();
        let forward = rigid.state.body_q.host().unwrap()[carrier].rotation
            * DVec3::from_array(spec.tracks[0].forward);
        assert!(forward.dot(DVec3::Y).abs() < 1e-6);
        let initial_chassis = rigid.state.body_q.host().unwrap()[chassis].position;
        let body_mass = rigid.model.body_mass.host().unwrap().to_vec();
        let body_com = rigid.model.body_com.host().unwrap().to_vec();
        let masses = belts.model.particle_mass.host().unwrap().to_vec();
        let initial_positions = belts.state.particle_q.host().unwrap().to_vec();
        let failure_reference = monitor_settling.then(|| serde_json::json!({
            "positions":belts.model.initial_particle_q.host().unwrap().iter().map(|q| q.to_array()).collect::<Vec<_>>(),
            "tet_indices":belts.model.tet_indices.host().unwrap(),
            "inverse_rest":belts.model.tet_dm_inv.host().unwrap().iter().map(|m| m.to_cols_array()).collect::<Vec<_>>(),
            "rest_volumes":belts.model.tet_rest_volume.host().unwrap(),
            "particle_inv_mass":belts.model.particle_inv_mass.host().unwrap(),
            "tet_mu":belts.model.tet_mu.host().unwrap(),
            "tet_lambda":belts.model.tet_lambda.host().unwrap(),
            "tet_viscosity":belts.model.tet_viscosity.host().unwrap().iter().map(|v| [v.shear,v.bulk]).collect::<Vec<_>>(),
            "tet_yield":belts.model.tet_yield_stress.host().unwrap(),
            "elastic_only":belts.model.tet_yield_stress.host().unwrap().iter().all(|v| *v > 1e29),
            "spring_a":belts.model.spring_a.host().unwrap(),
            "spring_b":belts.model.spring_b.host().unwrap(),
            "spring_rest_length":belts.model.spring_rest_length.host().unwrap(),
            "spring_tension_only":belts.model.spring_tension_only.host().unwrap(),
            "cable_bending_entries":belts.model.vertex_cable_bending_entries.host().unwrap(),
        }));
        let initial_poses = rigid.state.body_q.host().unwrap().to_vec();
        let triangles = belts
            .tracks
            .iter()
            .flat_map(|t| t.surface.iter().copied())
            .collect::<Vec<_>>();
        let total_mass = body_mass.iter().chain(&masses).sum::<f64>();
        assert!((total_mass - 750.0).abs() < 1e-5);
        let initial_first = rigid
            .state
            .body_q
            .host()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(i, q)| q.transform_point(body_com[i]) * body_mass[i])
            .sum::<DVec3>()
            + belts
                .state
                .particle_q
                .host()
                .unwrap()
                .iter()
                .zip(&masses)
                .map(|(q, m)| *q * *m)
                .sum::<DVec3>();
        let floor = belts
            .state
            .particle_q
            .host()
            .unwrap()
            .iter()
            .map(|p| p.y)
            .fold(f64::INFINITY, f64::min);
        let mut retention = Vec::new();
        let mut regions = vec![u32::MAX; initial_positions.len()];
        for (track, authored) in belts.tracks.iter().zip(&spec.tracks) {
            let points = authored
                .path
                .iter()
                .map(|p| DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64))
                .collect::<Vec<_>>();
            let path = BeltPath::from_closed_polyline(
                &points,
                authored.fem.as_ref().unwrap().thickness * 0.5,
            )
            .unwrap();
            let body = rigid.bodies[&track.carrier].index();
            let inverse = rigid.state.body_q.host().unwrap()[body].inverse();
            let reference = track
                .nodes
                .iter()
                .map(|&n| {
                    let p =
                        inverse.transform_point(belts.state.particle_q.host().unwrap()[n as usize]);
                    (p.x, radial(&path, p))
                })
                .collect::<Vec<_>>();
            let geometry = authored.fem.as_ref().unwrap();
            let base_nodes = authored.treads.len()
                * geometry.segments_per_pitch
                * geometry.width_stations.len()
                * (geometry.thickness_cells + 1);
            let guide_nodes = geometry
                .guide_rows
                .iter()
                .map(|g| {
                    authored.treads.len()
                        * (g.segment_span + 1)
                        * (g.width_cells + 1)
                        * g.depth_cells
                })
                .sum::<usize>();
            for (index, &node) in track.nodes.iter().enumerate() {
                regions[node as usize] = if index < base_nodes {
                    if index % (geometry.thickness_cells + 1) == geometry.thickness_cells / 2 { 0 } else { 1 }
                } else if index < base_nodes + guide_nodes { 2 } else { 3 };
            }
            retention.push((
                body,
                path,
                track.nodes.clone(),
                reference,
            ));
        }
        let drive_bodies = layout
            .tracks
            .iter()
            .map(|t| rigid.bodies[&t.sprocket].index() as u32)
            .collect::<Vec<_>>();
        let dofs = layout
            .tracks
            .iter()
            .map(|t| {
                rigid.model.joint_dof_offset.host().unwrap()[rigid.joints[&t.drive_joint].index()]
                    as usize
            })
            .collect::<Vec<_>>();
        let mut shapes =
            contact_control_test::frictionless_drive_contacts(&contacts, &drive_bodies, teeth);
        shapes.push(SoftRigidShapeGpu {
            position: [
                if ground { 0.0 } else { 100.0 },
                floor as f32 - 0.5,
                0.0,
                0.0,
            ],
            rotation: [0.0, 0.0, 0.0, 1.0],
            data: [5.0, 0.5, 5.0, 0.85],
            ids: [1, u32::MAX, 0, 0],
        });
        let shape_trace = shapes
            .iter()
            .map(|s| {
                serde_json::json!({
                    "position":s.position, "rotation":s.rotation, "dimensions":s.data, "ids":s.ids,
                })
            })
            .collect::<Vec<_>>();
        eprintln!("constructing full FEM island: effort={effort}, ground={ground}, teeth={teeth}");
        let mut island = FemGpuIsland::with_ball_joints(
            &device,
            &queue,
            FemRigidGpuScene {
                soft_model: belts.model,
                soft_state: belts.state,
                rigid_model: rigid.model,
                rigid_state: rigid.state,
                control: rigid.control,
                shapes,
            },
            FemRigidConfig {
                max_substep: substep,
                elastic_iterations,
                ..default()
            },
            &loops,
        )
        .unwrap();
        eprintln!("full FEM island ready: effort={effort}, ground={ground}, teeth={teeth}");
        if let Some(config) = material_contact {
            island.system.configure_material_contact(Some(config)).unwrap();
            island.system.configure_material_contact_reuse(reuse_iterations).unwrap();
        }
        if std::env::var("GEARBOX_FEM_GPU_TIMING").as_deref() == Ok("1") {
            island.system.configure_gpu_timing(true).unwrap();
        }
        if monitor_momentum {
            island.system.enable_momentum_diagnostics();
        }
        for &dof in &dofs {
            island.system.set_drive_effort(dof, effort).unwrap();
        }
        let directory = std::env::var_os("GEARBOX_FEM_CAPTURE_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("gearbox-fem-traction"));
        std::fs::create_dir_all(&directory).unwrap();
        let sample_directory =
            directory.join(format!("effort{effort}_ground{ground}_teeth{teeth}"));
        if capture_samples {
            std::fs::create_dir_all(&sample_directory).unwrap();
        }
        let mut next_sample = 64.min(steps);
        let mut settling = settling_test::Monitor::new(total_mass, DVec3::NEG_Y * 9.81);
        let mut equilibrium = None;
        let started = std::time::Instant::now();
        let mut reported = 0;
        let mut heartbeat = started;
        while island.clock.completed < steps {
            assert!(
                !device_lost.load(std::sync::atomic::Ordering::Acquire),
                "FEM device lost"
            );
            assert!(
                started.elapsed().as_secs() < if monitor_settling { 7200 } else { 1800 },
                "ground coupling timeout: effort={effort}, ground={ground}, teeth={teeth}, completed={}",
                island.clock.completed
            );
            let boundary = if capture_samples { next_sample.min(steps) } else { steps };
            if let Err(error) = island.advance(island.clock.submitted < boundary) {
                if monitor_settling {
                    let p = machine_gpu_test::read_floats::<4>(&device, &queue, island.positions());
                    let v = machine_gpu_test::read_floats::<4>(&device, &queue, island.system.soft_state().particle_qd.device_buffer().unwrap());
                    let diagnostics = machine_gpu_test::read_floats::<1>(&device, &queue, island.system.volume_diagnostics_buffer());
                    let failure = serde_json::json!({
                        "error":error.to_string(), "accepted_steps":island.clock.completed,
                        "submitted_steps":island.clock.submitted, "rejected_time":island.clock.submitted as f64 * substep,
                        "elapsed_wall_seconds":started.elapsed().as_secs_f64(),
                        "minimum_j":diagnostics[0][0], "invalid_tet":diagnostics[1][0].to_bits(),
                        "device_status":machine_gpu_test::read_floats::<1>(&device, &queue, island.system.status_buffer())[0][0].to_bits(),
                        "tet_materials":material_metadata, "molla_dependency":molla_dependency,
                        "solver_settings":solver_settings, "reference":failure_reference,
                        "gpu_phase_ticks":island.system.gpu_timing_ticks().map(|buffer|
                            machine_gpu_test::read_ticks(&device, &queue, buffer)),
                        "gpu_timestamp_period_ns":queue.get_timestamp_period(),
                        "material_contact_residuals":island.system.material_contact_diagnostics().map(|buffer|
                            machine_gpu_test::read_floats::<1>(&device, &queue, buffer).into_iter().flatten().collect::<Vec<_>>()),
                        "material_force_residual":island.system.material_force_diagnostics().map(|buffer|
                            machine_gpu_test::read_floats::<1>(&device, &queue, buffer)[0][0]),
                        "material_force_samples":island.system.material_force_samples().map(|buffer|
                            machine_gpu_test::read_floats::<4>(&device, &queue, buffer)),
                        "material_spring_stiffness":island.system.material_spring_stiffness().map(|buffer|
                            machine_gpu_test::read_floats::<1>(&device, &queue, buffer).into_iter().flatten().collect::<Vec<_>>()),
                        "plastic_history":machine_gpu_test::read_floats::<12>(&device, &queue, island.system.soft_state().tet_f_plastic.device_buffer().unwrap()),
                        "positions":p, "velocities":v, "regions":regions,
                        "shapes":shape_trace,
                        "initial_positions":initial_positions.iter().map(|p| p.to_array()).collect::<Vec<_>>(),
                        "particle_masses":masses,
                        "poses":machine_gpu_test::read_floats::<8>(&device, &queue, island.rigid_poses()),
                    });
                    let path = sample_directory.join("rejected.json");
                    std::fs::write(&path, serde_json::to_vec(&failure).unwrap()).unwrap();
                    eprintln!("rejected FEM state: {}", path.display());
                }
                panic!("FEM trajectory rejected: {error}");
            }
            if capture_samples && island.clock.completed == next_sample && !island.pending() {
                let p = machine_gpu_test::read_floats::<4>(&device, &queue, island.positions());
                let q = machine_gpu_test::read_floats::<8>(&device, &queue, island.rigid_poses());
                let v = machine_gpu_test::read_floats::<4>(
                    &device,
                    &queue,
                    island
                        .system
                        .soft_state()
                        .particle_qd
                        .device_buffer()
                        .unwrap(),
                );
                let qd = machine_gpu_test::read_floats::<1>(
                    &device,
                    &queue,
                    island
                        .system
                        .rigid_state()
                        .joint_qd
                        .device_buffer()
                        .unwrap(),
                );
                let momentum = island
                    .system
                    .momentum_diagnostics_buffer()
                    .map(|buffer| machine_gpu_test::read_floats::<4>(&device, &queue, buffer));
                let body_velocities = monitor_settling.then(|| {
                    machine_gpu_test::read_floats::<8>(
                        &device,
                        &queue,
                        island.system.rigid_state().body_qd.device_buffer().unwrap(),
                    )
                });
                if let Some(body_velocities) = &body_velocities {
                    let momentum = momentum.as_ref().unwrap();
                    assert_eq!(momentum.len(), 20);
                    assert!(
                        momentum
                            .iter()
                            .flatten()
                            .chain(body_velocities.iter().flatten())
                            .all(|v| v.is_finite())
                    );
                    let vector = |v: &[f32]| DVec3::new(v[0] as f64, v[1] as f64, v[2] as f64);
                    let report = settling.observe(settling_test::Sample {
                        time: island.completed_seconds(),
                        momentum: vector(&momentum[8]) + vector(&momentum[9]),
                        contact_impulse: vector(&momentum[16]) + vector(&momentum[17]),
                        max_particle_speed: v
                            .iter()
                            .map(|v| vector(v).length())
                            .fold(0.0, f64::max),
                        max_body_speed: body_velocities
                            .iter()
                            .map(|v| vector(v).length())
                            .fold(0.0, f64::max),
                        max_body_angular_speed: body_velocities
                            .iter()
                            .map(|v| vector(&v[4..]).length())
                            .fold(0.0, f64::max),
                    });
                    eprintln!("passive equilibrium: {report:?}");
                    equilibrium = Some(report);
                }
                assert!(
                    p.iter()
                        .flatten()
                        .chain(q.iter().flatten())
                        .chain(v.iter().flatten())
                        .chain(qd.iter().flatten())
                        .all(|v| v.is_finite())
                );
                let body_poses = q.into_iter().map(pose).collect::<Vec<_>>();
                let first = body_poses
                    .iter()
                    .enumerate()
                    .map(|(i, q)| q.transform_point(body_com[i]) * body_mass[i])
                    .sum::<DVec3>()
                    + p.iter()
                        .zip(&masses)
                        .map(|(p, m)| DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64) * *m)
                        .sum::<DVec3>();
                let com_travel = ((first - initial_first) / total_mass).dot(forward);
                let speeds = dofs.iter().map(|&d| qd[d][0]).collect::<Vec<_>>();
                eprintln!(
                    "ground trajectory: step={next_sample}, com_travel={com_travel}m, drive_speeds={speeds:?}rad/s"
                );
                let poses = body_poses
                    .iter()
                    .map(|q| {
                        let p = q.position.to_array();
                        let r = q.rotation.to_array();
                        [p[0], p[1], p[2], r[0], r[1], r[2], r[3]]
                    })
                    .collect::<Vec<_>>();
                let sample = serde_json::json!({
                    "step":next_sample, "time":island.completed_seconds(),
                    "accepted_steps":island.clock.completed, "submitted_steps":island.clock.submitted,
                    "device_status":machine_gpu_test::read_floats::<1>(&device, &queue, island.system.status_buffer())[0][0].to_bits(),
                    "reference":failure_reference, "regions":regions,
                    "initial_positions":initial_positions.iter().map(|p| p.to_array()).collect::<Vec<_>>(),
                    "com_travel":com_travel, "drive_speeds":speeds,
                    "positions":p.iter().map(|v| [v[0],v[1],v[2]]).collect::<Vec<_>>(),
                    "velocities":v, "poses":poses, "joint_velocities":qd,
                    "momentum_phase_pairs_then_cumulative_deltas":momentum,
                    "body_velocities":body_velocities, "equilibrium":equilibrium,
                    "stable_motor_feedback":true,
                    "tet_materials":material_metadata, "molla_dependency":molla_dependency,
                    "solver_settings":solver_settings,
                    "gpu_phase_ticks":island.system.gpu_timing_ticks().map(|buffer|
                        machine_gpu_test::read_ticks(&device, &queue, buffer)),
                    "gpu_timestamp_period_ns":queue.get_timestamp_period(),
                    "material_contact_residuals":island.system.material_contact_diagnostics().map(|buffer|
                        machine_gpu_test::read_floats::<1>(&device, &queue, buffer).into_iter().flatten().collect::<Vec<_>>()),
                    "material_force_residual":island.system.material_force_diagnostics().map(|buffer|
                        machine_gpu_test::read_floats::<1>(&device, &queue, buffer)[0][0]),
                    "material_force_samples":island.system.material_force_samples().map(|buffer|
                        machine_gpu_test::read_floats::<4>(&device, &queue, buffer)),
                    "material_spring_stiffness":island.system.material_spring_stiffness().map(|buffer|
                        machine_gpu_test::read_floats::<1>(&device, &queue, buffer).into_iter().flatten().collect::<Vec<_>>()),
                    "plastic_history":machine_gpu_test::read_floats::<12>(&device, &queue, island.system.soft_state().tet_f_plastic.device_buffer().unwrap()),
                });
                std::fs::write(
                    sample_directory.join(format!("step{next_sample:04}.json")),
                    serde_json::to_vec(&sample).unwrap(),
                )
                .unwrap();
                next_sample += 64;
            }
            if heartbeat.elapsed().as_secs() >= 15 {
                let queue_complete = island.system.poll_completion().unwrap();
                eprintln!(
                    "FEM heartbeat: submitted={}, completed={}, diagnostics_pending={}, queued={}, queue_complete={queue_complete}, phases={:?}, contacts={:?}, elapsed={:?}",
                    island.clock.submitted,
                    island.clock.completed,
                    island.diagnostics.pending(),
                    island.system.queued_substeps(),
                    island.system.completed_momentum_phases(),
                    island.system.completed_contact_passes(),
                    started.elapsed()
                );
                if std::env::var_os("GEARBOX_FEM_DIAG_WAIT").is_some() {
                    let result = device.wgpu_device().poll(wgpu::PollType::Wait {
                        submission_index: None,
                        timeout: Some(std::time::Duration::from_secs(1)),
                    });
                    eprintln!("FEM bounded diagnostic wait: {result:?}");
                }
                heartbeat = std::time::Instant::now();
            }
            if island.clock.completed >= reported + 64 {
                reported = island.clock.completed;
                eprintln!(
                    "ground effort={effort}, ground={ground}, teeth={teeth}: {reported}/{steps} in {:?}",
                    started.elapsed()
                );
            }
            std::thread::yield_now();
        }
        assert!(island.system.soft_state().particle_q.host().is_err());
        assert!(island.system.rigid_state().body_q.host().is_err());
        let positions = machine_gpu_test::read_floats::<4>(&device, &queue, island.positions());
        let body_poses = machine_gpu_test::read_floats::<8>(&device, &queue, island.rigid_poses());
        assert!(
            positions
                .iter()
                .flatten()
                .chain(body_poses.iter().flatten())
                .all(|v| v.is_finite())
        );
        let positions = positions
            .iter()
            .map(|q| DVec3::new(q[0] as f64, q[1] as f64, q[2] as f64))
            .collect::<Vec<_>>();
        let body_poses = body_poses.into_iter().map(pose).collect::<Vec<_>>();
        let momentum = island.system.momentum_diagnostics_buffer().map(|buffer| {
            let values = machine_gpu_test::read_floats::<4>(&device, &queue, buffer);
            assert_eq!(values.len(), 20);
            assert!(values.iter().flatten().all(|v| v.is_finite()));
            let along = |v: [f32; 4]| DVec3::new(v[0] as f64, v[1] as f64, v[2] as f64).dot(forward);
            let phase_deltas = (1..=4).map(|i| along(values[10 + i * 2]) + along(values[11 + i * 2])).collect::<Vec<_>>();
            eprintln!("cumulative forward momentum deltas (FEM, rigid, contact, drift)={phase_deltas:?} kg m/s; final={} kg m/s", along(values[8]) + along(values[9]));
            values
        });
        let final_first = body_poses
            .iter()
            .enumerate()
            .map(|(i, q)| q.transform_point(body_com[i]) * body_mass[i])
            .sum::<DVec3>()
            + positions
                .iter()
                .zip(&masses)
                .map(|(q, m)| *q * *m)
                .sum::<DVec3>();
        let mut departure = 0.0_f64;
        let mut region_departures = [0.0_f64; 4];
        let mut errors = vec![0.0; positions.len()];
        let mut worst = String::new();
        for (body, path, nodes, reference) in retention {
            let inverse = body_poses[body].inverse();
            for (index, (node, (x, offset))) in nodes.into_iter().zip(reference).enumerate() {
                let p = inverse.transform_point(positions[node as usize]);
                let lateral = (p.x - x).abs();
                let radial = (radial(&path, p) - offset).abs();
                let error = lateral.max(radial);
                let region = regions[node as usize] as usize;
                region_departures[region] = region_departures[region].max(error);
                errors[node as usize] = error;
                if error > departure {
                    departure = error;
                    worst = format!(
                        "body={body}, node={node}, local_index={index}, region={region}, local={p:?}, initial_x={x}, initial_radial={offset}, lateral_error={lateral}, radial_error={radial}"
                    );
                }
            }
        }
        let outcome = Outcome {
            com_travel: ((final_first - initial_first) / total_mass).dot(forward),
            chassis_travel: (body_poses[chassis].position - initial_chassis).dot(forward),
            retention: departure,
            penetration: if ground {
                (floor - positions.iter().map(|p| p.y).fold(f64::INFINITY, f64::min)).max(0.0)
            } else {
                0.0
            },
            equilibrium,
        };
        eprintln!(
            "ground outcome effort={effort}, ground={ground}, teeth={teeth}: {outcome:?}; accepted={}s; minJ={:?}; region_departures(cord,carcass,guide,tread)={region_departures:?}; worst={worst}; short coupling gate, not sustained driving acceptance",
            island.completed_seconds(),
            island.minimum_j
        );
        let poses = |values: &[Pose]| {
            values
                .iter()
                .map(|q| {
                    let p = q.position.to_array();
                    let r = q.rotation.to_array();
                    [p[0], p[1], p[2], r[0], r[1], r[2], r[3]]
                })
                .collect::<Vec<_>>()
        };
        let trace = serde_json::json!({
            "asset":std::env::var("GEARBOX_TRACK_ASSET").unwrap(),
            "frame":"island-y-up", "effort":effort, "ground":ground, "teeth":teeth,
            "stable_motor_feedback":true,
            "tet_materials":material_metadata, "molla_dependency":molla_dependency,
            "solver_settings":solver_settings,
            "drive_contact_control":"tooth boxes only; all smooth sprocket supports retained",
            "time":island.completed_seconds(), "minimum_j":island.minimum_j, "outcome":outcome,
            "initial_positions":initial_positions.iter().map(|p| p.to_array()).collect::<Vec<_>>(),
            "positions":positions.iter().map(|p| p.to_array()).collect::<Vec<_>>(),
            "triangles":triangles, "initial_poses":poses(&initial_poses), "poses":poses(&body_poses),
            "shapes":shape_trace, "regions":regions, "retention_errors":errors, "worst":worst,
            "particle_masses":masses, "body_masses":body_mass,
            "body_com":body_com.iter().map(|p| p.to_array()).collect::<Vec<_>>(),
            "forward":forward.to_array(), "drive_bodies":drive_bodies, "drive_dofs":dofs,
            "momentum_phase_pairs_then_cumulative_deltas":momentum,
        });
        let path = directory.join(format!("effort{effort}_ground{ground}_teeth{teeth}.json"));
        std::fs::write(&path, serde_json::to_vec(&trace).unwrap()).unwrap();
        eprintln!("ground diagnostic snapshot: {}", path.display());
        outcomes.push(outcome);
    }
    outcomes
}

#[test]
#[ignore = "test-only equilibrium readback through oslo make test-fem-gpu"]
fn authored_ceol_passive_equilibrium_probe() {
    let outcomes = run_trajectory(&[(0.0, true, true)], true, true, 2048, true);
    let outcome = &outcomes[0];
    assert!(outcome.retention < 0.020, "passive retention: {outcomes:?}");
    assert!(
        outcome.penetration < 0.002,
        "passive penetration: {outcomes:?}"
    );
    assert!(
        outcome.equilibrium.as_ref().unwrap().sustained,
        "not in passive equilibrium: {outcomes:?}"
    );
}

#[test]
#[ignore = "test-only short iteration-convergence probe, not settled acceptance"]
fn authored_ceol_viscosity_iteration_probe() {
    run_trajectory_configured(&[(0.0, true, true)], true, true, 256, true, 256, 1.0 / 19200.0);
}

#[test]
#[ignore = "test-only short timestep-convergence probe, not settled acceptance"]
fn authored_ceol_viscosity_half_step_probe() {
    run_trajectory_configured(&[(0.0, true, true)], true, true, 512, true, 256, 1.0 / 38400.0);
}

#[test]
#[ignore = "test-only short timestep-convergence probe, not settled acceptance"]
fn authored_ceol_viscosity_quarter_step_probe() {
    run_trajectory_configured(&[(0.0, true, true)], true, true, 1024, true, 256, 1.0 / 76800.0);
}

#[test]
#[ignore = "test-only short timestep-convergence probe, not settled acceptance"]
fn authored_ceol_viscosity_eighth_step_probe() {
    run_trajectory_configured(&[(0.0, true, true)], true, true, 2048, true, 256, 1.0 / 153600.0);
}

#[test]
#[ignore = "test-only single coupled substep, not settling or traction acceptance"]
fn authored_ceol_material_contact_single_step_probe() {
    run_trajectory_coupled(&[(0.0, true, true)], true, true, 1, true, 256, 1.0 / 19200.0,
        Some(molla_solvers::fem_rigid_gpu::MaterialContactConfig {
            iterations: 256, linear_tolerance: 1e-4, angular_tolerance: 1e-4,
        }), None);
}

#[test]
#[ignore = "test-only retained material substep, not settling or traction acceptance"]
fn authored_ceol_retained_material_single_step_probe() {
    run_trajectory_coupled(&[(0.0, true, true)], true, true, 1, true, 256, 1.0 / 19200.0,
        Some(molla_solvers::fem_rigid_gpu::MaterialContactConfig {
            iterations: 256, linear_tolerance: 1e-4, angular_tolerance: 1e-4,
        }), Some(8));
}

#[test]
#[ignore = "test-only retained material refinement, not settling or traction acceptance"]
fn authored_ceol_retained_material_512_step_probe() {
    run_trajectory_coupled(&[(0.0, true, true)], true, true, 1, true, 256, 1.0 / 19200.0,
        Some(molla_solvers::fem_rigid_gpu::MaterialContactConfig {
            iterations: 512, linear_tolerance: 1e-4, angular_tolerance: 1e-4,
        }), Some(8));
}

#[test]
#[ignore = "test-only trajectory readback through oslo make test-fem-gpu"]
fn authored_ceol_loaded_drive_trajectory_diagnostic() {
    let outcomes = run_cases_with_samples(&[(100.0, true, true)], true, true);
    assert!(
        outcomes[0].retention < 0.020,
        "loaded retention: {outcomes:?}"
    );
    assert!(
        outcomes[0].penetration < 0.002,
        "loaded penetration: {outcomes:?}"
    );
}

#[test]
#[ignore = "requires tread-complete GEARBOX_TRACK_ASSET through oslo make test-fem-gpu"]
fn authored_ceol_passive_ground_retention_diagnostic() {
    let outcomes = run_cases_with_samples(&[(0.0, true, true)], true, true);
    assert!(
        outcomes[0].retention < 0.020,
        "passive retention failed: {outcomes:?}"
    );
    assert!(
        outcomes[0].penetration < 0.002,
        "passive ground penetration: {outcomes:?}"
    );
}

#[test]
#[ignore = "requires tread-complete GEARBOX_TRACK_ASSET through oslo make test-fem-gpu"]
fn authored_ceol_powered_ground_coupling_has_causal_controls() {
    let outcomes = run_cases(
        &[
            (0.0, true, true),
            (100.0, false, true),
            (100.0, true, false),
            (100.0, true, true),
            (-100.0, true, true),
        ],
        false,
    );
    for outcome in &outcomes {
        assert!(
            outcome.retention < 0.020,
            "belt left wheel path: {outcomes:?}"
        );
        assert!(
            outcome.penetration < 0.002,
            "belt penetrated ground: {outcomes:?}"
        );
    }
    let passive = outcomes[0].com_travel;
    let air = outcomes[1].com_travel;
    let disconnected = outcomes[2].com_travel;
    let forward = outcomes[3].com_travel;
    let reverse = outcomes[4].com_travel;
    assert!(
        air.abs() < 1e-5,
        "airborne system gained horizontal COM travel: {outcomes:?}"
    );
    assert!(
        forward > passive + 1e-4 && forward > air + 1e-4 && forward > disconnected + 1e-4,
        "no causal tooth-to-ground propulsion: {outcomes:?}"
    );
    assert!(
        reverse < passive - 1e-4,
        "no reverse ground propulsion: {outcomes:?}"
    );
    assert!(
        outcomes[3].chassis_travel > outcomes[0].chassis_travel + 5e-5
            && outcomes[4].chassis_travel < outcomes[0].chassis_travel - 5e-5,
        "ground traction did not move chassis: {outcomes:?}"
    );
}

#[test]
#[ignore = "requires tread-complete GEARBOX_TRACK_ASSET through oslo make test-fem-gpu"]
fn authored_ceol_airborne_momentum_diagnostic() {
    let outcomes = run_cases(&[(100.0, false, true)], true);
    assert!(
        outcomes[0].retention < 0.020,
        "airborne retention failed: {outcomes:?}"
    );
    assert!(
        outcomes[0].com_travel.abs() < 1e-5,
        "airborne horizontal COM drift: {outcomes:?}"
    );
}

#[test]
#[ignore = "requires tread-complete GEARBOX_TRACK_ASSET through oslo make test-fem-gpu"]
fn authored_ceol_toothless_ground_retention_diagnostic() {
    let outcomes = run_cases(&[(100.0, true, false)], false);
    assert!(
        outcomes[0].retention < 0.020,
        "toothless retention failed: {outcomes:?}"
    );
    assert!(
        outcomes[0].penetration < 0.002,
        "toothless ground penetration: {outcomes:?}"
    );
}
