#import bevy_pbr::mesh_view_bindings::view
#import gearbox::draw_cloud_types::{CloudPosition, CloudSample, CloudAge, cloud_project_point, cloud_projected_edge, cloud_clip_accept, CLOUD_RADIUS_VALID}
#import gearbox::draw_cloud_color::{style, cloud_sample_color}
#import gearbox::draw_common::{srgb_to_linear, display_to_scene}
#import gearbox::draw_frost::background_frost

struct CloudPageUniform {
    world_from_local: mat4x4<f32>,
    clip_rotation: mat4x4<f32>,
    clip_min: vec4<f32>,
    clip_max: vec4<f32>,
    age: CloudAge,
};
@group(2) @binding(0) var<uniform> page: CloudPageUniform;
@group(2) @binding(1) var<storage, read> positions: array<CloudPosition>;
@group(2) @binding(2) var<storage, read> samples: array<CloudSample>;
#ifdef CLOUD_SELECTED
@group(2) @binding(3) var<storage, read> selected_indices: array<u32>;
#endif
@group(3) @binding(1) var lut_texture: texture_3d<f32>;
@group(3) @binding(2) var lut_sampler: sampler;

struct CloudOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) offset_px: vec2<f32>,
    @location(1) @interpolate(flat) radius_px: f32,
    @location(2) @interpolate(flat) rgba: vec4<f32>,
    @location(3) @interpolate(flat) linear_edge: u32,
};

@vertex
fn vertex(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) instance_index: u32) -> CloudOutput {
#ifdef CLOUD_SELECTED
    let ordinal = selected_indices[instance_index];
#else
    let ordinal = instance_index;
#endif
    let point = positions[ordinal];
    let sample = samples[ordinal];
    let world = page.world_from_local * vec4(point.xyz, 1.0);
    let center = view.clip_from_world * world;
    var out: CloudOutput;
    let radius_bits = bitcast<u32>(style.effects.w);
    let pixels = (radius_bits & 0x80000000u) != 0u;
    var metric = select(style.effects.w, 0.0, pixels);
    if (sample.flags & CLOUD_RADIUS_VALID) != 0u { metric = sample.radius_m; }
    let projected = cloud_project_point(center, view.viewport.zw, metric,
        select(0.0, bitcast<f32>(radius_bits & 0x7fffffffu), pixels), view.clip_from_view[1][1], vertex_index);
    out.position = projected.position;
    out.offset_px = projected.offset_px;
    out.radius_px = projected.radius_px;
    out.linear_edge = projected.linear_edge;
    out.rgba = cloud_sample_color(sample, page.age);
    if !(style.controls.x == 1u && style.controls.y == 1u) { out.rgba = vec4(srgb_to_linear(out.rgba.rgb), out.rgba.a); }
    out.rgba = vec4(display_to_scene(out.rgba.rgb, lut_texture, lut_sampler), out.rgba.a);
    if center.w <= 0.0 || center.z < 0.0 || center.z > center.w ||
        !cloud_clip_accept(page.clip_rotation, page.clip_min, page.clip_max, point.xyz) {
        out.position = vec4(2.0, 2.0, 2.0, 1.0);
        out.rgba = vec4(0.0);
    }
    return out;
}

@fragment
fn fragment(in: CloudOutput) -> @location(0) vec4<f32> {
    let edge = cloud_projected_edge(in.offset_px, in.radius_px, in.linear_edge);
    let coverage = clamp(edge + 0.5, 0.0, 1.0);
    let opacity = in.rgba.a * style.effects.x;
    let alpha = opacity * coverage;
    if alpha <= 0.0 { discard; }
    var rgb = in.rgba.rgb;
    let rim = 1.0 - clamp(edge / 1.5, 0.0, 1.0);
    rgb = min(rgb * (1.0 + min(style.effects.y * 0.12 * rim, 65504.0)), vec3(65504.0));
#ifdef CLOUD_OPAQUE
    if opacity < 1.0 { discard; }
#ifdef CLOUD_SINGLE_SAMPLE
    if coverage < 1.0 { discard; }
#endif
    return vec4(rgb, alpha);
#else
    if opacity >= 1.0 {
#ifdef CLOUD_SINGLE_SAMPLE
        if coverage >= 1.0 { discard; }
        return vec4(rgb * coverage, coverage);
#else
        discard;
#endif
    }
    return background_frost(vec4(rgb * alpha, alpha), in.position.xy, style.effects.z);
#endif
}
