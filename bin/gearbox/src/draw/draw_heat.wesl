import bevy_pbr::render::mesh_functions::{get_world_from_local, mesh_position_local_to_world};
import bevy_pbr::render::mesh_view_bindings::view;
import gearbox::draw::draw_frost::background_frost;
import gearbox::draw::draw_common::{srgb_to_linear, display_to_scene, technical_line, detail_visibility, ink_highlight, ground_clip, RULE_PX, CORNER_PX};

// Static heat maps with filtered cells, contours and screen-width registration edges.

struct HeatParams {
    frost: vec4<f32>,
    // Texture columns and rows, contour spacing (share of the map), glow.
    grid: vec4<f32>,
    // Fill opacity, hot threshold, hot glow, categorical palette flag.
    look: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> params: HeatParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var lut_texture: texture_3d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var lut_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var cell_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var cell_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var value_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var value_sampler: sampler;

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
    out.position = ground_clip(view.view_from_world * world, view.clip_from_view);
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
    let value = textureSample(value_texture, value_sampler, in.uv).rg;
    let t = value.r / max(value.g, 1e-5);
    let known_edge = (1.0 - smoothstep(0.4, 1.6, abs(value.g - 0.5) / max(fwidth(value.g), 1e-5)))
        * step(0.001, value.g) * (1.0 - step(0.999, value.g));
    if c.a < 0.004 {
        discard;
    }
    let col = display_to_scene(srgb_to_linear(c.rgb / c.a), lut_texture, lut_sampler);
    let alpha = c.a * params.look.x;
    let local_edge = min(fract(pix), 1.0 - fract(pix)) / w;
    let cells = (1.0 - smoothstep(0.2, 0.9, min(local_edge.x, local_edge.y)))
        * (1.0 - smoothstep(0.08, 0.22, max(w.x, w.y)));
    let outer_edge = min(pix, size - pix) / w;
    let frame = technical_line(min(outer_edge.x, outer_edge.y), 0.5 * RULE_PX, 1.0);
    let major_x = pix / 5.0;
    let major_w = fwidth(major_x);
    let major_distance = abs(fract(major_x + 0.5) - 0.5);
    let major = max(technical_line(major_distance.x, 0.006, major_w.x), technical_line(major_distance.y, 0.006, major_w.y))
        * detail_visibility(max(major_w.x, major_w.y));
    let corner = frame * (1.0 - smoothstep(CORNER_PX, 2.0 * CORNER_PX, max(outer_edge.x, outer_edge.y)));
    let structure = max(0.08 * cells, max(0.12 * major, max(0.28 * frame, 0.20 * known_edge)));
    var contour = 0.0;
    if params.grid.z > 0.0 {
        let x = t / params.grid.z;
        let fw = max(fwidth(x), 1e-4);
        let line = technical_line(abs(fract(x + 0.5) - 0.5), 0.012, fw);
        let resolved = (1.0 - smoothstep(0.2, 0.5, fw)) * step(0.999, value.g);
        contour = line * resolved;
    }
    let hot = smoothstep(params.look.y - 0.03, params.look.y + 0.03, t);
    let body = col * (1.0 - structure - 0.16 * contour);
    let light = ink_highlight(col) * (0.06 * frame + 0.08 * corner + params.look.z * hot * (1.0 - params.look.w))
        * c.a * params.grid.w;
    return background_frost(vec4(body * alpha + light, alpha), in.position.xy, params.frost.x);
}
