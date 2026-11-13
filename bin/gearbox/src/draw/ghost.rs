//! Ghost robots: shaded machine copies with sparse feature edges and local
//! section datums, fading along the requested pose trail.

#[path = "ghost_edges.rs"]
mod edges;

use std::collections::HashMap;

use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{MeshVertexBufferLayoutRef, VertexBufferLayout};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use gearbox_api::Instance;
use usd_bevy::{UsdPrimRef, route::subset::UsdSubset};

use super::DrawVisuals;

const GHOST_SHADER: &str = "embedded://gearbox/draw/draw_ghost.wgsl";

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct GhostParams {
    /// Display colour (sRGB) and opacity.
    pub color: Vec4,
    /// Glow, band rate Hz, phase seconds, frost strength.
    pub fx: Vec4,
    /// World positions expressed in the ghost's own Z-up frame.
    pub ghost_from_world: Mat4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
#[bind_group_data(GhostKey)]
pub struct GhostMaterial {
    #[uniform(0)]
    pub params: GhostParams,
    #[texture(1, dimension = "3d")]
    #[sampler(2)]
    pub lut: Handle<Image>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct GhostKey {
    frost: bool,
}

impl From<&GhostMaterial> for GhostKey {
    fn from(m: &GhostMaterial) -> Self {
        Self {
            frost: m.params.fx.w > 0.0,
        }
    }
}

impl Material for GhostMaterial {
    fn vertex_shader() -> ShaderRef {
        GHOST_SHADER.into()
    }

    fn fragment_shader() -> ShaderRef {
        GHOST_SHADER.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }

    fn reads_view_transmission_texture(&self) -> bool {
        self.params.fx.w > 0.0
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
        descriptor.vertex.buffers = vec![ghost_vertex_layout(layout)?];
        if key.bind_group_data.frost {
            super::material::frost_pipeline(descriptor);
        }
        if has_edge_data(layout) {
            descriptor.vertex.shader_defs.push("GHOST_EDGE_DATA".into());
            if let Some(fragment) = descriptor.fragment.as_mut() {
                fragment.shader_defs.push("GHOST_EDGE_DATA".into());
            }
        }
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

fn ghost_vertex_layout(
    layout: &MeshVertexBufferLayoutRef,
) -> Result<VertexBufferLayout, SpecializedMeshPipelineError> {
    let mut attributes = vec![Mesh::ATTRIBUTE_POSITION.at_shader_location(0)];
    if layout.0.contains(Mesh::ATTRIBUTE_NORMAL) {
        attributes.push(Mesh::ATTRIBUTE_NORMAL.at_shader_location(1));
    }
    if has_edge_data(layout) {
        attributes.push(edges::BARYCENTRIC.at_shader_location(2));
        attributes.push(edges::EDGE_MASK.at_shader_location(3));
        attributes.push(edges::EDGE_START.at_shader_location(4));
        attributes.push(edges::EDGE_END.at_shader_location(5));
    }
    Ok(layout.0.get_layout(&attributes)?)
}

fn has_edge_data(layout: &MeshVertexBufferLayoutRef) -> bool {
    [
        edges::BARYCENTRIC,
        edges::EDGE_MASK,
        edges::EDGE_START,
        edges::EDGE_END,
    ]
    .into_iter()
    .all(|a| layout.0.contains(a))
}

/// What a ghost entity asks for.
#[derive(Clone, Debug)]
pub struct GhostSpec {
    pub blur: f32,
    pub machine: String,
    pub instances: Vec<Instance>,
    pub color: [u8; 4],
    pub colors: Vec<u32>,
    pub glow: f32,
    pub pulse: f32,
    pub fade: f32,
    pub phase: f32,
}

struct GhostPalette {
    entries: Vec<Vec4>,
    poses: Vec<usize>,
}

impl GhostPalette {
    fn of(spec: &GhostSpec) -> Self {
        let base = u32::from_le_bytes(spec.color);
        let display =
            |packed: u32| Vec4::from_array(packed.to_le_bytes().map(|v| v as f32 / 255.0));
        let mut entries = vec![display(base)];
        let mut indices = HashMap::from([(base, 0)]);
        let poses = (0..spec.instances.len())
            .map(|index| {
                let packed = spec
                    .colors
                    .get(index)
                    .copied()
                    .filter(|color| *color != 0)
                    .unwrap_or(base);
                *indices.entry(packed).or_insert_with(|| {
                    let index = entries.len();
                    entries.push(display(packed));
                    index
                })
            })
            .collect();
        Self { entries, poses }
    }

    fn params(&self, spec: &GhostSpec, index: usize, ghost_from_world: Mat4) -> GhostParams {
        let count = self.poses.len();
        let share = if count > 1 {
            index as f32 / (count - 1) as f32
        } else {
            1.0
        };
        let mut color = self.entries[self.poses[index]];
        color.w *= spec.fade + (1.0 - spec.fade) * share;
        GhostParams {
            color,
            fx: Vec4::new(
                spec.glow,
                spec.pulse,
                spec.phase + index as f32 * 0.37,
                spec.blur,
            ),
            ghost_from_world,
        }
    }
}

/// Marks a ghost's mesh copies.
#[derive(Component)]
pub struct GhostCopy;

#[derive(Component)]
pub(super) struct GhostPattern {
    material: Handle<GhostMaterial>,
    ghost_from_part: Mat4,
}

pub(super) fn update_patterns(
    anchors: Query<(&GhostPattern, &GlobalTransform)>,
    mut materials: ResMut<Assets<GhostMaterial>>,
) {
    for (anchor, global) in &anchors {
        let transform = pattern_transform(anchor.ghost_from_part, Mat4::from(global.affine()));
        if materials
            .get(&anchor.material)
            .is_some_and(|m| m.params.ghost_from_world.abs_diff_eq(transform, 1e-5))
        {
            continue;
        }
        if let Some(mut material) = materials.get_mut(&anchor.material) {
            material.params.ghost_from_world = transform;
        }
    }
}

fn pattern_transform(ghost_from_part: Mat4, world_from_part: Mat4) -> Mat4 {
    ghost_from_part * world_from_part.inverse()
}

type GhostSources = (Without<GhostCopy>, Or<(With<UsdPrimRef>, With<UsdSubset>)>);

fn belongs_to_machine(
    index: &crate::prim_index::PrimIndex,
    root: Entity,
    entity: Entity,
    subset: bool,
    parent: Option<Entity>,
) -> bool {
    let source = if subset { parent } else { Some(entity) };
    source.is_some_and(|source| index.is_descendant_of(source, root))
}

fn source_visible(
    mut entity: Entity,
    hierarchy: &Query<(Option<&Visibility>, Option<&ChildOf>)>,
) -> bool {
    while let Ok((visibility, parent)) = hierarchy.get(entity) {
        match visibility {
            Some(Visibility::Hidden) => return false,
            Some(Visibility::Visible) => return true,
            _ => {}
        }
        let Some(parent) = parent else { break };
        entity = parent.parent();
    }
    true
}

pub(super) fn clear_copies(commands: &mut Commands, copies: &mut Vec<Entity>) {
    for copy in copies.drain(..) {
        commands.entity(copy).try_despawn();
    }
}

/// Refreshes ghost copies when visible, loaded source parts change.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_ghosts(
    mut commands: Commands,
    mut visuals: ResMut<DrawVisuals>,
    inventory: Res<crate::controller::ControllerInventory>,
    index: Res<crate::prim_index::PrimIndex>,
    luts: Option<Res<bevy::core_pipeline::tonemapping::TonemappingLuts>>,
    mut materials: ResMut<Assets<GhostMaterial>>,
    mut mesh_assets: ResMut<Assets<Mesh>>,
    mut mesh_events: MessageReader<AssetEvent<Mesh>>,
    mut edge_cache: Local<edges::Cache>,
    meshes: Query<
        (
            Entity,
            &Mesh3d,
            &GlobalTransform,
            Has<UsdSubset>,
            Option<&ChildOf>,
        ),
        GhostSources,
    >,
    globals: Query<&GlobalTransform>,
    hierarchy: Query<(Option<&Visibility>, Option<&ChildOf>)>,
) {
    for event in mesh_events.read() {
        edge_cache.refresh(event, &mut mesh_assets);
    }
    let Some(luts) = luts else {
        return;
    };
    let mut sources = HashMap::new();
    for visual in visuals.map.values_mut() {
        let Some(spec) = visual.ghost.as_ref() else {
            continue;
        };
        let machine = inventory
            .machines
            .iter()
            .find(|m| m.id == spec.machine || m.asset_label == spec.machine);
        let loaded = machine.and_then(|machine| {
            let scene_root = machine.scene_root?;
            let prim = index.find(scene_root, &machine.prim_path)?;
            let body = machine
                .body
                .as_deref()
                .and_then(|b| index.find(scene_root, b))
                .unwrap_or(prim);
            Some((prim, body))
        });
        let Some((prim, body)) = loaded else {
            clear_copies(&mut commands, &mut visual.ghost_copies);
            visual.ghost_sources.clear();
            continue;
        };
        let (Ok(root), Ok(base), Ok(body)) = (
            globals.get(visual.root),
            globals.get(prim),
            globals.get(body),
        ) else {
            continue;
        };
        let source_parts = sources.entry(prim).or_insert_with(|| {
            let mut parts: Vec<_> = meshes
                .iter()
                .filter(|(e, _, _, subset, parent)| {
                    belongs_to_machine(&index, prim, *e, *subset, parent.map(ChildOf::parent))
                })
                .filter(|(e, mesh, _, _, _)| {
                    mesh_assets.contains(mesh.id()) && source_visible(*e, &hierarchy)
                })
                .map(|(entity, mesh, global, _, _)| (entity, mesh.0.clone(), *global))
                .collect();
            parts.sort_unstable_by_key(|(entity, _, _)| entity.to_bits());
            parts
        });
        let signature: Vec<_> = source_parts
            .iter()
            .map(|(entity, mesh, _)| (*entity, mesh.id()))
            .collect();
        if visual.ghost_sources == signature
            && visual
                .ghost_copies
                .iter()
                .all(|entity| globals.contains(*entity))
        {
            continue;
        }
        clear_copies(&mut commands, &mut visual.ghost_copies);
        visual.ghost_sources = signature;
        let parts: Vec<_> = source_parts
            .iter()
            .map(|(_, mesh, global)| (edge_cache.mesh(mesh, &mut mesh_assets), *global))
            .collect();
        if parts.is_empty() {
            continue;
        }
        let to_root = root.affine().inverse();
        let base_r = to_root * base.affine();
        let body_r = to_root * body.affine();
        let forward = body_r.transform_vector3(Vec3::X);
        let yaw = forward.y.atan2(forward.x);
        let planar = Transform::from_translation(base_r.translation.into())
            .with_rotation(Quat::from_rotation_z(yaw));
        let to_planar = planar.compute_affine().inverse();
        let palette = GhostPalette::of(spec);
        for (k, inst) in spec.instances.iter().enumerate() {
            let [w, x, y, z] = inst.rotation;
            let q = Quat::from_xyzw(x, y, z, w);
            let pose = Transform::from_translation(Vec3::from_array(inst.position)).with_rotation(
                if q.length_squared() > 1e-8 {
                    q.normalize()
                } else {
                    Quat::IDENTITY
                },
            );
            let material = materials.add(GhostMaterial {
                params: palette.params(
                    spec,
                    k,
                    Mat4::from((root.affine() * pose.compute_affine()).inverse()),
                ),
                lut: luts.agx.clone(),
            });
            let place = pose.compute_affine() * to_planar;
            for (part, (mesh, gt)) in parts.iter().enumerate() {
                let local = place * to_root * gt.affine();
                let child = commands
                    .spawn((
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(material.clone()),
                        Transform::from_matrix(Mat4::from(local)),
                        Visibility::Inherited,
                        NoFrustumCulling,
                        NotShadowCaster,
                        NotShadowReceiver,
                        RenderLayers::layer(super::DRAW_LAYER),
                        GhostCopy,
                        ChildOf(visual.root),
                    ))
                    .id();
                if part == 0 {
                    commands.entity(child).insert(GhostPattern {
                        material: material.clone(),
                        ghost_from_part: Mat4::from(to_planar * to_root * gt.affine()),
                    });
                }
                visual.ghost_copies.push(child);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{MeshVertexBufferLayouts, PrimitiveTopology};

    fn color_spec(count: usize, colors: Vec<u32>) -> GhostSpec {
        GhostSpec {
            machine: "robot".into(),
            instances: vec![Instance::default(); count],
            color: [31, 79, 137, 204],
            colors,
            glow: 0.6,
            pulse: 0.5,
            fade: 0.15,
            phase: 0.23,
            blur: 0.0,
        }
    }

    #[test]
    fn ghost_pose_colors_override_rgba_and_zero_or_missing_inherit() {
        let tinted = gearbox_api::wire::draw::rgba(231, 27, 163, 93);
        let transparent = gearbox_api::wire::draw::rgba(17, 211, 68, 0);
        let spec = color_spec(5, vec![0, tinted, transparent, 0]);
        let palette = GhostPalette::of(&spec);
        assert_eq!(palette.entries.len(), 3);
        assert_eq!(palette.poses, [0, 1, 2, 0, 0]);
        let expected = [
            [31, 79, 137, 204],
            [231, 27, 163, 93],
            [17, 211, 68, 0],
            [31, 79, 137, 204],
            [31, 79, 137, 204],
        ];
        for (index, rgba) in expected.into_iter().enumerate() {
            let mut color = Vec4::from_array(rgba.map(|v| v as f32 / 255.0));
            color.w *= spec.fade + (1.0 - spec.fade) * index as f32 / 4.0;
            assert!(
                palette
                    .params(&spec, index, Mat4::IDENTITY)
                    .color
                    .abs_diff_eq(color, 1e-6)
            );
        }
        assert_eq!(palette.params(&spec, 2, Mat4::IDENTITY).color.w, 0.0);
    }

    #[test]
    fn ghost_palette_is_bounded_by_used_colors_and_ignores_extras() {
        let tint = gearbox_api::wire::draw::rgba(19, 217, 98, 255);
        let spec = color_spec(4, vec![tint, tint, 0, tint, 0xffff_ffff]);
        let palette = GhostPalette::of(&spec);
        assert_eq!(palette.entries.len(), 2);
        assert_eq!(palette.poses, [1, 1, 0, 1]);
        assert_eq!(
            GhostPalette::of(&color_spec(0, spec.colors)).entries.len(),
            1
        );
    }

    #[test]
    fn ghost_default_tint_trail_alpha_phase_and_frame_are_unchanged() {
        for count in [1, 4] {
            let spec = color_spec(count, Vec::new());
            let palette = GhostPalette::of(&spec);
            let matrix = Mat4::from_translation(Vec3::new(3.0, -7.0, 1.0));
            assert_eq!(palette.entries.len(), 1);
            for index in 0..count {
                let params = palette.params(&spec, index, matrix);
                let share = if count > 1 {
                    index as f32 / (count - 1) as f32
                } else {
                    1.0
                };
                assert_eq!(
                    params.color,
                    Vec4::new(
                        31.0 / 255.0,
                        79.0 / 255.0,
                        137.0 / 255.0,
                        204.0 / 255.0 * (spec.fade + (1.0 - spec.fade) * share)
                    )
                );
                assert_eq!(
                    params.fx,
                    Vec4::new(spec.glow, spec.pulse, spec.phase + index as f32 * 0.37, 0.0)
                );
                assert_eq!(params.ghost_from_world, matrix);
            }
        }
    }

    #[test]
    fn ghosts_include_usd_subsets_but_not_attached_overlays_or_other_machines() {
        let mut world = World::new();
        crate::prim_index::PrimIndex::track(&mut world);
        let root = world.spawn(UsdPrimRef::new("/Robot")).id();
        let other = world.spawn(UsdPrimRef::new("/Other")).id();
        let part = world
            .spawn((
                UsdPrimRef::new("/Robot/Body"),
                Mesh3d::default(),
                ChildOf(root),
            ))
            .id();
        let subset = world
            .spawn((UsdSubset("paint".into()), Mesh3d::default(), ChildOf(part)))
            .id();
        let overlay = world.spawn((Mesh3d::default(), ChildOf(part))).id();
        world.spawn((Mesh3d::default(), ChildOf(subset)));
        world.spawn((UsdSubset("other".into()), Mesh3d::default(), ChildOf(other)));
        world.spawn((UsdSubset("orphan".into()), Mesh3d::default()));
        world.spawn((
            UsdSubset("overlay".into()),
            Mesh3d::default(),
            ChildOf(overlay),
        ));
        world.spawn((
            UsdSubset("copy".into()),
            Mesh3d::default(),
            GhostCopy,
            ChildOf(part),
        ));
        let index = world.resource::<crate::prim_index::PrimIndex>().clone();
        assert!(!index.is_descendant_of(subset, root));
        let mut sources = world.query_filtered::<(Entity, Has<UsdSubset>, Option<&ChildOf>), (With<Mesh3d>, GhostSources)>();
        let selected: Vec<_> = sources
            .iter(&world)
            .filter(|(entity, subset, parent)| {
                belongs_to_machine(&index, root, *entity, *subset, parent.map(ChildOf::parent))
            })
            .map(|(entity, _, _)| entity)
            .collect();
        assert_eq!(selected.len(), 2);
        assert!(selected.contains(&part));
        assert!(selected.contains(&subset));
    }

    #[test]
    fn ghosts_accept_flat_and_smooth_source_meshes_without_changing_them() {
        let mut layouts = MeshVertexBufferLayouts::default();
        for smooth in [false, true] {
            let mut mesh = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::default(),
            );
            mesh.insert_attribute(
                Mesh::ATTRIBUTE_POSITION,
                vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            );
            if smooth {
                mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; 3]);
            }
            let source = mesh.get_mesh_vertex_buffer_layout(&mut layouts);
            let ghost = ghost_vertex_layout(&source).unwrap();
            assert_eq!(ghost.attributes.len(), if smooth { 2 } else { 1 });
            assert_eq!(ghost.attributes[0].shader_location, 0);
            if smooth {
                assert_eq!(ghost.attributes[1].shader_location, 1);
            }
            assert_eq!(mesh.contains_attribute(Mesh::ATTRIBUTE_NORMAL), smooth);
            assert_eq!(ghost.array_stride, source.0.layout().array_stride);
        }
    }

    #[test]
    fn ghost_visibility_respects_hidden_ancestors_and_visible_subsets() {
        let mut world = World::new();
        let root = world.spawn(Visibility::Inherited).id();
        let visible = world.spawn((Visibility::Inherited, ChildOf(root))).id();
        let subset = world
            .spawn((UsdSubset("paint".into()), ChildOf(visible)))
            .id();
        let proxy = world.spawn((Visibility::Hidden, ChildOf(root))).id();
        let collision_mesh = world.spawn(ChildOf(proxy)).id();
        let hidden_subset = world
            .spawn((UsdSubset("proxy".into()), ChildOf(collision_mesh)))
            .id();
        let hidden_mesh = world.spawn((Visibility::Hidden, ChildOf(visible))).id();
        let explicit = world.spawn((Visibility::Visible, ChildOf(proxy))).id();
        let mut system = bevy::ecs::system::SystemState::<
            Query<(Option<&Visibility>, Option<&ChildOf>)>,
        >::new(&mut world);
        let query = system.get(&world).unwrap();
        for entity in [root, visible, subset, explicit] {
            assert!(source_visible(entity, &query));
        }
        for entity in [proxy, collision_mesh, hidden_subset, hidden_mesh] {
            assert!(!source_visible(entity, &query));
        }
    }

    #[test]
    fn ghost_layout_still_requires_positions() {
        let mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        let source = mesh.get_mesh_vertex_buffer_layout(&mut MeshVertexBufferLayouts::default());
        assert!(ghost_vertex_layout(&source).is_err());
    }

    #[test]
    fn ghost_energy_has_coverage_independent_of_emission() {
        let shader = include_str!("draw_ghost.wgsl");
        let shared = include_str!("draw_common.wgsl");
        let coverage = shared
            .lines()
            .find(|line| line.contains("let alpha ="))
            .unwrap();
        assert!(coverage.contains("coverage") && coverage.contains("opacity"));
        assert!(!coverage.contains("glow"));
        assert!(shader.contains("energy_glass(in.color, params.color.a"));
        assert!(shader.contains("energy_packet(scan_phase, 0.018, fwidth(scan_phase), hz)"));
        let surface = include_str!("draw_surface.wgsl");
        assert!(surface.matches("energy_glass(base, in.color.a").count() >= 5);
        assert_eq!(surface.matches("return energy_coat(").count(), 5);
        assert!(surface.contains("return vec4(base * shade * in.color.a, in.color.a)"));
        for line in surface
            .lines()
            .filter(|line| line.contains("let coverage ="))
        {
            assert!(!line.contains("glow"));
            assert!(line.contains("activity"));
        }
        assert!(!include_str!("draw_heat.wgsl").contains("globals.time"));
    }

    #[test]
    fn ghost_pattern_stays_in_the_ghost_frame_when_the_world_moves() {
        let part = Mat4::from_scale_rotation_translation(
            Vec3::new(0.1, 0.2, 0.15),
            Quat::from_rotation_x(0.4),
            Vec3::new(1.0, 0.5, 0.7),
        );
        let point = Vec3::new(0.3, -0.5, 0.2);
        for position in [
            Vec3::ZERO,
            Vec3::new(300.0, -170.0, 90.0),
            Vec3::new(-10.0, 30.0, 5.0),
        ] {
            let world =
                Mat4::from_rotation_translation(Quat::from_rotation_y(0.7), position) * part;
            let local =
                pattern_transform(part, world).transform_point3(world.transform_point3(point));
            assert!(local.abs_diff_eq(part.transform_point3(point), 0.0002));
        }
    }

    #[test]
    fn ghost_parts_share_continuous_pattern_coordinates() {
        let pose =
            Mat4::from_rotation_translation(Quat::from_rotation_z(-0.8), Vec3::new(15.0, 4.0, 2.0));
        let expected = Vec3::new(1.0, 0.0, 0.8);
        for part in [
            Mat4::IDENTITY,
            Mat4::from_translation(Vec3::new(0.5, -1.0, 0.3)),
        ] {
            let world = pose * part;
            let local = part.inverse().transform_point3(expected);
            let pattern =
                pattern_transform(part, world).transform_point3(world.transform_point3(local));
            assert!(pattern.abs_diff_eq(expected, 1e-5));
        }
    }
}
