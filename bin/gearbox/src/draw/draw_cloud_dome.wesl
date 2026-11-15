// Multi-scale Eye-Dome Lighting over opaque cloud points (Boucheny; CloudCompare's full,
// half and quarter scales with Potree's log2 response). Level 0 of the dome texture holds
// the hardware depth and strength of the splat drawn at each pixel; `pull` fills each
// coarser level with the nearest splat of its 2x2 block, closing gaps between points.
// `fragment` mixes every level's ring shade and depth testing keeps it off nearer surfaces.
import bevy_render::view::View;

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var dome: texture_2d<f32>;

const RESPONSE_SCALE: f32 = 300.0;
const DEPTH_NUDGE: f32 = 1.000001;
const LEVELS: i32 = 3;
const SURFACE_SLOPE: f32 = 0.01;

struct Shade {
    @location(0) factor: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

fn log_distance(depth: f32) -> f32 {
    let point = view.view_from_clip * vec4(0.0, 0.0, depth, 1.0);
    return log2(max(-point.z / point.w, 1e-6));
}

// Shade of the splat at `texel` of `level` from its 8-texel ring, and the ring's filled share.
fn ring(texel: vec2<i32>, level: i32, strength: f32) -> vec2<f32> {
    let last = vec2<i32>(textureDimensions(dome, level)) - 1;
    let own = log_distance(textureLoad(dome, texel, level).x);
    var offsets = array(
        vec2(1, 0), vec2(1, 1), vec2(0, 1), vec2(-1, 1),
        vec2(-1, 0), vec2(-1, -1), vec2(0, -1), vec2(1, -1),
    );
    var response = 0.0;
    var filled = 0.0;
    for (var i = 0u; i < 8u; i++) {
        let neighbour = texel + offsets[i];
        if any(neighbour < vec2(0)) || any(neighbour > last) { continue; }
        let depth = textureLoad(dome, neighbour, level).x;
        if depth > 0.0 {
            response += max(0.0, own - log_distance(depth));
            filled += 1.0;
        }
    }
    let scale = exp2(f32(level));
    return vec2(exp(-response / (8.0 * scale) * RESPONSE_SCALE * strength), filled / 8.0);
}

@fragment
fn pull(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let texel = vec2<i32>(position.xy);
    let size = vec2<i32>(textureDimensions(dome));
    let first = texel * 2;
    let end = select(first + 1, size - 1, texel == size / 2 - 1);
    var nearest = vec4(0.0);
    for (var y = first.y; y <= end.y; y++) {
        for (var x = first.x; x <= end.x; x++) {
            let splat = textureLoad(dome, vec2(x, y), 0);
            if splat.x > nearest.x { nearest = splat; }
        }
    }
    return nearest;
}

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> Shade {
    let pixel = vec2<i32>(position.xy);
    let centre = textureLoad(dome, pixel, 0);
    if centre.x <= 0.0 || centre.y <= 0.0 { discard; }
    let own = log_distance(centre.x);
    let full = ring(pixel, 0, centre.y);
    var dark = full.y * (1.0 - full.x);
    var weight = full.y;
    // Coarse levels: bilinear taps weighted by level, ring fill and depth agreement.
    for (var level = 1; level < LEVELS; level++) {
        let scale = exp2(f32(level));
        let at = position.xy / scale - 0.5;
        let base = vec2<i32>(floor(at));
        let blend = at - floor(at);
        let last = vec2<i32>(textureDimensions(dome, level)) - 1;
        for (var tap = 0; tap < 4; tap++) {
            let corner = vec2(tap & 1, tap >> 1);
            let texel = clamp(base + corner, vec2(0), last);
            let depth = textureLoad(dome, texel, level).x;
            if depth <= 0.0 { continue; }
            let bilinear = mix(1.0 - blend, blend, vec2<f32>(corner));
            let gap = (own - log_distance(depth)) / (SURFACE_SLOPE * scale);
            let share = bilinear.x * bilinear.y * exp(-0.5 * gap * gap) / scale;
            let coarse = ring(texel, level, centre.y);
            dark += share * coarse.y * (1.0 - coarse.x);
            weight += share * coarse.y;
        }
    }
    if weight <= 0.0 { discard; }
    return Shade(vec4(vec3(1.0 - dark / weight), 1.0), min(centre.x * DEPTH_NUDGE, 1.0));
}
