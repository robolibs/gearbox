import gearbox::viewer::tyre_deform::{deform_contact, safe_normal};
import bevy_pbr::render::{mesh_functions, view_transformations::position_world_to_clip};
@if((PREPASS_PIPELINE))
import bevy_pbr::prepass::io::{Vertex, VertexOutput};
@if((!(PREPASS_PIPELINE)))
import bevy_pbr::render::forward_io::{Vertex, VertexOutput};


struct TyreFrame {
    to_contact: mat4x4<f32>,
    from_contact: mat4x4<f32>,
    envelope: vec4<f32>,
    ground: vec4<f32>,
}
struct TyreUniform { current: TyreFrame, previous: TyreFrame }
@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<storage, read> tyre: TyreUniform;

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

    @if((!PREPASS_PIPELINE || NORMAL_PREPASS_OR_DEFERRED_PREPASS) && (VERTEX_NORMALS))
    let to_frame = mat3x3(tyre.current.to_contact[0].xyz, tyre.current.to_contact[1].xyz, tyre.current.to_contact[2].xyz);
    @if((!PREPASS_PIPELINE || NORMAL_PREPASS_OR_DEFERRED_PREPASS) && (VERTEX_NORMALS))
    let from_frame = mat3x3(tyre.current.from_contact[0].xyz, tyre.current.from_contact[1].xyz, tyre.current.from_contact[2].xyz);
    @if((!PREPASS_PIPELINE || NORMAL_PREPASS_OR_DEFERRED_PREPASS) && (VERTEX_NORMALS))
    let point = (tyre.current.to_contact * vec4(input.position, 1.0)).xyz;
    @if((!PREPASS_PIPELINE || NORMAL_PREPASS_OR_DEFERRED_PREPASS) && (VERTEX_NORMALS))
    let normal = safe_normal(transpose(from_frame) * input.normal, vec3(0.0, 1.0, 0.0));
    @if((!PREPASS_PIPELINE || NORMAL_PREPASS_OR_DEFERRED_PREPASS) && (VERTEX_NORMALS))
    var axis = vec3(1.0, 0.0, 0.0);
    @if((!PREPASS_PIPELINE || NORMAL_PREPASS_OR_DEFERRED_PREPASS) && (VERTEX_NORMALS))
    if abs(normal.x) > 0.9 { axis = vec3(0.0, 0.0, 1.0); }
    @if((!PREPASS_PIPELINE || NORMAL_PREPASS_OR_DEFERRED_PREPASS) && (VERTEX_NORMALS))
    var tangent = safe_normal(cross(axis, normal), vec3(0.0, 0.0, 1.0));
    @if((!PREPASS_PIPELINE || NORMAL_PREPASS_OR_DEFERRED_PREPASS) && (VERTEX_NORMALS) && (VERTEX_TANGENTS))
    tangent = safe_normal(to_frame * input.tangent.xyz, tangent);
    @if((!PREPASS_PIPELINE || NORMAL_PREPASS_OR_DEFERRED_PREPASS) && (VERTEX_NORMALS))
    let bitangent = cross(normal, tangent);
    @if((!PREPASS_PIPELINE || NORMAL_PREPASS_OR_DEFERRED_PREPASS) && (VERTEX_NORMALS))
    let epsilon = max(tyre.current.envelope.x * 0.0001, 0.00001);
    @if((!PREPASS_PIPELINE || NORMAL_PREPASS_OR_DEFERRED_PREPASS) && (VERTEX_NORMALS))
    let center = deform_contact(point, tyre.current.envelope, tyre.current.ground);
    @if((!PREPASS_PIPELINE || NORMAL_PREPASS_OR_DEFERRED_PREPASS) && (VERTEX_NORMALS))
    let du = (deform_contact(point + tangent * epsilon, tyre.current.envelope, tyre.current.ground) - center) / epsilon;
    @if((!PREPASS_PIPELINE || NORMAL_PREPASS_OR_DEFERRED_PREPASS) && (VERTEX_NORMALS))
    let dv = (deform_contact(point + bitangent * epsilon, tyre.current.envelope, tyre.current.ground) - center) / epsilon;
    @if((!PREPASS_PIPELINE || NORMAL_PREPASS_OR_DEFERRED_PREPASS) && (VERTEX_NORMALS))
    let deformed_normal = safe_normal(cross(du, dv), normal);
    @if((!PREPASS_PIPELINE || NORMAL_PREPASS_OR_DEFERRED_PREPASS) && (VERTEX_NORMALS))
    let local_normal = safe_normal(transpose(to_frame) * deformed_normal, input.normal);
    @if((!PREPASS_PIPELINE || NORMAL_PREPASS_OR_DEFERRED_PREPASS) && (VERTEX_NORMALS))
    out.world_normal = mesh_functions::mesh_normal_local_to_world(local_normal, input.instance_index);
    @if((!PREPASS_PIPELINE || NORMAL_PREPASS_OR_DEFERRED_PREPASS) && (VERTEX_NORMALS) && (VERTEX_TANGENTS))
    let local_tangent = safe_normal(from_frame * du, input.tangent.xyz);
    @if((!PREPASS_PIPELINE || NORMAL_PREPASS_OR_DEFERRED_PREPASS) && (VERTEX_NORMALS) && (VERTEX_TANGENTS))
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(world_from_local, vec4(local_tangent, input.tangent.w), input.instance_index);
    @if((VERTEX_UVS_A))
    out.uv = input.uv;
    @if((VERTEX_UVS_B))
    out.uv_b = input.uv_b;
    @if((VERTEX_COLORS))
    out.color = input.color;
    @if((VERTEX_OUTPUT_INSTANCE_INDEX))
    out.instance_index = input.instance_index;
    @if((VISIBILITY_RANGE_DITHER))
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(input.instance_index, world_from_local[3]);
    @if((MOTION_VECTOR_PREPASS))
    let previous_model = mesh_functions::get_previous_world_from_local(input.instance_index);
    @if((MOTION_VECTOR_PREPASS))
    out.previous_world_position = mesh_functions::mesh_position_local_to_world(previous_model, vec4(deform_local(input.position, tyre.previous), 1.0));
    @if((UNCLIPPED_DEPTH_ORTHO_EMULATION))
    out.unclipped_depth = out.position.z;
    @if((UNCLIPPED_DEPTH_ORTHO_EMULATION))
    out.position.z = min(out.position.z, 1.0);
    return out;
}
