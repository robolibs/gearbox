use bevy::asset::RenderAssetUsages;
use bevy::mesh::{MeshVertexAttribute, MeshVertexBufferLayoutRef, VertexAttributeValues};
use bevy::pbr::{
    ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline,
};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Buffer, PrimitiveTopology, RenderPipelineDescriptor, SpecializedMeshPipelineError,
    VertexFormat,
};
use bevy::shader::ShaderRef;

pub(crate) mod frames;
pub(crate) mod rigid;

const NODES: MeshVertexAttribute =
    MeshVertexAttribute::new("FemTriangleNodes", 0x46454d01, VertexFormat::Uint32x3);
const NEIGHBOR_UVS: MeshVertexAttribute =
    MeshVertexAttribute::new("FemNeighborUvs", 0x46454d02, VertexFormat::Float32x4);
const SHADER: &str = "embedded://gearbox/physics/fem/surface.wgsl";

#[cfg(test)]
#[path = "surface_test.rs"]
mod render_tests;

pub(crate) type FemMaterial = ExtendedMaterial<StandardMaterial, FemExtension>;

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub(crate) struct FemExtension {
    /// Node positions for this render frame, in mesh-local coordinates.
    #[storage(100, read_only, buffer)]
    pub(crate) positions: Buffer,
    /// Node positions for the preceding render frame, in mesh-local coordinates.
    #[storage(101, read_only, buffer)]
    pub(crate) previous_positions: Buffer,
    #[storage(102, read_only, buffer)]
    pub(crate) validity: Buffer,
}

impl MaterialExtension for FemExtension {
    fn vertex_shader() -> ShaderRef {
        SHADER.into()
    }
    fn prepass_vertex_shader() -> ShaderRef {
        SHADER.into()
    }
    fn deferred_vertex_shader() -> ShaderRef {
        SHADER.into()
    }

    fn specialize(
        _: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.vertex.buffers = vec![layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(2),
            NODES.at_shader_location(8),
            NEIGHBOR_UVS.at_shader_location(9),
        ])?];
        Ok(())
    }
}

pub(crate) struct FemSurfacePlugin;

impl Plugin for FemSurfacePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "surface.wgsl");
        app.add_plugins(MaterialPlugin::<FemMaterial>::default());
        app.add_plugins(rigid::RigidSurfacePlugin);
        app.add_systems(
            Update,
            frames::capture_surfaces.after(super::advance_islands),
        );
    }
}

/// Static surface topology; GPU node buffers supply deformed positions and face frames.
pub(crate) fn surface_mesh(
    reference: &[[f32; 3]],
    uvs: &[[f32; 2]],
    triangles: &[[u32; 3]],
) -> Result<Mesh, String> {
    if reference.is_empty()
        || triangles.is_empty()
        || reference.len() != uvs.len()
        || reference
            .iter()
            .flatten()
            .chain(uvs.iter().flatten())
            .any(|v| !v.is_finite())
    {
        return Err("invalid FEM surface reference data".into());
    }
    let count = triangles
        .len()
        .checked_mul(3)
        .ok_or("FEM surface too large")?;
    let mut positions = Vec::with_capacity(count);
    let mut normals = Vec::with_capacity(count);
    let mut texcoords = Vec::with_capacity(count);
    let mut nodes = Vec::with_capacity(count);
    let mut neighbor_uvs = Vec::with_capacity(count);
    for triangle in triangles {
        if triangle
            .iter()
            .any(|&index| index as usize >= reference.len())
        {
            return Err("FEM surface node index out of range".into());
        }
        let points = triangle.map(|i| Vec3::from_array(reference[i as usize]));
        let normal = (points[1] - points[0]).cross(points[2] - points[0]);
        if !normal.is_finite() || normal.length_squared() <= 1e-20 {
            return Err("degenerate FEM surface triangle".into());
        }
        for corner in 0..3 {
            let [a, b, c] = [
                triangle[corner],
                triangle[(corner + 1) % 3],
                triangle[(corner + 2) % 3],
            ];
            positions.push(reference[a as usize]);
            normals.push(normal.normalize().to_array());
            texcoords.push(uvs[a as usize]);
            nodes.push([a, b, c]);
            neighbor_uvs.push([
                uvs[b as usize][0],
                uvs[b as usize][1],
                uvs[c as usize][0],
                uvs[c as usize][1],
            ]);
        }
    }
    Ok(Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_TANGENT,
        vec![[1.0_f32, 0.0, 0.0, 1.0]; count],
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, texcoords)
    .with_inserted_attribute(NODES, VertexAttributeValues::Uint32x3(nodes))
    .with_inserted_attribute(NEIGHBOR_UVS, neighbor_uvs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangle_nodes_keep_winding_and_explicit_indices() {
        let positions = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let uv = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        let mesh = surface_mesh(&positions, &uv, &[[0, 1, 2]]).unwrap();
        assert_eq!(mesh.count_vertices(), 3);
        assert!(
            matches!(mesh.attribute(NODES), Some(VertexAttributeValues::Uint32x3(v)) if v == &[[0,1,2], [1,2,0], [2,0,1]])
        );
        assert!(surface_mesh(&positions, &uv, &[[0, 1, 3]]).is_err());
        assert!(surface_mesh(&positions, &uv, &[[0, 1, 1]]).is_err());
        assert!(surface_mesh(&positions, &uv[..2], &[[0, 1, 2]]).is_err());
        let mut invalid = positions;
        invalid[0][0] = f32::NAN;
        assert!(surface_mesh(&invalid, &uv, &[[0, 1, 2]]).is_err());
    }
}
