//! Grids drawn as heat maps: the cells coloured once into a texture, laid
//! on a mesh that follows the terrain (and the cells' own heights), drawn
//! with soft cell edges, contours and a static registration frame.
//! Only the texture is rebuilt when just the cells change.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, CompareFunction, Extent3d, RenderPipelineDescriptor, ShaderType,
    SpecializedMeshPipelineError, TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use gearbox_api::wire::draw::unpack_rgba;
use gearbox_api::wire::draw_geom::colormap_rgba;
use gearbox_api::{DrawEntity, colormap, draw_style, grid_encoding};

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
const HOT_GLOW: f32 = 0.08;

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct HeatParams {
    pub frost: Vec4,
    /// Texture columns and rows, contour spacing in normalised value (0
    /// none), glow.
    pub grid: Vec4,
    /// Fill opacity, hot threshold, hot glow, categorical palette flag.
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
    frost: bool,
}

impl From<&HeatMaterial> for HeatKey {
    fn from(m: &HeatMaterial) -> Self {
        Self {
            on_top: m.on_top,
            frost: m.params.frost.x > 0.0,
        }
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

    fn reads_view_transmission_texture(&self) -> bool {
        self.params.frost.x > 0.0
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
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(1),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        if key.bind_group_data.frost {
            super::material::frost_pipeline(descriptor);
        }
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
    Some(Dims {
        cols,
        rows,
        cell_x,
        cell_y,
        stride,
    })
}

/// The cells of a grid as numbers, `None` where unknown.
fn decode(e: &DrawEntity) -> Vec<Option<f32>> {
    let unknown = e.prop_f32("unknown");
    let n = e.cell_count();
    let known = |v: f32| (v.is_finite() && unknown != Some(v)).then_some(v);
    match e.sub {
        grid_encoding::U16 => (0..n)
            .map(|i| known(u16::from_le_bytes([e.cells[2 * i], e.cells[2 * i + 1]]) as f32))
            .collect(),
        grid_encoding::F32 => (0..n)
            .map(|i| {
                known(f32::from_le_bytes([
                    e.cells[4 * i],
                    e.cells[4 * i + 1],
                    e.cells[4 * i + 2],
                    e.cells[4 * i + 3],
                ]))
            })
            .collect(),
        _ => e.cells.iter().map(|b| known(*b as f32)).collect(),
    }
}

/// The colormap a grid is drawn in, viridis when none is named.
fn map_of(e: &DrawEntity) -> u32 {
    if e.colormap == colormap::NONE {
        colormap::VIRIDIS
    } else {
        e.colormap
    }
}

/// A cell value as a share of the map: the ramp's range, or the palette's.
fn share(map: u32, v: f32, lo: f32, hi: f32) -> f32 {
    match map {
        colormap::COSTMAP | colormap::MAP => (v / 100.0).clamp(0.0, 1.0),
        colormap::RAW => (v / 255.0).clamp(0.0, 1.0),
        colormap::OKABE => 0.0,
        _ => {
            let span = hi - lo;
            if span.abs() > 1e-12 {
                ((v - lo) / span).clamp(0.0, 1.0)
            } else {
                0.5
            }
        }
    }
}

/// Premultiplied sRGB colours and normalised values with known-cell coverage.
pub fn images(e: &DrawEntity) -> Option<(Image, Image, UVec2)> {
    let d = dims(e)?;
    let cells = decode(e);
    let map = map_of(e);
    let (lo, hi) =
        gearbox_api::wire::draw_geom::value_range(cells.iter().flatten().copied(), e.vmin, e.vmax);
    let tint = if e.color != 0 {
        unpack_rgba(e.color)[3] as u32
    } else {
        255
    };
    let lut: Option<Vec<([u8; 4], u8)>> = (e.sub == grid_encoding::U8).then(|| {
        (0..=255u32)
            .map(|b| {
                let v = b as f32;
                (
                    colormap_rgba(map, v, lo, hi),
                    (share(map, v, lo, hi) * 255.0).round() as u8,
                )
            })
            .collect()
    });
    let (w, h) = (d.cols.div_ceil(d.stride), d.rows.div_ceil(d.stride));
    let mut rgba = vec![0u8; w * h * 4];
    let mut share_bytes = vec![0u8; w * h * 2];
    for y in 0..h {
        for x in 0..w {
            let i = (y * d.stride) * d.cols + x * d.stride;
            let Some(Some(v)) = cells.get(i) else {
                continue;
            };
            let (c, t) = match &lut {
                Some(lut) => lut[(*v as usize).min(255)],
                None => (
                    colormap_rgba(map, *v, lo, hi),
                    (share(map, *v, lo, hi) * 255.0).round() as u8,
                ),
            };
            let a = c[3] as u32 * tint / 255;
            let k = (y * w + x) * 4;
            rgba[k] = (c[0] as u32 * a / 255) as u8;
            rgba[k + 1] = (c[1] as u32 * a / 255) as u8;
            rgba[k + 2] = (c[2] as u32 * a / 255) as u8;
            rgba[k + 3] = a as u8;
            share_bytes[2 * (y * w + x)] = t;
            share_bytes[2 * (y * w + x) + 1] = 255;
        }
    }
    let image = |data: Vec<u8>, format: TextureFormat| {
        let mut img = Image::new(
            Extent3d {
                width: w as u32,
                height: h as u32,
                depth_or_array_layers: 1,
            },
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
    Some((
        image(rgba, TextureFormat::Rgba8Unorm),
        image(share_bytes, TextureFormat::Rg8Unorm),
        UVec2::new(w as u32, h as u32),
    ))
}

/// The look of a grid: contour spacing as a share of the map (from the
/// entity's `dash`, in value units) and glow.
pub fn params(e: &DrawEntity, size: UVec2) -> HeatParams {
    let map = map_of(e);
    let span = if colormap::is_palette(map) {
        100.0
    } else {
        let (lo, hi) = gearbox_api::wire::draw_geom::value_range(
            decode(e).into_iter().flatten(),
            e.vmin,
            e.vmax,
        );
        (hi - lo).abs().max(1e-9)
    };
    let contour = if e.dash > 0.0 && !colormap::is_palette(map) {
        e.dash / span
    } else {
        0.0
    };
    let glow = super::visual::effect(e.glow, super::visual::defaults(draw_style::HEAT).glow);
    HeatParams {
        frost: Vec4::new(super::visual::blur(e, 0.0), 0.0, 0.0, 0.0),
        grid: Vec4::new(size.x as f32, size.y as f32, contour, glow),
        look: Vec4::new(
            FILL_ALPHA,
            HOT,
            HOT_GLOW,
            if colormap::is_palette(map) { 1.0 } else { 0.0 },
        ),
    }
}

/// What the heat mesh depends on, to keep it when only cells change.
pub fn geometry_key(e: &DrawEntity, ground_transform: Option<Transform>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let (cols, rows, cx, cy) = e.grid_dims();
    (
        cols,
        rows,
        cx.to_bits(),
        cy.to_bits(),
        ground_transform.is_some(),
    )
        .hash(&mut h);
    e.values
        .iter()
        .map(|v| v.to_bits())
        .collect::<Vec<_>>()
        .hash(&mut h);
    e.props().get("z_offset").hash(&mut h);
    if let Some(transform) = ground_transform {
        (e.props().get("drape_anchor").as_deref() == Some("true")).hash(&mut h);
        let pose_height = if e.props().get("drape_pose_height").as_deref() == Some("true") {
            e.pz as f32
        } else {
            0.0
        };
        pose_height.to_bits().hash(&mut h);
        transform
            .translation
            .to_array()
            .map(f32::to_bits)
            .hash(&mut h);
        transform.rotation.to_array().map(f32::to_bits).hash(&mut h);
        transform.scale.to_array().map(f32::to_bits).hash(&mut h);
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
            (true, true) => cell.min(super::drape::STEP_M),
            (true, false) => cell,
            (false, true) => super::drape::STEP_M,
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
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
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
    fn terrain_geometry_cache_tracks_resolved_frame_not_cell_colors() {
        let e = DrawEntity::grid("t", "grid", 2, 2, 0.5, GridCells::U8(vec![1; 4]));
        let origin = Transform::IDENTITY;
        let key = geometry_key(&e, Some(origin));
        for transform in [
            Transform::from_xyz(10.0, 0.0, 0.0),
            Transform::from_rotation(Quat::from_rotation_y(0.2)),
            Transform::from_scale(Vec3::splat(2.0)),
        ] {
            assert_ne!(geometry_key(&e, Some(transform)), key);
        }
        let mut recolored =
            e.clone()
                .with_color(0xff00ffff)
                .with_colormap(colormap::TURBO, 0.0, 5.0);
        recolored.cells = vec![3; 4];
        assert_eq!(geometry_key(&recolored, Some(origin)), key);
        assert_eq!(
            geometry_key(&e.clone().at_yaw([10.0, 20.0, 0.0], 0.5), None),
            geometry_key(&e, None)
        );
        assert_ne!(
            geometry_key(&e.clone().with_values(&[1.0; 4]), Some(origin)),
            key
        );
        assert_ne!(geometry_key(&e, None), key);
        assert_ne!(
            geometry_key(&e.clone().with_prop("drape_anchor", "true"), Some(origin)),
            key
        );
        assert_eq!(
            geometry_key(&e.clone().with_prop("drape_anchor", "false"), Some(origin)),
            key
        );
        let raised = e
            .clone()
            .at_yaw([0.0, 0.0, 2.0], 0.0)
            .with_prop("drape_pose_height", "true");
        assert_ne!(geometry_key(&raised, Some(origin)), key);
        assert_eq!(geometry_key(&raised, None), geometry_key(&e, None));
    }

    #[test]
    fn cells_colour_through_the_map_and_unknown_ones_stay_clear() {
        let e = DrawEntity::grid(
            "t",
            "g",
            3,
            2,
            0.5,
            GridCells::U8(vec![0, 50, 100, 255, 99, 1]),
        )
        .with_colormap(colormap::COSTMAP, 0.0, 0.0)
        .with_prop("unknown", "255");
        let (rgba, shares, size) = images(&e).expect("a grid");
        assert_eq!(size, UVec2::new(3, 2));
        let px = |i: usize| &rgba.data.as_ref().unwrap()[4 * i..4 * i + 4];
        assert_eq!(px(0)[3], 0, "free space is clear in the costmap palette");
        assert_eq!(px(3), [0, 0, 0, 0], "the unknown value draws nothing");
        assert_eq!(px(2), [255, 0, 255, 255], "lethal is purple");
        let data = shares.data.as_ref().unwrap();
        assert_eq!(
            &data[0..2],
            &[0, 255],
            "transparent free space remains known"
        );
        assert_eq!(&data[4..6], &[255, 255]);
        assert_eq!(
            &data[6..8],
            &[0, 0],
            "unknown cells carry no interpolated value"
        );
        assert_eq!(shares.texture_descriptor.format, TextureFormat::Rg8Unorm);
    }

    #[test]
    fn heat_coverage_distinguishes_missing_and_transparent_cells() {
        let e = DrawEntity::grid(
            "t",
            "g",
            4,
            1,
            1.0,
            GridCells::F32(vec![f32::NAN, f32::INFINITY, -1.0, 0.5]),
        )
        .with_colormap(colormap::VIRIDIS, 0.0, 1.0)
        .with_prop("unknown", "-1");
        let (_, values, _) = images(&e).unwrap();
        assert_eq!(
            values.data.as_deref().unwrap(),
            &[0, 0, 0, 0, 0, 0, 128, 255]
        );
        assert_eq!(params(&e, UVec2::new(4, 1)).look.w, 0.0);
        let palette = e.with_colormap(colormap::COSTMAP, 0.0, 100.0);
        assert_eq!(params(&palette, UVec2::new(4, 1)).look.w, 1.0);
    }

    #[test]
    fn heat_controls_preserve_scalar_mapping_and_do_not_contour_categories() {
        let e = DrawEntity::grid("t", "g", 3, 1, 1.0, GridCells::F32(vec![0.0, 0.5, 1.0]))
            .with_colormap(colormap::VIRIDIS, 0.0, 1.0)
            .with_dash(0.25);
        let quiet = e.clone().with_glow(-1.0).with_pulse(-1.0);
        assert_eq!(images(&e).unwrap().0.data, images(&quiet).unwrap().0.data);
        let p = params(&quiet, UVec2::new(3, 1));
        assert_eq!(p.grid.z, 0.25);
        assert_eq!(p.grid.w, 0.0);
        assert!(p.look.x > 0.0);
        assert_eq!(
            params(
                &e.with_colormap(colormap::OKABE, 0.0, 1.0),
                UVec2::new(3, 1)
            )
            .grid
            .z,
            0.0
        );
        let hidden = images(&quiet.with_color(gearbox_api::wire::draw::rgba(20, 40, 60, 0)))
            .unwrap()
            .0;
        assert!(hidden.data.unwrap().iter().all(|b| *b == 0));
    }

    #[test]
    fn the_mesh_spans_the_grid_and_takes_the_heights() {
        let flat = DrawEntity::grid("t", "g", 4, 2, 0.5, GridCells::F32(vec![0.0; 8]));
        let m = mesh(&flat, None).unwrap();
        assert_eq!(m.count_vertices(), 4, "an undraped flat grid is one quad");
        let hilly = flat
            .clone()
            .with_values(&[0.0, 1.0, 2.0, 3.0, 0.0, 1.0, 2.0, 3.0]);
        let m = mesh(&hilly, None).unwrap();
        assert_eq!(m.count_vertices(), 5 * 3, "a vertex per cell corner");
        let ground = |_x: f32, _y: f32| 2.0;
        let draped = mesh(
            &DrawEntity::grid("t", "g", 10, 10, 1.0, GridCells::U8(vec![0; 100])),
            Some(&ground),
        )
        .unwrap();
        assert_eq!(
            draped.count_vertices(),
            21 * 21,
            "half a metre apart on the ground"
        );
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) =
            draped.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("positions");
        };
        assert!((p[0][2] - (2.0 + LIFT)).abs() < 1e-6);
    }

    #[test]
    fn coarse_height_cells_do_not_coarsen_the_terrain_mesh() {
        let e = DrawEntity::grid("t", "height", 2, 2, 4.0, GridCells::U8(vec![50; 4]))
            .with_values(&[1.0; 4]);
        let hill = |x: f32, y: f32| (x * 0.8).sin() * 0.4 + y * 0.2;
        let m = mesh(&e, Some(&hill)).unwrap();
        assert_eq!(m.count_vertices(), 17 * 17);
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) =
            m.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("positions");
        };
        for p in p {
            assert!((p[2] - hill(p[0], p[1]) - 1.0 - LIFT).abs() < 1e-5);
        }
    }
}
