// Filled draws: shapes, triangles, ground rings, beams, walls of light and
// decals, with a colour per vertex and a look painted over it, in the
// colour asked for once tonemapped. Colour leaves premultiplied, light
// added with no alpha.
#import bevy_pbr::mesh_functions::{get_world_from_local, mesh_position_local_to_world, mesh_normal_local_to_world}
#import bevy_pbr::mesh_view_bindings::{view, globals}
#import gearbox::draw_common::{srgb_to_linear, display_to_scene}

struct SurfaceParams {
    // x: 1 to shade.
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
const LOOK_FOOTPRINT: u32 = 8u;
const LOOK_SWEEP: u32 = 9u;
const LOOK_GHOST: u32 = 10u;
const LOOK_PLANE: u32 = 11u;
const LOOK_ZONE: u32 = 12u;
const TAU: f32 = 6.2831853;
const VERMILLION: vec3<f32> = vec3(0.67, 0.11, 0.0);

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    // Unit-shape coordinates, or (arc length, height share) on walls and
    // beams, or (distance from the edge) on decals; w: 2 on box faces.
    @location(3) local: vec4<f32>,
    // look, glow, pulse Hz, phase
    @location(4) fx: vec4<f32>,
    // urgency, size m, length or spacing m, fade
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
    var at_eye = view.view_from_world * world;
    if look == LOOK_RING || look == LOOK_FOOTPRINT || look == LOOK_DECAL || look == LOOK_ALERT || look == LOOK_ZONE
        || (look == LOOK_SWEEP && v.local.w > 0.5) {
        // Ground looks move up to 0.3 m towards the eye along their ray.
        let d = length(at_eye.xyz);
        at_eye = vec4(at_eye.xyz * (1.0 - min(0.3, 0.1 * d) / max(d, 1e-4)), 1.0);
    }
    out.position = view.clip_from_view * at_eye;
    out.world_position = world.xyz;
    out.world_normal = mesh_normal_local_to_world(v.normal, v.instance_index);
    out.color = vec4(display_to_scene(srgb_to_linear(v.color.rgb), lut_texture, lut_sampler), v.color.a);
    out.local = v.local;
    out.fx = v.fx;
    out.fx2 = v.fx2;
    return out;
}

// Coverage of a distance field `d` whose screen rate is `w`.
fn cover(d: f32, w: f32) -> f32 {
    return clamp(0.5 - d / max(w, 1e-6), 0.0, 1.0);
}

// A band of `duty` per period of `x`, edges smoothed over `w`.
fn band(x: f32, duty: f32, w: f32) -> f32 {
    let f = fract(x);
    let s = max(w, 1e-4);
    return smoothstep(0.0, s, f) * (1.0 - smoothstep(duty - s, duty, f));
}

// Distance to the nearest edge of a hexagon tiling of cell size 1.
fn hex_edge(p: vec2<f32>) -> f32 {
    let r = vec2(1.0, 1.7320508);
    let h = r * 0.5;
    let a = (p - r * floor(p / r)) - h;
    let b = (p - h - r * floor((p - h) / r)) - h;
    let g = select(b, a, dot(a, a) < dot(b, b));
    let q = abs(g);
    return 0.5 - max(dot(q, normalize(vec2(1.0, 1.7320508))), q.x);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let look = u32(in.fx.x + 0.5);
    let glow = in.fx.y;
    let hz = in.fx.z;
    let time = globals.time + in.fx.w;
    let urgency = in.fx2.x;
    let size = max(in.fx2.y, 1e-3);
    var n = normalize(in.world_normal);
    if !front {
        n = -n;
    }
    let to_eye = normalize(view.world_position - in.world_position);
    let ndv = abs(dot(n, to_eye));
    let base = in.color.rgb;

    // Distance fields used by the looks, with their screen rates.
    let p = in.local.xy * 2.0 * size;
    let r = length(p);
    let r_w = fwidth(r);
    let m = abs(in.local.xyz);
    let middle = m.x + m.y + m.z - max(max(m.x, m.y), m.z) - min(min(m.x, m.y), m.z);
    let edge_d = (0.5 - middle) * size;
    let edge_w = fwidth(edge_d);
    let stripe_x = (in.local.x + in.local.y + in.local.z) * size / max(in.fx2.z, 1e-3) - time * 0.8;
    let stripe_w = fwidth(stripe_x);
    let hex = in.local.xy * 9.0;
    let hex_w = fwidth(hex);
    let hex_line = 1.0 - smoothstep(0.0, max(hex_w.x, hex_w.y) * 1.5 + 0.02, hex_edge(hex));

    var over = vec3(0.0);
    var over_a = 0.0;
    var light = vec3(0.0);

    if look == LOOK_WALL {
        // A wall of light: bright foot and crest, a body fading upward,
        // fading out towards the line's start.
        let v = clamp(in.local.y, 0.0, 1.0);
        let height = in.fx2.y;
        let body = 0.6 * (1.0 - v) * (1.0 - v);
        let crest = exp(-pow((1.0 - v) * height / 0.012, 2.0));
        let foot = 0.8 * exp(-v * height / 0.025);
        let age = mix(in.fx2.w, 1.0, clamp(in.local.x / max(in.fx2.z, 1e-3), 0.0, 1.0));
        light = base * glow * (body + crest + foot) * age * in.color.a;
    } else if look == LOOK_RING {
        // A ring with rotating ticks and ripples running out.
        let ring_r = size * 0.8;
        var a = cover(abs(r - ring_r) - 0.035, r_w) + 0.6 * exp(-abs(r - ring_r) * 8.0);
        for (var k = 0; k < 3; k = k + 1) {
            let f = fract(time * hz + f32(k) / 3.0);
            a = a + 0.7 * cover(abs(r - f * size) - 0.025, r_w) * (1.0 - f) * (1.0 - f);
        }
        let turn = atan2(p.y, p.x) / TAU;
        a = a + 0.8 * step(fract(turn * 12.0 + time * 0.05), 0.4) * cover(abs(r - size * 0.93) - 0.04, r_w);
        a = a + 0.25 * exp(-r * r / (0.08 * size * size));
        light = base * glow * a * (1.0 - smoothstep(0.97, 1.0, r / size)) * in.color.a;
    } else if look == LOOK_BEAM {
        // An open column of light: soft at its sides, fading with height,
        // bands rising through it, a hot foot.
        let h = clamp(in.local.y, 0.0, 1.0);
        let fall = (1.0 - h) * (1.0 - h);
        let bands = exp(-pow((fract(h * 4.0 - time * 0.6) - 0.5) * 5.0, 2.0));
        let foot = exp(-h * 25.0) * 2.0;
        let pulse = 1.0 + 0.3 * sin(TAU * hz * time);
        let a = (fall * ndv * ndv * (0.35 + 0.6 * bands) + foot * ndv) * pulse;
        light = mix(base, vec3(1.0), 0.25) * glow * a * in.color.a;
    } else if look == LOOK_HOLO {
        // Glowing edges (boxes) or rim, faint faces, a scanline sweeping up.
        let fill = 0.06;
        let height_share = clamp(in.local.z + 0.5, 0.0, 1.0);
        let scan = exp(-pow((fract(time * hz) - height_share) * 18.0, 2.0)) * 0.5;
        var edges: f32;
        if in.local.w > 1.5 {
            edges = 1.4 * cover(edge_d - 0.015, edge_w) + 0.5 * exp(-max(edge_d, 0.0) / 0.05);
        } else {
            edges = 1.2 * pow(1.0 - ndv, 2.5);
        }
        light = base * glow * (fill + edges + scan) * in.color.a;
    } else if look == LOOK_SHIELD {
        // Fresnel rim, faint fill and weave, ripples quickening and turning
        // vermillion with urgency.
        if !front {
            discard;
        }
        let rim = pow(1.0 - ndv, 3.0);
        let v = in.local.z + 0.5;
        let rip = exp(-pow((fract(v * 1.5 - time * hz) - 0.5) * 16.0, 2.0));
        let tint = mix(base, VERMILLION, clamp(urgency, 0.0, 1.0));
        let a = 0.03 + 1.2 * rim + rip * (0.12 + 0.45 * urgency) + 0.04 * hex_line;
        light = tint * glow * a * in.color.a;
    } else if look == LOOK_FOOTPRINT {
        // A thin ring where the shield meets the ground.
        let edge = abs(r - size * 0.97);
        let tint = mix(base, VERMILLION, clamp(urgency, 0.0, 1.0));
        light = tint * glow * (cover(edge - 0.02, r_w) + 0.5 * exp(-edge * 10.0)) * 0.8 * in.color.a;
    } else if look == LOOK_ALERT {
        // Hazard stripes flashing on and off.
        let f = fract(stripe_x);
        let stripe = smoothstep(0.0, stripe_w * 1.5, f) * (1.0 - smoothstep(0.5 - stripe_w * 1.5, 0.5, f));
        let flash = 0.55 + 0.45 * step(0.5, fract(time * hz));
        over = mix(vec3(0.02), base, stripe);
        over_a = 0.85 * in.color.a;
        light = base * glow * 0.3 * stripe * flash * in.color.a;
    } else if look == LOOK_DECAL {
        // Fill fading inward from the outline.
        let d = max(in.local.x, 0.0);
        let a = 0.15 * (1.0 - smoothstep(0.0, 0.3, d)) + 0.025;
        light = base * glow * a * in.color.a;
    } else if look == LOOK_ZONE {
        // A zone: an inner glow along its edges, a light interior and a
        // faint static diagonal hatch.
        let d = max(in.local.x, 0.0);
        let x = (in.local.y + in.local.z) / 0.7;
        let hatch = band(x, 0.16, fwidth(x) * 1.5) * smoothstep(0.08, 0.35, d);
        let a = 0.32 * exp(-d / 0.09) + 0.12 * (1.0 - smoothstep(0.0, 0.4, d)) + 0.05 + 0.07 * hatch;
        light = base * glow * a * in.color.a;
    } else if look == LOOK_SWEEP {
        // A sensor's reach: falling fill, bright rim and sides, range rings
        // and a radar line sweeping across with a fading trail (flat), or a
        // scan plane running out along the frustum (w = 0).
        let r = clamp(in.local.x, 0.0, 1.0);
        let across = in.local.y;
        let reach = size * r;
        let ring_gap = select(size / 4.0, in.fx2.z, in.fx2.z > 1e-3);
        let rings = band(reach / ring_gap + 0.5, 0.06, fwidth(reach / ring_gap) * 1.5) * step(0.02, r) * (1.0 - step(0.985, r));
        let outer = 1.0 - smoothstep(0.0, max(fwidth(r) * 2.0, 1e-4), abs(1.0 - r) - 0.006);
        let side_d = (1.0 - abs(across)) * max(in.local.z, 1e-3) * reach;
        let side = 1.0 - smoothstep(0.0, max(fwidth(side_d) * 1.5, 1e-4), side_d - 0.02);
        var sweep = 0.0;
        if hz > 0.0 {
            if in.local.w > 0.5 {
                let at = fract(time * hz) * 2.0 - 1.0;
                let behind = at - across;
                sweep = select(0.0, exp(-behind * 6.0), behind >= 0.0) * smoothstep(0.0, 0.15, r);
            } else {
                let at = fract(time * hz);
                sweep = exp(-pow((r - at) * 24.0, 2.0));
            }
        }
        let fill = 0.07 * pow(1.0 - r, 0.6) + 0.02;
        let a = fill + 0.9 * max(outer, side * step(abs(across), 0.999) * 0.8) + 0.35 * rings + 0.45 * sweep;
        light = base * glow * a * in.color.a;
    } else if look == LOOK_PLANE {
        // An image plane: a faint fill, corner brackets and thirds.
        let uv = in.local.xy;
        let to_edge = min(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y));
        let e = fwidth(to_edge) * 1.5;
        let border = 1.0 - smoothstep(0.004, 0.004 + e, to_edge);
        let corner = step(max(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y)), 0.12) * (1.0 - smoothstep(0.012, 0.012 + e, to_edge));
        let thirds_x = abs(fract(uv.x * 3.0 + 0.5) - 0.5) / 3.0;
        let thirds_y = abs(fract(uv.y * 3.0 + 0.5) - 0.5) / 3.0;
        let thirds = (1.0 - smoothstep(0.0015, 0.0015 + fwidth(uv.x), thirds_x)) + (1.0 - smoothstep(0.0015, 0.0015 + fwidth(uv.y), thirds_y));
        let a = 0.035 + 0.35 * border + 1.1 * corner + 0.08 * min(thirds, 1.0);
        light = base * glow * a * in.color.a;
    } else if look == LOOK_GHOST {
        // A ghost: fresnel rim, faint fill, slow bands rising through it.
        let rim = pow(1.0 - ndv, 2.5);
        let h = in.local.z;
        let bands = band(h * 2.5 - time * hz, 0.12, fwidth(h * 2.5) * 1.5);
        let a = 0.05 + 1.0 * rim + 0.08 * bands;
        light = base * glow * a * in.color.a;
    } else {
        var shade = 1.0;
        if params.flags.x > 0.5 {
            let key = max(dot(n, to_eye), 0.0);
            let sky = n.y * 0.5 + 0.5;
            shade = 0.38 + 0.47 * key + 0.15 * sky + 0.12 * pow(key, 32.0);
        }
        over = base * shade;
        over_a = in.color.a;
    }

    if over_a <= 0.002 && dot(light, light) <= 1e-7 {
        discard;
    }
    return vec4(over * over_a + light, over_a);
}
