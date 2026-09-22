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
}

fn run_cases(cases: &[(f64, bool, bool)], monitor_momentum: bool) -> Vec<Outcome> {
    let (app, spec, layout, contacts) = machine_gpu_test::checked_wheel_contacts();
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
            retention.push((
                body,
                path,
                track.nodes.clone(),
                reference,
                base_nodes,
                base_nodes + guide_nodes,
                geometry.thickness_cells,
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
                max_substep: 1.0 / 19200.0,
                elastic_iterations: 128,
                ..default()
            },
            &loops,
        )
        .unwrap();
        eprintln!("full FEM island ready: effort={effort}, ground={ground}, teeth={teeth}");
        if monitor_momentum {
            island.system.enable_momentum_diagnostics();
        }
        for dof in dofs {
            island.system.set_drive_effort(dof, effort).unwrap();
        }
        let started = std::time::Instant::now();
        let mut reported = 0;
        let mut heartbeat = started;
        while island.clock.completed < 512 {
            assert!(
                !device_lost.load(std::sync::atomic::Ordering::Acquire),
                "FEM device lost"
            );
            assert!(
                started.elapsed().as_secs() < 1800,
                "ground coupling timeout: effort={effort}, ground={ground}, teeth={teeth}, completed={}",
                island.clock.completed
            );
            island.advance(island.clock.submitted < 512).unwrap();
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
                    "ground effort={effort}, ground={ground}, teeth={teeth}: {reported}/512 in {:?}",
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
        let mut regions = vec![u32::MAX; positions.len()];
        let mut errors = vec![0.0; positions.len()];
        let mut worst = String::new();
        for (body, path, nodes, reference, base_nodes, guide_end, thickness_cells) in retention {
            let inverse = body_poses[body].inverse();
            for (index, (node, (x, offset))) in nodes.into_iter().zip(reference).enumerate() {
                let p = inverse.transform_point(positions[node as usize]);
                let lateral = (p.x - x).abs();
                let radial = (radial(&path, p) - offset).abs();
                let error = lateral.max(radial);
                let region = if index < base_nodes {
                    if index % (thickness_cells + 1) == thickness_cells / 2 {
                        0
                    } else {
                        1
                    }
                } else if index < guide_end {
                    2
                } else {
                    3
                };
                region_departures[region] = region_departures[region].max(error);
                regions[node as usize] = region as u32;
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
        };
        eprintln!(
            "ground outcome effort={effort}, ground={ground}, teeth={teeth}: {outcome:?}; accepted={}s; minJ={:?}; region_departures(cord,carcass,guide,tread)={region_departures:?}; worst={worst}; short coupling gate, not sustained driving acceptance",
            island.completed_seconds(),
            island.minimum_j
        );
        let directory = std::env::var_os("GEARBOX_FEM_CAPTURE_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("gearbox-fem-traction"));
        std::fs::create_dir_all(&directory).unwrap();
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
            "drive_contact_control":"tooth boxes only; all smooth sprocket supports retained",
            "time":island.completed_seconds(), "minimum_j":island.minimum_j, "outcome":outcome,
            "initial_positions":initial_positions.iter().map(|p| p.to_array()).collect::<Vec<_>>(),
            "positions":positions.iter().map(|p| p.to_array()).collect::<Vec<_>>(),
            "triangles":triangles, "initial_poses":poses(&initial_poses), "poses":poses(&body_poses),
            "shapes":shape_trace, "regions":regions, "retention_errors":errors, "worst":worst,
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
#[ignore = "requires tread-complete GEARBOX_TRACK_ASSET through oslo make test-fem-gpu"]
fn authored_ceol_passive_ground_retention_diagnostic() {
    let outcomes = run_cases(&[(0.0, true, true)], false);
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
