// Sows the meadow's blades: one invocation per root of a chunk, culled and
// shaped once, appended as a record that `blade_draw.wgsl` bends a strip from.
// The placement, culling and shaping are the Ghost of Tsushima blade the
// per-chunk vertex shader used to repeat for every vertex.

#import bevy_pbr::mesh_view_bindings::{view, globals}
#import "embedded://gearbox_fields/grassland/shaders/palette.wgsl"::{noise, meadow_pattern, meadow_tint, grass_species, dry_growth}
#import "embedded://gearbox_fields/shaders/wind.wgsl"::blade_leans
#import "embedded://gearbox_fields/shaders/interaction.wgsl"::{WheelMapParams, sample_wheels, wheel_roll, scatter_roll}
#import "embedded://gearbox_fields/bare/shaders/cover.wgsl"::{worn, inside_field}

struct VegetationParams {
    corner: vec2<f32>,
    origin: vec2<f32>,
    texels_per_metre: f32,
    chunk_size: f32,
    fade_start: f32,
    fade_end: f32,
    blades_per_chunk: f32,
    texel_count: f32,
    inverse_square_thinning: u32,
    bounds: vec4<f32>,
    wheels: WheelMapParams,
    wind: vec4<f32>,
    follow_grass: f32,
    tread: vec4<f32>,
    soft_border: f32,
    way: mat4x4<f32>,
    way_more: mat4x4<f32>,
    way_shape: vec4<f32>,
};

struct BladeDispatch {
    band: vec4<f32>,
    // instances, blades per instance, segments, record capacity
    counts: vec4<u32>,
};

struct BladeArgs {
    index_count: u32,
    instance_count: u32,
    first_index: u32,
    base_vertex: i32,
    first_instance: u32,
    sown: atomic<u32>,
    pad0: u32,
    pad1: u32,
};

@group(1) @binding(0) var<uniform> job: BladeDispatch;
@group(1) @binding(1) var<storage, read_write> records: array<u32>;
@group(1) @binding(2) var<storage, read_write> args: BladeArgs;

@group(3) @binding(0) var heightmap: texture_2d<f32>;
@group(3) @binding(1) var heightmap_sampler: sampler;
@group(3) @binding(2) var<uniform> field: VegetationParams;
@group(3) @binding(3) var trample: texture_2d<u32>;

const RECORD_WORDS: u32 = 16u;
const BLADE_FADE_M: f32 = 4.0;
const DIRT_SLOPE_NORMAL_Y: f32 = 0.86;
const GOT_MIN_HEIGHT: f32 = 0.04;
const GOT_MAX_HEIGHT: f32 = 0.10;
const GOT_MIN_WIDTH: f32 = 0.003;
const GOT_MAX_WIDTH: f32 = 0.0055;
const CLUMP_CELL_M: f32 = 0.45;
const LOD_JITTER_M: f32 = 1.5;
const CLUMP_PULL: f32 = 0.0;

// PCG integer hashing of instance indices.
fn pcg(input: u32) -> u32 {
    let state = input * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn rand(seed: u32, salt: u32) -> f32 {
    return f32(pcg(seed ^ (salt * 0x9E3779B9u))) / 4294967295.0;
}

// A hash of a place, fine enough that neighbouring plants get unrelated rolls.
fn edge_roll(p: vec2<f32>) -> f32 {
    let q = vec2<i32>(floor(p * 97.0));
    var h = u32(q.x) * 0x9E3779B9u ^ u32(q.y) * 0x85EBCA6Bu;
    h = h ^ (h >> 15u); h = h * 0x2C1B3C6Du; h = h ^ (h >> 12u);
    return f32(h) / 4294967295.0;
}

fn within_field(world_xz: vec2<f32>) -> bool {
    let inside = inside_field(world_xz, field.bounds, vec4<f32>(field.soft_border));
    if (field.soft_border <= 0.0) {
        return inside >= 0.0;
    }
    let past = max(-inside, 0.0);
    return edge_roll(world_xz) < 1.0 - smoothstep(0.0, field.soft_border, past);
}

fn blade_fade_end(rank: f32) -> f32 {
    if field.inverse_square_thinning == 0u {
        return mix(field.fade_end, field.fade_start, sqrt(rank));
    }
    return field.fade_start * field.fade_end
        / (field.fade_start + (field.fade_end - field.fade_start) * sqrt(rank));
}

// Bilinear read of (height, normal.x, normal.z) at a world XZ position.
fn sample_field(world_xz: vec2<f32>) -> vec3<f32> {
    let t = (world_xz - field.origin) * field.texels_per_metre;
    let max_index = i32(field.texel_count) - 1;
    let i = clamp(vec2<i32>(floor(t)), vec2<i32>(0), vec2<i32>(max_index - 1));
    let f = clamp(t - vec2<f32>(i), vec2<f32>(0.0), vec2<f32>(1.0));
    let s00 = textureLoad(heightmap, i, 0).xyz;
    let s10 = textureLoad(heightmap, i + vec2<i32>(1, 0), 0).xyz;
    let s01 = textureLoad(heightmap, i + vec2<i32>(0, 1), 0).xyz;
    let s11 = textureLoad(heightmap, i + vec2<i32>(1, 1), 0).xyz;
    return mix(mix(s00, s10, f.x), mix(s01, s11, f.x), f.y);
}

fn way_wear(place: vec2<f32>) -> f32 {
    return worn(place, field.bounds, field.tread, field.way, field.way_more, field.way_shape);
}

struct Clump {
    centre: vec2<f32>,
    id: u32,
};

// Nearest Voronoi clump centre: blades of a clump share height, facing,
// lean and tone.
fn clump_of(p: vec2<f32>) -> Clump {
    let cell = vec2<i32>(floor(p / CLUMP_CELL_M));
    var best = Clump(p, 0u);
    var best_d = 1e9;
    for (var dz = -1; dz <= 1; dz = dz + 1) {
        for (var dx = -1; dx <= 1; dx = dx + 1) {
            let c = cell + vec2<i32>(dx, dz);
            let h = pcg(bitcast<u32>(c.x) * 73856093u ^ bitcast<u32>(c.y) * 19349663u ^ 0x2C1A5E7u);
            let centre = (vec2<f32>(c) + vec2<f32>(rand(h, 1u), rand(h, 2u))) * CLUMP_CELL_M;
            let d = dot(p - centre, p - centre);
            if (d < best_d) {
                best_d = d;
                best = Clump(centre, h);
            }
        }
    }
    return best;
}

// Low-discrepancy (R2) placement: any prefix of a chunk's instances covers it
// evenly, so thinning by rank never opens holes.
fn r2(index: u32, seed: u32) -> vec2<f32> {
    let x = index * 3242174889u + seed;
    let y = index * 2447445414u + pcg(seed);
    return vec2<f32>(f32(x >> 8u), f32(y >> 8u)) / 16777216.0;
}

struct Blade {
    alive: bool,
    root: vec3<f32>,
    width: f32,
    mid: vec3<f32>,
    tip: vec3<f32>,
    facing: vec2<f32>,
    roll: vec2<f32>,
    press: f32,
    flat: f32,
    slope: vec2<f32>,
    root_colour: vec3<f32>,
    tip_colour: vec3<f32>,
};

fn dead() -> Blade {
    var blade: Blade;
    blade.alive = false;
    return blade;
}

// Whether a sphere lies inside the view's side and near half spaces.
fn in_view(centre: vec3<f32>, radius: f32) -> bool {
    for (var i = 0u; i < 5u; i = i + 1u) {
        let plane = view.frustum[i];
        if (dot(plane.xyz, centre) + plane.w < -radius) {
            return false;
        }
    }
    return true;
}

fn sow(index: u32, twin: bool) -> Blade {
    let chunk_seed = pcg(bitcast<u32>(i32(field.corner.x)) * 73856093u
        ^ bitcast<u32>(i32(field.corner.y)) * 19349663u);
    let id = pcg(index ^ chunk_seed);
    let spot = field.corner + r2(index, chunk_seed) * field.chunk_size;
    let own_yaw = rand(id, 4u) * 6.2831853 + select(0.0, mix(0.6, 1.2, rand(id, 30u)), twin);
    var base_xz = spot;
    if (CLUMP_PULL > 0.0) {
        let pulled = clump_of(spot);
        base_xz = mix(spot, pulled.centre, CLUMP_PULL * rand(pulled.id, 7u));
    }
    if (twin) {
        base_xz += vec2<f32>(-sin(own_yaw), cos(own_yaw)) * mix(0.01, 0.03, rand(id, 31u));
    }

    // Density fades by rank; each blade swaps detail level at a jittered
    // band edge; blades under ~1.2 px wide are widened and thinned alike.
    let rank = f32(index) / max(field.blades_per_chunk, 1.0);
    let blade_end = blade_fade_end(rank);
    let jitter = (rand(id, 23u) - 0.5) * 2.0 * LOD_JITTER_M;
    let across = length(base_xz - view.world_position.xz);
    let across_px = across * 2.0 / (view.clip_from_view[1][1] * view.viewport.w);
    if (across >= blade_end || across >= job.band.y + jitter
        || rand(id, 19u) * max(1.0, 1.2 * across_px / GOT_MAX_WIDTH) >= 1.0) {
        return dead();
    }

    let sampled = sample_field(base_xz);
    let root = vec3<f32>(base_xz.x, sampled.x, base_xz.y);
    let ground_normal = normalize(vec3<f32>(sampled.y, 1.0, sampled.z));
    let distance = length(root - view.world_position);
    let coverage = 1.0 - smoothstep(max(field.fade_start, blade_end - BLADE_FADE_M), blade_end, distance);
    let in_band = (job.band.x <= 0.0 || distance >= job.band.x + jitter)
        && distance < job.band.y + jitter;
    let pixel_m = distance * 2.0 / (view.clip_from_view[1][1] * view.viewport.w);
    let widen = max(1.0, 1.2 * pixel_m / GOT_MAX_WIDTH);
    let alive = select(0.0, coverage, in_band && ground_normal.y >= DIRT_SLOPE_NORMAL_Y
        && within_field(base_xz) && rand(id, 19u) * widen < 1.0);
    if (alive <= 0.0) {
        return dead();
    }
    // A blade stands at most a quarter metre from its root.
    if (!in_view(root + vec3<f32>(0.0, 0.08, 0.0), 0.25)) {
        return dead();
    }

    // A way worn across the meadow takes the sward with it, blade by blade.
    let bared = way_wear(base_xz);
    if (rand(id, 41u) < smoothstep(0.08, 0.45, bared)) {
        return dead();
    }
    let clump = clump_of(spot);

    let species = grass_species(base_xz);
    let pick = rand(id, 13u);
    let fescue = pick < species.x;
    let rye = !fescue && pick < species.x + species.y;
    let height = mix(GOT_MIN_HEIGHT, GOT_MAX_HEIGHT, rand(id, 6u))
        * mix(0.9, 1.1, rand(clump.id, 3u)) * select(1.0, 0.8, fescue)
        * select(1.0, mix(0.75, 0.95, rand(id, 32u)), twin) * alive * dry_growth(base_xz)
        * (1.0 - bared * 0.55);
    let width = mix(GOT_MIN_WIDTH, GOT_MAX_WIDTH, rand(id, 7u))
        * select(select(1.0, 1.6, rye), 0.6, fescue) * widen * alive;

    // Facing follows the clump; blades lean out from its centre.
    let clump_yaw = rand(clump.id, 4u) * 6.2831853;
    let facing = normalize(mix(vec2<f32>(cos(own_yaw), sin(own_yaw)),
        vec2<f32>(cos(clump_yaw), sin(clump_yaw)), 0.6) + vec2<f32>(1e-4, 0.0));
    let outward = base_xz - clump.centre;
    let out_dir = select(facing, normalize(outward), dot(outward, outward) > 1e-6);
    let lean_xz = normalize(mix(vec2<f32>(cos(own_yaw + 1.7), sin(own_yaw + 1.7)), out_dir, 0.7)
        + vec2<f32>(1e-4, 0.0));
    let lean = vec3<f32>(lean_xz.x, 0.0, lean_xz.y)
        * mix(0.25, 0.7, rand(id, 8u)) * mix(0.8, 1.2, rand(clump.id, 5u));

    // Wind: the shared gust field pushes, and the blade's own bob runs
    // along it so it sways instead of pivoting.
    let gust = blade_leans(base_xz, globals.time, field.wind, rand(id, 9u));
    let downwind = vec3<f32>(gust.x, 0.0, gust.y);
    let sway_mid = downwind * gust.z * 0.3;
    let sway_tip = downwind * gust.w;

    // Wheels lay blades along their roll with some scatter, face up; the
    // stiffest spring back first as the track recovers.
    let pressed = sample_wheels(trample, field.wheels, base_xz);
    let flat = clamp(pressed.x, 0.0, 1.0);
    let roll = scatter_roll(wheel_roll(pressed), (rand(id, 37u) - 0.5) * 0.6);
    let up = vec3<f32>(0.0, 1.0, 0.0);
    let press = smoothstep(0.0, mix(0.4, 1.0, rand(id, 36u)), flat) * field.wheels.bend;
    let tip = mix((up + lean + sway_tip * (1.0 - flat)) * height, (roll * 0.92 + up * 0.06) * height, press);
    let mid = mix((up * 0.6 + lean * 0.2 + sway_mid * (1.0 - flat)) * height, (roll * 0.5 + up * 0.07) * height, press);

    // Dark roots to bright tips, the tip colour set by the clump; patches a
    // few metres across and tufts under a metre shade the sward from above.
    let tone = mix(0.9, 1.1, rand(id, 5u)) * mix(0.85, 1.12, rand(clump.id, 8u));
    let tint_patch = smoothstep(0.3, 0.7, noise(base_xz * 0.11 + vec2<f32>(13.0, -7.0)));
    let tuft = noise(base_xz * 1.3 + vec2<f32>(-29.0, 41.0));
    let mottle = mix(vec3<f32>(1.08, 1.02, 0.80), vec3<f32>(0.82, 0.95, 0.90), tint_patch) * mix(0.8, 1.15, tuft);
    let clump_hue = rand(clump.id, 6u);
    let dry = select(0.0, 0.3 + 0.7 * rand(id, 11u), rand(id, 12u) < 0.15);
    var base_colour = mix(vec3<f32>(0.045, 0.120, 0.022), vec3<f32>(0.055, 0.150, 0.026), clump_hue);
    var tip_colour = mix(vec3<f32>(0.50, 0.66, 0.18), vec3<f32>(0.62, 0.74, 0.28), clump_hue);
    tip_colour = mix(tip_colour, vec3<f32>(0.66, 0.58, 0.26), dry);
    if (fescue) {
        base_colour = vec3<f32>(0.045, 0.115, 0.065);
        tip_colour = vec3<f32>(0.40, 0.58, 0.30);
    } else if (rye) {
        base_colour = vec3<f32>(0.035, 0.125, 0.022);
        tip_colour = vec3<f32>(0.30, 0.62, 0.10);
    }
    let shade = tone * mottle * (1.0 - field.wheels.darkening * flat)
        * meadow_tint(meadow_pattern(base_xz));

    var blade: Blade;
    blade.alive = true;
    blade.root = root;
    blade.width = width;
    blade.mid = mid;
    blade.tip = tip;
    blade.facing = facing;
    blade.roll = roll.xz;
    blade.press = press;
    blade.flat = flat;
    blade.slope = sampled.yz;
    blade.root_colour = base_colour * shade;
    blade.tip_colour = tip_colour * shade;
    return blade;
}

fn write(at: u32, blade: Blade) {
    let o = at * RECORD_WORDS;
    records[o] = bitcast<u32>(blade.root.x);
    records[o + 1u] = bitcast<u32>(blade.root.y);
    records[o + 2u] = bitcast<u32>(blade.root.z);
    records[o + 3u] = bitcast<u32>(blade.width);
    records[o + 4u] = pack2x16float(blade.mid.xy);
    records[o + 5u] = pack2x16float(vec2<f32>(blade.mid.z, blade.tip.x));
    records[o + 6u] = pack2x16float(blade.tip.yz);
    records[o + 7u] = pack2x16float(blade.facing);
    records[o + 8u] = pack2x16float(blade.roll);
    records[o + 9u] = pack2x16float(vec2<f32>(blade.press, blade.flat));
    records[o + 10u] = pack2x16float(blade.slope);
    records[o + 11u] = pack2x16float(blade.root_colour.rg);
    records[o + 12u] = pack2x16float(vec2<f32>(blade.root_colour.b, blade.tip_colour.r));
    records[o + 13u] = pack2x16float(blade.tip_colour.gb);
    records[o + 14u] = job.counts.z;
    records[o + 15u] = 0u;
}

var<workgroup> kept: atomic<u32>;
var<workgroup> first: u32;

@compute @workgroup_size(64)
fn sow_blades(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    if (lane == 0u) {
        atomicStore(&kept, 0u);
    }
    workgroupBarrier();
    var blades: array<Blade, 2>;
    var count = 0u;
    if (gid.x < job.counts.x) {
        for (var twin = 0u; twin < min(job.counts.y, 2u); twin = twin + 1u) {
            let blade = sow(gid.x, twin == 1u);
            if (blade.alive) {
                blades[count] = blade;
                count = count + 1u;
            }
        }
    }
    let slot = atomicAdd(&kept, count);
    workgroupBarrier();
    if (lane == 0u) {
        first = atomicAdd(&args.sown, atomicLoad(&kept));
    }
    workgroupBarrier();
    let at = first + slot;
    for (var i = 0u; i < count; i = i + 1u) {
        if (at + i < job.counts.w) {
            write(at + i, blades[i]);
        }
    }
}

// The sown count, clamped to what the records hold, becomes the instance count.
@compute @workgroup_size(1)
fn finish() {
    args.instance_count = min(atomicLoad(&args.sown), job.counts.w);
}
