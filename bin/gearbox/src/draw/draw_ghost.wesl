import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_world, mesh_normal_local_to_world};
import bevy_pbr::render::mesh_view_bindings::{view, globals};
import gearbox::draw::draw_common::{srgb_to_linear, display_to_scene};

// Ghost robots: a machine's own meshes as a fresnel rim over a faint fill,
// with slow bands rising through them. Light only, added with no alpha.

struct GhostParams {
    color: vec4<f32>,
    // glow, band rate Hz, phase seconds, unused
    fx: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> params: GhostParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var lut_texture: texture_3d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var lut_sampler: sampler;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) @interpolate(flat) color: vec3<f32>,
    @location(3) height: f32,
};

@vertex
fn vertex(v: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world = mesh_position_local_to_world(get_world_from_local(v.instance_index), vec4(v.position, 1.0));
    out.position = view.clip_from_view * view.view_from_world * world;
    out.world_position = world.xyz;
    out.world_normal = mesh_normal_local_to_world(v.normal, v.instance_index);
    out.color = display_to_scene(srgb_to_linear(params.color.rgb), lut_texture, lut_sampler);
    out.height = world.y;
    return out;
}

fn band(x: f32, duty: f32, w: f32) -> f32 {
    let f = fract(x);
    let s = max(w, 1e-4);
    return smoothstep(0.0, s, f) * (1.0 - smoothstep(duty - s, duty, f));
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    var n = normalize(in.world_normal);
    if !front {
        n = -n;
    }
    let to_eye = normalize(view.world_position - in.world_position);
    let ndv = abs(dot(n, to_eye));
    let rim = pow(1.0 - ndv, 2.5);
    let t = globals.time + params.fx.z;
    let x = in.height * 3.0 - t * max(params.fx.y, 0.0);
    let bands = band(x, 0.1, fwidth(x) * 1.5);
    let glow = max(params.fx.x, 0.0);
    let a = (0.05 + 1.1 * rim + 0.07 * bands) * glow;
    let light = in.color * a * params.color.a;
    if dot(light, light) < 1e-8 {
        discard;
    }
    return vec4(light, 0.0);
}
