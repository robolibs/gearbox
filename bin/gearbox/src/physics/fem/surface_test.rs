use super::*;
use crate::physics::fem::FemRigidConfig;
use crate::physics::fem::{FemGpuIsland, tests::moving_scene};
use bevy::camera::visibility::NoFrustumCulling;
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

fn capture(renderer: &mut BevyViewportRenderer) -> CapturedBevyFrame {
    let mut latest = None;
    for _ in 0..40 {
        if let Some(frame) = renderer.render_next() {
            latest = Some(frame);
        }
        renderer
            .world_mut()
            .resource::<RenderDevice>()
            .wgpu_device()
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
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
        Some(Arc::new(|plugins| {
            plugins.disable::<PipelinedRenderingPlugin>()
        })),
        Some(Arc::new(|app| {
            app.add_plugins(bevy::log::LogPlugin::default());
            app.add_plugins(FemSurfacePlugin);
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
            app.sub_app_mut(bevy::render::RenderApp).add_systems(
                bevy::render::Render,
                (|cache: Res<PipelineCache>| {
                    for pipeline in cache.pipelines() {
                        if let CachedPipelineState::Err(error) = &pipeline.state {
                            match error {
                                bevy::shader::ShaderCacheError::ShaderNotLoaded(_)
                                | bevy::shader::ShaderCacheError::ShaderImportNotYetAvailable => {}
                                _ => panic!("native FEM pipeline error: {error:?}"),
                            }
                        }
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
    let mut island = FemGpuIsland::new(
        &device,
        &queue,
        moving_scene(),
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
    let material = world
        .resource_mut::<Assets<FemMaterial>>()
        .add(FemMaterial {
            base: StandardMaterial {
                base_color: Color::srgb(1.0, 0.0, 0.0),
                unlit: true,
                ..Default::default()
            },
            extension: FemExtension {
                positions: island.positions().clone().into(),
                previous_positions: island.positions().clone().into(),
            },
        });
    world.spawn((
        Mesh3d(mesh.clone()),
        MeshMaterial3d(material.clone()),
        NoFrustumCulling,
    ));
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
    save(&before, "before.png");
    island.system.step(512.0 * h).unwrap();
    assert!(island.system.soft_state().particle_q.host().is_err());
    renderer
        .world_mut()
        .resource_mut::<Assets<FemMaterial>>()
        .get_mut(&material)
        .unwrap()
        .extension
        .positions = island.positions().clone().into();
    let after = capture(&mut renderer);
    save(&after, "after.png");
    let movement = centroid(&after) - centroid(&before);
    assert!(
        (24.0..34.0).contains(&movement),
        "unexpected GPU node screen movement: {movement}"
    );
    let paused = capture(&mut renderer);
    assert_eq!(after.rgba, paused.rgba, "paused GPU geometry changed");
    let assets = renderer.world_mut().resource::<Assets<Mesh>>();
    assert!(
        matches!(assets.get(&mesh).unwrap().attribute(Mesh::ATTRIBUTE_POSITION),
        Some(VertexAttributeValues::Float32x3(v)) if v == &reference)
    );
    eprintln!("native FEM surface moved {movement:.3} pixels with unchanged CPU mesh");
}
