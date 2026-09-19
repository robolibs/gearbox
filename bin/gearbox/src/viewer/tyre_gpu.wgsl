#import "embedded://gearbox/viewer/tyre_deform.wgsl"::{deform_contact, safe_normal}
#import bevy_pbr::{mesh_functions, view_transformations::position_world_to_clip}
#ifdef PREPASS_PIPELINE
#import bevy_pbr::prepass_io::{Vertex, VertexOutput}
#ifdef NORMAL_PREPASS_OR_DEFERRED_PREPASS
#define TYRE_NORMALS
#endif
#else
#import bevy_pbr::forward_io::{Vertex, VertexOutput}
#define TYRE_NORMALS
#endif

struct TyreFrame {
    to_contact: mat4x4<f32>,
    from_contact: mat4x4<f32>,
    envelope: vec4<f32>,
    ground: vec4<f32>,
}
struct TyreUniform { current: TyreFrame, previous: TyreFrame }
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<storage, read> tyre: TyreUniform;

fn deform_local(point: vec3<f32>, frame: TyreFrame) -> vec3<f32> {
    let contact = (frame.to_contact * vec4(point, 1.0)).xyz;
    return (frame.from_contact * vec4(deform_contact(contact, frame.envelope, frame.ground), 1.0)).xyz;
}

@vertex
fn vertex(input: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(input.instance_index);
    let position = deform_local(input.position, tyre.current);
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4(position, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);

#ifdef TYRE_NORMALS
#ifdef VERTEX_NORMALS
    let to_frame = mat3x3(tyre.current.to_contact[0].xyz, tyre.current.to_contact[1].xyz, tyre.current.to_contact[2].xyz);
    let from_frame = mat3x3(tyre.current.from_contact[0].xyz, tyre.current.from_contact[1].xyz, tyre.current.from_contact[2].xyz);
    let point = (tyre.current.to_contact * vec4(input.position, 1.0)).xyz;
    let normal = safe_normal(transpose(from_frame) * input.normal, vec3(0.0, 1.0, 0.0));
    var axis = vec3(1.0, 0.0, 0.0);
    if abs(normal.x) > 0.9 { axis = vec3(0.0, 0.0, 1.0); }
    var tangent = safe_normal(cross(axis, normal), vec3(0.0, 0.0, 1.0));
#ifdef VERTEX_TANGENTS
    tangent = safe_normal(to_frame * input.tangent.xyz, tangent);
#endif
    let bitangent = cross(normal, tangent);
    let epsilon = max(tyre.current.envelope.x * 0.0001, 0.00001);
    let center = deform_contact(point, tyre.current.envelope, tyre.current.ground);
    let du = (deform_contact(point + tangent * epsilon, tyre.current.envelope, tyre.current.ground) - center) / epsilon;
    let dv = (deform_contact(point + bitangent * epsilon, tyre.current.envelope, tyre.current.ground) - center) / epsilon;
    let deformed_normal = safe_normal(cross(du, dv), normal);
    let local_normal = safe_normal(transpose(to_frame) * deformed_normal, input.normal);
    out.world_normal = mesh_functions::mesh_normal_local_to_world(local_normal, input.instance_index);
#ifdef VERTEX_TANGENTS
    let local_tangent = safe_normal(from_frame * du, input.tangent.xyz);
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(world_from_local, vec4(local_tangent, input.tangent.w), input.instance_index);
#endif
#endif
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
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = input.instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(input.instance_index, world_from_local[3]);
#endif
#ifdef MOTION_VECTOR_PREPASS
    let previous_model = mesh_functions::get_previous_world_from_local(input.instance_index);
    out.previous_world_position = mesh_functions::mesh_position_local_to_world(previous_model, vec4(deform_local(input.position, tyre.previous), 1.0));
#endif
#ifdef UNCLIPPED_DEPTH_ORTHO_EMULATION
    out.unclipped_depth = out.position.z;
    out.position.z = min(out.position.z, 1.0);
#endif
    return out;
}
