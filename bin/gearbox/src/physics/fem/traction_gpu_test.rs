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

#[derive(Debug)]
struct Outcome {
    com_travel: f64,
    chassis_travel: f64,
    retention: f64,
    penetration: f64,
}

#[test]
#[ignore = "requires tread-complete GEARBOX_TRACK_ASSET through oslo make test-fem-gpu"]
fn authored_ceol_powered_ground_coupling_has_causal_controls() {
    let (app, spec, layout, contacts) = machine_gpu_test::checked_wheel_contacts();
    assert!(
        spec.tracks
            .iter()
            .all(|t| t.fem.as_ref().unwrap().version == 2)
    );
    let (device, queue, _) = tests::gpu_island();
    let mut outcomes = Vec::new();
    for (effort, ground, teeth) in [
        (0.0, true, true),
        (100.0, false, true),
        (100.0, true, false),
        (100.0, true, true),
        (-100.0, true, true),
    ] {
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
            retention.push((body, path, track.nodes.clone(), reference));
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
        let mut shapes = contacts
            .shapes
            .iter()
            .filter(|s| teeth || !drive_bodies.contains(&s.ids[1]))
            .map(|s| {
                let mut s = *s;
                s.data[3] = 0.0;
                s
            })
            .collect::<Vec<_>>();
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
        for dof in dofs {
            island.system.set_drive_effort(dof, effort).unwrap();
        }
        let started = std::time::Instant::now();
        let mut reported = 0;
        while island.clock.completed < 512 {
            assert!(
                started.elapsed().as_secs() < 1800,
                "ground coupling timeout: effort={effort}, ground={ground}, teeth={teeth}, completed={}",
                island.clock.completed
            );
            island.advance(island.clock.submitted < 512).unwrap();
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
        for (body, path, nodes, reference) in retention {
            let inverse = body_poses[body].inverse();
            for (node, (x, offset)) in nodes.into_iter().zip(reference) {
                let p = inverse.transform_point(positions[node as usize]);
                departure = departure
                    .max((p.x - x).abs())
                    .max((radial(&path, p) - offset).abs());
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
            "ground outcome effort={effort}, ground={ground}, teeth={teeth}: {outcome:?}; accepted={}s; short coupling gate, not sustained driving acceptance",
            island.completed_seconds()
        );
        outcomes.push(outcome);
    }
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
