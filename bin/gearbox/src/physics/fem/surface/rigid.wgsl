#import bevy_pbr::{mesh_functions, view_transformations::position_world_to_clip}
#ifdef PREPASS_PIPELINE
#import bevy_pbr::prepass_io::{Vertex, VertexOutput}
#else
#import bevy_pbr::forward_io::{Vertex, VertexOutput}
#endif

struct Pose { position: vec4<f32>, rotation: vec4<f32> }
struct Validity { status: u32, minimum_j: f32, invalid_tet: u32 }
struct Bind {
    mesh_to_body: mat4x4<f32>, normal_to_body: mat4x4<f32>,
    origin: vec3<f32>, body: u32, handedness: f32,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<storage, read> poses: array<Pose>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var<storage, read> previous_poses: array<Pose>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var<storage, read> validity: Validity;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var<uniform> bind: Bind;

fn rotate(q: vec4<f32>, p: vec3<f32>) -> vec3<f32> {
    return p + 2.0 * cross(q.xyz, cross(q.xyz, p) + q.w * p);
}

@vertex
fn vertex(input: Vertex) -> VertexOutput {
    var out: VertexOutput;
    if validity.status != 0u || validity.invalid_tet != 0xffffffffu
        || !(validity.minimum_j > 0.5 && validity.minimum_j <= 3.402823466e+38)
        || bind.body >= arrayLength(&poses) || bind.body >= arrayLength(&previous_poses) {
        out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return out;
    }
    let q = poses[bind.body];
    let local = (bind.mesh_to_body * vec4<f32>(input.position, 1.0)).xyz;
    let position = bind.origin + q.position.xyz + rotate(q.rotation, local);
    out.world_position = vec4<f32>(position, 1.0);
    out.position = position_world_to_clip(position);
#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    out.unclipped_depth = out.position.z;
    out.position.z = min(out.position.z, 1.0);
#endif
#ifdef VERTEX_UVS_A
    out.uv = input.uv;
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = input.uv_b;
#endif
#ifdef VERTEX_COLORS
    out.color = input.color;
#endif
#ifdef PREPASS_PIPELINE
#ifdef NORMAL_PREPASS_OR_DEFERRED_PREPASS
#ifdef VERTEX_NORMALS
    out.world_normal = normalize(rotate(q.rotation, (bind.normal_to_body * vec4<f32>(input.normal, 0.0)).xyz));
#endif
#ifdef VERTEX_TANGENTS
    out.world_tangent = vec4<f32>(normalize(rotate(q.rotation, (bind.mesh_to_body * vec4<f32>(input.tangent.xyz, 0.0)).xyz)), input.tangent.w * bind.handedness);
#endif
#endif
#ifdef MOTION_VECTOR_PREPASS
    let previous = previous_poses[bind.body];
    out.previous_world_position = vec4<f32>(bind.origin + previous.position.xyz + rotate(previous.rotation, local), 1.0);
#endif
#else
#ifdef VERTEX_NORMALS
    out.world_normal = normalize(rotate(q.rotation, (bind.normal_to_body * vec4<f32>(input.normal, 0.0)).xyz));
#endif
#ifdef VERTEX_TANGENTS
    out.world_tangent = vec4<f32>(normalize(rotate(q.rotation, (bind.mesh_to_body * vec4<f32>(input.tangent.xyz, 0.0)).xyz)), input.tangent.w * bind.handedness);
#endif
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = input.instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(input.instance_index, vec4<f32>(bind.origin + q.position.xyz, 1.0));
#endif
    return out;
}
