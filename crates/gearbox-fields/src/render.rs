//! Shared instanced vegetation renderer and per-field wheel-map uploads.

use bevy::core_pipeline::core_3d::{Transparent3d, TransparentSortingInfo3d};
use bevy::ecs::query::QueryItem;
use bevy::ecs::system::{SystemParamItem, lifetimeless::*};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{
    MeshPipeline, MeshPipelineKey, MeshPipelineSystems, SetMeshViewBindGroup,
    SetMeshViewBindingArrayBindGroup, ViewKeyCache,
};
use bevy::prelude::*;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::mesh::allocator::MeshAllocator;
use bevy::render::mesh::{RenderMesh, RenderMeshBufferInfo};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_phase::{
    AddRenderCommand, DrawFunctions, PhaseItem, PhaseItemExtraIndex, RenderCommand,
    RenderCommandResult, SetItemPipeline, TrackedRenderPass, ViewSortedRenderPhases,
};
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::sync_component::SyncComponent;
use bevy::render::sync_world::MainEntity;
use bevy::render::texture::{FallbackImage, GpuImage};
use bevy::render::view::ExtractedView;
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

use super::contacts::{WheelContacts, trample_texel, tread_texel};
use super::profile::WheelMapParams;
use bevy::platform::collections::HashMap;
use std::ops::Range;

/// Per-chunk draw: the chunk corner and how many blade instances this frame.
#[derive(Component, Clone)]
pub struct VegetationChunk {
    pub corner: Vec2,
    /// Chunk side in metres.
    pub size: f32,
    pub instances: u32,
    pub capacity: f32,
    pub field_id: Entity,
    pub shader: Handle<Shader>,
    /// The layer's mesh, shared by all its chunks; chunks are not mesh
    /// instances, so Bevy uploads no per-chunk mesh data every frame.
    pub mesh: Handle<Mesh>,
    pub fade_start: f32,
    pub fade_end: f32,
    pub inverse_square_thinning: bool,
    /// Share of this layer allowed outside the ground's grass patches; nought
    /// scatters it by its own reckoning instead.
    pub follow_grass: f32,
    /// The wear of this field's surface, as `BareGround::tread`.
    pub tread: Vec4,
    /// How far this field's plants carry past its own edge, in metres.
    pub soft_border: f32,
    /// The line the wheels follow through this field.
    pub way: crate::layout::Way,
    /// Albedo of an asset clump layer; procedural layers bind the fallback.
    pub albedo: Option<Handle<Image>>,
    /// Index ranges of a clump mesh's variants, one draw each; empty draws it whole.
    pub variants: Vec<Range<u32>>,
}

impl SyncComponent for VegetationChunk {
    type Target = Self;
}

impl ExtractComponent for VegetationChunk {
    type QueryData = &'static VegetationChunk;
    type QueryFilter = ();
    type Out = Self;

    fn extract_component(item: QueryItem<'_, '_, Self::QueryData>) -> Option<Self> {
        Some(item.clone())
    }
}

/// The terrain heightmap (RGBA32F: height, normal x, normal z), the trample
/// map wheels write into (RG16Uint: press time, roll direction), and the
/// numbers the shader needs to place blades on them.
#[derive(Clone)]
pub struct FieldGpu {
    pub heightmap: Handle<Image>,
    pub trample: Handle<Image>,
    pub params: VegetationParams,
    pub footprint_length: f32,
    /// The wheel map carries tread coordinates in two more channels.
    pub tread: bool,
}

#[derive(Resource, ExtractResource, Clone, Default)]
pub struct RenderFields(pub HashMap<Entity, FieldGpu>, pub RetiredFields);

/// The fields of the ground that was just replaced, kept a moment so the new
/// fields can take over the wheel tracks where they overlap.
#[derive(Clone, Default)]
pub struct RetiredFields {
    pub fields: Vec<FieldGpu>,
    pub frames_left: u32,
}

#[derive(ShaderType, Clone, Copy, Debug, Default)]
pub struct VegetationParams {
    /// World XZ of the chunk corner; filled in per chunk.
    pub corner: Vec2,
    /// World XZ of texel (0, 0).
    pub origin: Vec2,
    /// Texels per metre.
    pub texels_per_metre: f32,
    /// Chunk side in metres; the chunk entity's translation is its corner.
    pub chunk_size: f32,
    /// Full density holds to `fade_start` and reaches zero at `fade_end`.
    pub fade_start: f32,
    pub fade_end: f32,
    /// Blades per chunk at full density.
    pub blades_per_chunk: f32,
    pub texel_count: f32,
    pub inverse_square_thinning: u32,
    pub bounds: Vec4,
    pub wheels: WheelMapParams,
    /// Downwind direction (x, z), speed in m/s and gustiness, for every layer.
    pub wind: Vec4,
    /// Nought to scatter by the layer's own reckoning; otherwise the share of
    /// this layer allowed outside the ground's grass patches.
    pub follow_grass: f32,
    /// How the wheels have worn this field: half the gauge between the ruts,
    /// half the width of one, how bare the rut is and how bare the rest is.
    pub tread: Vec4,
    /// How far this field's plants carry past its own edge, in metres.
    pub soft_border: f32,
    /// The line the wheels follow, as eight points two to a column, with
    /// (how many, half the worn width) beside it. Fewer than two points and the
    /// wear runs down the field's own long axis.
    pub way: Mat4,
    pub way_more: Mat4,
    pub way_shape: Vec4,
}

/// Carries the environment's wind to every field's vegetation uniforms,
/// writing only when it changed so the chunk uniforms are not rewritten.
pub(crate) fn sync_wind(
    settings: Res<crate::host::CoverWind>,
    mut fields: ResMut<RenderFields>,
) {
    let wind = settings.vector();
    if fields.0.values().all(|field| field.params.wind == wind) {
        return;
    }
    for field in fields.0.values_mut() {
        field.params.wind = wind;
    }
}

pub struct VegetationPlugin;

impl Plugin for VegetationPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            ExtractComponentPlugin::<VegetationChunk>::default(),
            ExtractResourcePlugin::<RenderFields>::default(),
        ));
        app.sub_app_mut(RenderApp)
            .add_render_command::<Transparent3d, DrawVegetation>()
            .init_resource::<SpecializedMeshPipelines<VegetationPipeline>>()
            .add_systems(
                RenderStartup,
                init_vegetation_pipeline.after(MeshPipelineSystems),
            )
            .init_resource::<VegetationUniforms>()
            .add_systems(
                Render,
                (
                    queue_vegetation.in_set(RenderSystems::QueueMeshes),
                    prepare_vegetation_uniforms.in_set(RenderSystems::PrepareResources),
                    (carry_wheel_tracks, stamp_wheel_contacts)
                        .chain()
                        .in_set(RenderSystems::PrepareResources),
                    prepare_vegetation_bind_group.in_set(RenderSystems::PrepareBindGroups),
                ),
            );
    }
}

#[allow(clippy::too_many_arguments)]
fn queue_vegetation(
    draw_functions: Res<DrawFunctions<Transparent3d>>,
    vegetation_pipeline: Res<VegetationPipeline>,
    mut pipelines: ResMut<SpecializedMeshPipelines<VegetationPipeline>>,
    pipeline_cache: Res<PipelineCache>,
    meshes: Res<RenderAssets<RenderMesh>>,
    chunks: Query<(Entity, &MainEntity, &VegetationChunk)>,
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
    views: Query<&ExtractedView>,
    view_key_cache: Res<ViewKeyCache>,
    fields: Res<RenderFields>,
    mut warned: Local<bool>,
) {
    if fields.0.is_empty() {
        return;
    }
    let draw_vegetation = draw_functions.read().id::<DrawVegetation>();
    for view in &views {
        let Some(phase) = phases.get_mut(&view.retained_view_entity) else {
            continue;
        };
        let Some(&view_key) = view_key_cache.get(&view.retained_view_entity) else {
            continue;
        };
        for (entity, main_entity, draw) in &chunks {
            if !fields.0.contains_key(&draw.field_id) || draw.instances == 0 {
                continue;
            }
            let Some(mesh) = meshes.get(draw.mesh.id()) else {
                continue;
            };
            let key = view_key
                | MeshPipelineKey::from_primitive_topology_and_strip_index(
                    mesh.primitive_topology(),
                    mesh.index_format(),
                );
            let pipeline = match pipelines.specialize(
                &pipeline_cache,
                &vegetation_pipeline,
                (key, draw.shader.clone()),
                &mesh.layout,
            ) {
                Ok(pipeline) => pipeline,
                Err(error) => {
                    if !*warned {
                        warn!("vegetation pipeline for {:?}: {error}", draw.shader.path());
                        *warned = true;
                    }
                    continue;
                }
            };

            let half = draw.size * 0.5;
            let mesh_center = Vec3::new(draw.corner.x + half, 0.0, draw.corner.y + half);
            phase.add_retained(Transparent3d {
                sorting_info: TransparentSortingInfo3d::Sorted {
                    mesh_center,
                    depth_bias: 0.0,
                },
                entity: (entity, *main_entity),
                pipeline,
                draw_function: draw_vegetation,
                distance: 0.0,
                batch_range: 0..1,
                extra_index: PhaseItemExtraIndex::None,
                indexed: true,
            });
        }
    }
}

/// One persistent uniform slot per chunk. A chunk writes its own slot when
/// it streams in or changes; the whole buffer is rewritten only when it
/// grows or the fields are rebuilt. Rewriting every chunk each frame cost
/// ~25-35 ms in uploads.
#[derive(Resource, Default)]
struct VegetationUniforms {
    buffer: Option<Buffer>,
    capacity: u32,
    next: u32,
    free: Vec<u32>,
    slots: HashMap<Entity, (u32, u64)>,
}

#[derive(Component)]
struct VegetationOffset(u32);

fn chunk_params(draw: &VegetationChunk, field: &FieldGpu) -> VegetationParams {
    let (way, way_more, way_shape) = draw.way.packed();
    VegetationParams {
        way,
        way_more,
        way_shape,
        corner: draw.corner,
        chunk_size: draw.size,
        blades_per_chunk: draw.capacity,
        fade_start: draw.fade_start,
        fade_end: draw.fade_end,
        inverse_square_thinning: u32::from(draw.inverse_square_thinning),
        follow_grass: draw.follow_grass,
        tread: draw.tread,
        soft_border: draw.soft_border,
        ..field.params
    }
}

fn chunk_digest(draw: &VegetationChunk) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut digest = std::hash::DefaultHasher::new();
    (draw.field_id, draw.inverse_square_thinning).hash(&mut digest);
    for value in [draw.corner.x, draw.corner.y, draw.size, draw.capacity, draw.fade_start, draw.fade_end] {
        value.to_bits().hash(&mut digest);
    }
    digest.finish()
}

fn encode_params(params: &VegetationParams) -> Vec<u8> {
    let mut bytes = encase::UniformBuffer::new(Vec::<u8>::new());
    bytes.write(params).expect("encode vegetation params");
    bytes.into_inner()
}

fn prepare_vegetation_uniforms(
    mut commands: Commands,
    fields: Res<RenderFields>,
    chunks: Query<(Entity, &VegetationChunk)>,
    uniforms: ResMut<VegetationUniforms>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
) {
    let stride = VegetationParams::min_size()
        .get()
        .next_multiple_of(render_device.limits().min_uniform_buffer_offset_alignment as u64);
    let VegetationUniforms { buffer, capacity, next, free, slots } = uniforms.into_inner();
    slots.retain(|entity, (slot, _)| {
        let live = chunks.contains(*entity);
        if !live {
            free.push(*slot);
        }
        live
    });
    let mut changed = Vec::new();
    for (entity, draw) in &chunks {
        let Some(field) = fields.0.get(&draw.field_id) else {
            continue;
        };
        let digest = chunk_digest(draw);
        let slot = match slots.get(&entity) {
            Some(&(_, known)) if known == digest => continue,
            Some(&(slot, _)) => slot,
            None => free.pop().unwrap_or_else(|| {
                *next += 1;
                *next - 1
            }),
        };
        slots.insert(entity, (slot, digest));
        commands.entity(entity).insert(VegetationOffset((slot as u64 * stride) as u32));
        changed.push((slot, chunk_params(draw, field)));
    }

    if buffer.is_none() || *next > *capacity || fields.is_changed() {
        *capacity = (*next).max(*capacity).max(256).next_power_of_two();
        let target = buffer.get_or_insert_with(|| {
            render_device.create_buffer(&BufferDescriptor {
                label: Some("vegetation chunk uniforms"),
                size: *capacity as u64 * stride,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        if target.size() < *capacity as u64 * stride {
            *target = render_device.create_buffer(&BufferDescriptor {
                label: Some("vegetation chunk uniforms"),
                size: *capacity as u64 * stride,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        let mut data = vec![0u8; (*capacity as u64 * stride) as usize];
        for (entity, draw) in &chunks {
            let (Some(field), Some(&(slot, _))) = (fields.0.get(&draw.field_id), slots.get(&entity))
            else {
                continue;
            };
            let bytes = encode_params(&chunk_params(draw, field));
            let at = (slot as u64 * stride) as usize;
            data[at..at + bytes.len()].copy_from_slice(&bytes);
        }
        render_queue.write_buffer(target, 0, &data);
        return;
    }
    let Some(target) = buffer.as_ref() else {
        return;
    };
    for (slot, params) in changed {
        render_queue.write_buffer(target, slot as u64 * stride, &encode_params(&params));
    }
}

/// Field bindings per field and albedo, and the empty group bound where the
/// mesh pipeline expects per-mesh data the vegetation shaders never read.
#[derive(Resource)]
struct FieldBindGroups {
    fields: HashMap<(Entity, Option<AssetId<Image>>), BindGroup>,
    empty: BindGroup,
}

/// Field bindings rebuilt against the current dynamic uniform buffer, one per
/// field and albedo in use.
#[allow(clippy::too_many_arguments)]
fn prepare_vegetation_bind_group(
    mut commands: Commands,
    fields: Res<RenderFields>,
    chunks: Query<&VegetationChunk>,
    images: Res<RenderAssets<GpuImage>>,
    fallback: Res<FallbackImage>,
    pipeline: Res<VegetationPipeline>,
    pipeline_cache: Res<PipelineCache>,
    render_device: Res<RenderDevice>,
    uniforms: Res<VegetationUniforms>,
    mut empty: Local<Option<BindGroup>>,
) {
    let mut groups = HashMap::default();
    if let Some(buffer) = uniforms.buffer.as_ref() {
        let binding = BufferBinding {
            buffer,
            offset: 0,
            size: Some(VegetationParams::min_size()),
        };
        for draw in &chunks {
            let key = (draw.field_id, draw.albedo.as_ref().map(Handle::id));
            if groups.contains_key(&key) {
                continue;
            }
            let Some(field) = fields.0.get(&draw.field_id) else {
                continue;
            };
            let albedo = match key.1 {
                Some(id) => images.get(id),
                None => Some(&fallback.d2),
            };
            let (Some(image), Some(trample), Some(albedo)) =
                (images.get(&field.heightmap), images.get(&field.trample), albedo)
            else {
                continue;
            };
            let group = render_device.create_bind_group(
                "field vegetation",
                &pipeline_cache.get_bind_group_layout(&pipeline.field_layout),
                &BindGroupEntries::sequential((
                    &image.texture_view,
                    &pipeline.sampler,
                    binding.clone(),
                    &trample.texture_view,
                    &albedo.texture_view,
                    &albedo.sampler,
                    &pipeline.wind_map,
                    &pipeline.wind_sampler,
                )),
            );
            groups.insert(key, group);
        }
    }
    let empty = empty
        .get_or_insert_with(|| {
            render_device.create_bind_group(
                "vegetation empty",
                &pipeline_cache.get_bind_group_layout(&pipeline.empty_layout),
                &[],
            )
        })
        .clone();
    commands.insert_resource(FieldBindGroups { fields: groups, empty });
}

/// Writes oriented wheel footprints with a timestamp and roll direction.
fn stamp_wheel_contacts(
    fields: Res<RenderFields>,
    contacts: Option<Res<WheelContacts>>,
    images: Res<RenderAssets<GpuImage>>,
    render_queue: Res<RenderQueue>,
    mut stamped: Local<bevy::platform::collections::HashSet<Entity>>,
    // What tread each texel already carries. A tractor's front and rear tyres
    // are different tyres running on lines of their own, and through a turn
    // they stop following one another, so each side's band becomes two passes
    // contesting the same texels — and the print wavers along the seam where
    // one gives way to the other. Only the widest tyre lays a tread; the others
    // write back whatever is already there, so a wheel crossing an older mark
    // leaves it exactly as it found it. Writing *nothing* there is what rubbed
    // marks out before, and is why this is kept rather than skipped.
    mut laid: Local<bevy::platform::collections::HashMap<(Entity, i32, i32), [u16; 2]>>,
) {
    stamped.retain(|entity| fields.0.contains_key(entity));
    laid.retain(|(entity, _, _), _| fields.0.contains_key(entity));
    let Some(contacts) = contacts else {
        return;
    };
    if contacts.contacts.is_empty() {
        return;
    }
    let widest = contacts.contacts.iter().fold(0.0f32, |most, c| most.max(c.width));
    for (&entity, field) in &fields.0 {
        let Some(trample) = images.get(&field.trample) else {
            continue;
        };
        let tpm = field.params.wheels.texels_per_metre;
        let width = field.params.wheels.width as i32;
        let height = field.params.wheels.height as i32;
        for contact in &contacts.contacts {
            // The footprint is a rectangle in the wheel's own frame: tyre width
            // across the axle, a short contact patch along the roll. Texel
            // centres sit on integer coordinates; a texel is stamped only when
            // its centre lies inside that rectangle, row by row.
            let centre = Vec2::new(
                (contact.position.x - field.params.wheels.origin.x) * tpm,
                (contact.position.z - field.params.wheels.origin.y) * tpm,
            );
            // The tread is measured across from the wheel's steadied line, not
            // from where the contact happens to be sitting this frame.
            let line = (contact.centreline - field.params.wheels.origin) * tpm;
            let roll = contact.direction.normalize_or(Vec2::X);
            let axle = roll.perp();
            let half_width = (contact.width * 0.5 * tpm).max(0.5);
            // Stamped a texel wider than the tyre on each side, and a texel
            // longer. Only whole texels can be written, so the stamped edge
            // lands wherever the grid happens to fall relative to the wheel —
            // and as a machine drifts sideways against that grid the band gains
            // a texel down one side, then metres later down the other. An
            // eighth of a metre, appearing and disappearing every few metres:
            // it is what makes a pressed track look like it keeps changing
            // width, in the grass as much as in the tread. Stamping past the
            // tyre lets the shader cut the true edge from the smooth offset
            // across the tyre instead, which no grid can step.
            let laid_width = half_width + 1.0;
            let footprint = contact.footprint_length(field.footprint_length);
            let half_length = (footprint * 0.5 * tpm).max(0.5) + 1.0;
            let reach = laid_width.hypot(half_length);
            let z0 = ((centre.y - reach).ceil() as i32).clamp(0, height - 1);
            let z1 = ((centre.y + reach).floor() as i32).clamp(0, height - 1);
            let x_lo = ((centre.x - reach).ceil() as i32).clamp(0, width - 1);
            let x_hi = ((centre.x + reach).floor() as i32).clamp(0, width - 1);
            for z in z0..=z1 {
                let inside = |x: i32| {
                    let d = Vec2::new(x as f32, z as f32) - centre;
                    d.dot(axle).abs() <= laid_width && d.dot(roll).abs() <= half_length
                };
                let Some(x0) = (x_lo..=x_hi).find(|x| inside(*x)) else {
                    continue;
                };
                let x1 = (x0..=x_hi).take_while(|x| inside(*x)).last().unwrap_or(x0);
                let width = (x1 - x0 + 1) as u32;
                // A tread map also records, per texel, how far the wheel had
                // rolled and where across the tyre the texel lies.
                let data: Vec<u16> = (x0..=x1)
                    .flat_map(|x| {
                        let d = Vec2::new(x as f32, z as f32) - centre;
                        let stamp = trample_texel(contacts.now, roll, d.dot(axle) / half_width, contact.scrub);
                        // Every wheel lays its own tread, and none of them ever
                        // writes the channels empty. A stamp replaces what was
                        // there, so writing nothing *is* rubbing out: a tractor
                        // steering one way and then the other sweeps its front
                        // wheels across the tracks its rear wheels left, and a
                        // narrow wheel told to lay no tread erased them at every
                        // crossing. A wheel passing over an older mark lays its
                        // own over the top, which is what happens on the ground.
                        let lays_tread = contact.width >= widest * 0.9;
                        let tread = field.tread.then(|| {
                            // How far along the track, measured from the wheel
                            // itself: the machine's rolled distance plus this
                            // texel's own offset from the contact, which is
                            // never more than a tyre's width.
                            //
                            // Never the texel's place projected on the heading,
                            // however tempting that looks. A projection has a
                            // lever arm — the distance to whatever it is
                            // measured from — and multiplies every wobble of
                            // the steering by it. Half a degree of correction
                            // a hundred metres out from the origin slides the
                            // whole tread sideways by several lug pitches,
                            // along the ruled line where one stamp gives way to
                            // the next. Rolled distance has no lever arm at all.
                            if !lays_tread {
                                // Whatever this texel already carries, put back
                                // unchanged. Nothing there yet is nothing to
                                // keep, and empty is then the truth rather than
                                // an erasure.
                                return laid.get(&(entity, x, z)).copied().unwrap_or([0u16, 0u16]);
                            }
                            let from_machine = Vec2::new(x as f32, z as f32)
                                - (contact.anchor - field.params.wheels.origin) * tpm;
                            let fresh = tread_texel(
                                contact.travelled + from_machine.dot(roll) / tpm,
                                (Vec2::new(x as f32, z as f32) - line).dot(axle) / tpm,
                                half_width / tpm);
                            laid.insert((entity, x, z), fresh);
                            fresh
                        });
                        stamp.into_iter().chain(tread.into_iter().flatten())
                    })
                    .collect();
                let texel_bytes = if field.tread { 8 } else { 4 };
                let mut target = trample.texture.as_image_copy();
                target.origin = Origin3d {
                    x: x0 as u32,
                    y: z as u32,
                    z: 0,
                };
                render_queue.write_texture(
                    target,
                    bytemuck::cast_slice(&data),
                    TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(width * texel_bytes),
                        rows_per_image: None,
                    },
                    Extent3d {
                        width,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                );
                if stamped.insert(entity) {
                    info!(
                        "field {entity}: first wheel stamp at {:?}, clock {:.2}, map origin {:?}",
                        contact.position, contacts.now, field.params.wheels.origin
                    );
                }
            }
        }
    }
}

#[derive(Resource)]
struct VegetationPipeline {
    mesh_pipeline: MeshPipeline,
    field_layout: BindGroupLayoutDescriptor,
    empty_layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    wind_map: TextureView,
    wind_sampler: Sampler,
}

fn init_vegetation_pipeline(
    mut commands: Commands,
    mesh_pipeline: Res<MeshPipeline>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
) {
    let field_layout = BindGroupLayoutDescriptor::new(
        "vegetation field layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::VERTEX_FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: false }),
                sampler(SamplerBindingType::NonFiltering),
                uniform_buffer::<VegetationParams>(true),
                texture_2d(TextureSampleType::Uint),
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    );
    let sampler = render_device.create_sampler(&SamplerDescriptor {
        label: Some("vegetation heightmap sampler"),
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Nearest,
        min_filter: FilterMode::Nearest,
        ..default()
    });
    let wind_map = render_device
        .create_texture_with_data(
            &render_queue,
            &TextureDescriptor {
                label: Some("vegetation gust map"),
                size: Extent3d {
                    width: super::wind_map::SIZE,
                    height: super::wind_map::SIZE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                usage: TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            TextureDataOrder::LayerMajor,
            &super::wind_map::bake(),
        )
        .create_view(&TextureViewDescriptor::default());
    let wind_sampler = render_device.create_sampler(&SamplerDescriptor {
        label: Some("vegetation gust sampler"),
        address_mode_u: AddressMode::Repeat,
        address_mode_v: AddressMode::Repeat,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..default()
    });
    commands.insert_resource(VegetationPipeline {
        mesh_pipeline: mesh_pipeline.clone(),
        field_layout,
        empty_layout: BindGroupLayoutDescriptor::new("vegetation empty layout", &[]),
        sampler,
        wind_map,
        wind_sampler,
    });
}

impl SpecializedMeshPipeline for VegetationPipeline {
    type Key = (MeshPipelineKey, Handle<Shader>);

    fn specialize(
        &self,
        key: Self::Key,
        layout: &MeshVertexBufferLayoutRef,
    ) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let mut descriptor = self.mesh_pipeline.specialize(key.0, layout)?;
        descriptor.vertex.shader = key.1.clone();
        let fragment = descriptor.fragment.as_mut().unwrap();
        fragment.shader = key.1;
        fragment
            .shader_defs
            .push("STANDARD_MATERIAL_DIFFUSE_TRANSMISSION".into());
        fragment
            .shader_defs
            .push("STANDARD_MATERIAL_DIFFUSE_OR_SPECULAR_TRANSMISSION".into());
        if let Some(mesh_group) = descriptor.layout.get_mut(2) {
            *mesh_group = self.empty_layout.clone();
        }
        descriptor.layout.push(self.field_layout.clone());
        descriptor.primitive.cull_mode = None;
        descriptor.multisample.alpha_to_coverage_enabled = descriptor.multisample.count > 1;
        Ok(descriptor)
    }
}

type DrawVegetation = (
    SetItemPipeline,
    SetMeshViewBindGroup<0>,
    SetMeshViewBindingArrayBindGroup<1>,
    SetEmptyBindGroup<2>,
    SetFieldBindGroup<3>,
    DrawBlades,
);

struct SetEmptyBindGroup<const I: usize>;

impl<P: PhaseItem, const I: usize> RenderCommand<P> for SetEmptyBindGroup<I> {
    type Param = Option<SRes<FieldBindGroups>>;
    type ViewQuery = ();
    type ItemQuery = ();

    #[inline]
    fn render<'w>(
        _item: &P,
        _view: (),
        _entity: Option<()>,
        groups: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let Some(groups) = groups else {
            return RenderCommandResult::Skip;
        };
        pass.set_bind_group(I, &groups.into_inner().empty, &[]);
        RenderCommandResult::Success
    }
}

struct SetFieldBindGroup<const I: usize>;

impl<P: PhaseItem, const I: usize> RenderCommand<P> for SetFieldBindGroup<I> {
    type Param = Option<SRes<FieldBindGroups>>;
    type ViewQuery = ();
    type ItemQuery = (Read<VegetationOffset>, Read<VegetationChunk>);

    #[inline]
    fn render<'w>(
        _item: &P,
        _view: (),
        offset: Option<(&'w VegetationOffset, &'w VegetationChunk)>,
        field: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let (Some(field), Some((offset, draw))) = (field, offset) else {
            return RenderCommandResult::Skip;
        };
        let key = (draw.field_id, draw.albedo.as_ref().map(Handle::id));
        let Some(group) = field.into_inner().fields.get(&key) else {
            return RenderCommandResult::Skip;
        };
        pass.set_bind_group(I, group, &[offset.0]);
        RenderCommandResult::Success
    }
}

struct DrawBlades;

impl<P: PhaseItem> RenderCommand<P> for DrawBlades {
    type Param = (SRes<RenderAssets<RenderMesh>>, SRes<MeshAllocator>);
    type ViewQuery = ();
    type ItemQuery = Read<VegetationChunk>;

    #[inline]
    fn render<'w>(
        _item: &P,
        _view: (),
        draw: Option<&'w VegetationChunk>,
        (meshes, mesh_allocator): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let mesh_allocator = mesh_allocator.into_inner();
        let Some(draw) = draw else {
            return RenderCommandResult::Skip;
        };
        let mesh_id = draw.mesh.id();
        let Some(gpu_mesh) = meshes.into_inner().get(mesh_id) else {
            return RenderCommandResult::Skip;
        };
        let Some(vertex_slice) = mesh_allocator.mesh_vertex_slice(&mesh_id) else {
            return RenderCommandResult::Skip;
        };
        pass.set_vertex_buffer(0, vertex_slice.buffer.slice(..));
        match &gpu_mesh.buffer_info {
            RenderMeshBufferInfo::Indexed {
                index_format,
                count,
            } => {
                let Some(index_slice) = mesh_allocator.mesh_index_slice(&mesh_id) else {
                    return RenderCommandResult::Skip;
                };
                pass.set_index_buffer(index_slice.buffer.slice(..), *index_format);
                let start = index_slice.range.start;
                if draw.variants.is_empty() {
                    pass.draw_indexed(
                        start..(start + count),
                        vertex_slice.range.start as i32,
                        0..draw.instances,
                    );
                }
                // One draw per clump variant; the first instance tells the
                // shader the variant count and which variant this is.
                let variants = draw.variants.len() as u32;
                for (variant, range) in draw.variants.iter().enumerate() {
                    let variant = variant as u32;
                    let instances = (draw.instances + variants - 1 - variant) / variants;
                    let first = (variants << 28) | (variant << 24);
                    pass.draw_indexed(
                        (start + range.start)..(start + range.end),
                        vertex_slice.range.start as i32,
                        first..first + instances,
                    );
                }
            }
            RenderMeshBufferInfo::NonIndexed => {
                pass.draw(vertex_slice.range, 0..draw.instances);
            }
        }
        RenderCommandResult::Success
    }
}

/// A new field takes the wheel tracks of the retired fields it overlaps: the
/// shared part of each old map is copied into the new one, texel for texel.
fn carry_wheel_tracks(
    fields: Res<RenderFields>,
    images: Res<RenderAssets<GpuImage>>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    mut carried: Local<bevy::platform::collections::HashSet<Entity>>,
) {
    carried.retain(|entity| fields.0.contains_key(entity));
    if fields.1.fields.is_empty() {
        return;
    }
    let mut encoder = render_device.create_command_encoder(&default());
    let mut copied = false;
    for (&entity, field) in &fields.0 {
        let Some(new) = images.get(&field.trample) else {
            continue;
        };
        if !carried.insert(entity) {
            continue;
        }
        let tpm = field.params.wheels.texels_per_metre;
        let bounds = field.params.bounds;
        for old in fields
            .1
            .fields
            .iter()
            .filter(|old| old.tread == field.tread && old.trample.id() != field.trample.id())
        {
            let Some(source) = images.get(&old.trample) else {
                continue;
            };
            let low = bounds.xy().max(old.params.bounds.xy());
            let high = bounds.zw().min(old.params.bounds.zw());
            if !low.cmplt(high).all() {
                continue;
            }
            let from = ((low - old.params.wheels.origin) * tpm).round().as_uvec2();
            let to = ((low - field.params.wheels.origin) * tpm).round().as_uvec2();
            let room = |size: Vec2, at: UVec2| (size.as_uvec2()).saturating_sub(at);
            let old_size = Vec2::new(old.params.wheels.width, old.params.wheels.height);
            let new_size = Vec2::new(field.params.wheels.width, field.params.wheels.height);
            let extent = ((high - low) * tpm)
                .floor()
                .as_uvec2()
                .min(room(old_size, from))
                .min(room(new_size, to));
            if extent.x == 0 || extent.y == 0 {
                continue;
            }
            let mut src = source.texture.as_image_copy();
            src.origin = Origin3d { x: from.x, y: from.y, z: 0 };
            let mut dst = new.texture.as_image_copy();
            dst.origin = Origin3d { x: to.x, y: to.y, z: 0 };
            encoder.copy_texture_to_texture(
                src,
                dst,
                Extent3d { width: extent.x, height: extent.y, depth_or_array_layers: 1 },
            );
            copied = true;
        }
    }
    if copied {
        render_queue.submit([encoder.finish()]);
    }
}
