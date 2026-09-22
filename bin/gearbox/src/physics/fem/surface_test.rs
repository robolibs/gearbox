use super::frames::{FemSurfaceBinding, FemSurfaceFrames};
use super::*;
use crate::physics::fem::FemRigidConfig;
use crate::physics::fem::{FemGpuIsland, tests::moving_scene};
use bevy::core_pipeline::prepass::{DepthPrepass, MotionVectorPrepass, NormalPrepass};
use bevy::render::error_handler::RenderErrorHandler;
use bevy::render::pipelined_rendering::PipelinedRenderingPlugin;
use bevy::render::render_resource::{CachedPipelineState, PipelineCache};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use mara::ui::modules::bevy::{
    BevyViewportRenderTarget, BevyViewportRenderer, BevyViewportSet, BevyViewportTexture,
    BevyViewportWgpuResources, CapturedBevyFrame,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Resource, Clone, Default)]
struct RenderReadiness(Arc<AtomicUsize>);

fn capture(renderer: &mut BevyViewportRenderer) -> CapturedBevyFrame {
    let mut latest = None;
    let readiness = renderer.world_mut().resource::<RenderReadiness>().clone();
    let mut last = readiness.0.load(Ordering::Acquire);
    let mut ready_frames = 0;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while ready_frames < 40 || latest.is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "native render pipelines/capture did not become ready"
        );
        if let Some(frame) = renderer.render_next() {
            latest = Some(frame);
        }
        renderer
            .world_mut()
            .resource::<RenderDevice>()
            .wgpu_device()
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let count = readiness.0.load(Ordering::Acquire);
        if count < last {
            ready_frames = count;
        } else {
            ready_frames += count - last;
        }
        last = count;
        std::thread::yield_now();
    }
    latest.expect("native renderer produced no captured image")
}

fn centroid(frame: &CapturedBevyFrame) -> f64 {
    let mut sum = 0usize;
    let mut count = 0usize;
    for (i, pixel) in frame.rgba.chunks_exact(4).enumerate() {
        if pixel[0] > 120 && pixel[1] < 70 && pixel[2] < 70 {
            count += 1;
            sum += i % frame.width as usize;
        }
    }
    assert!(
        count > 500,
        "FEM surface missing from image: {count} red pixels"
    );
    sum as f64 / count as f64
}

fn save(frame: &CapturedBevyFrame, name: &str) {
    let directory = std::env::var_os("GEARBOX_FEM_CAPTURE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("gearbox-fem-surface"));
    std::fs::create_dir_all(&directory).unwrap();
    let file = std::fs::File::create(directory.join(name)).unwrap();
    let mut encoder = png::Encoder::new(file, frame.width, frame.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&frame.rgba)
        .unwrap();
}

#[test]
#[ignore = "requires a real GPU through oslo make test-fem-gpu"]
fn native_surface_follows_gpu_nodes_with_static_cpu_mesh() {
    render_surface(false, false, false);
}

#[test]
#[ignore = "requires a real GPU through oslo make test-fem-gpu"]
fn native_surface_snapshots_with_pipelined_rendering() {
    render_surface(true, false, false);
}

#[test]
#[ignore = "requires a real GPU through oslo make test-fem-gpu"]
fn native_surface_rejects_gpu_invalidity_while_paused() {
    render_surface(true, true, false);
}

#[test]
#[ignore = "requires a real GPU through oslo make test-fem-gpu"]
fn native_rigid_surface_follows_gpu_poses_with_static_cpu_transform() {
    render_surface(true, false, true);
}

#[test]
#[ignore = "requires a real GPU through oslo make test-fem-gpu"]
fn native_rigid_surface_rejects_gpu_invalidity_while_paused() {
    render_surface(true, true, true);
}

fn black(frame: &CapturedBevyFrame) -> bool {
    frame
        .rgba
        .chunks_exact(4)
        .all(|p| p[0] == 0 && p[1] == 0 && p[2] == 0)
}

fn inject_validity(
    device: &RenderDevice,
    queue: &RenderQueue,
    island: &FemGpuIsland,
    words: [u32; 3],
) {
    let shader = device
        .wgpu_device()
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("FEM renderer validity fault injection"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "@group(0) @binding(0) var<storage, read_write> status: u32;
             @group(0) @binding(1) var<storage, read_write> volume: array<u32>;
             @compute @workgroup_size(1) fn main() {{
                 status = {}u; volume[0] = {}u; volume[1] = {}u;
             }}",
                    words[0], words[1], words[2]
                )
                .into(),
            ),
        });
    let pipeline = device
        .wgpu_device()
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
    let group = device
        .wgpu_device()
        .create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: island.status().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: island
                        .system
                        .volume_diagnostics_buffer()
                        .as_entire_binding(),
                },
            ],
        });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    queue.submit([encoder.finish()]);
}

fn render_surface(pipelined: bool, inverted: bool, rigid: bool) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let mut descriptor = wgpu::DeviceDescriptor::default();
    <crate::host::GearboxApp as mara::window::WindowApp>::configure_gpu_limits(
        &adapter.limits(),
        &mut descriptor.required_limits,
    );
    let (device, queue) = pollster::block_on(adapter.request_device(&descriptor)).unwrap();
    let mut renderer = BevyViewportRenderer::new(
        BevyViewportTexture::new(256, 192),
        Some(BevyViewportWgpuResources::new(device, queue, adapter)),
        Some(Arc::new(move |plugins| {
            if pipelined {
                plugins
            } else {
                plugins.disable::<PipelinedRenderingPlugin>()
            }
        })),
        Some(Arc::new(move |app| {
            static LOGGING: std::sync::Once = std::sync::Once::new();
            LOGGING.call_once(|| {
                app.add_plugins(bevy::log::LogPlugin::default());
            });
            app.add_plugins(FemSurfacePlugin);
            app.insert_resource(gearbox_api::PhysicsActive(false));
            if !inverted {
                app.add_systems(Update, crate::physics::fem::advance_islands);
            }
            app.add_systems(
                Startup,
                (|target: Res<BevyViewportRenderTarget>,
                  mut cameras: Query<&mut bevy::camera::RenderTarget>| {
                    for mut camera in &mut cameras {
                        *camera = bevy::camera::RenderTarget::Image(target.0.clone().into());
                    }
                })
                .after(BevyViewportSet::SetupTarget),
            );
            app.insert_resource(ClearColor(Color::BLACK));
            app.insert_resource(RenderErrorHandler(|error, _, _| {
                panic!("native FEM rendering error: {error:?}");
            }));
            let readiness = RenderReadiness::default();
            app.insert_resource(readiness.clone());
            app.sub_app_mut(bevy::render::RenderApp)
                .insert_resource(readiness);
            app.sub_app_mut(bevy::render::RenderApp).add_systems(
                bevy::render::Render,
                (|cache: Res<PipelineCache>, readiness: Res<RenderReadiness>| {
                    for pipeline in cache.pipelines() {
                        if let CachedPipelineState::Err(error) = &pipeline.state {
                            match error {
                                bevy::shader::ShaderCacheError::ShaderNotLoaded(_)
                                | bevy::shader::ShaderCacheError::ShaderImportNotYetAvailable => {}
                                _ => panic!("native FEM pipeline error: {error:?}"),
                            }
                        }
                    }
                    if cache.pipelines().next().is_some()
                        && cache
                            .pipelines()
                            .all(|p| matches!(&p.state, CachedPipelineState::Ok(_)))
                        && cache.waiting_pipelines().next().is_none()
                    {
                        readiness.0.fetch_add(1, Ordering::Release);
                    } else {
                        readiness.0.store(0, Ordering::Release);
                    }
                })
                .after(bevy::render::RenderSystems::Render),
            );
        })),
    );
    let world = renderer.world_mut();
    let device = world.resource::<RenderDevice>().clone();
    let queue = world.resource::<RenderQueue>().clone();
    let h = 1.0 / 4096.0;
    let mut scene = moving_scene();
    if rigid {
        use molla_core::{BodyId, WorldId};
        use molla_math::{Transform as Pose, Vec3 as Vector};
        use molla_sim::{BodyParams, ModelBuilder};
        let mut builder = ModelBuilder::new();
        builder.set_world_gravity(WorldId(0), Vector::ZERO);
        builder.begin_articulation();
        let base = builder.add_body(BodyParams::new());
        builder
            .add_joint_fixed(BodyId::NONE, base, Pose::IDENTITY, Pose::IDENTITY)
            .unwrap();
        let body = builder.add_body(BodyParams::new());
        builder
            .add_joint_prismatic(base, body, Pose::IDENTITY, Pose::IDENTITY, Vector::X)
            .unwrap();
        scene.rigid_model = builder.build().unwrap();
        scene.rigid_state = scene.rigid_model.state().unwrap();
        scene.rigid_state.joint_qd.host_mut().unwrap()[0] = 0.3;
        scene.control = scene.rigid_model.control();
    }
    if inverted {
        scene.soft_state.particle_q.host_mut().unwrap()[3].z = -0.1;
    }
    let island = FemGpuIsland::new(
        &device,
        &queue,
        scene,
        FemRigidConfig {
            max_substep: h,
            elastic_iterations: 4,
            ..Default::default()
        },
    )
    .unwrap();
    let reference = [[0.0, 0.0, 0.0], [0.1, 0.0, 0.0], [0.0, 0.1, 0.0]];
    let mesh = world.resource_mut::<Assets<Mesh>>().add(
        surface_mesh(
            &reference,
            &[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            &[[0, 1, 2]],
        )
        .unwrap(),
    );
    let frames = FemSurfaceFrames::new(&device, &queue, &island);
    let entity = world.spawn_empty().id();
    let base = StandardMaterial {
        base_color: Color::srgb(1.0, 0.0, 0.0),
        unlit: true,
        ..Default::default()
    };
    let surface_entity = if rigid {
        let binding = super::rigid::FemRigidBinding::new(
            entity,
            &frames,
            &mut world.resource_mut::<Assets<super::rigid::RigidMaterial>>(),
            base,
            1,
            Vec3::X * 0.02,
            Mat4::from_translation(Vec3::X * 0.01),
        )
        .unwrap();
        world
            .spawn((Mesh3d(mesh.clone()), binding.material(), binding))
            .id()
    } else {
        let binding = FemSurfaceBinding::new(
            entity,
            &frames,
            &mut world.resource_mut::<Assets<FemMaterial>>(),
            base,
        );
        world
            .spawn((Mesh3d(mesh.clone()), binding.material(), binding))
            .id()
    };
    world.entity_mut(entity).insert((island, frames));
    world.spawn((
        Camera3d::default(),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                viewport_height: 0.25,
            },
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_xyz(0.05, 0.05, 0.4).looking_at(Vec3::new(0.05, 0.05, 0.0), Vec3::Y),
        DepthPrepass,
        NormalPrepass,
        MotionVectorPrepass,
    ));
    let before = capture(&mut renderer);
    if inverted {
        assert!(black(&before), "initial inverted mesh was rendered");
        let mut faults = vec![[0, 1.0_f32.to_bits(), 0]];
        faults.extend([1, 2, 4, 8, 16, u32::MAX].map(|s| [s, 1.0_f32.to_bits(), u32::MAX]));
        faults.extend(
            [f32::NAN, f32::INFINITY, -1.0, 0.0, 0.49, 0.5].map(|j| [0, j.to_bits(), u32::MAX]),
        );
        for words in faults {
            let island = renderer.world_mut().get::<FemGpuIsland>(entity).unwrap();
            inject_validity(&device, &queue, island, words);
            assert!(
                black(&capture(&mut renderer)),
                "invalid GPU diagnostics rendered: {words:?}"
            );
        }
        let island = renderer.world_mut().get::<FemGpuIsland>(entity).unwrap();
        assert!(island.failure().is_none());
        assert_eq!(island.completed_seconds(), 0.0);
        assert!(!island.pending());
        inject_validity(&device, &queue, island, [0, 1.0_f32.to_bits(), u32::MAX]);
        assert!(centroid(&capture(&mut renderer)).is_finite());
        return;
    }
    save(
        &before,
        &format!(
            "{}{}",
            if rigid { "rigid-" } else { "" },
            if pipelined {
                "pipelined-before.png"
            } else {
                "before.png"
            }
        ),
    );
    {
        let mut island = renderer
            .world_mut()
            .get_mut::<FemGpuIsland>(entity)
            .unwrap();
        island.system.step(512.0 * h).unwrap();
        assert!(island.system.soft_state().particle_q.host().is_err());
        assert!(island.system.rigid_state().body_q.host().is_err());
    }
    let after = capture(&mut renderer);
    save(
        &after,
        &format!(
            "{}{}",
            if rigid { "rigid-" } else { "" },
            if pipelined {
                "pipelined-after.png"
            } else {
                "after.png"
            }
        ),
    );
    let movement = centroid(&after) - centroid(&before);
    assert!(
        (24.0..34.0).contains(&movement),
        "unexpected GPU node screen movement: {movement}"
    );
    let paused = capture(&mut renderer);
    assert_eq!(after.rgba, paused.rgba, "paused GPU geometry changed");
    if pipelined {
        renderer
            .world_mut()
            .resource_mut::<gearbox_api::PhysicsActive>()
            .0 = true;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        loop {
            renderer.render_next();
            let island = renderer.world_mut().get::<FemGpuIsland>(entity).unwrap();
            assert!(island.failure().is_none(), "{:?}", island.failure());
            if island.completed_seconds() >= 128.0 * h {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "pipelined FEM stalled: submitted={}, completed={}, pending={}, accepted={}s",
                island.clock.submitted,
                island.clock.completed,
                island.pending(),
                island.completed_seconds()
            );
            std::thread::yield_now();
        }
        renderer
            .world_mut()
            .resource_mut::<gearbox_api::PhysicsActive>()
            .0 = false;
        let moving = capture(&mut renderer);
        let delta = centroid(&moving) - centroid(&after);
        assert!((6.0..10.0).contains(&delta), "pipelined movement: {delta}");
        let settled = capture(&mut renderer);
        assert_eq!(moving.rgba, settled.rgba, "pipelined pause did not settle");
        save(
            &moving,
            if rigid {
                "rigid-pipelined-streaming.png"
            } else {
                "pipelined-streaming.png"
            },
        );
    }
    assert_eq!(
        renderer
            .world_mut()
            .get::<Transform>(surface_entity)
            .unwrap(),
        &Transform::IDENTITY
    );
    let assets = renderer.world_mut().resource::<Assets<Mesh>>();
    assert!(
        matches!(assets.get(&mesh).unwrap().attribute(Mesh::ATTRIBUTE_POSITION),
        Some(VertexAttributeValues::Float32x3(v)) if v == &reference)
    );
    renderer.world_mut().despawn(entity);
    let removed = capture(&mut renderer);
    assert!(black(&removed), "orphan FEM surface remained visible");
    eprintln!("native FEM surface moved {movement:.3} pixels with unchanged CPU mesh");
}
