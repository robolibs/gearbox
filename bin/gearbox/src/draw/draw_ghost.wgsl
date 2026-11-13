// Shaded ghost bodies, crease edges and ghost-local section datums.
#import bevy_pbr::mesh_functions::{get_world_from_local, mesh_position_local_to_world, mesh_normal_local_to_world}
#import bevy_pbr::mesh_view_bindings::{view, globals}
#import gearbox::draw_frost::background_frost
#import gearbox::draw_common::{srgb_to_linear, display_to_scene, technical_line, detail_visibility, energy_packet, energy_glass, energy_glass_shade, STRUCTURE_CORNER_GAIN, BRACKET_FRACTION}

struct GhostParams {
    color: vec4<f32>,
    // glow, band rate Hz, phase seconds, frost strength
    fx: vec4<f32>,
    ghost_from_world: mat4x4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: GhostParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var lut_texture: texture_3d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var lut_sampler: sampler;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
#ifdef VERTEX_NORMALS
    @location(1) normal: vec3<f32>,
#endif
#ifdef GHOST_EDGE_DATA
    @location(2) barycentric: vec3<f32>,
    @location(3) edge_mask: vec3<f32>,
    @location(4) edge_start: vec3<f32>,
    @location(5) edge_end: vec3<f32>,
#endif
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
#ifdef VERTEX_NORMALS
    @location(1) world_normal: vec3<f32>,
#endif
    @location(2) @interpolate(flat) color: vec3<f32>,
    @location(3) local: vec3<f32>,
#ifdef GHOST_EDGE_DATA
    @location(4) barycentric: vec3<f32>,
    @location(5) @interpolate(flat) edge_mask: vec3<f32>,
    @location(6) @interpolate(flat) edge_start: vec3<f32>,
    @location(7) @interpolate(flat) edge_end: vec3<f32>,
#endif
};

@vertex
fn vertex(v: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world = mesh_position_local_to_world(get_world_from_local(v.instance_index), vec4(v.position, 1.0));
    out.position = view.clip_from_view * view.view_from_world * world;
    out.world_position = world.xyz;
#ifdef VERTEX_NORMALS
    out.world_normal = mesh_normal_local_to_world(v.normal, v.instance_index);
#endif
    out.color = display_to_scene(srgb_to_linear(params.color.rgb), lut_texture, lut_sampler);
    out.local = (params.ghost_from_world * world).xyz;
#ifdef GHOST_EDGE_DATA
    out.barycentric = v.barycentric;
    out.edge_mask = v.edge_mask;
    out.edge_start = v.edge_start;
    out.edge_end = v.edge_end;
#endif
    return out;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
#ifdef VERTEX_NORMALS
    var raw_normal = in.world_normal;
#else
    var raw_normal = cross(dpdy(in.world_position), dpdx(in.world_position));
#endif
    var n = raw_normal / max(length(raw_normal), 1e-6);
    if !front {
        n = -n;
    }
    let to_eye = normalize(view.world_position - in.world_position);
    let ndv = abs(dot(n, to_eye));
    let rim = pow(clamp(1.0 - ndv, 0.0, 1.0), 5.0);
    let shade = energy_glass_shade(n, ndv);
    var edges = 0.0;
#ifdef GHOST_EDGE_DATA
    let pixel = max(fwidth(in.barycentric), vec3(1e-5));
    let distance = in.barycentric / pixel;
    let visible = smoothstep(vec3(3.0), vec3(9.0), 1.0 / pixel);
    let mask = vec3<u32>(in.edge_mask);
    let along = in.barycentric.zxy / max(in.barycentric.yzx + in.barycentric.zxy, vec3(1e-5));
    let phase = mix(in.edge_start, in.edge_end, along);
    let local = select(clamp(phase, vec3(0.0), vec3(1.0)), fract(phase), (mask & vec3(8u)) != vec3(0u));
    let aa = max(fwidth(phase), vec3(1e-4));
    let start = (1.0 - smoothstep(vec3(BRACKET_FRACTION) - aa, vec3(BRACKET_FRACTION) + aa, local)) * vec3<f32>((mask >> vec3(1u)) & vec3(1u));
    let end = smoothstep(vec3(1.0 - BRACKET_FRACTION) - aa, vec3(1.0 - BRACKET_FRACTION) + aa, local) * vec3<f32>((mask >> vec3(2u)) & vec3(1u));
    let accent = max(start, end) * (1.0 - smoothstep(vec3(0.08), vec3(0.2), aa));
    let width = vec3(0.85) * mix(vec3(1.0), vec3(STRUCTURE_CORNER_GAIN), accent);
    let edge = (1.0 - smoothstep(width - vec3(0.4), width + vec3(0.4), distance)) * vec3<f32>(mask & vec3(1u)) * visible;
    edges = max(edge.x, max(edge.y, edge.z));
#endif
    let z = in.local.z * 2.0;
    let z_width = fwidth(z);
    let datum = technical_line(abs(fract(z + 0.5) - 0.5), 0.003, z_width) * detail_visibility(z_width);
    let structure = max(edges, 0.28 * datum);
    let hz = max(params.fx.y, 0.0);
    let scan_phase = z * 0.25 - (globals.time + params.fx.z) * hz;
    let scan = energy_packet(scan_phase, 0.018, fwidth(scan_phase), hz);
    let behind = fract(-scan_phase);
    let wake = exp(-behind * 18.0) * smoothstep(0.0, 0.014, behind) * detail_visibility(fwidth(scan_phase)) * select(0.0, 1.0, hz > 0.0);
    let glow = max(params.fx.x, 0.0);
    let activity = scan * (0.35 + 0.65 * structure);
    let glass = energy_glass(in.color, params.color.a, shade, rim, structure, activity, wake, glow);
    if glass.a <= 0.002 {
        discard;
    }
    return background_frost(glass, in.position.xy, params.fx.w);
}
