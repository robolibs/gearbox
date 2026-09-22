#import bevy_pbr::{mesh_functions, view_transformations::position_world_to_clip}
#ifdef PREPASS_PIPELINE
#import bevy_pbr::prepass_io::VertexOutput
#else
#import bevy_pbr::forward_io::VertexOutput
#endif

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<storage, read> positions: array<vec4<f32>>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var<storage, read> previous_positions: array<vec4<f32>>;

struct Validity {
    status: u32,
    minimum_j: f32,
    invalid_tet: u32,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var<storage, read> validity: Validity;

struct Input {
    @builtin(instance_index) instance_index: u32,
    @location(0) reference: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(8) nodes: vec3<u32>,
    @location(9) neighbor_uvs: vec4<f32>,
}

@vertex
fn vertex(input: Input) -> VertexOutput {
    var out: VertexOutput;
    if validity.status != 0u || validity.invalid_tet != 0xffffffffu
        || !(validity.minimum_j > 0.5 && validity.minimum_j <= 3.402823466e+38) {
        out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return out;
    }
    let p = positions[input.nodes.x].xyz;
    let edge1 = positions[input.nodes.y].xyz - p;
    let edge2 = positions[input.nodes.z].xyz - p;
    let normal = normalize(cross(edge1, edge2));
    let uv1 = input.neighbor_uvs.xy - input.uv;
    let uv2 = input.neighbor_uvs.zw - input.uv;
    let determinant = uv1.x * uv2.y - uv1.y * uv2.x;
    var tangent = normalize(edge1);
    var handedness = 1.0;
    if abs(determinant) > 1e-8 {
        tangent = normalize((edge1 * uv2.y - edge2 * uv1.y) / determinant);
        let bitangent = (edge2 * uv1.x - edge1 * uv2.x) / determinant;
        handedness = select(-1.0, 1.0, dot(cross(normal, tangent), bitangent) >= 0.0);
    }
    let world = mesh_functions::get_world_from_local(input.instance_index);
    out.world_position = mesh_functions::mesh_position_local_to_world(world, vec4<f32>(p, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    out.unclipped_depth = out.position.z;
    out.position.z = min(out.position.z, 1.0);
#endif
#ifdef VERTEX_UVS_A
    out.uv = input.uv;
#endif
#ifdef PREPASS_PIPELINE
#ifdef NORMAL_PREPASS_OR_DEFERRED_PREPASS
    out.world_normal = mesh_functions::mesh_normal_local_to_world(normal, input.instance_index);
#ifdef VERTEX_TANGENTS
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(world, vec4<f32>(tangent, handedness), input.instance_index);
#endif
#endif
#else
    out.world_normal = mesh_functions::mesh_normal_local_to_world(normal, input.instance_index);
#ifdef VERTEX_TANGENTS
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(world, vec4<f32>(tangent, handedness), input.instance_index);
#endif
#endif
#ifdef PREPASS_PIPELINE
#ifdef MOTION_VECTOR_PREPASS
    let previous_world = mesh_functions::get_previous_world_from_local(input.instance_index);
    out.previous_world_position = mesh_functions::mesh_position_local_to_world(previous_world, vec4<f32>(previous_positions[input.nodes.x].xyz, 1.0));
#endif
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = input.instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(input.instance_index, world[3]);
#endif
    return out;
}
