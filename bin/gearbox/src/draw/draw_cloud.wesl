import bevy_pbr::render::mesh_view_bindings::view;
import gearbox::draw::draw_cloud_types::{CloudPosition, CloudSample, CloudPageUniform, cloud_project_point, cloud_radius_px, cloud_projected_edge, cloud_clip_accept, CLOUD_ALL_VALID};
import gearbox::draw::draw_cloud_color::{style, cloud_sample_color};
import gearbox::draw::draw_common::{srgb_to_linear, display_to_scene};
import gearbox::draw::draw_frost::background_frost;

@group(2) @binding(0) var<uniform> page: CloudPageUniform;
@group(2) @binding(1) var<storage, read> positions: array<CloudPosition>;
@group(2) @binding(2) var<storage, read> attributes: array<u32>;
@if((CLOUD_LOD))
@group(2) @binding(3) var<storage, read> order: array<u32>;
// Per node of the page: (first instance, refinement-order position) the view's LOD pass chose.
@if((CLOUD_LOD))
@group(2) @binding(4) var<storage, read> table: array<vec2<u32>>;
@group(3) @binding(1) var lut_texture: texture_3d<f32>;
@group(3) @binding(2) var lut_sampler: sampler;

// The point an instance draws: the last node starting at or before it, offset into its range.
@if((CLOUD_LOD))
fn lod_ordinal(instance: u32) -> u32 {
    var low = 0u;
    var high = max(page.tree.x, 1u);
    while low + 1u < high {
        let middle = (low + high) / 2u;
        if table[middle].x <= instance { low = middle; } else { high = middle; }
    }
    return order[table[low].y + instance - table[low].x];
}

struct CloudOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) offset_px: vec2<f32>,
    @location(1) @interpolate(flat) radius_px: f32,
    @location(2) @interpolate(flat) rgba: vec4<f32>,
    @location(3) @interpolate(flat) linear_edge: u32,
};

@vertex
fn vertex(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) instance_index: u32) -> CloudOutput {
    @if((CLOUD_LOD))
    let ordinal = lod_ordinal(instance_index);
    @if((!(CLOUD_LOD)))
    let ordinal = instance_index;
    let stored = positions[ordinal];
    let local = vec3(stored.x, stored.y, stored.z);
    let packing = page.attributes;
    let base = ordinal * packing.x;
    let colors = packing.x - packing.z;
    var sample = CloudSample(packing.y, 1u, vec2(0u));
    if colors > 0u { sample.words.x = attributes[base]; }
    if colors > 1u { sample.words.y = attributes[base + 1u]; }
    if packing.w != CLOUD_ALL_VALID { sample.valid = (attributes[packing.w + ordinal / 32u] >> (ordinal % 32u)) & 1u; }
    let world = page.world_from_local * vec4(local, 1.0);
    let center = view.clip_from_world * world;
    var out: CloudOutput;
    let radius_bits = bitcast<u32>(style.effects.w);
    let pixels = (radius_bits & 0x80000000u) != 0u;
    var metric = select(style.effects.w, 0.0, pixels);
    if packing.z != 0u {
        let radius = bitcast<f32>(attributes[base + colors]);
        if radius > 0.0 { metric = radius; }
    }
    var pixel_radius = select(0.0, bitcast<f32>(radius_bits & 0x7fffffffu), pixels);
    // A coarse record grows to the spacing it stands for when that is larger.
    let projection_y = view.clip_from_view[1][1];
    let drawn_px = cloud_radius_px(metric, pixel_radius, projection_y, view.viewport.w, center.w);
    if cloud_radius_px(page.coverage.x, 0.0, projection_y, view.viewport.w, center.w) > drawn_px {
        metric = page.coverage.x;
        pixel_radius = 0.0;
    }
    let projected = cloud_project_point(center, view.viewport.zw, metric, pixel_radius, projection_y, vertex_index);
    out.position = projected.position;
    out.offset_px = projected.offset_px;
    out.radius_px = projected.radius_px;
    out.linear_edge = projected.linear_edge;
    out.rgba = cloud_sample_color(sample, page.age);
    if !(style.controls.x == 1u && style.controls.y == 1u) { out.rgba = vec4(srgb_to_linear(out.rgba.rgb), out.rgba.a); }
    out.rgba = vec4(display_to_scene(out.rgba.rgb, lut_texture, lut_sampler), out.rgba.a);
    if center.w <= 0.0 || center.z < 0.0 || center.z > center.w ||
        !cloud_clip_accept(page.clip_rotation, page.clip_min, page.clip_max, local) {
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
    @if((CLOUD_OPAQUE))
    if opacity < 1.0 { discard; }
    @if((CLOUD_OPAQUE) && (CLOUD_SINGLE_SAMPLE))
    if coverage < 1.0 { discard; }
    @if((CLOUD_OPAQUE))
    return vec4(rgb, alpha);
    @if((!(CLOUD_OPAQUE)))
    if opacity >= 1.0 {
        @if((CLOUD_SINGLE_SAMPLE))
        if coverage >= 1.0 { discard; }
        @if((CLOUD_SINGLE_SAMPLE))
        return vec4(rgb * coverage, coverage);
        @if((!(CLOUD_SINGLE_SAMPLE)))
        discard;
    }
    @if((!(CLOUD_OPAQUE)))
    return background_frost(vec4(rgb * alpha, alpha), in.position.xy, style.effects.z);
}
