//! Grids drawn as heat maps: the cells coloured once into a texture, laid
//! on a mesh that follows the terrain (and the cells' own heights), drawn
//! with soft cell edges, optional contour lines and the hottest cells
//! glowing. Only the texture is rebuilt when just the cells change.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, CompareFunction, Extent3d, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, TextureDimension,
    TextureFormat,
};
use bevy::shader::ShaderRef;
use gearbox_api::wire::draw::unpack_rgba;
use gearbox_api::wire::draw_geom::colormap_rgba;
use gearbox_api::{DrawEntity, colormap, grid_encoding};

use super::mesh::Ground;

const HEAT_SHADER: &str = "embedded://gearbox/draw/draw_heat.wgsl";
/// Largest texture side; bigger grids are sampled down to it.
const MAX_SIDE: usize = 4096;
/// Most vertices along one side of a heat mesh.
const MAX_STEPS: usize = 768;
/// Lift of the map off the ground, metres.
const LIFT: f32 = 0.035;
/// Opacity of the fill, before the cells' own.
const FILL_ALPHA: f32 = 0.55;
/// Normalised value from which cells glow.
const HOT: f32 = 0.9;
/// How brightly hot cells glow.
const HOT_GLOW: f32 = 0.3;

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct HeatParams {
    /// Texture columns and rows, contour spacing in normalised value (0
    /// none), glow.
    pub grid: Vec4,
    /// Fill opacity, hot threshold, hot glow, unused.
    pub look: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
#[bind_group_data(HeatKey)]
pub struct HeatMaterial {
    #[uniform(0)]
    pub params: HeatParams,
    #[texture(1, dimension = "3d")]
    #[sampler(2)]
    pub lut: Handle<Image>,
    #[texture(3)]
    #[sampler(4)]
    pub cells: Handle<Image>,
    #[texture(5)]
    #[sampler(6)]
    pub values: Handle<Image>,
    pub on_top: bool,
}

#[repr(C)]
#[derive(Eq, PartialEq, Hash, Copy, Clone)]
pub struct HeatKey {
    on_top: bool,
}

impl From<&HeatMaterial> for HeatKey {
    fn from(m: &HeatMaterial) -> Self {
        Self { on_top: m.on_top }
    }
}

impl Material for HeatMaterial {
    fn vertex_shader() -> ShaderRef {
        HEAT_SHADER.into()
    }

    fn fragment_shader() -> ShaderRef {
        HEAT_SHADER.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }

    fn depth_bias(&self) -> f32 {
        if self.on_top { 1.0e7 } else { -0.5 }
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
        let vertex_layout = layout.0.get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0), Mesh::ATTRIBUTE_UV_0.at_shader_location(1)])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        descriptor.primitive.cull_mode = None;
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(false);
            if key.bind_group_data.on_top {
                depth.depth_compare = Some(CompareFunction::Always);
            }
        }
        Ok(())
    }
}

/// Columns, rows and cell sizes of a grid, and the stride it is sampled at.
struct Dims {
    cols: usize,
    rows: usize,
    cell_x: f32,
    cell_y: f32,
    stride: usize,
}

fn dims(e: &DrawEntity) -> Option<Dims> {
    let (cols, rows, cell_x, cell_y) = e.grid_dims();
    if cols == 0 || rows == 0 {
        return None;
    }
    let stride = cols.max(rows).div_ceil(MAX_SIDE).max(1);
    Some(Dims { cols, rows, cell_x, cell_y, stride })
}

/// The cells of a grid as numbers, `None` where unknown.
fn decode(e: &DrawEntity) -> Vec<Option<f32>> {
    let unknown = e.prop_f32("unknown");
    let n = e.cell_count();
    let known = |v: f32| (v.is_finite() && unknown != Some(v)).then_some(v);
    match e.sub {
        grid_encoding::U16 => (0..n).map(|i| known(u16::from_le_bytes([e.cells[2 * i], e.cells[2 * i + 1]]) as f32)).collect(),
        grid_encoding::F32 => {
            (0..n).map(|i| known(f32::from_le_bytes([e.cells[4 * i], e.cells[4 * i + 1], e.cells[4 * i + 2], e.cells[4 * i + 3]]))).collect()
        }
        _ => e.cells.iter().map(|b| known(*b as f32)).collect(),
    }
}

/// The colormap a grid is drawn in, viridis when none is named.
fn map_of(e: &DrawEntity) -> u32 {
    if e.colormap == colormap::NONE { colormap::VIRIDIS } else { e.colormap }
}

/// A cell value as a share of the map: the ramp's range, or the palette's.
fn share(map: u32, v: f32, lo: f32, hi: f32) -> f32 {
    match map {
        colormap::COSTMAP | colormap::MAP => (v / 100.0).clamp(0.0, 1.0),
        colormap::RAW => (v / 255.0).clamp(0.0, 1.0),
        colormap::OKABE => 0.0,
        _ => {
            let span = hi - lo;
            if span.abs() > 1e-12 { ((v - lo) / span).clamp(0.0, 1.0) } else { 0.5 }
        }
    }
}

/// The colour texture (premultiplied sRGB) and the value texture (a share
/// of the map) of a grid.
pub fn images(e: &DrawEntity) -> Option<(Image, Image, UVec2)> {
    let d = dims(e)?;
    let cells = decode(e);
    let map = map_of(e);
    let (lo, hi) = gearbox_api::wire::draw_geom::value_range(cells.iter().flatten().copied(), e.vmin, e.vmax);
    let tint = if e.color != 0 { unpack_rgba(e.color)[3] as u32 } else { 255 };
    let lut: Option<Vec<([u8; 4], u8)>> = (e.sub == grid_encoding::U8).then(|| {
        (0..=255u32)
            .map(|b| {
                let v = b as f32;
                (colormap_rgba(map, v, lo, hi), (share(map, v, lo, hi) * 255.0).round() as u8)
            })
            .collect()
    });
    let (w, h) = (d.cols.div_ceil(d.stride), d.rows.div_ceil(d.stride));
    let mut rgba = vec![0u8; w * h * 4];
    let mut share_bytes = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = (y * d.stride) * d.cols + x * d.stride;
            let Some(Some(v)) = cells.get(i) else {
                continue;
            };
            let (c, t) = match &lut {
                Some(lut) => lut[(*v as usize).min(255)],
                None => (colormap_rgba(map, *v, lo, hi), (share(map, *v, lo, hi) * 255.0).round() as u8),
            };
            let a = c[3] as u32 * tint / 255;
            let k = (y * w + x) * 4;
            rgba[k] = (c[0] as u32 * a / 255) as u8;
            rgba[k + 1] = (c[1] as u32 * a / 255) as u8;
            rgba[k + 2] = (c[2] as u32 * a / 255) as u8;
            rgba[k + 3] = a as u8;
            share_bytes[y * w + x] = t;
        }
    }
    let image = |data: Vec<u8>, format: TextureFormat| {
        let mut img = Image::new(
            Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
            TextureDimension::D2,
            data,
            format,
            RenderAssetUsages::RENDER_WORLD,
        );
        img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::ClampToEdge,
            address_mode_v: ImageAddressMode::ClampToEdge,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            ..default()
        });
        img
    };
    Some((image(rgba, TextureFormat::Rgba8Unorm), image(share_bytes, TextureFormat::R8Unorm), UVec2::new(w as u32, h as u32)))
}

/// The look of a grid: contour spacing as a share of the map (from the
/// entity's `dash`, in value units) and glow.
pub fn params(e: &DrawEntity, size: UVec2) -> HeatParams {
    let map = map_of(e);
    let span = if colormap::is_palette(map) {
        100.0
    } else {
        let (lo, hi) = gearbox_api::wire::draw_geom::value_range(decode(e).into_iter().flatten(), e.vmin, e.vmax);
        (hi - lo).abs().max(1e-9)
    };
    let contour = if e.dash > 0.0 { e.dash / span } else { 0.0 };
    let glow = if e.glow > 0.0 { e.glow } else if e.glow < 0.0 { 0.0 } else { 1.0 };
    HeatParams { grid: Vec4::new(size.x as f32, size.y as f32, contour, glow), look: Vec4::new(FILL_ALPHA, HOT, HOT_GLOW, 0.0) }
}

/// What the heat mesh depends on, to keep it when only cells change.
pub fn geometry_key(e: &DrawEntity, draped: bool) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let (cols, rows, cx, cy) = e.grid_dims();
    (cols, rows, cx.to_bits(), cy.to_bits(), draped).hash(&mut h);
    e.values.iter().map(|v| v.to_bits()).collect::<Vec<_>>().hash(&mut h);
    e.props().get("z_offset").hash(&mut h);
    if draped {
        [e.px, e.py, e.pz, e.qw, e.qx, e.qy, e.qz].map(f64::to_bits).hash(&mut h);
    }
    h.finish()
}

/// The surface a grid lies on: a lattice no coarser than half a metre when
/// draped (or a cell when it has heights), lifted by the cells' heights.
pub fn mesh(e: &DrawEntity, ground: Ground) -> Option<Mesh> {
    let d = dims(e)?;
    let (width, height) = (d.cols as f32 * d.cell_x, d.rows as f32 * d.cell_y);
    let heights = e.values.len() == d.cols * d.rows;
    let lift = LIFT + e.prop_f32("z_offset").unwrap_or(0.0);
    let step = |size: f32, cell: f32| -> usize {
        let wanted = match (heights, ground.is_some()) {
            (true, _) => cell,
            (false, true) => 0.5,
            (false, false) => size,
        };
        ((size / wanted.max(1e-3)).ceil() as usize).clamp(1, MAX_STEPS)
    };
    let (nx, ny) = (step(width, d.cell_x), step(height, d.cell_y));
    let elevation = |x: f32, y: f32| -> f32 {
        if !heights {
            return 0.0;
        }
        let gx = (x / d.cell_x - 0.5).clamp(0.0, (d.cols - 1) as f32);
        let gy = (y / d.cell_y - 0.5).clamp(0.0, (d.rows - 1) as f32);
        let (x0, y0) = (gx.floor() as usize, gy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(d.cols - 1), (y0 + 1).min(d.rows - 1));
        let (fx, fy) = (gx - x0 as f32, gy - y0 as f32);
        let v = |cx: usize, cy: usize| e.values[cy * d.cols + cx];
        let a = v(x0, y0) + (v(x1, y0) - v(x0, y0)) * fx;
        let b = v(x0, y1) + (v(x1, y1) - v(x0, y1)) * fx;
        a + (b - a) * fy
    };
    let mut positions = Vec::with_capacity((nx + 1) * (ny + 1));
    let mut uvs = Vec::with_capacity((nx + 1) * (ny + 1));
    for j in 0..=ny {
        for i in 0..=nx {
            let (u, v) = (i as f32 / nx as f32, j as f32 / ny as f32);
            let (x, y) = (u * width, v * height);
            let z = lift + elevation(x, y) + ground.map_or(0.0, |g| g(x, y));
            positions.push([x, y, z]);
            uvs.push([u, v]);
        }
    }
    let row = nx as u32 + 1;
    let mut indices = Vec::with_capacity(nx * ny * 6);
    for j in 0..ny as u32 {
        for i in 0..nx as u32 {
            let (a, b) = (j * row + i, (j + 1) * row + i);
            indices.extend([a, a + 1, b, a + 1, b + 1, b]);
        }
    }
    Some(
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
            .with_inserted_indices(Indices::U32(indices)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gearbox_api::GridCells;

    #[test]
    fn cells_colour_through_the_map_and_unknown_ones_stay_clear() {
        let e = DrawEntity::grid("t", "g", 3, 2, 0.5, GridCells::U8(vec![0, 50, 100, 255, 99, 1]))
            .with_colormap(colormap::COSTMAP, 0.0, 0.0)
            .with_prop("unknown", "255");
        let (rgba, shares, size) = images(&e).expect("a grid");
        assert_eq!(size, UVec2::new(3, 2));
        let px = |i: usize| &rgba.data.as_ref().unwrap()[4 * i..4 * i + 4];
        assert_eq!(px(0)[3], 0, "free space is clear in the costmap palette");
        assert_eq!(px(3), [0, 0, 0, 0], "the unknown value draws nothing");
        assert_eq!(px(2), [255, 0, 255, 255], "lethal is purple");
        assert_eq!(shares.data.as_ref().unwrap()[2], 255);
    }

    #[test]
    fn the_mesh_spans_the_grid_and_takes_the_heights() {
        let flat = DrawEntity::grid("t", "g", 4, 2, 0.5, GridCells::F32(vec![0.0; 8]));
        let m = mesh(&flat, None).unwrap();
        assert_eq!(m.count_vertices(), 4, "an undraped flat grid is one quad");
        let hilly = flat.clone().with_values(&[0.0, 1.0, 2.0, 3.0, 0.0, 1.0, 2.0, 3.0]);
        let m = mesh(&hilly, None).unwrap();
        assert_eq!(m.count_vertices(), 5 * 3, "a vertex per cell corner");
        let ground = |_x: f32, _y: f32| 2.0;
        let draped = mesh(&DrawEntity::grid("t", "g", 10, 10, 1.0, GridCells::U8(vec![0; 100])), Some(&ground)).unwrap();
        assert_eq!(draped.count_vertices(), 21 * 21, "half a metre apart on the ground");
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) = draped.attribute(Mesh::ATTRIBUTE_POSITION) else {
            panic!("positions");
        };
        assert!((p[0][2] - (2.0 + LIFT)).abs() < 1e-6);
    }
}
