//! What drifts in the air.
//!
//! One field of motes is drawn for each kind the world has anywhere: a box of
//! them follows the view, and each mote asks the biome under it whether it
//! belongs there at all. So chaff hangs over stubble and stops at the field's
//! edge, pollen thins away with the meadow, and none of it is placed by hand.
//! Nothing is remembered between frames: a mote's whole life is its index.

use bevy::asset::embedded_asset;
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::pbr::{Material, MaterialPipeline, MaterialPlugin, MeshMaterial3d};
use bevy::shader::ShaderRef;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};

use crate::biome::Border;
use crate::blend::Biomes;
use crate::layer::{Covers, MoteKind};

/// The wind the motes ride. The host sets it; it is the same wind the plants get.
#[derive(Resource, Clone, Debug)]
pub struct AmbientWind {
    pub heading_deg: f32,
    pub speed_mps: f32,
    pub gustiness: f32,
    /// A tiling gust map, 256 m to the tile: red broad gusts, green their
    /// detail, blue the turn of the wind. Without one the air is only stirred.
    pub gust_map: Option<Handle<Image>>,
}

impl Default for AmbientWind {
    fn default() -> Self {
        Self { heading_deg: 36.87, speed_mps: 4.0, gustiness: 0.7, gust_map: None }
    }
}

/// How much air is drawn at once, and how high the ground under the view is.
#[derive(Resource, Clone, Copy, Debug)]
pub struct MoteBudget {
    /// The side of the box of air that follows the view.
    pub extent_m: f32,
    /// Most motes of one kind drawn at once.
    pub most: u32,
    /// Everything thinned by this: 0 is still air, 1 the full recipe.
    pub density: f32,
    /// Height of the ground under the view.
    pub ground_m: f32,
}

impl Default for MoteBudget {
    fn default() -> Self {
        Self { extent_m: 48.0, most: 3000, density: 1.0, ground_m: 0.0 }
    }
}

/// Most regions the motes can be told about.
const REGIONS: usize = 16;

#[derive(Clone, Copy, Debug, Default, ShaderType, Reflect)]
pub struct MoteField {
    pub colour: Vec4,
    /// Downwind x, downwind z, speed, gustiness.
    pub wind: Vec4,
    /// Motes a hectare where no region covers the ground.
    pub background: Vec4,
    pub size_m: Vec2,
    pub rise_mps: f32,
    pub drag: f32,
    pub ceiling_m: f32,
    pub reach_m: f32,
    pub extent_m: f32,
    pub ground_m: f32,
    pub count: u32,
    pub kind: u32,
    pub region_count: u32,
    pub most_per_hectare: f32,
    pub seed: f32,
    pub pad: f32,
    /// Each region's bounds: min x, min z, max x, max z.
    pub regions: [Vec4; REGIONS],
    /// Motes a hectare in that region, the metres its border takes, and
    /// whether that border is hard.
    pub region_air: [Vec4; REGIONS],
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct MoteMaterial {
    #[uniform(0)]
    pub field: MoteField,
    /// The gust map the plants lean in, so the air moves with them.
    #[texture(1)]
    #[sampler(2)]
    pub gust_map: Option<Handle<Image>>,
}

impl Material for MoteMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://bevy_biomes/shaders/motes.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://bevy_biomes/shaders/motes.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.vertex.buffers =
            vec![layout.0.get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0)])?];
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// The field of one kind of mote.
#[derive(Component)]
struct MoteFieldOf(MoteKind);

pub struct MotesPlugin;

impl Plugin for MotesPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/motes.wgsl");
        app.add_plugins(MaterialPlugin::<MoteMaterial>::default())
            .add_systems(Update, (spawn_mote_fields, carry_mote_fields).chain());
    }
}

/// A quad for each mote: the corner in x and y, the mote's own number in z.
fn mote_mesh(most: u32) -> Mesh {
    let corners = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
    let mut positions = Vec::with_capacity(most as usize * 4);
    let mut indices = Vec::with_capacity(most as usize * 6);
    for mote in 0..most {
        for corner in corners {
            positions.push([corner[0], corner[1], mote as f32]);
        }
        let base = mote * 4;
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_indices(Indices::U32(indices))
}

/// One field for every kind of mote the world has anywhere.
fn spawn_mote_fields(
    mut commands: Commands,
    biomes: Option<Res<Biomes>>,
    covers: Res<Covers>,
    budget: Res<MoteBudget>,
    wind: Res<AmbientWind>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<MoteMaterial>>,
    fields: Query<(Entity, &MoteFieldOf)>,
    mut mesh_of: Local<Option<(u32, Handle<Mesh>)>>,
) {
    let Some(biomes) = biomes else {
        return;
    };
    if !biomes.is_changed()
        && !covers.is_changed()
        && !budget.is_changed()
        && !wind.is_changed()
        && !fields.is_empty()
    {
        return;
    }
    let mut wanted: Vec<MoteKind> = Vec::new();
    for biome in std::iter::once(biomes.background).chain(biomes.regions.iter().map(|r| r.biome)) {
        let Some(cover) = covers.get(biome) else {
            continue;
        };
        for air in &cover.air {
            if !wanted.contains(&air.kind) {
                wanted.push(air.kind);
            }
        }
    }
    for (entity, held) in &fields {
        if !wanted.contains(&held.0) {
            commands.entity(entity).despawn();
        }
    }
    let mesh = match mesh_of.as_ref() {
        Some((most, mesh)) if *most == budget.most => mesh.clone(),
        _ => {
            let mesh = meshes.add(mote_mesh(budget.most));
            *mesh_of = Some((budget.most, mesh.clone()));
            mesh
        }
    };
    for kind in wanted {
        if fields.iter().any(|(_, held)| held.0 == kind) {
            continue;
        }
        commands.spawn((
            Name::new(format!("Motes {kind:?}")),
            MoteFieldOf(kind),
            Mesh3d(mesh.clone()),
            MeshMaterial3d(materials.add(MoteMaterial {
                field: MoteField::default(),
                gust_map: wind.gust_map.clone(),
            })),
            Transform::IDENTITY,
            NoFrustumCulling,
            NotShadowCaster,
            NotShadowReceiver,
        ));
    }
}

/// The box of air follows the view, and its recipe is refreshed from the
/// biomes under it.
fn carry_mote_fields(
    biomes: Option<Res<Biomes>>,
    covers: Res<Covers>,
    budget: Res<MoteBudget>,
    wind: Res<AmbientWind>,
    mut materials: ResMut<Assets<MoteMaterial>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    mut fields: Query<(&MoteFieldOf, &MeshMaterial3d<MoteMaterial>, &mut Transform)>,
) {
    let (Some(biomes), Ok(camera)) = (biomes, cameras.single()) else {
        return;
    };
    let heading = wind.heading_deg.to_radians();
    let downwind = Vec2::new(heading.sin(), heading.cos());
    for (kind, handle, mut transform) in &mut fields {
        transform.translation = camera.translation();
        let Some(mut material) = materials.get_mut(&handle.0) else {
            continue;
        };
        if material.gust_map != wind.gust_map {
            material.gust_map = wind.gust_map.clone();
        }
        let air = |biome| {
            covers
                .get(biome)
                .and_then(|cover| cover.air.iter().find(|air| air.kind == kind.0).copied())
        };
        let Some(recipe) = air(biomes.background).or_else(|| {
            biomes.regions.iter().find_map(|region| air(region.biome))
        }) else {
            material.field.count = 0;
            continue;
        };
        let mut field = MoteField {
            colour: LinearRgba::from(recipe.colour).to_vec4(),
            wind: Vec4::new(downwind.x, downwind.y, wind.speed_mps, wind.gustiness),
            background: Vec4::new(
                air(biomes.background).map_or(0.0, |air| air.per_hectare) * budget.density,
                0.0,
                0.0,
                0.0,
            ),
            size_m: Vec2::new(recipe.size_mm[0], recipe.size_mm[1]) * 0.001,
            rise_mps: recipe.rise_mps,
            drag: recipe.drag,
            ceiling_m: recipe.ceiling_m.max(0.1),
            reach_m: recipe.reach_m.max(1.0),
            extent_m: budget.extent_m.max(4.0),
            ground_m: budget.ground_m,
            kind: kind.0 as u32,
            seed: kind.0 as u32 as f32 * 17.13,
            ..default()
        };
        let mut most_per_hectare = field.background.x;
        for (slot, region) in biomes.regions.iter().take(REGIONS).enumerate() {
            let here = air(region.biome).map_or(0.0, |air| air.per_hectare) * budget.density;
            let (metres, hard) = match region.border {
                Border::Hard => (0.0, 1.0),
                Border::Soft { metres } => (metres, 0.0),
            };
            field.regions[slot] =
                Vec4::new(region.bounds.min.x, region.bounds.min.y, region.bounds.max.x, region.bounds.max.y);
            field.region_air[slot] = Vec4::new(here, metres, hard, 0.0);
            field.region_count = slot as u32 + 1;
            most_per_hectare = most_per_hectare.max(here);
        }
        // A hectare is ten thousand square metres; the box holds its share.
        let area_ha = (field.extent_m * field.extent_m) / 10_000.0;
        field.most_per_hectare = most_per_hectare.max(1.0e-4);
        field.count = ((most_per_hectare * area_ha).ceil() as u32).min(budget.most);
        material.field = field;
    }
}
