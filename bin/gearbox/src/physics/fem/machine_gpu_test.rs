use super::*;
use crate::physics::{MollaBackend, PhysicsWorld};
use std::path::Path;

fn read_floats<const N: usize>(
    device: &RenderDevice,
    queue: &RenderQueue,
    source: &wgpu::Buffer,
) -> Vec<[f32; N]> {
    let gpu = device.wgpu_device();
    let staging = gpu.create_buffer(&wgpu::BufferDescriptor {
        label: Some("CEOL hitch acceptance"),
        size: source.size(),
        mapped_at_creation: false,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
    });
    let mut encoder = gpu.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(source, 0, &staging, 0, source.size());
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    staging
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
    gpu.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    staging
        .slice(..)
        .get_mapped_range()
        .chunks_exact(N * 4)
        .map(|bytes| {
            std::array::from_fn(|i| f32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap()))
        })
        .collect()
}

fn asset() -> (
    App,
    crate::controller::MachineInstanceSpec,
    machine::FemMachineLayout,
) {
    let path = std::env::var("GEARBOX_TRACK_ASSET").expect("GEARBOX_TRACK_ASSET");
    let source = usd_bevy::UsdSource::from_file(Path::new(&path)).unwrap();
    let stage = source.open_stage().unwrap();
    let mut spec = crate::controller::discover_machines_from_stage(&stage)
        .unwrap()
        .remove(0);
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
    let layout = machine::FemMachineLayout::inspect(app.world_mut(), &spec).unwrap();
    (app, spec, layout)
}

#[test]
#[ignore = "requires GPU and GEARBOX_TRACK_ASSET through oslo make test-fem-gpu"]
fn authored_ceol_hitch_closes_on_gpu_without_cpu_pose_updates() {
    let (app, _, layout) = asset();
    let (device, queue, _) = tests::gpu_island();
    let mut errors = Vec::new();
    for closed in [false, true] {
        let prepared = rigid_machine::FemRigidMachine::prepare(app.world(), &layout).unwrap();
        assert_eq!(prepared.model.body_count, 22);
        assert_eq!(prepared.model.joint_dof_total, 29);
        let loops = prepared.ball_joints().unwrap();
        assert_eq!(loops.len(), 2);
        let driven = prepared
            .model
            .joint_child
            .host()
            .unwrap()
            .iter()
            .position(|&body| body == loops[0].body_a as i32)
            .unwrap();
        let dof = prepared.model.joint_dof_offset.host().unwrap()[driven] as usize;
        let mut scene = tests::moving_scene();
        scene.rigid_model = prepared.model;
        scene.rigid_state = prepared.state;
        scene.control = prepared.control;
        scene.control.joint_force.host_mut().unwrap()[dof] = 100.0;
        let mut island = FemGpuIsland::with_ball_joints(
            &device,
            &queue,
            scene,
            FemRigidConfig {
                max_substep: 1.0 / 4096.0,
                elastic_iterations: 4,
                contact: molla_solvers::soft_rigid_contact::SoftRigidContactConfig {
                    iterations: 64,
                    ..default()
                },
                ..default()
            },
            if closed { &loops } else { &[] },
        )
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while island.clock.completed < 128 {
            assert!(
                std::time::Instant::now() < deadline,
                "CEOL hitch GPU pacing timed out"
            );
            island.advance(island.clock.submitted < 128).unwrap();
            std::thread::yield_now();
        }
        assert_eq!(island.clock.submitted, 128);
        assert!(island.system.rigid_state().body_q.host().is_err());
        let values = read_floats::<8>(&device, &queue, island.rigid_poses());
        let anchor = |body: u32, local: [f32; 3]| {
            let p = values[body as usize];
            Vec3::new(p[0], p[1], p[2])
                + Quat::from_xyzw(p[4], p[5], p[6], p[7]) * Vec3::from_array(local)
        };
        let gaps: Vec<_> = loops
            .iter()
            .map(|j| (anchor(j.body_a, j.anchor_a) - anchor(j.body_b, j.anchor_b)).length())
            .collect();
        let error = gaps.iter().copied().fold(0.0_f32, f32::max);
        eprintln!(
            "authored CEOL hitch closed={closed}: gaps={gaps:?}, accepted={}s",
            island.completed_seconds()
        );
        errors.push(error);
    }
    assert!(
        errors[0] > 1e-4,
        "unclosed hitch did not separate: {errors:?}"
    );
    assert!(
        errors[1] < 1e-4 && errors[1] < errors[0] * 0.1,
        "hitch closure failed: {errors:?}"
    );
}

#[test]
#[ignore = "requires GPU and GEARBOX_TRACK_ASSET through oslo make test-fem-gpu"]
fn authored_ceol_belts_relax_on_shared_gpu_without_contacts() {
    let (app, spec, layout) = asset();
    let mut rigid = rigid_machine::FemRigidMachine::prepare(app.world(), &layout).unwrap();
    let belts = track_mesh::FemTrackMeshes::prepare(app.world(), &spec, &layout, &rigid).unwrap();
    rigid.partition_belt_mass(&belts).unwrap();
    let loops = rigid.ball_joints().unwrap();
    let initial = belts.state.particle_q.host().unwrap().to_vec();
    let mass = rigid.model.body_mass.host().unwrap().iter().sum::<f64>()
        + belts
            .model
            .particle_mass
            .host()
            .unwrap()
            .iter()
            .sum::<f64>();
    assert!((mass - 750.0).abs() < 1e-5);
    let scene = FemRigidGpuScene {
        soft_model: belts.model,
        soft_state: belts.state,
        rigid_model: rigid.model,
        rigid_state: rigid.state,
        control: rigid.control,
        shapes: Vec::new(),
    };
    let (device, queue, _) = tests::gpu_island();
    let mut island = FemGpuIsland::with_ball_joints(
        &device,
        &queue,
        scene,
        FemRigidConfig {
            max_substep: 1.0 / 19200.0,
            elastic_iterations: 128,
            ..default()
        },
        &loops,
    )
    .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while island.clock.completed < 16 {
        assert!(
            std::time::Instant::now() < deadline,
            "authored belt GPU acceptance timed out"
        );
        island.advance(island.clock.submitted < 16).unwrap();
        std::thread::yield_now();
    }
    assert!(island.system.soft_state().particle_q.host().is_err());
    assert!(island.system.rigid_state().body_q.host().is_err());
    assert_eq!(island.clock.submitted, 16);
    let final_positions = read_floats::<4>(&device, &queue, island.positions());
    assert_eq!(final_positions.len(), initial.len());
    let displacement = final_positions
        .iter()
        .zip(&initial)
        .map(|(p, q)| {
            let p = molla_math::Vec3::new(p[0] as f64, p[1] as f64, p[2] as f64);
            assert!(p.is_finite());
            (p - *q).length()
        })
        .fold(0.0_f64, f64::max);
    assert!(displacement > 1e-7, "no GPU belt deformation measured");
    assert!(island.minimum_j.unwrap() > 0.5);
    eprintln!(
        "authored belts GPU relaxation: nodes={} accepted={}s minJ={} max displacement={}m, no contacts or traction",
        initial.len(),
        island.completed_seconds(),
        island.minimum_j.unwrap(),
        displacement
    );
}

#[test]
#[ignore = "requires GPU and GEARBOX_TRACK_ASSET through oslo make test-fem-gpu"]
fn authored_ceol_ground_contact_uses_bounded_gpu_storage() {
    let (app, spec, layout) = asset();
    let (device, queue, _) = tests::gpu_island();
    let mut momenta = Vec::new();
    for ground_enabled in [false, true] {
        let mut rigid = rigid_machine::FemRigidMachine::prepare(app.world(), &layout).unwrap();
        let mut belts =
            track_mesh::FemTrackMeshes::prepare(app.world(), &spec, &layout, &rigid).unwrap();
        rigid.partition_belt_mass(&belts).unwrap();
        let loops = rigid.ball_joints().unwrap();
        let masses = belts.model.particle_mass.host().unwrap().to_vec();
        let floor = belts
            .state
            .particle_q
            .host()
            .unwrap()
            .iter()
            .map(|p| p.y)
            .fold(f64::INFINITY, f64::min);
        belts
            .state
            .particle_qd
            .host_mut()
            .unwrap()
            .fill(-molla_math::Vec3::Y * 0.2);
        let shapes = (0..13)
            .map(|i| molla_solvers::fem_rigid_gpu::SoftRigidShapeGpu {
                position: [
                    if i == 0 && ground_enabled {
                        0.0
                    } else {
                        100.0 + i as f32 * 20.0
                    },
                    floor as f32 - 0.5,
                    0.0,
                    0.0,
                ],
                rotation: [0.0, 0.0, 0.0, 1.0],
                data: [5.0, 0.5, 5.0, 0.6],
                ids: [1, u32::MAX, 0, 0],
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
        assert_eq!(island.system.contact_row_count(), 53_760 * 4 + 2);
        let response_bytes = u64::from(island.system.contact_row_count()) * 3 * 29 * 4;
        assert!(response_bytes < 128 * 1024 * 1024);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while island.clock.completed < 8 {
            assert!(
                std::time::Instant::now() < deadline,
                "authored ground contact timed out"
            );
            island.advance(island.clock.submitted < 8).unwrap();
            std::thread::yield_now();
        }
        assert!(island.system.soft_state().particle_q.host().is_err());
        assert!(island.system.rigid_state().body_q.host().is_err());
        let velocities = read_floats::<4>(
            &device,
            &queue,
            island
                .system
                .soft_state()
                .particle_qd
                .device_buffer()
                .unwrap(),
        );
        let momentum = velocities
            .iter()
            .zip(&masses)
            .map(|(v, m)| {
                assert!(v.iter().all(|x| x.is_finite()));
                v[1] as f64 * m
            })
            .sum::<f64>();
        assert!(island.minimum_j.unwrap() > 0.5);
        eprintln!(
            "authored CEOL ground={ground_enabled}: momentum_y={momentum}, response_bytes={response_bytes}, minJ={}, accepted={}s; no rollers or propulsion",
            island.minimum_j.unwrap(),
            island.completed_seconds()
        );
        momenta.push(momentum);
    }
    assert!(
        momenta[1] > momenta[0] + 0.01,
        "ground produced no upward response: {momenta:?}"
    );
}

#[test]
#[ignore = "requires GPU and GEARBOX_TRACK_ASSET through oslo make test-fem-gpu"]
fn authored_ceol_wheel_contacts_have_clearance_and_run_on_gpu() {
    use molla_math::{Quat as DQuat, Vec3 as DVec3};
    use std::collections::BTreeSet;

    let (app, mut spec, layout) = asset();
    let rigid = rigid_machine::FemRigidMachine::prepare(app.world(), &layout).unwrap();
    let belts = track_mesh::FemTrackMeshes::prepare(app.world(), &spec, &layout, &rigid).unwrap();
    let contacts =
        track_contacts::FemTrackContacts::prepare(app.world(), &spec, &layout, &rigid).unwrap();
    assert_eq!(contacts.shapes.len(), 66);
    let saved = spec.tracks[0].fem_contacts.clone();
    let sprocket = spec.tracks[0].sprocket.clone();
    for fault in 0..4 {
        let data = spec.tracks[0].fem_contacts.as_mut().unwrap();
        match fault {
            0 => data.shapes[0].body = spec.body.clone().unwrap(),
            1 => data.shapes[0].rotation = [0.0; 4],
            2 => data.shapes.retain(|s| s.body != sprocket),
            _ => data.shapes[0].friction = f64::NAN,
        }
        assert!(
            track_contacts::FemTrackContacts::prepare(app.world(), &spec, &layout, &rigid).is_err()
        );
        spec.tracks[0].fem_contacts = saved.clone();
    }
    let points = belts.state.particle_q.host().unwrap();
    let triangles: Vec<_> = belts
        .tracks
        .iter()
        .flat_map(|t| t.surface.iter().copied())
        .collect();
    let vertices: BTreeSet<_> = triangles.iter().flatten().copied().collect();
    let edges: BTreeSet<_> = triangles
        .iter()
        .flat_map(|t| [[t[0], t[1]], [t[1], t[2]], [t[2], t[0]]])
        .map(|mut pair| {
            pair.sort();
            pair
        })
        .collect();
    let mut samples: Vec<_> = vertices.into_iter().map(|i| points[i as usize]).collect();
    samples.extend(
        edges
            .into_iter()
            .map(|[a, b]| (points[a as usize] + points[b as usize]) * 0.5),
    );
    samples.extend(
        triangles
            .iter()
            .map(|t| (points[t[0] as usize] + points[t[1] as usize] + points[t[2] as usize]) / 3.0),
    );
    assert_eq!(samples.len(), 53_760);
    let mut worst = (0.0_f64, String::new());
    for (shape, name) in contacts.shapes.iter().zip(&contacts.names) {
        let body = rigid.state.body_q.host().unwrap()[shape.ids[1] as usize].inverse();
        let rotation = DQuat::from_xyzw(
            shape.rotation[0] as f64,
            shape.rotation[1] as f64,
            shape.rotation[2] as f64,
            shape.rotation[3] as f64,
        )
        .normalize()
        .conjugate();
        let center = DVec3::new(
            shape.position[0] as f64,
            shape.position[1] as f64,
            shape.position[2] as f64,
        );
        let mut penetration = 0.0_f64;
        for point in &samples {
            let p = rotation * (body.transform_point(*point) - center);
            let distance = if shape.ids[0] == 1 {
                let d = p.abs()
                    - DVec3::new(
                        shape.data[0] as f64,
                        shape.data[1] as f64,
                        shape.data[2] as f64,
                    );
                d.max(DVec3::ZERO).length() + d.max_element().min(0.0)
            } else {
                let radial = p.x.hypot(p.z) - shape.data[0] as f64;
                let axial = p.y.abs() - shape.data[1] as f64;
                radial.max(0.0).hypot(axial.max(0.0)) + radial.max(axial).min(0.0)
            };
            penetration = penetration.max(-distance);
        }
        if penetration > worst.0 {
            worst = (penetration, name.clone());
        }
    }
    eprintln!(
        "authored wheel initial clearance: worst penetration={}m at {}; sampled features={}",
        worst.0,
        worst.1,
        samples.len()
    );
    assert!(
        worst.0 < 0.0005,
        "wheel geometry intersects installed belt: {worst:?}"
    );
    let (device, queue, _) = tests::gpu_island();
    let mut responses = Vec::new();
    let steps = 512;
    for enabled in [false, true] {
        let mut rigid = rigid_machine::FemRigidMachine::prepare(app.world(), &layout).unwrap();
        let belts =
            track_mesh::FemTrackMeshes::prepare(app.world(), &spec, &layout, &rigid).unwrap();
        rigid.partition_belt_mass(&belts).unwrap();
        let loops = rigid.ball_joints().unwrap();
        let floor = belts
            .state
            .particle_q
            .host()
            .unwrap()
            .iter()
            .map(|p| p.y)
            .fold(f64::INFINITY, f64::min);
        let mut shapes = if enabled {
            contacts.shapes.clone()
        } else {
            Vec::new()
        };
        shapes.push(molla_solvers::fem_rigid_gpu::SoftRigidShapeGpu {
            position: [0.0, floor as f32 - 0.5, 0.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            data: [5.0, 0.5, 5.0, 0.6],
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
        let started = std::time::Instant::now();
        let deadline = started + std::time::Duration::from_secs(600);
        let mut reported = 0;
        while island.clock.completed < steps {
            assert!(
                std::time::Instant::now() < deadline,
                "authored wheel contact timed out: enabled={enabled}, completed={}, submitted={}",
                island.clock.completed,
                island.clock.submitted
            );
            island.advance(island.clock.submitted < steps).unwrap();
            if island.clock.completed >= reported + 64 {
                reported = island.clock.completed;
                eprintln!(
                    "wheel settling enabled={enabled}: {reported}/{steps} steps in {:?}",
                    started.elapsed()
                );
            }
            std::thread::yield_now();
        }
        assert!(island.minimum_j.unwrap() > 0.5);
        assert!(island.system.soft_state().particle_q.host().is_err());
        assert!(island.system.rigid_state().body_q.host().is_err());
        responses.push(read_floats::<1>(
            &device,
            &queue,
            island
                .system
                .rigid_state()
                .joint_qd
                .device_buffer()
                .unwrap(),
        ));
        eprintln!(
            "authored wheel contacts enabled={enabled}: {} accepted steps on ground, minJ={}; no powered-drive acceptance",
            island.clock.completed,
            island.minimum_j.unwrap()
        );
    }
    let difference = responses[0]
        .iter()
        .zip(&responses[1])
        .map(|(a, b)| (a[0] - b[0]).abs())
        .fold(0.0_f32, f32::max);
    assert!(responses.iter().flatten().all(|v| v[0].is_finite()));
    eprintln!("authored wheel contact rigid velocity response: {difference}");
    assert!(
        difference > 1e-5,
        "no contact reaction reached the articulation"
    );
}
