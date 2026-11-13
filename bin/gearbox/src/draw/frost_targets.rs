use std::sync::Arc;

use bevy::pbr::ViewTransmissionTexture;
use bevy::prelude::*;
use bevy::render::{
    render_resource::*,
    renderer::{RenderDevice, RenderQueue},
};
use gearbox_api::draw_data::{Charge, DrawingBackend, GpuCharge, GpuClass, MemoryClass};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Failure {
    Cpu,
    Gpu,
    Extent,
}

impl Failure {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Cpu => "drawing frost CPU ownership budget exhausted; preparation will retry",
            Self::Gpu => "drawing frost GPU auxiliary budget exhausted; preparation will retry",
            Self::Extent => "drawing frost texture extent or format exceeds supported limits",
        }
    }
}

pub(super) fn bytes(size: UVec2, format: TextureFormat, limit: u32) -> Result<u64, Failure> {
    if size.x == 0
        || size.y == 0
        || size.x > limit
        || size.y > limit
        || format.block_dimensions() != (1, 1)
    {
        return Err(Failure::Extent);
    }
    let stride = format.block_copy_size(None).ok_or(Failure::Extent)?;
    u64::from(size.x)
        .checked_mul(u64::from(size.y))
        .and_then(|pixels| pixels.checked_mul(u64::from(stride)))
        .ok_or(Failure::Extent)
}

pub(super) struct Target {
    pub texture: Texture,
    pub view: TextureView,
    pub sampler: Sampler,
    pub owned: bool,
    queue: RenderQueue,
    gpu: Option<GpuCharge>,
    _cpu: Option<Charge>,
}

impl Target {
    pub fn prepare(
        device: &RenderDevice,
        queue: &RenderQueue,
        backend: Option<&DrawingBackend>,
        size: UVec2,
        format: TextureFormat,
        borrowed: Option<&ViewTransmissionTexture>,
    ) -> Result<Arc<Self>, Failure> {
        let bytes = bytes(size, format, device.limits().max_texture_dimension_2d)?;
        if borrowed.is_some_and(|source| {
            source.texture.width() != size.x
                || source.texture.height() != size.y
                || source.texture.format() != format
        }) {
            return Err(Failure::Extent);
        }
        let cpu = backend
            .map(|backend| {
                let bytes = (size_of::<Self>() + size_of::<Arc<Self>>() * 4 + 512) as u64;
                backend
                    .budget()
                    .reserve(bytes)
                    .and_then(|mut r| r.charge(bytes, MemoryClass::Metadata))
                    .map_err(|_| Failure::Cpu)
            })
            .transpose()?;
        let gpu = backend
            .map(|backend| {
                backend
                    .gpu()
                    .reserve_class(bytes, GpuClass::Auxiliary)
                    .ok_or(Failure::Gpu)
            })
            .transpose()?;
        let (texture, view, sampler) = if let Some(source) = borrowed {
            (
                source.texture.clone(),
                source.view.clone(),
                source.sampler.clone(),
            )
        } else {
            let texture = device.create_texture(&TextureDescriptor {
                label: Some("drawing frost scene"),
                size: Extent3d {
                    width: size.x,
                    height: size.y,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format,
                usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&TextureViewDescriptor::default());
            let sampler = device.create_sampler(&SamplerDescriptor {
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                ..Default::default()
            });
            (texture, view, sampler)
        };
        Ok(Arc::new(Self {
            texture,
            view,
            sampler,
            owned: borrowed.is_none(),
            queue: queue.clone(),
            gpu,
            _cpu: cpu,
        }))
    }

    pub fn matches(&self, size: Option<UVec2>, format: TextureFormat) -> bool {
        size == Some(UVec2::new(self.texture.width(), self.texture.height()))
            && self.texture.format() == format
    }

    pub fn binding(&self) -> ViewTransmissionTexture {
        ViewTransmissionTexture {
            texture: self.texture.clone(),
            view: self.view.clone(),
            sampler: self.sampler.clone(),
        }
    }
}

impl Drop for Target {
    fn drop(&mut self) {
        if let Some(mut gpu) = self.gpu.take() {
            gpu.retire();
            let texture = self.texture.clone();
            let cpu = self._cpu.take();
            self.queue.submit([]);
            self.queue
                .on_submitted_work_done(move || drop((gpu, cpu, texture)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_target_bytes_use_actual_extent_format_and_no_msaa_multiplier() {
        assert_eq!(
            bytes(UVec2::new(1200, 800), TextureFormat::Rgba16Float, 8192),
            Ok(7_680_000)
        );
        assert_eq!(
            bytes(UVec2::new(640, 480), TextureFormat::Rgba8Unorm, 8192),
            Ok(1_228_800)
        );
        for size in [UVec2::ZERO, UVec2::new(8193, 1), UVec2::new(1, 8193)] {
            assert_eq!(
                bytes(size, TextureFormat::Rgba16Float, 8192),
                Err(Failure::Extent)
            );
        }
        assert_eq!(
            bytes(UVec2::ONE, TextureFormat::Bc1RgbaUnorm, 8192),
            Err(Failure::Extent)
        );
    }
}
