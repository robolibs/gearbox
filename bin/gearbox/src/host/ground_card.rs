//! Terrain-mounted card mesh and font-atlas rasterization.

use crate::viewer::machine_context::MachineCard;
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};

pub(super) struct GroundCard {
    entity: Entity,
    mesh: Handle<Mesh>,
    image: Handle<Image>,
    material: Handle<StandardMaterial>,
    text: String,
    aspect: f32,
}

impl GroundCard {
    pub fn new(world: &mut World) -> Self {
        let mesh = world.resource_mut::<Assets<Mesh>>().add(Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        ));
        let image = world.resource_mut::<Assets<Image>>().add(Image::default());
        let material = world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                base_color_texture: Some(image.clone()),
                unlit: true,
                alpha_mode: AlphaMode::Blend,
                cull_mode: None,
                double_sided: true,
                ..default()
            });
        let entity = world
            .spawn((
                Name::new("Machine ground card"),
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::default(),
                Visibility::Hidden,
                bevy::light::NotShadowCaster,
                bevy::light::NotShadowReceiver,
                bevy::camera::visibility::NoFrustumCulling,
            ))
            .id();
        Self {
            entity,
            mesh,
            image,
            material,
            text: String::new(),
            aspect: 1.0,
        }
    }

    pub fn hide(&self, world: &mut World) {
        if let Some(mut visibility) = world.get_mut::<Visibility>(self.entity) {
            *visibility = Visibility::Hidden;
        }
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        world: &mut World,
        card: &MachineCard,
        text: &str,
        selected: bool,
    ) -> [Vec3; 4] {
        if self.text != text {
            let (image, aspect) = label_image(ctx, text);
            if let Some(mut asset) = world.resource_mut::<Assets<Image>>().get_mut(&self.image) {
                *asset = image;
            }
            self.text = text.to_owned();
            self.aspect = aspect;
        }
        let height = 0.38;
        let width = height * self.aspect;
        let point = |u: f32, v: f32| {
            let p = card.ground_center
                + card.ground_right * ((u - 0.5) * width)
                + card.ground_down * ((v - 0.5) * height);
            let ground = world
                .get_resource::<crate::terrain::ProceduralTerrain>()
                .and_then(|t| t.surface_height_m(p.x, p.z))
                .unwrap_or_else(|| crate::world::terrain_height_m(p.x, p.z));
            Vec3::new(p.x, ground + 0.10, p.z)
        };
        let corners = [
            point(0.0, 0.0),
            point(1.0, 0.0),
            point(1.0, 1.0),
            point(0.0, 1.0),
        ];
        let columns = (width / 0.1).ceil().max(1.0) as usize;
        let rows = 4;
        let mut positions = Vec::new();
        let mut uvs = Vec::new();
        let mut indices = Vec::new();
        for row in 0..=rows {
            for column in 0..=columns {
                let u = column as f32 / columns as f32;
                let v = row as f32 / rows as f32;
                positions.push(point(u, v).to_array());
                uvs.push([u, v]);
                if row < rows && column < columns {
                    let a = (row * (columns + 1) + column) as u32;
                    let b = a + (columns + 1) as u32;
                    indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
                }
            }
        }
        let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
        let mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(indices));
        if let Some(mut asset) = world.resource_mut::<Assets<Mesh>>().get_mut(&self.mesh) {
            *asset = mesh;
        }
        if let Some(mut material) = world
            .resource_mut::<Assets<StandardMaterial>>()
            .get_mut(&self.material)
        {
            material.base_color = crate::viewer::machine_context::ring_color(selected).into();
        }
        if let Some(mut visibility) = world.get_mut::<Visibility>(self.entity) {
            *visibility = Visibility::Visible;
        }
        corners
    }
}

fn label_image(ctx: &egui::Context, text: &str) -> (Image, f32) {
    let galley = ctx.fonts_mut(|f| {
        f.layout_no_wrap(
            text.to_owned(),
            egui::FontId::proportional(48.0),
            egui::Color32::WHITE,
        )
    });
    let atlas = ctx.fonts(|f| f.image());
    let padding = 20.0;
    let height = (galley.size().y + padding).ceil() as usize;
    let width = ((galley.size().x + padding * 2.0).ceil() as usize).max(height);
    let mut data = vec![255u8; width * height * 4];
    for y in 0..height {
        for x in 0..width {
            let r = height as f32 * 0.5;
            let cx = (x as f32).clamp(r, width as f32 - r);
            let d = ((x as f32 - cx).powi(2) + (y as f32 - r).powi(2)).sqrt();
            data[(y * width + x) * 4 + 3] = ((r - d).clamp(0.0, 1.0) * 255.0) as u8;
        }
    }
    for row in &galley.rows {
        let mesh = &row.visuals.mesh;
        for quad in mesh.vertices[row.visuals.glyph_vertex_range.clone()].chunks_exact(4) {
            let bounds = egui::Rect::from_points(&quad.iter().map(|v| v.pos).collect::<Vec<_>>());
            let uv = egui::Rect::from_points(&quad.iter().map(|v| v.uv).collect::<Vec<_>>());
            let offset = row.pos.to_vec2() + egui::vec2(padding, padding * 0.5);
            for y in (bounds.top() + offset.y).floor().max(0.0) as usize
                ..((bounds.bottom() + offset.y).ceil() as usize).min(height)
            {
                for x in (bounds.left() + offset.x).floor().max(0.0) as usize
                    ..((bounds.right() + offset.x).ceil() as usize).min(width)
                {
                    let u = ((x as f32 + 0.5 - offset.x - bounds.left()) / bounds.width())
                        .clamp(0.0, 1.0);
                    let v = ((y as f32 + 0.5 - offset.y - bounds.top()) / bounds.height())
                        .clamp(0.0, 1.0);
                    let ax = (uv.left() + u * uv.width()) as usize;
                    let ay = (uv.top() + v * uv.height()) as usize;
                    if ax >= atlas.size[0] || ay >= atlas.size[1] {
                        continue;
                    }
                    let alpha = atlas.pixels[ay * atlas.size[0] + ax].a() as f32 / 255.0;
                    let ink = (255.0 * (1.0 - alpha * 0.94)) as u8;
                    let index = (y * width + x) * 4;
                    for channel in &mut data[index..index + 3] {
                        *channel = (*channel).min(ink);
                    }
                }
            }
        }
    }
    (
        Image::new(
            Extent3d {
                width: width as u32,
                height: height as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        ),
        width as f32 / height as f32,
    )
}
