#import bevy_pbr::{
    mesh_functions,
    forward_io::{Vertex, VertexOutput},
    view_transformations::position_world_to_clip,
}
#import bevy_pbr::mesh_view_bindings::globals

// x: sway amplitude (m), y: speed, z/w: wind direction on the ground.
@group(#{MATERIAL_BIND_GROUP}) @binding(100)
var<uniform> wind: vec4<f32>;

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    var world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));

    // Blades bend from the root (uv.y = 0) to the tip (uv.y = 1); the
    // phase drifts across the field so gusts travel instead of pulsing.
#ifdef VERTEX_UVS_A
    let t = vertex.uv.y;
#else
    let t = 0.0;
#endif
    let phase = world_position.x * 0.31 + world_position.z * 0.23;
    let gust = sin(globals.time * wind.y + phase) * 0.6
        + sin(globals.time * wind.y * 2.7 + phase * 2.1) * 0.25
        + sin(globals.time * 0.37 + phase * 0.11) * 0.15;
    let bend = t * t * wind.x * (0.55 + gust);
    world_position = vec4<f32>(
        world_position.x + bend * wind.z,
        world_position.y - abs(bend) * 0.15,
        world_position.z + bend * wind.w,
        1.0,
    );

    out.world_position = world_position;
    out.position = position_world_to_clip(world_position.xyz);
#ifdef VERTEX_NORMALS
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);
#endif
#ifdef VERTEX_UVS_A
    out.uv = vertex.uv;
#endif
#ifdef VERTEX_COLORS
    out.color = vertex.color;
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        vertex.instance_index, world_from_local[3]);
#endif
    return out;
}
