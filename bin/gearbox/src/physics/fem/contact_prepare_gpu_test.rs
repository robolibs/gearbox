use super::*;
use molla_kernels::soft_rigid_contact::{
    SoftRigidContactBindings, SoftRigidContactKernel, SoftRigidSampleGpu,
};
use molla_kernels::soft_rigid_solve::{
    SoftRigidSolveBindings, SoftRigidSolveKernel, SoftRigidSolveSettings,
};
use molla_kernels::{JacobianKernel, JacobianRequest, MassMatrixKernel};
use wgpu::util::DeviceExt;

fn buffer<T: bytemuck::Pod>(device: &wgpu::Device, data: &[T]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("native CEOL contact fixture"),
        contents: bytemuck::cast_slice(data),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    })
}

#[test]
#[ignore = "requires GEARBOX_TRACK_ASSET and GPU through oslo make test-fem-gpu"]
fn authored_ceol_contact_preparation_completes_without_prediction() {
    let (app, spec, layout, contacts) = machine_gpu_test::checked_wheel_contacts();
    let (render_device, render_queue, _) = tests::gpu_island();
    let device = render_device.wgpu_device();
    let queue = &**render_queue.0;
    let mut rigid = rigid_machine::FemRigidMachine::prepare(app.world(), &layout).unwrap();
    let mut belts =
        track_mesh::FemTrackMeshes::prepare(app.world(), &spec, &layout, &rigid).unwrap();
    rigid.partition_belt_mass(&belts).unwrap();
    let loops = rigid.ball_joints().unwrap();
    let drive_bodies: Vec<_> = layout
        .tracks
        .iter()
        .map(|t| rigid.bodies[&t.sprocket].index() as u32)
        .collect();
    let mut shapes =
        contact_control_test::frictionless_drive_contacts(&contacts, &drive_bodies, true);
    let floor = belts
        .state
        .particle_q
        .host()
        .unwrap()
        .iter()
        .map(|p| p.y)
        .fold(f64::INFINITY, f64::min);
    shapes.push(molla_solvers::fem_rigid_gpu::SoftRigidShapeGpu {
        position: [100.0, floor as f32 - 0.5, 0.0, 0.0],
        rotation: [0.0, 0.0, 0.0, 1.0],
        data: [5.0, 0.5, 5.0, 0.85],
        ids: [1, u32::MAX, 0, 0],
    });
    let tets: Vec<_> = belts
        .model
        .tet_indices
        .host()
        .unwrap()
        .chunks_exact(4)
        .map(|t| [t[0], t[1], t[2], t[3]])
        .collect();
    let triangles = molla_sim::compute_tet_surface_triangles(&tets);
    let mut nodes: Vec<_> = triangles.iter().flatten().copied().collect();
    nodes.sort_unstable();
    nodes.dedup();
    let mut samples: Vec<_> = nodes
        .into_iter()
        .map(|n| SoftRigidSampleGpu {
            nodes: [n, 0, 0, 1],
            weights: [1.0, 0.0, 0.0, 0.0],
        })
        .collect();
    let mut edges = std::collections::BTreeSet::new();
    for &[a, b, c] in &triangles {
        for (a, b) in [(a, b), (b, c), (c, a)] {
            edges.insert((a.min(b), a.max(b)));
        }
    }
    samples.extend(edges.into_iter().map(|(a, b)| SoftRigidSampleGpu {
        nodes: [a, b, 0, 2],
        weights: [0.5, 0.5, 0.0, 0.0],
    }));
    samples.extend(triangles.iter().map(|&[a, b, c]| SoftRigidSampleGpu {
        nodes: [a, b, c, 3],
        weights: [1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0, 0.0],
    }));
    molla_sim::eval_fk(&rigid.model, &mut rigid.state).unwrap();
    molla_sim::eval_body_velocities(&rigid.model, &mut rigid.state).unwrap();
    let dofs = rigid.model.joint_dof_total;
    let bodies = rigid.model.body_count;
    let particles = belts.model.particle_count;
    let requests: Vec<_> = rigid
        .model
        .body_com
        .host()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(body, p)| JacobianRequest {
            articulation: 0,
            target_body: body as u32,
            body_q_offset: 0,
            output_offset: (body * 6 * dofs) as u32,
            point_local: [p.x as f32, p.y as f32, p.z as f32, 0.0],
        })
        .collect();
    rigid.model.upload_to_device(device, queue).unwrap();
    rigid.state.upload_to_device(device, queue).unwrap();
    belts.model.upload_to_device(device, queue).unwrap();
    belts.state.upload_to_device(device, queue).unwrap();
    let mass = MassMatrixKernel::new_on_device(device, queue, &rigid.model).unwrap();
    let mass_buffer = mass.create_output_buffer(device);
    mass.dispatch_on_device(device, queue, &rigid.model, &rigid.state, &mass_buffer)
        .unwrap();
    let jacobian = JacobianKernel::new_on_device(device, queue, &rigid.model).unwrap();
    let jacobians = jacobian.create_output_buffer(device, bodies * 6 * dofs);
    jacobian
        .dispatch_with_body_q_on_device(
            device,
            queue,
            &rigid.model,
            rigid.state.body_q.device_buffer().unwrap(),
            &requests,
            &jacobians,
        )
        .unwrap();
    let budget = (128 * 1024 * 1024_u64)
        .min(device.limits().max_storage_buffer_binding_size as u64)
        .min(device.limits().max_buffer_size);
    let rows = (budget / (12 * dofs as u64).max(80))
        .min(device.limits().max_compute_workgroups_per_dimension as u64 * 64);
    let capacity =
        (rows - loops.len() as u64).min(samples.len() as u64 * shapes.len().min(4) as u64) as u32;
    let gather = SoftRigidContactKernel::new_compact_sampled_with_ball_joints(
        device,
        queue,
        samples.len() as u32,
        shapes.len() as u32,
        bodies as u32,
        capacity,
        &loops,
    )
    .unwrap();
    let surface = buffer(device, &samples);
    let shapes = buffer(device, &shapes);
    let mut encoder = device.create_command_encoder(&default());
    gather
        .encode(
            device,
            &mut encoder,
            SoftRigidContactBindings {
                positions: belts.state.particle_q.device_buffer().unwrap(),
                radii: belts.model.particle_radius.device_buffer().unwrap(),
                surface: &surface,
                shapes: &shapes,
                body_poses: rigid.state.body_q.device_buffer().unwrap(),
                particle_count: particles as u32,
                body_count: bodies as u32,
            },
            FemRigidConfig::default().contact_margin as f32,
        )
        .unwrap();
    let submission = queue.submit([encoder.finish()]);
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
    let inputs = [
        ("positions", belts.state.particle_q.device_buffer().unwrap()),
        (
            "velocities",
            belts.state.particle_qd.device_buffer().unwrap(),
        ),
        (
            "inverse_mass",
            belts.model.particle_inv_mass.device_buffer().unwrap(),
        ),
        ("contacts", gather.rows()),
        ("body_poses", rigid.state.body_q.device_buffer().unwrap()),
        ("body_com", rigid.model.body_com.device_buffer().unwrap()),
        ("jacobians", &jacobians),
        ("mass_matrix", &mass_buffer),
        (
            "rigid_velocities",
            rigid.state.joint_qd.device_buffer().unwrap(),
        ),
        (
            "body_velocities",
            rigid.state.body_qd.device_buffer().unwrap(),
        ),
    ];
    if let Some(path) = std::env::var_os("GEARBOX_FEM_CAPTURE_DIR") {
        let path = std::path::PathBuf::from(path);
        std::fs::create_dir_all(&path).unwrap();
        for (name, input) in inputs {
            let data = machine_gpu_test::read_floats::<1>(&render_device, &render_queue, input);
            std::fs::write(
                path.join(format!("{name}.bin")),
                bytemuck::cast_slice(&data),
            )
            .unwrap();
        }
        std::fs::write(
            path.join("dimensions.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "dofs": dofs, "bodies": bodies, "particles": particles,
                "rows": gather.row_count(), "dt": 1.0 / 19200.0,
                "samples": samples.len(),
            }))
            .unwrap(),
        )
        .unwrap();
        eprintln!("native contact inputs saved to {}", path.display());
    }
    prepare(&render_device, &render_queue, &inputs);
}

#[test]
#[ignore = "requires GEARBOX_FEM_CAPTURE_DIR and GPU through oslo make test-fem-gpu"]
fn captured_ceol_contact_preparation_completes() {
    let path = std::path::PathBuf::from(
        std::env::var_os("GEARBOX_FEM_CAPTURE_DIR").expect("GEARBOX_FEM_CAPTURE_DIR"),
    );
    let (device, queue, _) = tests::gpu_island();
    let names = [
        "positions",
        "velocities",
        "inverse_mass",
        "contacts",
        "body_poses",
        "body_com",
        "jacobians",
        "mass_matrix",
        "rigid_velocities",
        "body_velocities",
    ];
    let buffers: Vec<_> = names
        .iter()
        .map(|name| {
            let bytes = std::fs::read(path.join(format!("{name}.bin"))).unwrap();
            buffer(device.wgpu_device(), &bytes)
        })
        .collect();
    let inputs: Vec<_> = names.into_iter().zip(&buffers).collect();
    prepare(&device, &queue, &inputs);
}

fn prepare(
    render_device: &RenderDevice,
    render_queue: &RenderQueue,
    inputs: &[(&str, &wgpu::Buffer)],
) {
    let device = render_device.wgpu_device();
    let queue = &**render_queue.0;
    let dofs = inputs[8].1.size() / 4;
    let particles = inputs[0].1.size() / 16;
    let bodies = inputs[4].1.size() / 32;
    let rows = inputs[3].1.size() / 80;
    let kernel = SoftRigidSolveKernel::new(device, dofs as u32, rows as u32).unwrap();
    let mut encoder = device.create_command_encoder(&default());
    kernel
        .encode_preparation(
            device,
            &mut encoder,
            SoftRigidSolveBindings {
                positions: inputs[0].1,
                velocities: inputs[1].1,
                inverse_mass: inputs[2].1,
                contacts: inputs[3].1,
                body_poses: inputs[4].1,
                body_com: inputs[5].1,
                jacobians: inputs[6].1,
                mass_matrix: inputs[7].1,
                rigid_velocities: inputs[8].1,
                body_velocities: inputs[9].1,
                particle_count: particles as u32,
                body_count: bodies as u32,
            },
            SoftRigidSolveSettings {
                iterations: 1,
                dt: 1.0 / 19200.0,
                penetration_recovery: FemRigidConfig::default().contact.penetration_recovery as f32,
                max_recovery_speed: FemRigidConfig::default().contact.max_recovery_speed as f32,
            },
        )
        .unwrap();
    eprintln!(
        "native contact preparation submit: dofs={dofs}, rows={}",
        rows
    );
    let start = std::time::Instant::now();
    let submission = queue.submit([encoder.finish()]);
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("native contact preparation exceeded 30 seconds");
    eprintln!(
        "native contact preparation complete in {:?}",
        start.elapsed()
    );
    let status = machine_gpu_test::read_floats::<1>(&render_device, &render_queue, kernel.status());
    assert_eq!(status[0][0].to_bits(), 0);
    let contacts = machine_gpu_test::read_floats::<20>(render_device, render_queue, inputs[3].1);
    let responses =
        machine_gpu_test::read_floats::<8>(render_device, render_queue, kernel.row_state());
    for (contact, response) in contacts.iter().zip(&responses) {
        assert!(response.iter().all(|v| v.is_finite()));
        assert_eq!(&response[3..], &[0.0; 5]);
        match contact[11].to_bits() {
            0 => assert_eq!(*response, [0.0; 8]),
            1 => assert!(response[0] > 0.0 && response[1] > 0.0),
            3 => assert!(response[..3].iter().all(|v| *v >= 0.0)),
            kind => panic!("unexpected contact type {kind}"),
        }
    }
}
