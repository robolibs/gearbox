use super::*;
use crate::physics::{MollaBackend, PhysicsWorld};
use std::path::Path;

fn poses(device: &RenderDevice, queue: &RenderQueue, source: &wgpu::Buffer) -> Vec<[f32; 8]> {
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
        .chunks_exact(32)
        .map(|bytes| {
            std::array::from_fn(|i| f32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap()))
        })
        .collect()
}

#[test]
#[ignore = "requires GPU and GEARBOX_TRACK_ASSET through oslo make test-fem-gpu"]
fn authored_ceol_hitch_closes_on_gpu_without_cpu_pose_updates() {
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
        let values = poses(&device, &queue, island.rigid_poses());
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
