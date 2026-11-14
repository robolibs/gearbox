import bevy_pbr::render::mesh_view_bindings::{view, globals};
import gearbox::draw::draw_cloud_types::{CloudPosition, CloudSample, CloudAge, cloud_project_point, cloud_radius_px, cloud_projected_edge, cloud_clip_accept, CLOUD_RADIUS_VALID};
import gearbox::draw::draw_cloud_color::{style, cloud_sample_color};
import gearbox::draw::draw_common::{srgb_to_linear, display_to_scene, hud_sweep, hud_ladder, hud_iso_distance};
import gearbox::draw::draw_frost::background_frost;

struct CloudPageUniform {
    world_from_local: mat4x4<f32>,
    clip_rotation: mat4x4<f32>,
    clip_min: vec4<f32>,
    clip_max: vec4<f32>,
    age: CloudAge,
    // x: the radius in metres a point is drawn at least; 0 at full detail.
    coverage: vec4<f32>,
};
@group(2) @binding(0) var<uniform> page: CloudPageUniform;
@group(2) @binding(1) var<storage, read> positions: array<CloudPosition>;
@group(2) @binding(2) var<storage, read> samples: array<CloudSample>;
@if((CLOUD_SELECTED))
@group(2) @binding(3) var<storage, read> selected_indices: array<u32>;
@group(3) @binding(1) var lut_texture: texture_3d<f32>;
@group(3) @binding(2) var lut_sampler: sampler;

struct CloudOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) offset_px: vec2<f32>,
    @location(1) @interpolate(flat) radius_px: f32,
    @location(2) @interpolate(flat) rgba: vec4<f32>,
    @location(3) @interpolate(flat) linear_edge: u32,
    // x: the sensor ping's ring over this point, y: its wake, z: on a contour height; zero without glow.
    @location(4) @interpolate(flat) ping: vec3<f32>,
};

// Emission per unit of glow, as a multiple of the point's colour.
const POINT_EMISSION: f32 = 2.0;
// Sensor ping: rings racing outward from the viewer across glowing clouds.
const PING_PERIOD_M: f32 = 60.0;
const PING_SPEED_MPS: f32 = 20.0;
const PING_WAKE_M: f32 = 6.0;
const PING_FRONT_M: f32 = 0.35;
// Topographic hologram: points on contour heights light up, spaced for the view distance.
const CONTOUR_BASE_M: f32 = 0.25;
const CONTOUR_MIN_PX: f32 = 6.0;

@vertex
fn vertex(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) instance_index: u32) -> CloudOutput {
    @if((CLOUD_SELECTED))
    let ordinal = selected_indices[instance_index];
    @if((!(CLOUD_SELECTED)))
    let ordinal = instance_index;
    let point = positions[ordinal];
    let sample = samples[ordinal];
    let world = page.world_from_local * vec4(point.xyz, 1.0);
    let center = view.clip_from_world * world;
    var out: CloudOutput;
    let radius_bits = bitcast<u32>(style.effects.w);
    let pixels = (radius_bits & 0x80000000u) != 0u;
    var metric = select(style.effects.w, 0.0, pixels);
    if (sample.flags & CLOUD_RADIUS_VALID) != 0u { metric = sample.radius_m; }
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
    let metres_per_px = 2.0 * abs(center.w) / max(abs(projection_y) * view.viewport.w, 1e-6);
    let reach = length(world.xz - view.world_position.xz);
    let ring = hud_sweep(reach, globals.time, PING_PERIOD_M, PING_SPEED_MPS, PING_WAKE_M, max(metres_per_px, PING_FRONT_M));
    let spacing = hud_ladder(CONTOUR_BASE_M, metres_per_px, CONTOUR_MIN_PX);
    let band = max(1.2 * metres_per_px, 0.03);
    let contour = 1.0 - smoothstep(0.5 * band, band, hud_iso_distance(world.y + 0.5 * spacing, spacing));
    out.ping = vec3(ring, contour) * min(max(style.effects.y, 0.0) * 4.0, 1.0);
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
    // Glow turns the point into a light source: the whole point emits, its rim a little more.
    let rim = 1.0 - clamp(edge / 1.5, 0.0, 1.0);
    rgb = min(rgb * (1.0 + min(style.effects.y * (POINT_EMISSION + 0.12 * rim), 65504.0)), vec3(65504.0));
    rgb = min(rgb * (1.0 + 3.0 * in.ping.x + 0.8 * in.ping.y + 2.2 * in.ping.z), vec3(65504.0));
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
