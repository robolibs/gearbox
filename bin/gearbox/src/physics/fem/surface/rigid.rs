use super::frames::FemSurfaceFrames;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Buffer, BufferId, ShaderType};
use bevy::shader::ShaderRef;

pub(crate) type RigidMaterial = ExtendedMaterial<StandardMaterial, RigidExtension>;

#[derive(Clone, Copy, Debug, ShaderType)]
pub(crate) struct RigidBind {
    mesh_to_body: Mat4,
    normal_to_body: Mat4,
    origin: Vec3,
    body: u32,
    handedness: f32,
}

impl RigidBind {
    fn new(body: u32, count: u64, origin: Vec3, mesh_to_body: Mat4) -> Result<Self, String> {
        let determinant = mesh_to_body.determinant();
        if u64::from(body) >= count
            || !origin.is_finite()
            || !mesh_to_body.is_finite()
            || !determinant.is_finite()
            || determinant.abs() < 1e-12
            || mesh_to_body.row(3) != Vec4::W
        {
            return Err("invalid FEM rigid mesh bind transform or body".into());
        }
        let normal_to_body = mesh_to_body.inverse().transpose();
        if !normal_to_body.is_finite() {
            return Err("unrepresentable FEM rigid normal transform".into());
        }
        Ok(Self {
            mesh_to_body,
            normal_to_body,
            origin,
            body,
            handedness: determinant.signum(),
        })
    }
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(crate) struct RigidExtension {
    #[storage(100, read_only, buffer)]
    pub(super) poses: Buffer,
    #[storage(101, read_only, buffer)]
    pub(super) previous_poses: Buffer,
    #[storage(102, read_only, buffer)]
    pub(super) validity: Buffer,
    #[uniform(103)]
    pub(super) bind: RigidBind,
}

impl MaterialExtension for RigidExtension {
    fn vertex_shader() -> ShaderRef {
        "embedded://gearbox/physics/fem/surface/rigid.wgsl".into()
    }
    fn prepass_vertex_shader() -> ShaderRef {
        Self::vertex_shader()
    }
    fn deferred_vertex_shader() -> ShaderRef {
        Self::vertex_shader()
    }
}

pub(super) struct RigidSurfacePlugin;

impl Plugin for RigidSurfacePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "rigid.wgsl");
        app.add_plugins(MaterialPlugin::<RigidMaterial>::default());
    }
}

/// Immutable mesh-to-body binding; current and previous poses come from GPU frame snapshots.
#[derive(Component)]
#[require(NoFrustumCulling)]
pub(crate) struct FemRigidBinding {
    pub(super) island: Entity,
    pub(super) generation: BufferId,
    pub(super) materials: [Handle<RigidMaterial>; 3],
}

impl FemRigidBinding {
    pub(crate) fn new(
        island: Entity,
        frames: &FemSurfaceFrames,
        materials: &mut Assets<RigidMaterial>,
        base: StandardMaterial,
        body: u32,
        origin: Vec3,
        mesh_to_body: Mat4,
    ) -> Result<Self, String> {
        let bind = RigidBind::new(body, frames.body_poses[0].size() / 32, origin, mesh_to_body)?;
        Ok(Self {
            island,
            generation: frames.body_poses[0].id(),
            materials: std::array::from_fn(|slot| {
                materials.add(RigidMaterial {
                    base: base.clone(),
                    extension: frames.rigid_extension(slot, bind),
                })
            }),
        })
    }

    pub(crate) fn material(&self) -> MeshMaterial3d<RigidMaterial> {
        MeshMaterial3d(self.materials[0].clone())
    }
}

#[test]
fn rigid_bind_preserves_normals_and_rejects_invalid_frames() {
    let matrix = Mat4::from_scale_rotation_translation(
        Vec3::new(-2.0, 3.0, 0.5),
        Quat::from_rotation_y(0.6),
        Vec3::new(1.0, 2.0, 3.0),
    );
    let bind = RigidBind::new(1, 2, Vec3::X, matrix).unwrap();
    assert_eq!(bind.handedness, -1.0);
    let tangent = matrix.transform_vector3(Vec3::new(1.0, 1.0, 0.0));
    let normal = bind
        .normal_to_body
        .transform_vector3(Vec3::new(1.0, -1.0, 0.0));
    assert!(normal.dot(tangent).abs() < 1e-6);
    assert!(RigidBind::new(2, 2, Vec3::ZERO, matrix).is_err());
    assert!(RigidBind::new(0, 0, Vec3::ZERO, matrix).is_err());
    assert!(RigidBind::new(0, 1, Vec3::NAN, matrix).is_err());
    assert!(RigidBind::new(0, 1, Vec3::ZERO, Mat4::ZERO).is_err());
    let mut projective = Mat4::IDENTITY;
    projective.x_axis.w = 0.1;
    assert!(RigidBind::new(0, 1, Vec3::ZERO, projective).is_err());
}
