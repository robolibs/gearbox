use super::{FemExtension, FemMaterial};
use crate::physics::fem::FemGpuIsland;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::prelude::*;
use bevy::render::render_resource::{Buffer, BufferId};
use bevy::render::renderer::{RenderDevice, RenderQueue};

/// Three GPU snapshots: this frame, its predecessor, and the next writable slot.
#[derive(Component)]
pub(crate) struct FemSurfaceFrames {
    positions: [Buffer; 3],
    pub(super) body_poses: [Buffer; 3],
    validity: [Buffer; 3],
    current: usize,
}

impl FemSurfaceFrames {
    pub(crate) fn new(device: &RenderDevice, queue: &RenderQueue, island: &FemGpuIsland) -> Self {
        let source = island.positions();
        let positions = std::array::from_fn(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("FEM render frame positions"),
                size: source.size(),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        });
        let validity = std::array::from_fn(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("FEM render frame validity"),
                size: 12,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        let body_source = island.rigid_poses();
        let body_poses = std::array::from_fn(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("FEM render frame rigid poses"),
                size: body_source.size(),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        for destination in &body_poses {
            encoder.copy_buffer_to_buffer(body_source, 0, destination, 0, body_source.size());
        }
        for (destination, diagnostic) in positions.iter().zip(&validity) {
            encoder.copy_buffer_to_buffer(source, 0, destination, 0, source.size());
            copy_validity(&mut encoder, island, diagnostic);
        }
        queue.submit([encoder.finish()]);
        Self {
            positions,
            body_poses,
            validity,
            current: 0,
        }
    }

    fn capture(&mut self, device: &RenderDevice, queue: &RenderQueue, island: &FemGpuIsland) {
        let next = (self.current + 1) % 3;
        let source = island.positions();
        assert_eq!(source.size(), self.positions[next].size());
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(source, 0, &self.positions[next], 0, source.size());
        let body_source = island.rigid_poses();
        assert_eq!(body_source.size(), self.body_poses[next].size());
        encoder.copy_buffer_to_buffer(
            body_source,
            0,
            &self.body_poses[next],
            0,
            body_source.size(),
        );
        copy_validity(&mut encoder, island, &self.validity[next]);
        queue.submit([encoder.finish()]);
        self.current = next;
    }

    fn extension(&self, slot: usize) -> FemExtension {
        FemExtension {
            positions: self.positions[slot].clone(),
            previous_positions: self.positions[(slot + 2) % 3].clone(),
            validity: self.validity[slot].clone(),
        }
    }

    pub(super) fn rigid_extension(
        &self,
        slot: usize,
        bind: super::rigid::RigidBind,
    ) -> super::rigid::RigidExtension {
        super::rigid::RigidExtension {
            poses: self.body_poses[slot].clone(),
            previous_poses: self.body_poses[(slot + 2) % 3].clone(),
            validity: self.validity[slot].clone(),
            bind,
        }
    }
}

fn copy_validity(encoder: &mut wgpu::CommandEncoder, island: &FemGpuIsland, destination: &Buffer) {
    encoder.copy_buffer_to_buffer(island.status(), 0, destination, 0, 4);
    encoder.copy_buffer_to_buffer(
        island.system.volume_diagnostics_buffer(),
        0,
        destination,
        4,
        8,
    );
}

/// Surface materials with immutable buffer bindings for each frame slot.
#[derive(Component)]
#[require(NoFrustumCulling)]
pub(crate) struct FemSurfaceBinding {
    island: Entity,
    generation: BufferId,
    materials: [Handle<FemMaterial>; 3],
}

impl FemSurfaceBinding {
    pub(crate) fn new(
        island: Entity,
        frames: &FemSurfaceFrames,
        materials: &mut Assets<FemMaterial>,
        base: StandardMaterial,
    ) -> Self {
        Self {
            island,
            generation: frames.positions[0].id(),
            materials: std::array::from_fn(|slot| {
                materials.add(FemMaterial {
                    base: base.clone(),
                    extension: frames.extension(slot),
                })
            }),
        }
    }

    pub(crate) fn material(&self) -> MeshMaterial3d<FemMaterial> {
        MeshMaterial3d(self.materials[0].clone())
    }
}

pub(super) fn capture_surfaces(
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut islands: Query<(&FemGpuIsland, &mut FemSurfaceFrames)>,
    mut surfaces: Query<(
        &FemSurfaceBinding,
        &mut MeshMaterial3d<FemMaterial>,
        &mut Visibility,
    )>,
    mut rigid_surfaces: Query<
        (
            &super::rigid::FemRigidBinding,
            &mut MeshMaterial3d<super::rigid::RigidMaterial>,
            &mut Visibility,
        ),
        Without<FemSurfaceBinding>,
    >,
) {
    for (island, mut frames) in &mut islands {
        if island.failure().is_none() {
            frames.capture(&device, &queue, island);
        }
    }
    for (binding, mut material, mut visibility) in &mut surfaces {
        match islands.get(binding.island) {
            Ok((island, frames))
                if island.failure().is_none() && frames.positions[0].id() == binding.generation =>
            {
                material.0 = binding.materials[frames.current].clone();
            }
            _ => *visibility = Visibility::Hidden,
        }
    }
    for (binding, mut material, mut visibility) in &mut rigid_surfaces {
        match islands.get(binding.island) {
            Ok((island, frames))
                if island.failure().is_none()
                    && frames.body_poses[0].id() == binding.generation =>
            {
                material.0 = binding.materials[frames.current].clone();
            }
            _ => *visibility = Visibility::Hidden,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pair(
        device: &RenderDevice,
        queue: &RenderQueue,
        frames: &FemSurfaceFrames,
    ) -> [f32; 2] {
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("FEM render history test readback"),
            size: 32,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let pair = frames.extension(frames.current);
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&pair.positions, 0, &staging, 0, 16);
        encoder.copy_buffer_to_buffer(&pair.previous_positions, 0, &staging, 16, 16);
        queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                tx.send(result).unwrap();
            });
        device
            .wgpu_device()
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        rx.recv().unwrap().unwrap();
        let data = staging.slice(..).get_mapped_range();
        let result = [
            f32::from_le_bytes(data[..4].try_into().unwrap()),
            f32::from_le_bytes(data[16..20].try_into().unwrap()),
        ];
        drop(data);
        staging.unmap();
        result
    }

    #[test]
    #[ignore = "requires a real GPU through oslo make test-fem-gpu"]
    fn snapshots_keep_render_history_across_pause_and_wrap() {
        let (device, queue, mut island) = crate::physics::fem::tests::gpu_island();
        let mut frames = FemSurfaceFrames::new(&device, &queue, &island);
        let ids = frames.positions.each_ref().map(|buffer| buffer.id());
        assert_eq!(sample_pair(&device, &queue, &frames), [0.0, 0.0]);
        let mut previous = 0.0;
        for frame in 0..12 {
            if frame % 3 != 1 {
                island.system.step(16.0 / 4096.0).unwrap();
            }
            frames.capture(&device, &queue, &island);
            let [current, history] = sample_pair(&device, &queue, &frames);
            assert_eq!(
                history, previous,
                "history is not the preceding rendered frame"
            );
            if frame % 3 == 1 {
                assert_eq!(current, previous, "paused snapshots changed");
            } else {
                assert!((current - previous - 0.3 * 16.0 / 4096.0).abs() < 1e-5);
            }
            assert_eq!(frames.positions.each_ref().map(|buffer| buffer.id()), ids);
            assert!(island.system.soft_state().particle_q.host().is_err());
            previous = current;
        }
    }
}
