use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;
use gearbox_api::wire::{draw_data::*, draw_geom};

#[derive(Clone, ShaderType)]
pub(super) struct Style {
    pub constant: Vec4,
    pub missing: Vec4,
    pub controls: UVec4,
    pub effects: Vec4,
    pub ramp: [Vec4; 256],
    pub categories: [UVec4; 64],
    /// x: Eye-Dome Lighting strength of the opaque points.
    pub shading: Vec4,
}

fn color(bytes: [u8; 4]) -> Vec4 {
    Vec4::from_array(bytes.map(|v| v as f32 / 255.0))
}

impl Style {
    pub fn new(presentation: &Presentation) -> Self {
        let mut result = Self {
            constant: Vec4::ONE,
            missing: color([130, 130, 130, 64]),
            controls: UVec4::ZERO,
            effects: Vec4::new(
                presentation.opacity,
                presentation.glow,
                presentation.blur,
                match presentation.size {
                    PointSize::Metres { radius } => radius,
                    PointSize::Pixels { radius } => -radius,
                },
            ),
            ramp: [Vec4::ONE; 256],
            categories: [UVec4::ZERO; 64],
            shading: Vec4::new(presentation.eye_dome, 0.0, 0.0, 0.0),
        };
        let mut map = None;
        match &presentation.color {
            ColorMode::Constant { rgba } => result.constant = color(*rgba),
            ColorMode::Attribute { space, .. } => {
                result.controls.x = 1;
                result.controls.y = u32::from(*space == ColorSpace::Linear);
            }
            ColorMode::Scalar {
                colormap, missing, ..
            } => {
                result.controls.x = 2;
                result.missing = color(*missing);
                map = Some(*colormap);
            }
            ColorMode::Category {
                entries, missing, ..
            } => {
                result.controls.x = 3;
                result.controls.z = entries.len() as u32;
                result.missing = color(*missing);
                for (target, entry) in result.categories.iter_mut().zip(entries) {
                    *target = UVec4::new(entry.value, u32::from_le_bytes(entry.rgba), 0, 0);
                }
            }
            ColorMode::Axis { colormap, .. } | ColorMode::Range { colormap, .. } => {
                result.controls.x = 2;
                result.missing = Vec4::ONE;
                map = Some(*colormap);
            }
            ColorMode::Age {
                clock, colormap, ..
            } => {
                result.controls.x = 4;
                result.controls.w = u32::from(*clock == AgeClock::Source);
                map = Some(*colormap);
            }
            _ => {}
        }
        if let Some(map) = map {
            result.controls.y = match map {
                gearbox_api::colormap::COSTMAP | gearbox_api::colormap::MAP => 1,
                gearbox_api::colormap::RAW => 2,
                gearbox_api::colormap::OKABE => 3,
                _ => 0,
            };
            for (index, entry) in result.ramp.iter_mut().enumerate() {
                *entry = color(if result.controls.y == 0 {
                    draw_geom::ramp(map, index as f32 / 255.0)
                } else {
                    draw_geom::palette(map, index as f32)
                });
            }
        }
        result
    }
}

#[derive(Clone, ShaderType)]
pub(super) struct PageUniform {
    pub world_from_local: Mat4,
    pub clip_rotation: Mat4,
    pub clip_min: Vec4,
    pub clip_max: Vec4,
    pub age: super::age::AgeUniform,
    /// x: the radius in metres a point is drawn at least, so a page's sparse
    /// coarse records still cover it; 0 at full detail.
    pub coverage: Vec4,
    /// Where the page's attribute words sit: see `appearance::Layout::uniform`.
    pub attributes: UVec4,
    /// x: glyph radius in metres, y: glyph radius in pixels, z: local position error in metres.
    pub lod: Vec4,
    /// x: hierarchy nodes the GPU level-of-detail pass lists, 0 for coarse pages.
    pub tree: UVec4,
}

#[cfg(test)]
pub(super) fn uniform_bytes<
    T: ShaderType + bevy::render::render_resource::encase::internal::WriteInto,
>(
    value: &T,
) -> Vec<u8> {
    let mut bytes = bevy::render::render_resource::encase::UniformBuffer::new(Vec::new());
    bytes.write(value).expect("cloud uniform layout");
    bytes.into_inner()
}

pub(super) fn owned_uniform<
    T: ShaderType + bevy::render::render_resource::encase::internal::WriteInto,
>(
    budget: &gearbox_api::draw_data::CpuBudget,
    value: &T,
) -> Result<super::owned::List<u8>, gearbox_api::draw_data::StorageError> {
    let count = usize::try_from(value.size().get())
        .map_err(|_| gearbox_api::draw_data::StorageError::Allocation)?;
    let mut bytes =
        super::owned::List::new(budget, count, gearbox_api::draw_data::MemoryClass::Scratch)?;
    bytes.resize_with(count, || 0)?;
    bevy::render::render_resource::encase::UniformBuffer::new(&mut bytes[..])
        .write(value)
        .expect("cloud uniform layout");
    Ok(bytes)
}
