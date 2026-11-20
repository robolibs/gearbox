//! The two draw materials: strokes (lines, points, arrows and outlines as
//! capsule quads) and surfaces (shapes, triangles, rings, beams, walls of
//! light and decals). Both carry the AgX tonemapping table, so the colours
//! asked for are the colours shown, and blend premultiplied: what a
//! fragment covers with alpha, and light it adds with none.

use bevy::mesh::{MeshVertexAttribute, MeshVertexBufferLayoutRef};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, CompareFunction, Face, RenderPipelineDescriptor, SpecializedMeshPipelineError,
    VertexFormat,
};
use bevy::shader::ShaderRef;

pub const ATTR_END: MeshVertexAttribute =
    MeshVertexAttribute::new("DrawEnd", 0x6b1d_4e01, VertexFormat::Float32x3);
pub const ATTR_COLOR_A: MeshVertexAttribute =
    MeshVertexAttribute::new("DrawColorA", 0x6b1d_4e02, VertexFormat::Unorm8x4);
pub const ATTR_COLOR_B: MeshVertexAttribute =
    MeshVertexAttribute::new("DrawColorB", 0x6b1d_4e03, VertexFormat::Unorm8x4);
pub const ATTR_RADII: MeshVertexAttribute =
    MeshVertexAttribute::new("DrawRadii", 0x6b1d_4e04, VertexFormat::Float32x2);
pub const ATTR_DIST: MeshVertexAttribute =
    MeshVertexAttribute::new("DrawDist", 0x6b1d_4e05, VertexFormat::Float32x2);
pub const ATTR_CODE: MeshVertexAttribute =
    MeshVertexAttribute::new("DrawCode", 0x6b1d_4e06, VertexFormat::Uint32);
pub const ATTR_FILL_COLOR: MeshVertexAttribute =
    MeshVertexAttribute::new("DrawFillColor", 0x6b1d_4e07, VertexFormat::Unorm8x4);
/// Strokes: glow, pulse Hz, flow speed, spacing.
pub const ATTR_FX: MeshVertexAttribute =
    MeshVertexAttribute::new("DrawFx", 0x6b1d_4e08, VertexFormat::Float32x4);
/// Strokes: fade, strip length, urgency, phase.
pub const ATTR_FX2: MeshVertexAttribute =
    MeshVertexAttribute::new("DrawFx2", 0x6b1d_4e09, VertexFormat::Float32x4);
/// Surfaces: unit-shape coordinates, or arc length and height share.
pub const ATTR_LOCAL: MeshVertexAttribute =
    MeshVertexAttribute::new("DrawLocal", 0x6b1d_4e0a, VertexFormat::Float32x4);
/// Surfaces: look, glow, pulse Hz, phase.
pub const ATTR_SURFACE_FX: MeshVertexAttribute =
    MeshVertexAttribute::new("DrawSurfaceFx", 0x6b1d_4e0b, VertexFormat::Float32x4);
/// Strokes: the strip's point before the segment.
pub const ATTR_PREV: MeshVertexAttribute =
    MeshVertexAttribute::new("DrawPrev", 0x6b1d_4e0c, VertexFormat::Float32x3);
/// Strokes: the strip's point after the segment.
pub const ATTR_NEXT: MeshVertexAttribute =
    MeshVertexAttribute::new("DrawNext", 0x6b1d_4e0d, VertexFormat::Float32x3);
/// Surfaces: urgency, size, length or spacing, fade.
pub const ATTR_SURFACE_FX2: MeshVertexAttribute =
    MeshVertexAttribute::new("DrawSurfaceFx2", 0x6b1d_4e0e, VertexFormat::Float32x4);
pub const ATTR_TERRAIN: MeshVertexAttribute =
    MeshVertexAttribute::new("DrawTerrain", 0x6b1d_4e0f, VertexFormat::Float32x4);

const STROKE_SHADER: &str = "embedded://gearbox/draw/draw_stroke.wesl";
const SURFACE_SHADER: &str = "embedded://gearbox/draw/draw_surface.wesl";
/// Sort bias that puts on-top draws after everything else blended.
const ON_TOP_BIAS: f32 = 1.0e7;
/// Sort bias that blends strokes after fills at the same distance.
const STROKE_BIAS: f32 = 0.5;

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
#[bind_group_data(StrokeKey)]
pub struct StrokeMaterial {
    /// x: frost strength, y: opacity.
    #[uniform(0)]
    pub finish: Vec4,
    #[texture(1, dimension = "3d")]
    #[sampler(2)]
    pub lut: Handle<Image>,
    pub on_top: bool,
    /// Opaque splats: alpha to coverage, depth written.
    pub splat: bool,
}

#[repr(C)]
#[derive(Eq, PartialEq, Hash, Copy, Clone)]
pub struct StrokeKey {
    on_top: bool,
    splat: bool,
    frost: bool,
}

impl From<&StrokeMaterial> for StrokeKey {
    fn from(m: &StrokeMaterial) -> Self {
        Self {
            on_top: m.on_top,
            splat: m.splat,
            frost: m.reads_view_transmission_texture(),
        }
    }
}

impl Material for StrokeMaterial {
    fn vertex_shader() -> ShaderRef {
        STROKE_SHADER.into()
    }

    fn fragment_shader() -> ShaderRef {
        STROKE_SHADER.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        if self.splat && !self.on_top {
            AlphaMode::AlphaToCoverage
        } else {
            AlphaMode::Premultiplied
        }
    }

    fn reads_view_transmission_texture(&self) -> bool {
        self.finish.x > 0.0 && (!self.splat || self.on_top)
    }

    fn depth_bias(&self) -> f32 {
        STROKE_BIAS + if self.on_top { ON_TOP_BIAS } else { 0.0 }
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let mut attributes = vec![
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            ATTR_END.at_shader_location(1),
            ATTR_COLOR_A.at_shader_location(2),
            ATTR_COLOR_B.at_shader_location(3),
            ATTR_RADII.at_shader_location(4),
            ATTR_DIST.at_shader_location(5),
            ATTR_CODE.at_shader_location(6),
            ATTR_FX.at_shader_location(7),
            ATTR_FX2.at_shader_location(8),
            ATTR_PREV.at_shader_location(9),
            ATTR_NEXT.at_shader_location(10),
        ];
        if layout.0.contains(ATTR_TERRAIN) {
            attributes.push(ATTR_TERRAIN.at_shader_location(11));
            descriptor.vertex.shader_defs.push("DRAW_TERRAIN".into());
        }
        let vertex_layout = layout.0.get_layout(&attributes)?;
        descriptor.vertex.buffers = vec![vertex_layout];
        descriptor.primitive.cull_mode = None;
        let k = key.bind_group_data;
        let splat = k.splat && !k.on_top;
        if splat && let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader_defs.push("DRAW_SPLAT".into());
        }
        depth_state(descriptor, k.on_top, splat);
        if k.frost {
            frost_pipeline(descriptor);
        }
        Ok(())
    }
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
#[bind_group_data(SurfaceKey)]
pub struct SurfaceMaterial {
    /// x: 1 to shade with a headlight, y: frost strength, z: opacity.
    #[uniform(0)]
    pub flags: Vec4,
    #[texture(1, dimension = "3d")]
    #[sampler(2)]
    pub lut: Handle<Image>,
    pub on_top: bool,
    pub translucent: bool,
    pub double_sided: bool,
}

#[repr(C)]
#[derive(Eq, PartialEq, Hash, Copy, Clone)]
pub struct SurfaceKey {
    on_top: bool,
    translucent: bool,
    double_sided: bool,
    frost: bool,
}

impl From<&SurfaceMaterial> for SurfaceKey {
    fn from(m: &SurfaceMaterial) -> Self {
        Self {
            on_top: m.on_top,
            translucent: m.translucent,
            double_sided: m.double_sided,
            frost: m.reads_view_transmission_texture(),
        }
    }
}

impl Material for SurfaceMaterial {
    fn vertex_shader() -> ShaderRef {
        SURFACE_SHADER.into()
    }

    fn fragment_shader() -> ShaderRef {
        SURFACE_SHADER.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        if self.translucent || self.on_top {
            AlphaMode::Premultiplied
        } else {
            AlphaMode::Opaque
        }
    }

    fn reads_view_transmission_texture(&self) -> bool {
        self.flags.y > 0.0 && (self.translucent || self.on_top)
    }

    fn depth_bias(&self) -> f32 {
        if self.on_top { ON_TOP_BIAS } else { 0.0 }
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
            ATTR_FILL_COLOR.at_shader_location(2),
            ATTR_LOCAL.at_shader_location(3),
            ATTR_SURFACE_FX.at_shader_location(4),
            ATTR_SURFACE_FX2.at_shader_location(5),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        let k = key.bind_group_data;
        descriptor.primitive.cull_mode = if k.double_sided {
            None
        } else {
            Some(Face::Back)
        };
        depth_state(descriptor, k.on_top, !k.translucent);
        if k.frost {
            frost_pipeline(descriptor);
        }
        Ok(())
    }
}

/// Depth writes for opaque surfaces only; on-top draws skip the depth test.
fn depth_state(descriptor: &mut RenderPipelineDescriptor, on_top: bool, opaque: bool) {
    if let Some(depth) = descriptor.depth_stencil.as_mut() {
        depth.depth_write_enabled = Some(opaque && !on_top);
        if on_top {
            depth.depth_compare = Some(CompareFunction::Always);
        }
    }
}

pub(super) fn frost_pipeline(descriptor: &mut RenderPipelineDescriptor) {
    if let Some(fragment) = descriptor.fragment.as_mut() {
        fragment.shader_defs.push("DRAW_FROST".into());
        for target in fragment.targets.iter_mut().flatten() {
            target.blend =
                Some(bevy::render::render_resource::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        }
    }
    if let Some(depth) = descriptor.depth_stencil.as_mut() {
        depth.depth_write_enabled = Some(false);
    }
}

#[cfg(test)]
mod frost_tests {
    use super::*;

    #[test]
    fn opaque_data_keeps_its_depth_path_when_blur_is_requested() {
        let mut stroke = StrokeMaterial {
            finish: Vec4::X,
            lut: Default::default(),
            on_top: false,
            splat: true,
        };
        assert!(!stroke.reads_view_transmission_texture());
        assert_eq!(stroke.alpha_mode(), AlphaMode::AlphaToCoverage);
        stroke.splat = false;
        assert!(stroke.reads_view_transmission_texture());
        let mut surface = SurfaceMaterial {
            flags: Vec4::Y,
            lut: Default::default(),
            on_top: false,
            translucent: false,
            double_sided: false,
        };
        assert!(!surface.reads_view_transmission_texture());
        surface.translucent = true;
        assert!(surface.reads_view_transmission_texture());
        surface.flags.y = 0.0;
        assert_eq!(surface.alpha_mode(), AlphaMode::Premultiplied);
        assert!(!surface.reads_view_transmission_texture());
    }
}
