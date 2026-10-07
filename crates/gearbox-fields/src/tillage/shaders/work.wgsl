// Ground a tool has worked: reading the work map, and the shape and colour
// the soil takes where it has. Shared by the cover a tool works and by the
// relief layer that raises the worked soil, so the two agree where the one
// hands over to the other.

#import "embedded://gearbox_fields/shaders/interaction.wgsl"::WheelMapParams

// What the work map says of one point.
struct Work {
    // How much of this point is worked, nought to one, blended across texels:
    // the edge of a pass runs smooth rather than in texel steps.
    amount: f32,
    // The tool that cut it: nought for none, one for a disc harrow.
    kind: u32,
    // The way the tool went, as a unit world XZ direction.
    along: vec2<f32>,
    // How far across the tool's width the point lay, in metres from its middle.
    across: f32,
    // How deep the tool worked, in metres.
    depth: f32,
}

const WORK_ACROSS_STEP_M: f32 = 0.005;
const WORK_ACROSS_BIAS: f32 = 32768.0;
const WORK_DEPTH_MAX_M: f32 = 0.30;
const WORK_TAU: f32 = 6.28318530718;

fn worked_texel(state: u32) -> f32 {
    return select(0.0, 1.0, (state >> 12u) != 0u);
}

// The work map at a world point. Which tool, which way and where across come
// from the nearest worked texel; where across is carried on from that texel's
// middle along its own axle, which is exact through a pass and only seams
// where two passes meet, as the soil itself does.
fn work_at(map: texture_2d<u32>, params: WheelMapParams, world_xz: vec2<f32>) -> Work {
    var out: Work;
    out.amount = 0.0;
    out.kind = 0u;
    out.along = vec2<f32>(1.0, 0.0);
    out.across = 0.0;
    out.depth = 0.0;
    // An empty map stands in for ground no tool can work: its own size, not
    // the field's, bounds the read.
    let size = vec2<i32>(textureDimensions(map));
    let last = size - vec2<i32>(1);
    let t = (world_xz - params.origin) * params.texels_per_metre;
    if (any(t < vec2<f32>(0.0)) || any(t > vec2<f32>(last))) {
        return out;
    }
    let i = clamp(vec2<i32>(floor(t)), vec2<i32>(0), max(last - vec2<i32>(1), vec2<i32>(0)));
    let f = clamp(t - vec2<f32>(i), vec2<f32>(0.0), vec2<f32>(1.0));
    let a = textureLoad(map, i, 0).xy;
    let b = textureLoad(map, i + vec2<i32>(1, 0), 0).xy;
    let c = textureLoad(map, i + vec2<i32>(0, 1), 0).xy;
    let d = textureLoad(map, i + vec2<i32>(1, 1), 0).xy;
    let wa = (1.0 - f.x) * (1.0 - f.y) * worked_texel(a.x);
    let wb = f.x * (1.0 - f.y) * worked_texel(b.x);
    let wc = (1.0 - f.x) * f.y * worked_texel(c.x);
    let wd = f.x * f.y * worked_texel(d.x);
    out.amount = wa + wb + wc + wd;
    if (out.amount <= 0.0) {
        return out;
    }
    var best = a;
    var best_at = i;
    var best_weight = wa;
    if (wb > best_weight) { best = b; best_at = i + vec2<i32>(1, 0); best_weight = wb; }
    if (wc > best_weight) { best = c; best_at = i + vec2<i32>(0, 1); best_weight = wc; }
    if (wd > best_weight) { best = d; best_at = i + vec2<i32>(1, 1); best_weight = wd; }
    out.kind = best.x >> 12u;
    let angle = f32((best.x >> 4u) & 255u) / 255.0 * WORK_TAU - WORK_TAU * 0.5;
    out.along = vec2<f32>(cos(angle), sin(angle));
    out.depth = f32(best.x & 15u) / 15.0 * WORK_DEPTH_MAX_M;
    let axle = vec2<f32>(-out.along.y, out.along.x);
    let middle = params.origin + vec2<f32>(best_at) / params.texels_per_metre;
    out.across = (f32(best.y) - WORK_ACROSS_BIAS) * WORK_ACROSS_STEP_M + dot(world_xz - middle, axle);
    return out;
}

// --- The soil a disc harrow leaves -----------------------------------------

// Discs a quarter of a metre apart, each cutting and throwing a little soil
// sideways: low ridges between their cuts, half a spacing out of step between
// the front gang and the rear one.
const DISC_SPACING_M: f32 = 0.25;
const RIDGE_HEIGHT_M: f32 = 0.045;
// Cut soil takes up more room than settled soil does: the worked strip stands
// proud of the stubble beside it.
const LOOSENED_M: f32 = 0.055;
// Clods the discs break the soil into: fist-sized and smaller.
const CLOD_HEIGHT_M: f32 = 0.03;
const CLOD_SIZE_M: f32 = 0.075;

fn soil_hash2(p: vec2<f32>) -> vec2<f32> {
    let q = vec2<f32>(dot(p, vec2<f32>(127.1, 311.7)), dot(p, vec2<f32>(269.5, 183.3)));
    return fract(sin(q) * 43758.5453);
}

fn soil_hash1(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

fn soil_value(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = soil_hash1(i);
    let b = soil_hash1(i + vec2<f32>(1.0, 0.0));
    let c = soil_hash1(i + vec2<f32>(0.0, 1.0));
    let d = soil_hash1(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Rounded lumps on a jittered grid: each cell's clod is a dome, highest at
// its own middle and gone by the edge of its neighbour. Returns 0..1.
fn soil_clods(p: vec2<f32>) -> f32 {
    let cell = floor(p);
    let inside = fract(p);
    var lump = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let at = vec2<f32>(f32(x), f32(y));
            let jitter = soil_hash2(cell + at);
            let middle = at + 0.15 + jitter * 0.7;
            let size = 0.45 + soil_hash1(cell + at + 17.0) * 0.45;
            let gap = length(inside - middle) / size;
            lump = max(lump, (1.0 - gap * gap) * (0.6 + 0.4 * jitter.x));
        }
    }
    return clamp(lump, 0.0, 1.0);
}

// The worked soil's height above the ground it was cut from, in metres, and
// how much of that is ridge rather than clod: the colour reads both.
fn soil_relief(work: Work, world_xz: vec2<f32>) -> vec2<f32> {
    let along_m = dot(world_xz, work.along);
    // The ridges wander a little, as a disc rides over a stone and back.
    let wander = (soil_value(vec2<f32>(along_m * 0.9, work.across * 0.4)) - 0.5) * 0.06;
    let phase = (work.across + wander) / DISC_SPACING_M;
    let crest = 0.5 + 0.5 * cos(phase * WORK_TAU);
    let shoulder = 0.5 + 0.5 * cos((phase + 0.5) * WORK_TAU);
    // A disc throws its soil one way, so a ridge is steeper on one side.
    let ridge = pow(crest, 1.6) * 0.8 + pow(shoulder, 3.0) * 0.2;
    // Where the soil broke into clods the ridge does not run clean.
    let broken = 0.65 + 0.7 * soil_value(vec2<f32>(along_m * 2.3, phase * 1.7));
    let clods = soil_clods(world_xz / CLOD_SIZE_M) * 0.7
        + soil_clods(world_xz / (CLOD_SIZE_M * 2.6) + 31.0) * 0.45;
    let height = LOOSENED_M + RIDGE_HEIGHT_M * (ridge - 0.5) * broken + CLOD_HEIGHT_M * (clods - 0.55);
    return vec2<f32>(height, ridge);
}

// The slope of the ridges alone, in world XZ: what a surface too coarse to
// carry a clod still shows of the discs' work in its light, worked out from
// the ridge's own shape rather than by sampling it twice more.
fn ridge_slope(work: Work) -> vec2<f32> {
    let phase = work.across / DISC_SPACING_M;
    let crest = 0.5 + 0.5 * cos(phase * WORK_TAU);
    let rate = -0.5 * sin(phase * WORK_TAU) * WORK_TAU / DISC_SPACING_M;
    let slope = RIDGE_HEIGHT_M * 0.8 * 1.6 * pow(max(crest, 1.0e-4), 0.6) * rate;
    return vec2<f32>(-work.along.y, work.along.x) * slope;
}

// Crumb finer than any vertex, as a slope: the faces of small clods catching
// the sun. Value noise rather than the clods themselves, which cost a cell
// search per sample.
fn crumb_slope(world_xz: vec2<f32>) -> vec2<f32> {
    let p = world_xz / 0.022;
    let h = soil_value(p);
    return vec2<f32>(soil_value(p + vec2<f32>(0.35, 0.0)) - h, soil_value(p + vec2<f32>(0.0, 0.35)) - h) * 1.6;
}

// The colour of freshly worked soil: dark and damp where it was turned up,
// drier and paler on the crests and clod tops the air has reached, with
// chopped straw flecked through it where the discs mixed the stubble in.
fn soil_colour(base: vec3<f32>, work: Work, world_xz: vec2<f32>, relief: vec2<f32>) -> vec3<f32> {
    // The texture is the grain alone. Its own colour is the pale, olive crust
    // the stubble stood in; turned up, the earth under it is brown.
    let grain = clamp(dot(base, vec3<f32>(0.3, 0.59, 0.11)) / 0.085, 0.6, 1.4);
    let damp = vec3<f32>(0.072, 0.050, 0.033) * grain;
    let dry = vec3<f32>(0.170, 0.124, 0.084) * grain;
    let top = clamp((relief.x - LOOSENED_M) / (RIDGE_HEIGHT_M + CLOD_HEIGHT_M) * 0.5 + 0.5, 0.0, 1.0);
    let airing = smoothstep(0.45, 1.0, top) * (0.5 + 0.5 * soil_value(world_xz * 3.1));
    var colour = mix(damp, dry, airing);
    // The hollows between the ridges hold shade and damp.
    colour *= mix(0.72, 1.08, top);
    // Chopped straw: short flecks laid along the way the tool went, dulled by
    // the soil thrown over them.
    let along_m = dot(world_xz, work.along);
    let across_m = dot(world_xz, vec2<f32>(-work.along.y, work.along.x));
    let fleck = soil_hash1(floor(vec2<f32>(along_m / 0.06, across_m / 0.012)));
    let straw = step(0.95, fleck) * step(0.45, soil_value(world_xz * 1.7));
    colour = mix(colour, vec3<f32>(0.26, 0.20, 0.10), straw * 0.8);
    return colour;
}
