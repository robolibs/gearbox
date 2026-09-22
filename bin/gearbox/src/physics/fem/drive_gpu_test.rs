use super::*;
use molla_math::{Quat as DQuat, Transform as Pose, Vec3 as DVec3};
use molla_sim::soft_belt::path::BeltPath;

#[test]
#[ignore = "requires pitch-matched GEARBOX_TRACK_ASSET through oslo make test-fem-gpu"]
fn authored_ceol_teeth_drive_rubber_without_friction() {
    let (app, spec, layout, contacts) = machine_gpu_test::checked_wheel_contacts();
    assert!(
        spec.tracks
            .iter()
            .all(|t| t.fem.as_ref().unwrap().sprocket_teeth == 14)
    );
    let (device, queue, _) = tests::gpu_island();
    let mut outcomes = Vec::new();
    for (effort, teeth) in [(0.0, true), (100.0, false), (100.0, true), (-100.0, true)] {
        let mut rigid = rigid_machine::FemRigidMachine::prepare(app.world(), &layout).unwrap();
        let mut belts =
            track_mesh::FemTrackMeshes::prepare(app.world(), &spec, &layout, &rigid).unwrap();
        rigid.partition_belt_mass(&belts).unwrap();
        rigid.model.gravity.host_mut().unwrap().fill(DVec3::ZERO);
        belts.model.gravity.host_mut().unwrap().fill(DVec3::ZERO);
        let loops = rigid.ball_joints().unwrap();
        let drive_bodies: Vec<_> = layout
            .tracks
            .iter()
            .map(|t| rigid.bodies[&t.sprocket].index() as u32)
            .collect();
        let drive_dofs: Vec<_> = layout
            .tracks
            .iter()
            .map(|t| {
                let joint = rigid.joints[&t.drive_joint].index();
                rigid.model.joint_dof_offset.host().unwrap()[joint] as usize
            })
            .collect();
        let masses = belts.model.particle_mass.host().unwrap().to_vec();
        let mut measurements = Vec::new();
        for (track, authored) in belts.tracks.iter().zip(&spec.tracks) {
            let points: Vec<_> = authored
                .path
                .iter()
                .map(|p| DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64))
                .collect();
            let path = BeltPath::from_closed_polyline(
                &points,
                authored.fem.as_ref().unwrap().thickness * 0.5,
            )
            .unwrap();
            let carrier = rigid.bodies[&track.carrier].index();
            let inverse = rigid.state.body_q.host().unwrap()[carrier].inverse();
            let phases: Vec<_> = track
                .nodes
                .iter()
                .map(|&node| {
                    let mut p = inverse
                        .transform_point(belts.state.particle_q.host().unwrap()[node as usize]);
                    p.x = 0.0;
                    path.project(p).unwrap().0
                })
                .collect();
            measurements.push((carrier, path, track.nodes.clone(), phases));
        }
        let shapes = contacts
            .shapes
            .iter()
            .filter(|s| teeth || !drive_bodies.contains(&s.ids[1]))
            .map(|s| {
                let mut s = *s;
                s.data[3] = 0.0;
                s
            })
            .collect();
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
        for &dof in &drive_dofs {
            island.system.set_drive_effort(dof, effort).unwrap();
        }
        let started = std::time::Instant::now();
        let mut reported = 0;
        while island.clock.completed < 512 {
            assert!(
                started.elapsed().as_secs() < 600,
                "drive timeout: effort={effort}, teeth={teeth}, completed={}",
                island.clock.completed
            );
            island.advance(island.clock.submitted < 512).unwrap();
            if island.clock.completed >= reported + 64 {
                reported = island.clock.completed;
                eprintln!(
                    "drive effort={effort}, teeth={teeth}: {reported}/512 steps in {:?}",
                    started.elapsed()
                );
            }
            std::thread::yield_now();
        }
        assert!(island.system.soft_state().particle_q.host().is_err());
        assert!(island.system.rigid_state().body_q.host().is_err());
        let positions = machine_gpu_test::read_floats::<4>(&device, &queue, island.positions());
        let poses = machine_gpu_test::read_floats::<8>(&device, &queue, island.rigid_poses());
        let velocity = machine_gpu_test::read_floats::<1>(
            &device,
            &queue,
            island
                .system
                .rigid_state()
                .joint_qd
                .device_buffer()
                .unwrap(),
        );
        assert!(
            positions
                .iter()
                .flatten()
                .chain(poses.iter().flatten())
                .chain(velocity.iter().flatten())
                .all(|v| v.is_finite())
        );
        let mut travel = Vec::new();
        for (carrier, path, nodes, phases) in measurements {
            let q = poses[carrier];
            let inverse = Pose {
                position: DVec3::new(q[0] as f64, q[1] as f64, q[2] as f64),
                rotation: DQuat::from_xyzw(q[4] as f64, q[5] as f64, q[6] as f64, q[7] as f64)
                    .normalize(),
            }
            .inverse();
            let mut sum = 0.0;
            let mut mass = 0.0;
            for (node, initial) in nodes.into_iter().zip(phases) {
                let q = positions[node as usize];
                let mut p =
                    inverse.transform_point(DVec3::new(q[0] as f64, q[1] as f64, q[2] as f64));
                p.x = 0.0;
                let current = path.project(p).unwrap().0;
                let delta = (current - initial + path.length() * 0.5).rem_euclid(path.length())
                    - path.length() * 0.5;
                sum += delta * masses[node as usize];
                mass += masses[node as usize];
            }
            travel.push(sum / mass);
        }
        let speeds: Vec<_> = drive_dofs
            .iter()
            .map(|&dof| velocity[dof][0] as f64)
            .collect();
        eprintln!(
            "drive outcome effort={effort}, teeth={teeth}, friction=0, gravity=0: travel={travel:?}, sprocket_speed={speeds:?}, minJ={:?}",
            island.minimum_j
        );
        outcomes.push((travel, speeds));
    }
    for side in 0..2 {
        let passive = outcomes[0].0[side];
        let disconnected = outcomes[1].0[side];
        let forward = outcomes[2].0[side];
        let reverse = outcomes[3].0[side];
        assert!(
            forward > passive + 1e-4 && forward > disconnected + 1e-4,
            "no positive tooth drive on side {side}: {outcomes:?}"
        );
        assert!(
            reverse < passive - 1e-4,
            "no reverse tooth drive on side {side}: {outcomes:?}"
        );
        assert!(
            outcomes[2].1[side] < outcomes[1].1[side] - 1e-3,
            "belt did not load drive {side}: {outcomes:?}"
        );
    }
}
