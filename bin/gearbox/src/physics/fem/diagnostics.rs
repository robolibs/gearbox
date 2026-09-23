use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

use molla_core::{Error, Result};
use molla_solvers::fem_rigid_gpu::FemRigidGpuSystem;

pub(super) struct Diagnostics {
    staging: wgpu::Buffer,
    completion: Arc<AtomicU8>,
    pending: bool,
}

impl Diagnostics {
    pub(super) fn pending(&self) -> bool {
        self.pending
    }

    pub(super) fn new(device: &wgpu::Device) -> Self {
        Self {
            staging: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("FEM status and minimum-volume readback"),
                size: 12,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            completion: Arc::new(AtomicU8::new(0)),
            pending: false,
        }
    }

    pub(super) fn submit(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        system: &FemRigidGpuSystem,
    ) {
        assert!(!self.pending);
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(system.status_buffer(), 0, &self.staging, 0, 4);
        encoder.copy_buffer_to_buffer(system.volume_diagnostics_buffer(), 0, &self.staging, 4, 8);
        queue.submit([encoder.finish()]);
        self.completion.store(0, Ordering::Release);
        self.pending = true;
        let completion = self.completion.clone();
        self.staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                completion.store(if result.is_ok() { 1 } else { 2 }, Ordering::Release);
            });
    }

    pub(super) fn take_ready(&mut self) -> Result<Option<f32>> {
        if !self.pending {
            return Ok(None);
        }
        match self.completion.load(Ordering::Acquire) {
            0 => return Ok(None),
            2 => {
                self.pending = false;
                return Err(Error::Gpu("FEM diagnostic readback failed".into()));
            }
            _ => {}
        }
        let mapped = self.staging.slice(..).get_mapped_range();
        let words = std::array::from_fn(|i| {
            u32::from_le_bytes(mapped[4 * i..4 * i + 4].try_into().unwrap())
        });
        drop(mapped);
        self.staging.unmap();
        self.pending = false;
        validate(words).map(Some)
    }
}

fn validate([status, volume, invalid_tet]: [u32; 3]) -> Result<f32> {
    let minimum_j = f32::from_bits(volume);
    if status != 0 || invalid_tet != u32::MAX || !minimum_j.is_finite() || minimum_j <= 0.5 {
        return Err(Error::Solver(format!(
            "FEM acceptance failed: status={status:#x}, minimum J={minimum_j}, invalid tet={invalid_tet}"
        )));
    }
    Ok(minimum_j)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_sticky_flags_invalid_tets_and_volume_failures() {
        for status in [1, 2, 4, 8, 16, 32, 64, u32::MAX] {
            assert!(validate([status, 1.0_f32.to_bits(), u32::MAX]).is_err());
        }
        assert!(validate([0, 1.0_f32.to_bits(), 0]).is_err());
        for j in [f32::NAN, f32::INFINITY, -1.0, 0.0, 0.49, 0.5] {
            assert!(validate([0, j.to_bits(), u32::MAX]).is_err());
        }
        for j in [0.5001_f32, 1.0, 1.5] {
            assert_eq!(validate([0, j.to_bits(), u32::MAX]).unwrap(), j);
        }
    }
}
