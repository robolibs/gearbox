// Filled draws: shapes, triangles, ground rings, beams, walls of light and
// decals, with a colour per vertex and a look painted over it, in the
// colour asked for once tonemapped. Colour leaves premultiplied, light
// added with no alpha.
#import bevy_pbr::mesh_functions::{get_world_from_local, mesh_position_local_to_world, mesh_normal_local_to_world}
#import bevy_pbr::mesh_view_bindings::{view, globals}
#import gearbox::draw_frost::background_frost
#import gearbox::draw_common::{srgb_to_linear, display_to_scene, technical_line, detail_visibility, energy_packet, energy_coat, energy_glass, energy_glass_shade, ground_clip}

struct SurfaceParams {
    // Shading flag, frost strength, unused, unused.
    flags: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: SurfaceParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var lut_texture: texture_3d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var lut_sampler: sampler;

const LOOK_WALL: u32 = 1u;
const LOOK_RING: u32 = 2u;
const LOOK_HOLO: u32 = 3u;
const LOOK_SHIELD: u32 = 4u;
const LOOK_ALERT: u32 = 5u;
const LOOK_DECAL: u32 = 6u;
const LOOK_BEAM: u32 = 7u;
const LOOK_SWEEP: u32 = 9u;
const LOOK_GHOST: u32 = 10u;
const LOOK_PLANE: u32 = 11u;
const LOOK_ZONE: u32 = 12u;
const LOOK_FIELD: u32 = 13u;
const TAU: f32 = 6.2831853;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    // Unit-shape coordinates, or (arc length, height share) on walls and
    // beams, or edge distance on decals; FIELD: local metres and face axis.
    @location(3) local: vec4<f32>,
    // look, glow, pulse Hz, phase
    @location(4) fx: vec4<f32>,
    // urgency, size m, spacing m, fade; FIELD: full XYZ size, panel spacing.
    @location(5) fx2: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) local: vec4<f32>,
    @location(4) @interpolate(flat) fx: vec4<f32>,
    @location(5) @interpolate(flat) fx2: vec4<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world = mesh_position_local_to_world(get_world_from_local(v.instance_index), vec4(v.position, 1.0));
    let look = u32(v.fx.x + 0.5);
    let at_eye = view.view_from_world * world;
    out.position = view.clip_from_view * at_eye;
    if look == LOOK_RING || look == LOOK_DECAL || look == LOOK_ALERT || look == LOOK_ZONE
        || (look == LOOK_SWEEP && v.local.w > 0.5) {
        out.position = ground_clip(at_eye, view.clip_from_view);
    }
    out.world_position = world.xyz;
    out.world_normal = mesh_normal_local_to_world(v.normal, v.instance_index);
    out.color = vec4(display_to_scene(srgb_to_linear(v.color.rgb), lut_texture, lut_sampler), v.color.a);
    out.local = v.local;
    out.fx = v.fx;
    out.fx2 = v.fx2;
    return out;
}

// Pixel-filtered periodic bands, retaining their average when minified.
fn band(x: f32, duty: f32, w: f32) -> f32 {
    let s = max(w, 1e-4);
    let lo = x - 0.5 * s;
    let hi = x + 0.5 * s;
    let a = floor(lo) * duty + min(fract(lo), duty);
    let b = floor(hi) * duty + min(fract(hi), duty);
    return clamp((b - a) / s, 0.0, 1.0);
}

fn soft_line(distance: f32, radius: f32, footprint: f32) -> f32 {
    return technical_line(distance, radius, footprint);
}

fn pulse_line(phase: f32, radius: f32, footprint: f32) -> f32 {
    let distance = abs(fract(phase + 0.5) - 0.5);
    return soft_line(distance, radius, footprint) * detail_visibility(footprint);
}

fn energy_wake(phase: f32, footprint: f32, hz: f32) -> f32 {
    let behind = fract(-phase);
    return exp(-behind * 18.0) * smoothstep(0.0, 0.014, behind) * detail_visibility(footprint) * select(0.0, 1.0, hz > 0.0);
}

fn surface(in: VertexOutput, front: bool) -> vec4<f32> {
    let look = u32(in.fx.x + 0.5);
    let glow = in.fx.y;
    let hz = in.fx.z;
    let time = globals.time + in.fx.w;
    let size = max(in.fx2.y, 1e-3);
    var n = normalize(in.world_normal);
    if !front {
        n = -n;
    }
    let to_eye = normalize(view.world_position - in.world_position);
    let ndv = abs(dot(n, to_eye));
    let base = in.color.rgb;
    let outlined = in.local.w >= 8.0;
    let local_kind = in.local.w - select(0.0, 8.0, outlined);

    // Distance fields used by the looks, with their screen rates.
    let p = in.local.xy * 2.0 * size;
    let r = length(p);
    let r_w = fwidth(r);
    let m = abs(in.local.xyz);
    let middle = m.x + m.y + m.z - max(max(m.x, m.y), m.z) - min(min(m.x, m.y), m.z);
    let edge_d = (0.5 - middle) * size;
    let edge_w = fwidth(edge_d);
    let polygon = local_kind > 2.5;
    let stripe_position = select((in.local.x + in.local.y + in.local.z) * size, in.local.y + in.local.z, polygon);
    let shade = energy_glass_shade(n, ndv);
    let rim = pow(clamp(1.0 - ndv, 0.0, 1.0), 5.0);

    if look == LOOK_WALL {
        let v = clamp(in.local.y, 0.0, 1.0);
        let along = in.local.x;
        let across_w = fwidth(v * size);
        let foot = soft_line(v * size, 0.008, across_w);
        let crest = soft_line((1.0 - v) * size, 0.006, across_w);
        let lane_a = soft_line((v - 0.32) * size, 0.003, across_w);
        let lane_b = soft_line((v - 0.68) * size, 0.002, across_w);
        let phase_a = along / 3.8 - time * hz;
        let phase_b = along / 5.6 - time * hz + 0.37;
        let packet_a = energy_packet(phase_a, 0.08, fwidth(phase_a), hz);
        let packet_b = energy_packet(phase_b, 0.06, fwidth(phase_b), hz);
        let activity = lane_a * packet_a + 0.6 * lane_b * packet_b;
        let wake = lane_a * energy_wake(phase_a, fwidth(phase_a), hz)
            + 0.5 * lane_b * energy_wake(phase_b, fwidth(phase_b), hz);
        let polish = exp(-pow((v - 0.70) * 5.0, 2.0)) * (0.25 + 0.75 * rim);
        let age = mix(in.fx2.w, 1.0, smoothstep(0.0, 1.0, along / max(in.fx2.z, 1e-3)));
        let structure = max(foot, 0.75 * crest);
        let coverage = 0.20 + 0.12 * polish + 0.22 * structure + 0.08 * activity;
        let depth_tint = base * (0.65 + 0.35 * smoothstep(0.0, 0.8, v));
        return energy_coat(depth_tint, in.color.a * age, shade, coverage, 0.12 * rim,
            max(structure, 0.12 * polish), activity, wake, glow);
    } else if look == LOOK_RING {
        let radial = r / size;
        let edge_fade = 1.0 - smoothstep(0.86, 1.0, radial);
        let core = soft_line(r - size * 0.73, 0.014, r_w);
        let turn = atan2(p.y, p.x) / TAU;
        let ticks_x = turn * 16.0;
        let ticks = pulse_line(ticks_x, 0.06, fwidth(ticks_x)) * soft_line(r - size * 0.85, size * 0.04, r_w);
        let inner = band(turn * 4.0 + 0.04, 0.14, fwidth(turn * 4.0)) * soft_line(r - size * 0.54, 0.009, r_w);
        let structure = max(core, max(0.65 * ticks, 0.55 * inner));
        var ripples = 0.0;
        for (var k = 0; k < 3; k = k + 1) {
            let f = fract(time * hz + f32(k) / 3.0);
            let envelope = smoothstep(0.0, 0.12, f) * pow(1.0 - f, 2.0);
            ripples += soft_line(r - f * size, 0.016, r_w) * envelope * select(0.0, 1.0, hz > 0.0);
        }
        let activity = max(energy_packet(turn - time * hz, 0.045, fwidth(turn), hz) * core, ripples);
        let coverage = 0.48 * structure + 0.16 * activity;
        return energy_coat(base, in.color.a * edge_fade, 0.8, coverage, 0.0, structure, activity, 0.0, glow);
    } else if look == LOOK_BEAM {
        let h = clamp(in.local.y, 0.0, 1.0);
        let fall = exp(-h * 2.1) * (1.0 - smoothstep(0.82, 1.0, h));
        let rise = h * 3.0 - time * hz;
        let rise_w = fwidth(rise);
        let front = energy_packet(rise, 0.045, rise_w, hz);
        let wake = energy_wake(rise, rise_w, hz);
        let lanes_x = in.local.x * 8.0 + 0.04 * sin(h * 7.0);
        let lanes = pulse_line(lanes_x, 0.035, fwidth(lanes_x));
        let stations = pulse_line(h * 4.0, 0.018, fwidth(h * 4.0));
        let structure = max(0.6 * lanes, 0.22 * stations) * fall;
        let profile = 0.25 + 0.75 * pow(ndv, 2.2);
        let foot = exp(-h * 32.0) * ndv;
        let activity = front * (0.6 + 0.4 * lanes) * fall * profile;
        let coverage = 0.12 * fall * profile + 0.32 * structure + 0.16 * activity + 0.04 * wake * fall * profile;
        return energy_coat(base, in.color.a, shade, coverage, 0.0, max(structure, 0.65 * foot), activity, wake * fall * profile, glow);
    } else if look == LOOK_HOLO || look == LOOK_GHOST {
        let height_share = clamp(in.local.z + 0.5, 0.0, 1.0);
        let scan_x = height_share - time * hz;
        let scan_w = fwidth(scan_x);
        let scan = energy_packet(scan_x, 0.022, scan_w, hz);
        let wake = energy_wake(scan_x, scan_w, hz);
        let lattice_x = height_share * 4.0;
        let lattice = pulse_line(lattice_x, 0.009, fwidth(lattice_x));
        var edges: f32;
        if local_kind > 1.5 {
            edges = select(soft_line(edge_d, 0.009, edge_w), 0.0, outlined);
        } else {
            let meridians = max(soft_line(in.local.x, 0.003, fwidth(in.local.x)), soft_line(in.local.y, 0.003, fwidth(in.local.y)));
            edges = max(0.60 * rim, select(0.35 * meridians, 0.0, outlined));
        }
        let structure = max(edges, 0.35 * lattice);
        let activity = scan * (0.35 + 0.65 * structure);
        return energy_glass(base, in.color.a, shade, rim, structure, activity, wake, glow);
    } else if look == LOOK_FIELD {
        let extent = max(in.fx2.xyz, vec3(1e-4));
        let uv = select(select(in.local.yz, in.local.xz, local_kind > 0.5), in.local.xy, local_kind > 1.5);
        let face_size = select(select(extent.yz, extent.xz, local_kind > 0.5), extent.xy, local_kind > 1.5);
        let pitch = max(in.fx2.w, 1e-3);
        let grid = uv / pitch;
        let grid_w = fwidth(grid);
        let distance = abs(fract(grid + 0.5) - 0.5) * pitch;
        let detail = 1.0 - smoothstep(vec2(0.2), vec2(0.65), grid_w);
        let major = max(soft_line(distance.x, 0.006, grid_w.x * pitch) * detail.x,
            soft_line(distance.y, 0.006, grid_w.y * pitch) * detail.y);
        let edge = max(face_size * 0.5 - abs(uv), vec2(0.0));
        let edge_width = fwidth(uv);
        let edge_lines = vec2(soft_line(edge.x, 0.009, edge_width.x), soft_line(edge.y, 0.009, edge_width.y));
        let perimeter = select(max(edge_lines.x, edge_lines.y), 0.0, outlined);
        let corner_span = min(0.18, 0.22 * min(face_size.x, face_size.y));
        let corner_gate = 1.0 - smoothstep(vec2(corner_span * 0.65), vec2(corner_span), edge);
        let corners = select(max(edge_lines.x * corner_gate.y, edge_lines.y * corner_gate.x), 0.0, outlined);
        let scan_x = dot(in.local.xyz / extent, vec3(0.2, 0.15, 0.65)) + 0.5 - time * hz;
        let scan = energy_packet(scan_x, 0.025, fwidth(scan_x), hz);
        let structure = max(corners, max(0.35 * major, 0.55 * perimeter));
        let activity = scan * max(major, perimeter);
        return energy_glass(base, in.color.a, shade, rim, structure, activity, 0.0, glow);
    } else if look == LOOK_SHIELD {
        if !front {
            discard;
        }
        let v = in.local.z + 0.5;
        let rise = v * 1.5 - time * hz;
        let rise_w = fwidth(rise);
        let rip = energy_packet(rise, 0.035, rise_w, hz);
        let wake = energy_wake(rise, rise_w, hz);
        let sections = pulse_line(in.local.z * 4.0, 0.012, fwidth(in.local.z * 4.0));
        let meridian = select(max(soft_line(in.local.x, 0.004, fwidth(in.local.x)), soft_line(in.local.y, 0.004, fwidth(in.local.y))), 0.0, outlined);
        let boundary = select(soft_line(edge_d, 0.009, edge_w), 0.0, outlined);
        let structure = select(max(0.65 * sections, 0.5 * meridian), max(boundary, 0.3 * sections), local_kind > 1.5);
        let activity = rip * (0.25 + 0.75 * structure);
        return energy_glass(base, in.color.a, shade, rim, structure, activity, wake, glow);
    } else if look == LOOK_ALERT {
        let static_x = stripe_position / max(in.fx2.z, 1e-3);
        let stripe = band(static_x, 0.45, fwidth(static_x));
        let packet_x = static_x * 0.25 - time * hz;
        let activity = energy_packet(packet_x, 0.055, fwidth(packet_x), hz);
        let frame = select(1.0, 0.0, outlined || (polygon && local_kind > 3.5));
        let boundary = frame * select(soft_line(edge_d, 0.012, edge_w), soft_line(in.local.x, 0.018, fwidth(in.local.x)), polygon);
        let coverage = 0.24 + 0.32 * boundary + 0.16 * activity * max(stripe, boundary);
        return energy_coat(base * mix(0.16, 1.0, stripe), in.color.a, shade, coverage, 0.0,
            max(0.25 * stripe, boundary), activity * max(stripe, boundary), 0.0, glow);
    } else if look == LOOK_DECAL || look == LOOK_ZONE {
        let d = max(in.local.x, 0.0);
        let x = (in.local.y + in.local.z) / 0.7;
        let hatch = pulse_line(x, 0.018, fwidth(x)) * (1.0 - smoothstep(0.12, 0.25, d));
        let edge = select(soft_line(d, 0.014, fwidth(d)), 0.0, outlined || local_kind > 3.5);
        let structure = max(edge, select(0.0, 0.24 * hatch, look == LOOK_ZONE));
        let phase = x * 0.25 - time * hz;
        let activity = energy_packet(phase, 0.04, fwidth(phase), hz) * structure;
        return energy_glass(base, in.color.a, 0.75, 0.0, structure, activity, 0.0, glow);
    } else if look == LOOK_SWEEP {
        let radial = clamp(in.local.x, 0.0, 1.0);
        let across = in.local.y;
        let reach = size * radial;
        let ring_gap = select(size / 4.0, in.fx2.z, in.fx2.z > 1e-3);
        let ring_x = reach / ring_gap;
        let rings = pulse_line(ring_x, 0.012, fwidth(ring_x)) * smoothstep(0.02, 0.08, radial);
        if local_kind < 0.5 {
            let phase = radial - time * hz;
            let activity = energy_packet(phase, 0.018, fwidth(phase), hz);
            let wake = energy_wake(phase, fwidth(phase), hz);
            return energy_glass(base, in.color.a * 0.45, shade, 0.0, 0.35 * rings, activity, wake, glow);
        }
        let outer = soft_line((1.0 - radial) * size, 0.035, fwidth(reach));
        let side_d = (1.0 - abs(across)) * max(in.local.z, 1e-3) * reach;
        let side = soft_line(side_d, 0.025, fwidth(side_d));
        let feather = smoothstep(0.0, 0.035, radial) * (1.0 - smoothstep(0.88, 1.0, radial));
        let side_feather = 1.0 - smoothstep(0.88, 1.0, abs(across));
        var sweep = 0.0;
        if hz > 0.0 {
            let phase = across * 0.5 + 0.5 - time * hz;
            sweep = energy_packet(phase, 0.012, fwidth(phase), hz) + 0.45 * energy_wake(phase, fwidth(phase), hz);
        }
        let bounds = select(0.5 * outer + 0.4 * side * step(abs(across), 0.999), 0.0, outlined);
        let structure = max(bounds, 0.26 * rings * feather);
        let activity = sweep * side_feather * feather;
        let coverage = 0.12 * side_feather * feather + 0.32 * structure + 0.16 * activity;
        return energy_coat(base, in.color.a, 0.75, coverage, 0.0, structure, activity, 0.0, glow);
    } else if look == LOOK_PLANE {
        let uv = in.local.xy;
        let to_edge = min(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y));
        let e = fwidth(to_edge) * 1.5;
        let border = soft_line(to_edge, 0.003, e);
        let corner_distance = max(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y));
        let corner = (1.0 - smoothstep(0.10, 0.15, corner_distance)) * soft_line(to_edge, 0.01, e);
        let structure = select(max(corner, 0.55 * border), 0.0, outlined);
        let phase = (uv.x + uv.y) * 0.5 - time * hz;
        let activity = energy_packet(phase, 0.04, fwidth(phase), hz) * structure;
        return energy_glass(base, in.color.a, 0.75, 0.0, structure, activity, 0.0, glow);
    } else {
        var shade = 1.0;
        if params.flags.x > 0.5 {
            let key = max(dot(n, to_eye), 0.0);
            let sky = n.y * 0.5 + 0.5;
            shade = 0.38 + 0.47 * key + 0.15 * sky + 0.12 * pow(key, 32.0);
        }
        return vec4(base * shade * in.color.a, in.color.a);
    }
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    return background_frost(surface(in, front), in.position.xy, params.flags.y);
}
