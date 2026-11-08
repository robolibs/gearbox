//! Ghost robots: see-through copies of a machine's own meshes at each pose
//! of a ghost entity, a fresnel rim over a faint fill, fading along the
//! trail so the last pose is the brightest. Built once the machine's meshes
//! are in the scene, rebuilt when the entity changes.

use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::shader::ShaderRef;
use gearbox_api::Instance;

use super::DrawVisuals;

const GHOST_SHADER: &str = "embedded://gearbox/draw/draw_ghost.wgsl";

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct GhostParams {
    /// Display colour (sRGB) and opacity.
    pub color: Vec4,
    /// Glow, band rate Hz, phase seconds, unused.
    pub fx: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct GhostMaterial {
    #[uniform(0)]
    pub params: GhostParams,
    #[texture(1, dimension = "3d")]
    #[sampler(2)]
    pub lut: Handle<Image>,
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
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0), Mesh::ATTRIBUTE_NORMAL.at_shader_location(1)])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

/// What a ghost entity asks for.
#[derive(Clone, Debug)]
pub struct GhostSpec {
    pub machine: String,
    pub instances: Vec<Instance>,
    pub color: [u8; 4],
    pub glow: f32,
    pub pulse: f32,
    pub fade: f32,
    pub phase: f32,
}

/// Marks a ghost's mesh copies.
#[derive(Component)]
pub struct GhostCopy;

/// Spawns the copies of every ghost not built yet whose machine is loaded.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_ghosts(
    mut commands: Commands,
    mut visuals: ResMut<DrawVisuals>,
    inventory: Res<crate::controller::ControllerInventory>,
    index: Res<crate::prim_index::PrimIndex>,
    luts: Option<Res<bevy::core_pipeline::tonemapping::TonemappingLuts>>,
    mut materials: ResMut<Assets<GhostMaterial>>,
    meshes: Query<(Entity, &Mesh3d, &GlobalTransform), Without<GhostCopy>>,
    globals: Query<&GlobalTransform>,
) {
    let Some(luts) = luts else {
        return;
    };
    for visual in visuals.map.values_mut() {
        let Some(spec) = visual.ghost.as_ref().filter(|_| !visual.ghost_built) else {
            continue;
        };
        let Some(machine) = inventory.machines.iter().find(|m| m.id == spec.machine || m.asset_label == spec.machine) else {
            continue;
        };
        let Some(scene_root) = machine.scene_root else {
            continue;
        };
        let Some(prim) = index.find(scene_root, &machine.prim_path) else {
            continue;
        };
        let body = machine.body.as_deref().and_then(|b| index.find(scene_root, b)).unwrap_or(prim);
        let (Ok(root), Ok(base), Ok(body)) = (globals.get(visual.root), globals.get(prim), globals.get(body)) else {
            continue;
        };
        let parts: Vec<(Handle<Mesh>, GlobalTransform)> =
            meshes.iter().filter(|(e, _, _)| index.is_descendant_of(*e, prim)).map(|(_, m, gt)| (m.0.clone(), *gt)).collect();
        if parts.is_empty() {
            continue;
        }
        let to_root = root.affine().inverse();
        let base_r = to_root * base.affine();
        let body_r = to_root * body.affine();
        let forward = body_r.transform_vector3(Vec3::X);
        let yaw = forward.y.atan2(forward.x);
        let planar = Transform::from_translation(base_r.translation.into()).with_rotation(Quat::from_rotation_z(yaw));
        let to_planar = planar.compute_affine().inverse();
        let n = spec.instances.len();
        for (k, inst) in spec.instances.iter().enumerate() {
            let [w, x, y, z] = inst.rotation;
            let q = Quat::from_xyzw(x, y, z, w);
            let pose = Transform::from_translation(Vec3::from_array(inst.position)).with_rotation(if q.length_squared() > 1e-8 { q.normalize() } else { Quat::IDENTITY });
            let share = if n > 1 { k as f32 / (n - 1) as f32 } else { 1.0 };
            let alpha = spec.color[3] as f32 / 255.0 * (spec.fade + (1.0 - spec.fade) * share);
            let c = spec.color;
            let material = materials.add(GhostMaterial {
                params: GhostParams {
                    color: Vec4::new(c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, alpha),
                    fx: Vec4::new(spec.glow, spec.pulse, spec.phase + k as f32 * 0.37, 0.0),
                },
                lut: luts.agx.clone(),
            });
            let place = pose.compute_affine() * to_planar;
            for (mesh, gt) in &parts {
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
                visual.ghost_copies.push(child);
            }
        }
        visual.ghost_built = true;
    }
}
