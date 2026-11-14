@if((!(DRAW_COMPACT_POINTS))) import bevy_pbr::render::mesh_functions::get_world_from_local;
import bevy_pbr::render::mesh_view_bindings::{view, globals};
import gearbox::draw::draw_frost::background_frost;
import gearbox::draw::draw_common::{srgb_to_linear, display_to_scene, technical_line, detail_visibility, ink_highlight, compact_halo_px, energy_packet, energy_coat, ground_clip, HALO_GAIN};

// Strokes: every segment and point is a quad around its capsule, facing the
// camera or lying flat in the entity's XY plane, built here from the
// segment's ends. Joints are cut on the bisector of the two segments: a
// strip covers each pixel once. The fragment stage cuts the capsule out
// with a one-pixel coverage edge and paints the entity's style; colour
// leaves premultiplied, light added with no alpha.

struct StrokeParams {
    unused: vec4<f32>,
    @if((DRAW_COMPACT_POINTS))
    world_from_local: mat4x4<f32>,
    @if((DRAW_COMPACT_POINTS))
    fx: vec4<f32>,
    @if((DRAW_COMPACT_POINTS))
    fx2: vec4<f32>,
    @if((DRAW_COMPACT_POINTS))
    code: vec4<u32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> params: StrokeParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var lut_texture: texture_3d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var lut_sampler: sampler;

@if((DRAW_COMPACT_POINTS))
struct CompactPointSample { rgba: u32, radius: f32, phase: f32 };
@if((DRAW_COMPACT_POINTS))
struct CompactPointPosition { x: f32, y: f32, z: f32 };
@if((DRAW_COMPACT_POINTS))
@group(2) @binding(0) var<storage, read> compact_positions: array<CompactPointPosition>;
@if((DRAW_COMPACT_POINTS))
@group(2) @binding(1) var<storage, read> compact_samples: array<CompactPointSample>;

const FLAG_TUBE: u32 = 1u;
const FLAG_SPHERE: u32 = 2u;
const FLAG_FLAT_START: u32 = 4u;
const FLAG_FLAT_END: u32 = 8u;
const FLAG_DASHED: u32 = 16u;
const FLAG_FLAT: u32 = 32u;
const FLAG_ADD: u32 = 64u;
const FLAG_HEAD: u32 = 128u;
const FLAG_SQUARE: u32 = 256u;
const FLAG_TERRAIN: u32 = 512u;

const STYLE_RIBBON_FLOW: u32 = 2u;
const STYLE_TUBE_PULSE: u32 = 3u;
const STYLE_HAIRLINE: u32 = 4u;
const STYLE_LIGHT_WALL: u32 = 5u;
const STYLE_HOLO: u32 = 7u;
const STYLE_ALERT: u32 = 9u;
const STYLE_ORB: u32 = 10u;
const STYLE_ARROW_GLOW: u32 = 12u;
const STYLE_POINT_GLOW: u32 = 13u;
const STYLE_SPLAT: u32 = 16u;
const STYLE_SWEEP: u32 = 17u;
const STYLE_RULER: u32 = 19u;

const AA_MARGIN: f32 = 1.5;
const MIN_RADIUS_PX: f32 = 0.75;
const TAU: f32 = 6.2831853;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) a: vec3<f32>,
    @location(1) b: vec3<f32>,
    @location(2) color_a: vec4<f32>,
    @location(3) color_b: vec4<f32>,
    @location(4) radii: vec2<f32>,
    @location(5) dist: vec2<f32>,
    @location(6) code: u32,
    // glow, pulse Hz, flow m/s, spacing m
    @location(7) fx: vec4<f32>,
    // fade, strip length m, urgency, phase
    @location(8) fx2: vec4<f32>,
    // The strip's point before a and after b; equal to a or b at its ends.
    @location(9) prev: vec3<f32>,
    @location(10) next: vec3<f32>,
    @if((DRAW_TERRAIN))
    @location(11) terrain: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    // Screen: a and b in pixels. Flat: segment length in metres.
    @location(0) @interpolate(flat) seg: vec4<f32>,
    // Radius at a and b (pixels, or metres when flat), thin-line fade, halo.
    @location(1) @interpolate(flat) radii: vec4<f32>,
    @location(2) @interpolate(flat) color_a: vec4<f32>,
    @location(3) @interpolate(flat) color_b: vec4<f32>,
    // Arc length at a and b, radius at a and b, in metres.
    @location(4) @interpolate(flat) dist: vec4<f32>,
    @location(5) @interpolate(flat) code: u32,
    // View z and world radius at a; clip w at both ends.
    @location(6) @interpolate(flat) sphere: vec4<f32>,
    @location(7) @interpolate(flat) fx: vec4<f32>,
    @location(8) @interpolate(flat) fx2: vec4<f32>,
    // Bisector normals at a and b in the segment's (across, along) frame.
    @location(9) @interpolate(flat) joint: vec4<f32>,
    // Flat: across and along the segment from a, metres.
    @location(10) ribbon: vec2<f32>,
};

struct FragmentOutput {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

fn is_ortho() -> bool {
    return view.clip_from_view[3][3] == 1.0;
}

fn px_per_unit(w: f32) -> f32 {
    return 0.5 * view.viewport.w * view.clip_from_view[1][1] / max(w, 1e-6);
}

fn ndc_to_px(ndc: vec2<f32>) -> vec2<f32> {
    return (ndc * vec2(0.5, -0.5) + 0.5) * view.viewport.zw + view.viewport.xy;
}

fn px_to_ndc(px: vec2<f32>) -> vec2<f32> {
    return ((px - view.viewport.xy) / view.viewport.zw - 0.5) * vec2(2.0, -2.0);
}

fn depth_of(view_z: f32) -> f32 {
    let c = view.clip_from_view * vec4(0.0, 0.0, view_z, 1.0);
    return c.z / c.w;
}

fn screen_of(world: vec3<f32>) -> vec2<f32> {
    var v = view.view_from_world * vec4(world, 1.0);
    v.z = min(v.z, -1e-3);
    let c = view.clip_from_view * v;
    return ndc_to_px(c.xy / c.w);
}

// Bisector normal of a joint from `dir_in` to `dir_out` in the frame
// (`across`, `along`); zero when there is no joint or it folds back.
fn bisector(dir_in: vec2<f32>, dir_out: vec2<f32>, across: vec2<f32>, along: vec2<f32>, joined: bool) -> vec2<f32> {
    let sum = dir_in + dir_out;
    if !joined || dot(dir_in, dir_out) < -0.85 || dot(sum, sum) < 1e-8 {
        return vec2(0.0);
    }
    let n = normalize(sum);
    return vec2(dot(n, across), dot(n, along));
}

// Halo reach in pixels of a stroke `radius` pixels wide with `glow`.
fn halo_px(radius: f32, glow: f32) -> f32 {
    return compact_halo_px(radius, glow);
}

@if((DRAW_COMPACT_POINTS))
@vertex
fn vertex(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) instance_index: u32) -> VertexOutput {
    let corners = array<u32, 6>(0u, 1u, 3u, 0u, 3u, 2u);
    let stored = compact_positions[instance_index];
    let point = vec3(stored.x, stored.y, stored.z);
    let sample = compact_samples[instance_index];
    var v: Vertex;
    v.a = point;
    v.b = point;
    v.prev = point;
    v.next = point;
    v.color_a = unpack4x8unorm(sample.rgba);
    v.color_b = v.color_a;
    v.radii = vec2(sample.radius);
    v.code = params.code.x | corners[vertex_index];
    v.fx = params.fx;
    v.fx2 = vec4(params.fx2.xyz, sample.phase);
    return stroke_vertex(v, params.world_from_local);
}
@if((!(DRAW_COMPACT_POINTS)))
@vertex
fn vertex(v: Vertex) -> VertexOutput {
    return stroke_vertex(v, get_world_from_local(v.instance_index));
}

fn stroke_vertex(v: Vertex, world_from_local: mat4x4<f32>) -> VertexOutput {
    var out: VertexOutput;
    let corner = v.code & 3u;
    let flags = (v.code >> 2u) & 0xfffu;
    let flat = (flags & FLAG_FLAT) != 0u;
    let style = (v.code >> 14u) & 0x3fu;
    let at_b = (corner & 1u) == 1u;
    let side = select(-1.0, 1.0, (corner & 2u) == 2u);
    out.code = v.code >> 2u;
    out.color_a = vec4(display_to_scene(srgb_to_linear(v.color_a.rgb), lut_texture, lut_sampler), v.color_a.a);
    out.color_b = vec4(display_to_scene(srgb_to_linear(v.color_b.rgb), lut_texture, lut_sampler), v.color_b.a);
    out.fx = v.fx;
    out.fx2 = v.fx2;

    let wa = (world_from_local * vec4(v.a, 1.0)).xyz;
    let wb = (world_from_local * vec4(v.b, 1.0)).xyz;
    let wp = (world_from_local * vec4(v.prev, 1.0)).xyz;
    let wn = (world_from_local * vec4(v.next, 1.0)).xyz;
    var va = view.view_from_world * vec4(wa, 1.0);
    var vb = view.view_from_world * vec4(wb, 1.0);
    var interval = vec2(0.0, 1.0);

    // Segment clipped to the near plane.
    let limit = -max(view.clip_from_view[3][2], 1e-4) * 1.001;
    if !is_ortho() {
        if va.z > limit && vb.z > limit {
            out.position = vec4(0.0, 0.0, -2.0, 1.0);
            return out;
        }
        if va.z > limit {
            interval.x = (limit - va.z) / (vb.z - va.z);
        }
        if vb.z > limit {
            interval.y = (limit - va.z) / (vb.z - va.z);
        }
        let start = va;
        va = mix(start, vb, interval.x);
        vb = mix(start, vb, interval.y);
    }
    let ca = view.clip_from_view * va;
    let cb = view.clip_from_view * vb;
    let ppu_a = px_per_unit(ca.w);
    let ppu_b = px_per_unit(cb.w);
    let clipped = select(interval, vec2(0.0, 1.0), flat);
    let color_a = out.color_a;
    out.color_a = mix(color_a, out.color_b, clipped.x);
    out.color_b = mix(color_a, out.color_b, clipped.y);
    let arc = vec2(mix(v.dist.x, v.dist.y, clipped.x), mix(v.dist.x, v.dist.y, clipped.y));

    // Radii in pixels: positive is metres, negative pixels.
    var ra = mix(select(-v.radii.x, v.radii.x * ppu_a, v.radii.x > 0.0),
        select(-v.radii.y, v.radii.y * ppu_a, v.radii.y > 0.0), clipped.x);
    var rb = mix(select(-v.radii.x, v.radii.x * ppu_b, v.radii.x > 0.0),
        select(-v.radii.y, v.radii.y * ppu_b, v.radii.y > 0.0), clipped.y);
    var fade = 1.0;
    let widest = max(ra, rb);
    if widest < MIN_RADIUS_PX {
        let k = MIN_RADIUS_PX / max(widest, 1e-4);
        fade = widest / MIN_RADIUS_PX;
        ra = ra * k;
        rb = rb * k;
    }
    let glows = style != STYLE_SPLAT && style != STYLE_HAIRLINE;
    let halo = select(0.0, halo_px(max(ra, rb), v.fx.x), glows);
    out.dist = vec4(arc, ra / ppu_a, rb / ppu_b);
    out.sphere = vec4(va.z, ra / ppu_a, ca.w, cb.w);
    let has_prev = distance(v.prev, v.a) > 1e-6 && clipped.x == 0.0;
    let has_next = distance(v.next, v.b) > 1e-6 && clipped.y == 1.0;

    if flat {
        // A ribbon in the entity's XY plane, sized in metres.
        let up = normalize((world_from_local * vec4(0.0, 0.0, 1.0, 0.0)).xyz);
        let flat_dir = in_plane(wb - wa, up);
        let along = select(normalize((world_from_local * vec4(1.0, 0.0, 0.0, 0.0)).xyz), flat_dir.xyz, flat_dir.w > 1e-5);
        let len = flat_dir.w;
        let across = cross(up, along);
        let tangent = select(along, (wb - wa) / max(distance(wa, wb), 1e-6), distance(wa, wb) > 1e-6);
        let normal = normalize(cross(across, tangent));
        let ray = select(normalize(view.world_position.xyz - 0.5 * (wa + wb)), view.world_from_view[2].xyz, is_ortho());
        let margin_mpp = max(1.0 / ppu_a, 1.0 / ppu_b) / max(abs(dot(normal, ray)), 0.06);
        let incoming = in_plane(wa - wp, up);
        let outgoing = in_plane(wn - wb, up);
        let d_in = incoming.xyz;
        let d_out = outgoing.xyz;
        out.joint = vec4(
            bisector(vec2(dot(d_in, across), dot(d_in, along)), vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(0.0, 1.0), has_prev && incoming.w > 1e-6),
            bisector(vec2(0.0, 1.0), vec2(dot(d_out, across), dot(d_out, along)), vec2(1.0, 0.0), vec2(0.0, 1.0), has_next && outgoing.w > 1e-6),
        );
        @if((DRAW_TERRAIN))
        if (flags & FLAG_TERRAIN) != 0u {
            let radius = max(v.radii.x, v.radii.y);
            let margin = (AA_MARGIN + halo) * margin_mpp;
            var start = select(v.radii.x, 0.0, (flags & FLAG_FLAT_START) != 0u);
            var end = select(v.radii.y, 0.0, (flags & FLAG_FLAT_END) != 0u);
            if out.joint.y > 0.0 { start = radius * abs(out.joint.x) / out.joint.y; }
            if out.joint.w > 0.0 { end = radius * abs(out.joint.z) / out.joint.w; }
            let reach = mix(-start, len + end, v.terrain.y) + (2.0 * v.terrain.y - 1.0) * margin;
            let x = v.terrain.x * (radius + margin);
            let t = select(0.0, reach / max(len, 1e-6), len > 1e-6);
            let centre = mix(wa, wb, t);
            let world = centre + across * x + up * (0.004 + v.terrain.z + v.terrain.w * margin);
            out.position = ground_clip(view.view_from_world * vec4(world, 1.0), view.clip_from_view);
            out.ribbon = vec2(x, reach);
            out.seg = vec4(len, 0.0, 0.0, 0.0);
            out.radii = vec4(v.radii, 1.0, halo);
            out.dist = vec4(v.dist, v.radii);
            return out;
        }
        let ra_m = ra / ppu_a;
        let rb_m = rb / ppu_b;
        let margin = (AA_MARGIN + halo) * margin_mpp;
        let half_width = max(ra_m, rb_m) + margin;
        var reach: f32;
        if at_b {
            reach = len + select(rb_m + margin, margin, (flags & FLAG_FLAT_END) != 0u);
        } else {
            reach = -select(ra_m + margin, margin, (flags & FLAG_FLAT_START) != 0u);
        }
        let centre = select(wa + along * reach, mix(wa, wb, reach / max(len, 1e-6)), len > 1e-6);
        let world = centre + across * side * half_width + up * 0.004;
        out.position = ground_clip(view.view_from_world * vec4(world, 1.0), view.clip_from_view);
        out.ribbon = vec2(side * half_width, reach);
        out.seg = vec4(len, 0.0, 0.0, 0.0);
        out.radii = vec4(ra_m, rb_m, fade, halo);
        return out;
    }

    let sa = ndc_to_px(ca.xy / ca.w);
    let sb = ndc_to_px(cb.xy / cb.w);
    let d = sb - sa;
    let len = length(d);
    let dir = select(vec2(1.0, 0.0), d / max(len, 1e-6), len > 1e-4);
    let perp = vec2(-dir.y, dir.x);
    let margin = AA_MARGIN + halo;
    let half_width = max(ra, rb) + margin;
    var reach: f32;
    if at_b {
        reach = select(rb + margin, margin, (flags & FLAG_FLAT_END) != 0u);
    } else {
        reach = -select(ra + margin, margin, (flags & FLAG_FLAT_START) != 0u);
    }
    let base = select(sa, sb, at_b);
    let px = base + dir * reach + perp * side * half_width;
    let clip = select(ca, cb, at_b);
    let in_dir = normalize(sa - screen_of(wp) + vec2(1e-6, 0.0));
    let out_dir = normalize(screen_of(wn) - sb + vec2(1e-6, 0.0));
    out.joint = vec4(
        bisector(in_dir, dir, perp, dir, has_prev && len > 1e-4),
        bisector(dir, out_dir, perp, dir, has_next && len > 1e-4),
    );

    // Depth of the tube's near surface.
    var depth = clip.z / clip.w;
    if !is_ortho() {
        let r_world = select(ra / ppu_a, rb / ppu_b, at_b);
        let vz = min(select(va.z, vb.z, at_b) + r_world, limit);
        depth = depth_of(vz);
    }
    out.position = vec4(px_to_ndc(px) * clip.w, depth * clip.w, clip.w);
    out.seg = vec4(sa, sb);
    out.radii = vec4(ra, rb, fade, halo);
    out.ribbon = vec2(0.0);
    return out;
}

// Unit direction of `span` within the plane normal to `up`, and its length.
fn in_plane(span: vec3<f32>, up: vec3<f32>) -> vec4<f32> {
    let flat = span - up * dot(span, up);
    let len = length(flat);
    let fallback = normalize(cross(up, vec3(0.0, 0.0, 1.0) + vec3(1e-3, 0.0, 0.0)));
    return vec4(select(fallback, flat / max(len, 1e-6), len > 1e-5), len);
}

// Signed distance to a capsule whose radius runs from r1 at the origin to r2
// at (0, h).
fn capsule_distance(q: vec2<f32>, r1: f32, r2: f32, h: f32) -> f32 {
    let p = vec2(abs(q.x), q.y);
    if h < 1e-5 || abs(r1 - r2) >= h {
        return min(length(p) - r1, length(p - vec2(0.0, h)) - r2);
    }
    let b = (r1 - r2) / h;
    let a = sqrt(1.0 - b * b);
    let k = dot(p, vec2(-b, a));
    if k < 0.0 {
        return length(p) - r1;
    }
    if k > a * h {
        return length(p - vec2(0.0, h)) - r2;
    }
    return dot(p, vec2(a, b)) - r1;
}

// A band of `duty` per period of `x`, edges smoothed over `aa`.
fn band(x: f32, duty: f32, aa: f32) -> f32 {
    let d = abs(fract(x - 0.5 * duty + 0.5) - 0.5);
    let w = max(aa, 1e-4);
    let sharp = 1.0 - smoothstep(0.5 * duty - w, 0.5 * duty + w, d);
    return mix(sharp, duty, smoothstep(0.2, 0.5, aa));
}

@fragment
fn fragment(in: VertexOutput) -> FragmentOutput {
    var out: FragmentOutput;
    let flags = in.code & 0xfffu;
    let style = (in.code >> 12u) & 0x3fu;
    let flat = (flags & FLAG_FLAT) != 0u;

    // Position across and along the segment, in pixels or (flat) metres.
    let a = in.seg.xy;
    let ab = in.seg.zw - a;
    let h_screen = length(ab);
    let dir = select(vec2(1.0, 0.0), ab / max(h_screen, 1e-6), h_screen > 1e-4);
    let rel = in.position.xy - a;
    let q = select(vec2(dot(rel, vec2(-dir.y, dir.x)), dot(rel, dir)), in.ribbon, flat);
    let h = select(h_screen, in.seg.x, flat);
    let unit = select(1.0, max(fwidth(in.ribbon.x), 1e-6), flat);
    let along_unit = select(1.0, max(fwidth(in.ribbon.y), 1e-6), flat);
    let r1 = in.radii.x;
    let r2 = in.radii.y;
    let t = clamp(q.y / max(h, 1e-6), 0.0, 1.0);
    let weighted = vec2(1.0 - t, t) / max(in.sphere.zw, vec2(1e-6));
    let world_t = select(weighted.y / max(weighted.x + weighted.y, 1e-20), t, flat);
    let s = mix(in.dist.x, in.dist.y, world_t);
    let s_aa = fwidth(s);
    let r_m = max(mix(in.dist.z, in.dist.w, world_t), 1e-4);
    let r_at = max(mix(r1, r2, t), 1e-4);
    let u = clamp(q.x / r_at, -1.0, 1.0);
    let across_m = u * r_m;
    let across_aa = fwidth(across_m);
    let radial = abs(q.x) / r_at;
    let radial_aa = max(fwidth(radial), 1e-4);

    let time = globals.time + in.fx2.w;
    let glow = in.fx.x;
    let hz = in.fx.y;
    let flow = in.fx.z;
    let spacing = max(in.fx.w, 1e-3);
    let length_m = max(in.fx2.y, 1e-4);
    let along = clamp(s / length_m, 0.0, 1.0);

    var contour = capsule_distance(q, r1, r2, h);
    if (flags & FLAG_SQUARE) != 0u && h < 1e-4 {
        contour = max(abs(q.x), abs(q.y)) - r1;
    }
    let contour_unit = select(1.0, max(fwidth(contour), 1e-6), flat);
    let dist_px = contour / contour_unit;
    var cover = clamp(0.5 - dist_px, 0.0, 1.0);
    if (flags & FLAG_FLAT_START) != 0u {
        cover = cover * clamp(q.y / along_unit + 0.5, 0.0, 1.0);
    }
    if (flags & FLAG_FLAT_END) != 0u {
        cover = cover * clamp((h - q.y) / along_unit + 0.5, 0.0, 1.0);
    }
    // Internal joints have one owner; antialiasing applies to the outer contour.
    var keep = 1.0;
    if dot(in.joint.xy, in.joint.xy) > 0.0 {
        keep = keep * select(0.0, 1.0, dot(q, in.joint.xy) >= 0.0);
    }
    if dot(in.joint.zw, in.joint.zw) > 0.0 {
        keep = keep * select(0.0, 1.0, dot(q - vec2(0.0, h), in.joint.zw) < 0.0);
    }
    cover = cover * keep;
    let halo_reach = in.radii.w;
    let outside = max(dist_px, 0.0) / max(halo_reach, 1e-3);
    let halo_edge = 1.0 - smoothstep(0.68, 1.0, outside);
    let halo_band = 1.0 - smoothstep(0.0, 1.5, -dist_px);
    let halo = select(0.0, exp(-3.0 * outside * outside) * halo_edge * halo_band * keep, halo_reach > 0.0 && outside < 1.0);
    let corona = select(0.0, exp(-18.0 * outside * outside) * halo_edge * halo_band * keep, halo_reach > 0.0 && outside < 1.0);

    let color = mix(in.color_a, in.color_b, world_t);
    let base = color.rgb;
    var over = vec3(0.0);
    var over_a = 0.0;
    var light = vec3(0.0);
    out.depth = in.position.z;

    if style == STYLE_RIBBON_FLOW {
        let v = radial;
        let x = (s - flow * time) / spacing + v * 0.35;
        let w = s_aa / spacing + 0.35 * radial_aa;
        let detail = detail_visibility(w) * smoothstep(2.0, 5.0, r_at / unit);
        let shoulder = 1.0 - smoothstep(0.28, 1.0, v);
        let edge = smoothstep(0.0, max(0.22 * r_at / unit, 1.0), -dist_px) * keep;
        let chev = band(x, 0.22, w) * shoulder * detail * edge;
        let rails = technical_line((v - 0.62) * r_at / unit, 0.65, 1.0) * detail * edge;
        let station_x = s / (spacing * 5.0);
        let station = technical_line(abs(fract(station_x + 0.5) - 0.5), 0.012, s_aa / (spacing * 5.0))
            * smoothstep(0.55, 0.75, v) * detail;
        let terminal = max(1.0 - smoothstep(0.0, max(0.06, s_aa), s),
            1.0 - smoothstep(0.0, max(0.06, s_aa), length_m - s));
        let structure = max(max(rails, station), max(chev, terminal));
        var dashes = 1.0;
        if (flags & FLAG_DASHED) != 0u {
            dashes = band(s / (2.0 * spacing), 0.6, s_aa / spacing);
        }
        let activity = select(0.0, chev, abs(flow) > 0.0);
        let coat = energy_coat(base, color.a * dashes, 0.75, (0.16 + 0.32 * structure) * edge,
            0.0, structure * edge, activity, 0.0, glow);
        over_a = coat.a;
        light = coat.rgb + ink_highlight(base) * glow * (HALO_GAIN * halo + 0.24 * corona) * color.a * dashes;
    } else if style == STYLE_TUBE_PULSE {
        let nz = sqrt(max(1.0 - u * u, 0.0));
        let shade = 0.62 + 0.38 * nz;
        let tail = mix(1.0, in.fx2.x, along);
        let pulse = energy_packet(along - time * hz, 0.045, s_aa / length_m, hz);
        let wake = energy_packet(along - time * hz, 0.13, s_aa / length_m, hz);
        let spine = exp(-8.0 * radial * radial) * cover;
        let coat = energy_coat(base, color.a * tail, shade, (0.48 + 0.16 * pulse) * cover,
            0.0, 0.5 * spine, pulse * spine, wake * spine, glow);
        over_a = coat.a;
        light = coat.rgb + ink_highlight(base) * glow * (HALO_GAIN * halo + 0.24 * corona) * color.a * tail;
    } else if style == STYLE_LIGHT_WALL {
        let life = mix(max(in.fx2.x, 0.18), 1.0, smoothstep(0.0, 1.0, along));
        let spine = exp(-7.0 * radial * radial) * cover;
        let wave = select(1.0, 0.80 + 0.20 * cos(TAU * (s / max(spacing * 4.0, 0.4) - time * hz)), hz > 0.0);
        let activity = select(0.0, spine * wave, hz > 0.0);
        let coat = energy_coat(base, color.a * life, 0.8 + 0.2 * wave, (0.48 + 0.16 * wave) * cover,
            0.0, 0.5 * spine, activity, 0.0, glow);
        over_a = coat.a;
        light = coat.rgb + ink_highlight(base) * glow * (HALO_GAIN * halo + 0.24 * corona) * color.a * life;
    } else if style == STYLE_HAIRLINE {
        let weight = select(color.a, 0.06, color.a > 0.999);
        light = base * (0.85 + 0.15 * glow) * weight * cover;
    } else if style == STYLE_ORB {
        let r = length(q) / max(r1, 1e-4);
        let rr = max(r1 / unit, 1e-4);
        let detail = smoothstep(3.0, 7.0, rr);
        let rim = technical_line((r - 0.76) * rr, 0.55, 1.0);
        let glyph_ticks = max(technical_line(q.x / unit, 0.45, 1.0), technical_line(q.y / unit, 0.45, 1.0))
            * smoothstep(0.60, 0.75, r) * (1.0 - smoothstep(0.9, 1.0, r));
        let centre = 1.0 - smoothstep(0.16, 0.24, r);
        let core = exp(-24.0 * r * r) * cover;
        let shell = exp(-3.5 * r * r) * cover;
        let symbol = mix(1.0, max(max(rim, glyph_ticks), centre), detail) * cover;
        let turn = atan2(q.y, q.x) / TAU;
        let activity = energy_packet(turn - time * hz, 0.07, fwidth(turn), hz) * rim * detail * cover;
        let coat = energy_coat(base, color.a, 0.65 + 0.35 * sqrt(max(1.0 - r * r, 0.0)),
            0.16 * shell + 0.32 * symbol + 0.16 * core + 0.16 * activity, 0.0, max(symbol, core), activity, 0.0, glow);
        over_a = coat.a;
        light = coat.rgb + ink_highlight(base) * glow * (HALO_GAIN * halo + 0.24 * corona) * color.a;
    } else if style == STYLE_SPLAT {
        // A flat splat, a touch lighter at its middle; no halo.
        let rr = clamp(length(q) / max(r1, 1e-4), 0.0, 1.0);
        over = base * (0.82 + 0.22 * (1.0 - rr * rr));
        over_a = cover * color.a;
    } else if style == STYLE_ALERT {
        // Diagonal hazard stripes with low-contrast breathing.
        let x = (s + across_m) / spacing - time * hz * 0.35;
        let stripe = band(x, 0.5, (s_aa + across_aa) / spacing * 1.5);
        let flash = 0.82 + 0.18 * sin(TAU * min(hz, 2.0) * time);
        let activity = select(0.0, stripe * flash * cover, hz > 0.0);
        let coat = energy_coat(base * mix(0.16, 1.0, stripe), color.a, 1.0, (0.24 + 0.32 * stripe) * cover,
            0.0, 0.25 * stripe * cover, activity, 0.0, glow);
        over_a = coat.a;
        light = coat.rgb + ink_highlight(base) * glow * (HALO_GAIN * halo + 0.24 * corona) * color.a;
    } else {
        var shade = 1.0;
        if (flags & FLAG_SPHERE) != 0u {
            let rr = clamp(length(q) / max(r1, 1e-4), 0.0, 1.0);
            let nz = sqrt(max(1.0 - rr * rr, 0.0));
            let n = vec3(q.x / max(r1, 1e-4), -q.y / max(r1, 1e-4), nz);
            let key = max(dot(n, normalize(vec3(-0.35, 0.45, 0.82))), 0.0);
            shade = 0.45 + 0.55 * key;
            if !is_ortho() && !flat {
                out.depth = depth_of(min(in.sphere.x + in.sphere.y * nz, -1e-4));
            }
        } else if (flags & FLAG_TUBE) != 0u {
            let nz = sqrt(max(1.0 - u * u, 0.0));
            shade = 0.62 + 0.38 * nz;
        }
        var gain = 1.0;
        if style == STYLE_ARROW_GLOW && (flags & FLAG_HEAD) == 0u {
            gain = mix(0.72, 1.0, along);
        } else if style == STYLE_ARROW_GLOW {
            gain = 1.0;
        }
        var activity = 0.0;
        if style == STYLE_ARROW_GLOW {
            activity = energy_packet(along - time * hz, 0.055, s_aa / length_m, hz);
        } else if style == STYLE_POINT_GLOW || style == STYLE_HOLO {
            let rr = length(q) / r_at;
            let turn = atan2(q.y, q.x) / TAU;
            let rim = technical_line((rr - 0.72) * r_at / unit, 0.65, 1.0);
            activity = energy_packet(turn - time * hz, 0.07, fwidth(turn), hz) * rim * smoothstep(4.0, 10.0, r_at / unit);
        }
        var dashes = 1.0;
        if (flags & FLAG_DASHED) != 0u {
            dashes = band((s - flow * time) / spacing, 0.55, s_aa / spacing * 1.5);
        }
        let reach = select(1.0, mix(in.fx2.x, 1.0, along), in.fx2.x < 0.999);
        let alpha = color.a * cover * dashes * reach;
        if (flags & FLAG_ADD) != 0u {
            light = base * shade * gain * (1.0 + 0.20 * activity) * alpha
                + base * glow * (HALO_GAIN * halo + 0.24 * corona + 0.45 * cover) * color.a * dashes * reach;
        } else {
            over = base * shade * mix(1.0, gain, 0.5) * (1.0 + 0.20 * activity);
            over_a = alpha;
            let core_radius = select(radial, length(q) / r_at, h < 1e-4);
            let highlight = exp(-8.0 * core_radius * core_radius) * cover;
            let core_gain = select(0.38, 0.75, style == STYLE_ARROW_GLOW || style == STYLE_POINT_GLOW);
            light = ink_highlight(base) * gain * (HALO_GAIN * halo + 0.24 * corona
                + (core_gain + 0.45 * activity) * highlight) * glow * color.a * dashes * reach;
        }
    }

    @if((!(DRAW_SPLAT)))
    if (flags & FLAG_ADD) != 0u {
        light = light + over * over_a;
        over_a = 0.0;
    }
    over_a = over_a * in.radii.z;
    light = light * in.radii.z;
    // Opaque splats: coverage becomes the sample mask, depth is written.
    @if((DRAW_SPLAT) && (ALPHA_TO_COVERAGE))
    if over_a <= 0.02 {
        discard;
    }
    @if((DRAW_SPLAT) && (!(ALPHA_TO_COVERAGE)))
    if over_a < 0.5 {
        discard;
    }
    @if((DRAW_SPLAT))
    out.color = vec4(over + light, over_a);
    @if((DRAW_SPLAT))
    out.depth = in.position.z;
    @if((DRAW_SPLAT))
    return out;
    @if((!(DRAW_SPLAT)))
    if over_a <= 0.002 && dot(light, light) <= 1e-7 {
        discard;
    }
    @if((!(DRAW_SPLAT)))
    out.color = background_frost(vec4(over * over_a + light, over_a), in.position.xy, params.unused.x);
    @if((!(DRAW_SPLAT)))
    return out;
}
