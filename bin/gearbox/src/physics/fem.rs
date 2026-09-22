use std::sync::Arc;

use bevy::prelude::*;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use gearbox_api::PhysicsActive;
use molla_solvers::fem_rigid::FemRigidConfig;
use molla_solvers::fem_rigid_gpu::{FemRigidGpuScene, FemRigidGpuSystem, SoftRigidBallJoint};

mod diagnostics;
#[cfg(test)]
mod machine_gpu_test;
#[cfg(test)]
mod drive_gpu_test;
#[cfg(test)]
mod traction_gpu_test;
pub(crate) mod machine;
pub(crate) mod rigid_machine;
pub(crate) mod track_mesh;
pub(crate) mod track_contacts;
pub(super) mod render_health;
pub(crate) mod surface;

#[derive(Default)]
struct IslandClock {
    submitted: u64,
    completed: u64,
}

impl IslandClock {
    fn pending(&self) -> bool {
        self.submitted != self.completed
    }

    fn submit(&mut self) {
        assert!(!self.pending());
        self.submitted += 1;
    }

    fn complete(&mut self) {
        self.completed = self.submitted;
    }
}

/// One GPU-owned articulation and its coupled deformable meshes.
#[derive(Component)]
pub(crate) struct FemGpuIsland {
    system: FemRigidGpuSystem,
    clock: IslandClock,
    substep: f64,
    failure: Option<String>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    diagnostics: diagnostics::Diagnostics,
    minimum_j: Option<f32>,
}

impl FemGpuIsland {
    pub(crate) fn new(
        device: &RenderDevice,
        queue: &RenderQueue,
        scene: FemRigidGpuScene,
        config: FemRigidConfig,
    ) -> molla_core::Result<Self> {
        Self::with_ball_joints(device, queue, scene, config, &[])
    }

    pub(crate) fn with_ball_joints(
        device: &RenderDevice,
        queue: &RenderQueue,
        scene: FemRigidGpuScene,
        config: FemRigidConfig,
        joints: &[SoftRigidBallJoint],
    ) -> molla_core::Result<Self> {
        let substep = config.max_substep;
        let system = FemRigidGpuSystem::new_surface_sampled_with_ball_joints_on_device(
            Arc::new(device.wgpu_device().clone()),
            Arc::new((**queue.0).clone()),
            scene,
            config,
            joints,
        )?;
        let mut diagnostics = diagnostics::Diagnostics::new(device.wgpu_device());
        diagnostics.submit(device.wgpu_device(), queue, &system);
        Ok(Self {
            system,
            clock: IslandClock::default(),
            substep,
            failure: None,
            device: device.wgpu_device().clone(),
            queue: (**queue.0).clone(),
            diagnostics,
            minimum_j: None,
        })
    }

    pub(crate) fn completed_seconds(&self) -> f64 {
        self.clock.completed as f64 * self.substep
    }

    pub(crate) fn pending(&self) -> bool {
        self.clock.pending()
    }

    pub(crate) fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    pub(crate) fn positions(&self) -> &wgpu::Buffer {
        self.system.soft_state().particle_q.device_buffer().unwrap()
    }

    pub(crate) fn rigid_poses(&self) -> &wgpu::Buffer {
        self.system.rigid_state().body_q.device_buffer().unwrap()
    }

    pub(crate) fn status(&self) -> &wgpu::Buffer {
        self.system.status_buffer()
    }

    pub(crate) fn set_drive_velocity(&mut self, dof: usize, target: f64) -> molla_core::Result<()> {
        self.system.set_drive_velocity(dof, target)
    }

    fn advance(&mut self, active: bool) -> molla_core::Result<()> {
        if self.diagnostics.pending() {
            self.device.poll(wgpu::PollType::Poll).map_err(|error| {
                molla_core::Error::Gpu(format!("FEM diagnostic poll failed: {error}"))
            })?;
        }
        if let Some(minimum_j) = self.diagnostics.take_ready()? {
            self.minimum_j = Some(minimum_j);
            self.clock.complete();
        }
        if active
            && self.minimum_j.is_some()
            && !self.clock.pending()
            && self.system.try_step_substep(self.substep)?
        {
            self.clock.submit();
            self.diagnostics
                .submit(&self.device, &self.queue, &self.system);
        }
        Ok(())
    }
}

pub(super) fn advance_islands(
    active: Res<PhysicsActive>,
    renderer_fault: Option<Res<render_health::RendererFault>>,
    mut islands: Query<&mut FemGpuIsland>,
) {
    for mut island in &mut islands {
        if island.failure.is_some() {
            continue;
        }
        if let Some(fault) = renderer_fault.as_ref() {
            island.failure = Some(fault.0.clone());
            continue;
        }
        if let Err(error) = island.advance(active.0) {
            error!("FEM island stopped: {error}");
            island.failure = Some(error.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn moving_scene() -> FemRigidGpuScene {
        use molla_core::{BodyId, WorldId};
        use molla_math::{Transform, Vec3};
        use molla_sim::{BodyParams, ModelBuilder};
        let mut soft = ModelBuilder::new();
        soft.set_world_gravity(WorldId(0), Vec3::ZERO);
        for point in [Vec3::ZERO, Vec3::X * 0.1, Vec3::Y * 0.1, Vec3::Z * 0.1] {
            soft.add_particle(point, 0.1, 0.0, WorldId(0));
        }
        soft.add_tet_mesh(&[[0, 1, 2, 3]], 2e6, 0.45, None).unwrap();
        let soft_model = soft.build().unwrap();
        let mut soft_state = soft_model.state().unwrap();
        soft_state
            .particle_qd
            .host_mut()
            .unwrap()
            .fill(Vec3::X * 0.3);
        let mut rigid = ModelBuilder::new();
        rigid.set_world_gravity(WorldId(0), Vec3::ZERO);
        rigid.begin_articulation();
        let body = rigid.add_body(BodyParams::new());
        rigid
            .add_joint_revolute(
                BodyId::NONE,
                body,
                Transform::IDENTITY,
                Transform::IDENTITY,
                Vec3::X,
            )
            .unwrap();
        let rigid_model = rigid.build().unwrap();
        let rigid_state = rigid_model.state().unwrap();
        let control = rigid_model.control();
        FemRigidGpuScene {
            soft_model,
            soft_state,
            rigid_model,
            rigid_state,
            control,
            shapes: Vec::new(),
        }
    }

    pub(super) fn gpu_island() -> (RenderDevice, RenderQueue, FemGpuIsland) {
        use bevy::render::renderer::WgpuWrapper;
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let mut descriptor = wgpu::DeviceDescriptor::default();
        <crate::host::GearboxApp as mara::window::WindowApp>::configure_gpu_limits(
            &adapter.limits(),
            &mut descriptor.required_limits,
        );
        let (device, queue) = pollster::block_on(adapter.request_device(&descriptor)).unwrap();
        let device = RenderDevice::from(device);
        let queue = RenderQueue(Arc::new(WgpuWrapper::new(queue)));
        let h = 1.0 / 4096.0;
        let island = FemGpuIsland::new(
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
        (device, queue, island)
    }

    #[test]
    #[ignore = "requires a real GPU through oslo make test-fem-gpu"]
    fn native_fem_rejects_initial_inversion_without_advancing() {
        let (device, queue, _) = gpu_island();
        for active in [false, true] {
            let mut scene = moving_scene();
            scene.soft_state.particle_q.host_mut().unwrap()[3].z = -0.1;
            let island = FemGpuIsland::new(
                &device,
                &queue,
                scene,
                FemRigidConfig {
                    max_substep: 1.0 / 4096.0,
                    elastic_iterations: 4,
                    ..Default::default()
                },
            )
            .unwrap();
            let mut app = App::new();
            app.insert_resource(PhysicsActive(active))
                .add_systems(Update, advance_islands);
            let entity = app.world_mut().spawn(island).id();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
            loop {
                app.update();
                let island = app.world().get::<FemGpuIsland>(entity).unwrap();
                assert_eq!(island.clock.submitted, 0);
                assert_eq!(island.completed_seconds(), 0.0);
                if let Some(error) = island.failure() {
                    assert!(error.contains("status=0x8"), "{error}");
                    break;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
        }
    }

    #[test]
    #[ignore = "requires a real GPU through oslo make test-fem-gpu"]
    fn native_fem_pacing_honors_pause_and_completed_time() {
        let (device, queue, island) = gpu_island();
        let h = 1.0 / 4096.0;
        let mut app = App::new();
        app.insert_resource(PhysicsActive(false))
            .add_systems(Update, advance_islands);
        let entity = app.world_mut().spawn(island).id();
        let initial_deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while app
            .world()
            .get::<FemGpuIsland>(entity)
            .unwrap()
            .minimum_j
            .is_none()
        {
            assert!(std::time::Instant::now() < initial_deadline);
            app.update();
            assert!(
                app.world()
                    .get::<FemGpuIsland>(entity)
                    .unwrap()
                    .failure()
                    .is_none()
            );
            std::thread::yield_now();
        }
        assert_eq!(
            app.world()
                .get::<FemGpuIsland>(entity)
                .unwrap()
                .completed_seconds(),
            0.0
        );
        app.world_mut().resource_mut::<PhysicsActive>().0 = true;
        app.update();
        let island = app.world().get::<FemGpuIsland>(entity).unwrap();
        assert!(island.failure().is_none(), "{:?}", island.failure());
        assert!(island.pending());
        assert_eq!(island.completed_seconds(), 0.0);
        app.world_mut().resource_mut::<PhysicsActive>().0 = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while app.world().get::<FemGpuIsland>(entity).unwrap().pending() {
            assert!(std::time::Instant::now() < deadline);
            app.update();
            let island = app.world().get::<FemGpuIsland>(entity).unwrap();
            assert!(island.failure().is_none(), "{:?}", island.failure());
            std::thread::yield_now();
        }
        for _ in 0..3 {
            app.update();
        }
        let island = app.world().get::<FemGpuIsland>(entity).unwrap();
        assert_eq!(island.completed_seconds(), h);
        assert_eq!(island.clock.submitted, 1);
        assert!(island.minimum_j.is_some_and(|j| j > 0.5));
        assert!(island.system.soft_state().particle_q.host().is_err());
        app.world_mut().resource_mut::<PhysicsActive>().0 = true;
        let mut previous = 1;
        while app
            .world()
            .get::<FemGpuIsland>(entity)
            .unwrap()
            .clock
            .completed
            < 4
        {
            assert!(std::time::Instant::now() < deadline);
            app.update();
            let island = app.world().get::<FemGpuIsland>(entity).unwrap();
            assert!(island.failure().is_none(), "{:?}", island.failure());
            assert!(island.clock.submitted - previous <= 1);
            assert!(island.clock.submitted - island.clock.completed <= 1);
            previous = island.clock.submitted;
            std::thread::yield_now();
        }
        let island = app.world().get::<FemGpuIsland>(entity).unwrap();
        let output = device.wgpu_device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("FEM scheduling acceptance snapshot"),
            size: 32,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device
            .wgpu_device()
            .create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(island.status(), 0, &output, 0, 4);
        encoder.copy_buffer_to_buffer(island.positions(), 0, &output, 16, 16);
        queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        output
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                tx.send(result).unwrap();
            });
        device
            .wgpu_device()
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        rx.recv().unwrap().unwrap();
        let bytes = output.slice(..).get_mapped_range();
        assert_eq!(&bytes[..4], &[0; 4]);
        let x = f32::from_le_bytes(bytes[16..20].try_into().unwrap());
        assert!(x.is_finite() && x > 0.0001, "node did not advance: {x}");
        drop(bytes);
        output.unmap();
    }

    #[test]
    #[ignore = "requires a real GPU through oslo make test-fem-gpu"]
    fn native_fem_stops_on_gpu_status_without_accepting_time() {
        let (device, queue, island) = gpu_island();
        let device = device.wgpu_device();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("FEM status fault injection"),
            source: wgpu::ShaderSource::Wgsl(
                "@group(0) @binding(0) var<storage, read_write> status: atomic<u32>;
                 @compute @workgroup_size(1) fn main() { atomicOr(&status, 8u); }"
                    .into(),
            ),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: island.status().as_entire_binding(),
            }],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        queue.submit([encoder.finish()]);
        let mut app = App::new();
        app.insert_resource(PhysicsActive(true))
            .add_systems(Update, advance_islands);
        let entity = app.world_mut().spawn(island).id();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while app
            .world()
            .get::<FemGpuIsland>(entity)
            .unwrap()
            .failure()
            .is_none()
        {
            assert!(std::time::Instant::now() < deadline);
            app.update();
            std::thread::yield_now();
        }
        for _ in 0..3 {
            app.update();
        }
        let island = app.world().get::<FemGpuIsland>(entity).unwrap();
        assert!(island.failure().unwrap().contains("status=0x8"));
        assert_eq!(island.clock.submitted, 1);
        assert_eq!(island.completed_seconds(), 0.0);
        assert!(island.system.soft_state().particle_q.host().is_err());
    }

    #[test]
    #[ignore = "requires a real GPU through oslo make test-fem-gpu"]
    fn native_fem_latches_renderer_fault_before_any_new_work() {
        let (_, _, island) = gpu_island();
        let mut app = App::new();
        app.insert_resource(PhysicsActive(true))
            .insert_resource(render_health::RendererFault("test renderer lost".into()))
            .add_systems(Update, advance_islands);
        let entity = app.world_mut().spawn(island).id();
        app.update();
        app.world_mut()
            .remove_resource::<render_health::RendererFault>();
        for _ in 0..3 {
            app.update();
        }
        let island = app.world().get::<FemGpuIsland>(entity).unwrap();
        assert_eq!(island.failure(), Some("test renderer lost"));
        assert_eq!(island.clock.submitted, 0);
        assert_eq!(island.completed_seconds(), 0.0);
    }

    #[test]
    fn fem_clock_advances_only_completed_submissions() {
        let mut clock = IslandClock::default();
        clock.complete();
        assert_eq!(clock.completed, 0);
        for expected in 1..=65 {
            assert!(!clock.pending());
            clock.submit();
            assert!(clock.pending());
            assert_eq!(clock.completed, expected - 1);
            clock.complete();
            assert!(!clock.pending());
            assert_eq!(clock.completed, expected);
            clock.complete();
            assert_eq!(clock.completed, expected);
        }
    }

    #[test]
    #[should_panic]
    fn fem_clock_rejects_duplicate_pending_work() {
        let mut clock = IslandClock::default();
        clock.submit();
        clock.submit();
    }
}
