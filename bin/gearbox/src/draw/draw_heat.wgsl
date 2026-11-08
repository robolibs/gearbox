// Heat maps: a grid's cells from a texture, crisp cell edges softened over
// one pixel, contour lines from the smooth value, the hottest cells adding
// light. Colour leaves premultiplied in the colour asked for once tonemapped.
#import bevy_pbr::mesh_functions::{get_world_from_local, mesh_position_local_to_world}
#import bevy_pbr::mesh_view_bindings::view
#import gearbox::draw_common::{srgb_to_linear, display_to_scene}

struct HeatParams {
    // Texture columns and rows, contour spacing (share of the map), glow.
    grid: vec4<f32>,
    // Fill opacity, hot threshold, hot glow, unused.
    look: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: HeatParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var lut_texture: texture_3d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var lut_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var cell_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var cell_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var value_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var value_sampler: sampler;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world = mesh_position_local_to_world(get_world_from_local(v.instance_index), vec4(v.position, 1.0));
    // Moved up to 0.3 m towards the eye along its ray, above short grass.
    var at_eye = view.view_from_world * world;
    let d = length(at_eye.xyz);
    at_eye = vec4(at_eye.xyz * (1.0 - min(0.3, 0.1 * d) / max(d, 1e-4)), 1.0);
    out.position = view.clip_from_view * at_eye;
    out.uv = v.uv;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let size = params.grid.xy;
    let pix = in.uv * size;
    let w = max(fwidth(pix), vec2(1e-5));
    let snapped = (floor(pix) + 0.5 + clamp((fract(pix) - 0.5) / w, vec2(-0.5), vec2(0.5))) / size;
    let c = textureSample(cell_texture, cell_sampler, snapped);
    let t = textureSample(value_texture, value_sampler, in.uv).r;
    if c.a < 0.004 {
        discard;
    }
    let col = display_to_scene(srgb_to_linear(c.rgb / c.a), lut_texture, lut_sampler);
    let alpha = c.a * params.look.x;
    var light = vec3(0.0);
    if params.grid.z > 0.0 {
        let x = t / params.grid.z;
        let fw = max(fwidth(x), 1e-4);
        let line = 1.0 - smoothstep(0.0, fw * 1.5, abs(fract(x + 0.5) - 0.5) - fw * 0.3);
        light = light + mix(col, vec3(1.0), 0.5) * 0.55 * line * c.a;
    }
    let hot = smoothstep(params.look.y - 0.03, params.look.y + 0.03, t);
    light = light + col * params.look.z * hot * c.a * params.grid.w;
    return vec4(col * alpha + light, alpha);
}
