#import bevy_pbr::{
    mesh_view_bindings::{view, globals},
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    pbr_types::{pbr_input_new, STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing, calculate_view},
}

#import "embedded://gearbox_fields/grassland/shaders/palette.wgsl"::{noise, meadow_pattern, meadow_tint, grass_species, species_tint, dry_country, dry_growth, meadow_share}
#import "embedded://gearbox_fields/shaders/canopy.wgsl"::{canopy_vertex, canopy_alpha}
#import "embedded://gearbox_fields/shaders/wind.wgsl"::{plant_lean, blade_leans}
#import "embedded://gearbox_fields/shaders/surface_detail.wgsl"::{surface_lighting, foliage_normal}

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

@group(3) @binding(0) var heightmap: texture_2d<f32>;
@group(3) @binding(1) var heightmap_sampler: sampler;
@group(3) @binding(2) var<uniform> field: VegetationParams;
@group(3) @binding(3) var trample: texture_2d<u32>;

fn sample_trample(world_xz: vec2<f32>) -> vec3<f32> {
    return sample_wheels(trample, field.wheels, world_xz);
}

// A hash of a place, fine enough that neighbouring plants get unrelated rolls.
fn edge_roll(p: vec2<f32>) -> f32 {
    let q = vec2<i32>(floor(p * 97.0));
    var h = u32(q.x) * 0x9E3779B9u ^ u32(q.y) * 0x85EBCA6Bu;
    h = h ^ (h >> 15u); h = h * 0x2C1B3C6Du; h = h ^ (h >> 12u);
    return f32(h) / 4294967295.0;
}

// A hard border ends a field along a ruled line. A soft one lets what grows
// here carry past its own edge, thinning as it goes, so two fields interleave
// over that distance instead of butting against one another.
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

// The blade template: position.x is the side (-1, 0, 1), position.y the
// height fraction. Everything else is derived from the instance index.
struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) @interpolate(perspective, centroid) world_position: vec4<f32>,
    @location(1) @interpolate(perspective, centroid) world_normal: vec3<f32>,
    @location(2) @interpolate(perspective, centroid) color: vec4<f32>,
    @location(3) @interpolate(perspective, centroid) canopy_uv: vec3<f32>,
    @location(4) @interpolate(perspective, centroid) ground_normal: vec3<f32>,
};

// Sway is a fraction of the blade height.
const WIND_SWAY: f32 = 0.3;
const BLADE_MIN_HEIGHT: f32 = 0.04;
const BLADE_MAX_HEIGHT: f32 = 0.10;
const BLADE_MIN_WIDTH: f32 = 0.006;
const BLADE_MAX_WIDTH: f32 = 0.011;
const BLADE_FADE_M: f32 = 4.0;
const DIRT_SLOPE_NORMAL_Y: f32 = 0.86;

// PCG integer hashing of instance indices.
fn pcg(input: u32) -> u32 {
    let state = input * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn rand(seed: u32, salt: u32) -> f32 {
    return f32(pcg(seed ^ (salt * 0x9E3779B9u))) / 4294967295.0;
}

fn hash11(p: f32) -> f32 {
    return rand(bitcast<u32>(p), 7u);
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

// How far a way has worn the sward away here. Read by both blade paths and by
// the meadow material, so the grass stops exactly where the earth shows.
fn way_wear(place: vec2<f32>) -> f32 {
    return worn(place, field.bounds, field.tread, field.way, field.way_more, field.way_shape);
}

fn meadow_noise(p: vec2<f32>) -> f32 {
    let cell = vec2<i32>(floor(p));
    let f = fract(p);
    let blend = f * f * (3.0 - 2.0 * f);
    let a = pcg(bitcast<u32>(cell.x) ^ pcg(bitcast<u32>(cell.y)));
    let b = pcg(bitcast<u32>(cell.x + 1) ^ pcg(bitcast<u32>(cell.y)));
    let c = pcg(bitcast<u32>(cell.x) ^ pcg(bitcast<u32>(cell.y + 1)));
    let d = pcg(bitcast<u32>(cell.x + 1) ^ pcg(bitcast<u32>(cell.y + 1)));
    return mix(mix(rand(a, 31u), rand(b, 31u), blend.x),
        mix(rand(c, 31u), rand(d, 31u), blend.x), blend.y);
}

fn meadow_detail(vertex: Vertex) -> VertexOutput {
    let chunk_seed = pcg(bitcast<u32>(i32(field.corner.x)) * 73856093u
        ^ bitcast<u32>(i32(field.corner.y)) * 19349663u);
    let id = pcg(vertex.instance_index ^ chunk_seed ^ 0x8DA6B343u);
    let base = field.corner + vec2<f32>(rand(id, 1u), rand(id, 2u)) * field.chunk_size;
    let sampled = sample_field(base);
    let ground = vec3<f32>(base.x, sampled.x, base.y);
    let ground_normal = normalize(vec3<f32>(sampled.y, 1.0, sampled.z));
    let distance = length(ground - view.world_position);
    let rank = f32(vertex.instance_index) / max(field.blades_per_chunk, 1.0);
    let end = blade_fade_end(rank);
    let coverage = (1.0 - smoothstep(max(field.fade_start, end - BLADE_FADE_M), end, distance))
        * select(0.0, 1.0, ground_normal.y >= DIRT_SLOPE_NORMAL_Y && within_field(base));
    if (coverage <= 0.0) {
        return culled_vertex();
    }
    // Nothing stands in a road. A flat chance against the wear left a seventh
    // of the sward upright in a fully worn track, which reads as grass dropped
    // on a road rather than a road worn through grass; this clears it by the
    // time the ground is half bare and thins the verge either side of that.
    if (rand(id, 41u) < smoothstep(0.08, 0.45, way_wear(base))) {
        return culled_vertex();
    }
    let habitat = meadow_noise(base * 0.8 + vec2<f32>(7.2, 3.1));
    let clover = rand(id, 9u) < 0.18 + smoothstep(0.38, 0.70, habitat) * 0.62;
    let rosette = !clover && rand(id, 17u) < 0.14;
    let leaf = u32(vertex.position.z - 1.0);
    let t = vertex.position.y;
    let side = vertex.position.x;
    let yaw = rand(id, 4u) * 6.2831853;
    let tone = 0.85 + 0.25 * rand(id, 5u);
    var offset: vec3<f32>;
    var normal: vec3<f32>;
    var color: vec3<f32>;

    if (clover) {
        let head = leaf / 3u;
        let stem_yaw = yaw + f32(head) * 2.7;
        let stem_dir = vec3<f32>(cos(stem_yaw), 0.0, sin(stem_yaw));
        let angle = yaw + f32(leaf % 3u) * 2.0943951 + f32(head) * 0.9;
        let forward = vec3<f32>(cos(angle), 0.0, sin(angle));
        let right = vec3<f32>(-sin(angle), 0.0, cos(angle));
        let size = 0.8 + 0.4 * rand(id, 11u + head);
        let leaf_width = 0.011 * pow(max(sin(t * 3.14159265), 0.0), 0.65);
        let cup = 0.003 * side * side * sin(t * 3.14159265);
        offset = stem_dir * 0.035 + vec3<f32>(0.0, 0.105 + 0.015 * f32(head), 0.0)
            + forward * (0.002 + t * 0.031 * size)
            + right * (side * leaf_width * 0.5 * size)
            + vec3<f32>(0.0, 0.009 * t - 0.006 * t * t + cup, 0.0);
        let band = 1.0 - smoothstep(0.035, 0.105, abs(t - (0.54 + 0.16 * abs(side))));
        color = mix(vec3<f32>(0.055, 0.18, 0.065), vec3<f32>(0.24, 0.36, 0.17), band * 0.7) * tone;
        normal = normalize(ground_normal - forward * 0.15 + right * side * 0.25);
    } else if (rosette) {
        let angle = yaw + f32(leaf) * 2.3999632;
        let forward = vec3<f32>(cos(angle), 0.0, sin(angle));
        let right = vec3<f32>(-sin(angle), 0.0, cos(angle));
        let size = 0.8 + 0.35 * rand(id, 20u + leaf);
        let profile = pow(max(sin(t * 3.14159265), 0.0), 0.8);
        offset = forward * (0.008 + t * 0.085 * size)
            + right * (side * 0.010 * profile * size)
            + vec3<f32>(0.0, 0.035 + 0.095 * sin(t * 2.0), 0.0);
        normal = normalize(ground_normal - forward * cos(t * 2.0) * 0.7 + right * side * 0.2);
        let vein = 1.0 - abs(side);
        color = mix(vec3<f32>(0.09, 0.21, 0.035), vec3<f32>(0.16, 0.29, 0.07), vein * 0.4) * tone;
    } else {
        return meadow_grass(base, ground, ground_normal, id, leaf, t, side, yaw, tone, coverage, distance);
    }

    // Far plants settle onto the sward's far colour, as its blades do: a dark
    // leaf a few pixels wide reads as a speck, not as a plant.
    color = mix(color, vec3<f32>(0.16, 0.233, 0.067) * tone, smoothstep(10.0, 40.0, distance));

    let pressed = sample_trample(base);
    let flat = clamp(pressed.x, 0.0, 1.0);
    offset += wheel_roll(pressed) * (flat * offset.y * 0.9);
    offset.y *= 1.0 - flat * field.wheels.bend;
    offset += plant_lean(base, globals.time, field.wind, rand(id, 9u), t)
        * (max(offset.y, 0.0) * t * 1.2 * (1.0 - flat));
    let p = ground + offset * coverage;
    var out: VertexOutput;
    out.world_position = vec4<f32>(p, 1.0);
    out.clip_position = view.clip_from_world * out.world_position;
    // Far plants take the ground's normal: one tilted away from a low sun
    // would otherwise drop out black against lit ground.
    let settle = smoothstep(6.0, 30.0, distance) * 0.9;
    out.world_normal = normalize(mix(normal, ground_normal, max(flat, settle)));
    out.ground_normal = ground_normal;
    out.color = vec4<f32>(color * (1.0 - field.wheels.darkening * flat), 1.0);
    return out;
}

// The meadow's taller grasses shaped like the sward's blades (talk): a Bezier
// leaf arching out of its tuft, swaying in the shared wind, widened on screen
// when seen edge-on, with rounded sky-leaning normals.
fn meadow_grass(base: vec2<f32>, ground: vec3<f32>, ground_normal: vec3<f32>, id: u32, leaf: u32,
    t: f32, side: f32, yaw: f32, tone: f32, coverage: f32, distance: f32) -> VertexOutput {
    let angle = yaw + f32(leaf) * 2.3999632 + rand(id, 12u + leaf) * 0.3;
    let out_dir = vec3<f32>(cos(angle), 0.0, sin(angle));
    let species = grass_species(base);
    let variety = rand(id, 10u);
    let fine = variety < 0.25 + 0.5 * species.x;
    let broad = !fine && variety > 0.8 - 0.5 * species.y;
    let height = mix(0.12, 0.20, rand(id, 20u + leaf)) * select(1.0, 0.9, broad) * coverage
        * dry_growth(base) * (1.0 - way_wear(base) * 0.55);
    let pixel_m = distance * 2.0 / (view.clip_from_view[1][1] * view.viewport.w);
    let base_w = mix(0.0025, 0.004, rand(id, 30u + leaf)) * select(select(1.0, 1.8, broad), 0.6, fine);
    let width = base_w * max(1.0, 1.2 * pixel_m / base_w) * coverage;
    let arch = select(select(0.45, 0.7, broad), 0.3, fine) + 0.25 * rand(id, 40u + leaf);

    // Arched out of the tuft, pushed downwind; wheels lay it along the roll.
    let up = vec3<f32>(0.0, 1.0, 0.0);
    let pressed = sample_trample(base);
    let flat = clamp(pressed.x, 0.0, 1.0);
    let roll = scatter_roll(wheel_roll(pressed), (rand(id, 37u + leaf) - 0.5) * 0.6);
    let press = smoothstep(0.0, mix(0.4, 1.0, rand(id, 36u + leaf)), flat) * field.wheels.bend;
    let seed = rand(id, 9u + leaf);
    let gust = blade_leans(base, globals.time, field.wind, seed);
    let downwind = vec3<f32>(gust.x, 0.0, gust.y);
    let leans = gust.zw;
    let sway_mid = downwind * leans.x * 0.3;
    let sway_tip = downwind * leans.y;
    let tip = mix((up * (1.0 - 0.4 * arch) + out_dir * arch + sway_tip * (1.0 - flat)) * height,
        (roll * 0.92 + up * 0.06) * height, press);
    let mid = mix((up * 0.6 + out_dir * (0.3 * arch) + sway_mid * (1.0 - flat)) * height,
        (roll * 0.5 + up * 0.07) * height, press);

    let axis = normalize(bezier_tangent(mid, tip, t) + up * 1e-4);
    let right = normalize(cross(axis, out_dir) + vec3<f32>(1e-5, 0.0, 0.0));
    let normal = normalize(cross(right, axis));
    let half_w = width * 0.5 * ease_out(1.0 - t, 2.0) * side;
    let p = ground + bezier(mid, tip, t) + right * half_w;
    let to_eye = normalize(view.world_position - p);
    let face_xz = normalize(vec2<f32>(normal.x, normal.z) + vec2<f32>(1e-5, 0.0));
    let eye_xz = normalize(vec2<f32>(to_eye.x, to_eye.z) + vec2<f32>(1e-5, 0.0));
    let facing_eye = abs(dot(face_xz, eye_xz));
    let thicken = ease_out(1.0 - facing_eye, 4.0) * smoothstep(0.0, 0.2, facing_eye);
    var view_pos = view.view_from_world * vec4<f32>(p, 1.0);
    let right_view = (view.view_from_world * vec4<f32>(right, 0.0)).x;
    view_pos.x += thicken * select(-1.0, 1.0, right_view >= 0.0) * half_w;

    // Dark roots to bright tips per species, settling with distance.
    let rounded = normalize(normal + right * side * 0.6);
    let sky = normalize(mix(ground_normal, rounded, 0.35));
    let settle = smoothstep(8.0, 40.0, distance);
    var root = vec3<f32>(0.05, 0.12, 0.025);
    var top = vec3<f32>(0.46, 0.62, 0.17);
    if (fine) {
        root = vec3<f32>(0.055, 0.12, 0.055);
        top = vec3<f32>(0.44, 0.60, 0.26);
    } else if (broad) {
        root = vec3<f32>(0.045, 0.11, 0.022);
        top = vec3<f32>(0.34, 0.56, 0.14);
    }
    let occlusion = mix(0.5, 1.0, t * t);
    let ramp = mix(root, top, t * t) * tone * occlusion;
    // Far tufts take the sward's far colour: a darker tuft a few pixels wide
    // reads as a speck.
    let average = mix(mix(root, top, 0.35) * 0.7, vec3<f32>(0.16, 0.233, 0.067), 0.75) * tone;
    let colour = mix(ramp, average, smoothstep(4.0, 20.0, distance));

    var out: VertexOutput;
    out.world_position = vec4<f32>(p, 1.0);
    out.clip_position = view.clip_from_view * view_pos;
    out.world_normal = normalize(mix(mix(sky, ground_normal, settle * 0.8), ground_normal, flat));
    out.ground_normal = ground_normal;
    out.canopy_uv = vec3<f32>(side, t, 0.0);
    out.color = vec4<f32>(colour * (1.0 - field.wheels.darkening * flat), 1.0);
    return out;
}

const GOT_MIN_HEIGHT: f32 = 0.04;
const GOT_MAX_HEIGHT: f32 = 0.10;
const GOT_MIN_WIDTH: f32 = 0.003;
const GOT_MAX_WIDTH: f32 = 0.0055;
const CLUMP_CELL_M: f32 = 0.45;
const LOD_JITTER_M: f32 = 1.5;

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

// Quadratic Bezier from the root (origin) through `mid` to `tip`.
fn bezier(mid: vec3<f32>, tip: vec3<f32>, t: f32) -> vec3<f32> {
    return 2.0 * (1.0 - t) * t * mid + t * t * tip;
}

fn bezier_tangent(mid: vec3<f32>, tip: vec3<f32>, t: f32) -> vec3<f32> {
    return 2.0 * (1.0 - t) * mid + 2.0 * t * (tip - mid);
}

const CLUMP_PULL: f32 = 0.0;

// Low-discrepancy (R2) placement: any prefix of a chunk's instances covers it
// evenly, so thinning by rank never opens holes (the talk's jittered grid).
fn r2(index: u32, seed: u32) -> vec2<f32> {
    let x = index * 3242174889u + seed;
    let y = index * 2447445414u + pcg(seed);
    return vec2<f32>(f32(x >> 8u), f32(y >> 8u)) / 16777216.0;
}

fn ease_out(x: f32, power: f32) -> f32 {
    return 1.0 - pow(1.0 - x, power);
}

// A Ghost of Tsushima style blade: evenly placed, pulled into clump tufts,
// folded into twin blades from one root, a tapered Bezier shaped by the
// clump and the wind, thickened along the screen when seen edge-on, with
// rounded sky-leaning normals and a dark-root to bright-tip ramp.
// vertex.uv is the distance band this detail level draws in.
fn got_blade(vertex: Vertex) -> VertexOutput {
    let chunk_seed = pcg(bitcast<u32>(i32(field.corner.x)) * 73856093u
        ^ bitcast<u32>(i32(field.corner.y)) * 19349663u);
    let id = pcg(vertex.instance_index ^ chunk_seed);
    let spot = field.corner + r2(vertex.instance_index, chunk_seed) * field.chunk_size;
    let clump = clump_of(spot);
    let twin = vertex.normal.x > 0.5;
    let own_yaw = rand(id, 4u) * 6.2831853 + select(0.0, mix(0.6, 1.2, rand(id, 30u)), twin);
    var base_xz = mix(spot, clump.centre, CLUMP_PULL * rand(clump.id, 7u));
    if (twin) {
        base_xz += vec2<f32>(-sin(own_yaw), cos(own_yaw)) * mix(0.01, 0.03, rand(id, 31u));
    }
    let sampled = sample_field(base_xz);
    let root = vec3<f32>(base_xz.x, sampled.x, base_xz.y);
    let ground_normal = normalize(vec3<f32>(sampled.y, 1.0, sampled.z));
    let distance = length(root - view.world_position);

    // Density fades by rank; each blade swaps detail level at a jittered
    // band edge; blades under ~1.2 px wide are widened and thinned alike.
    let rank = f32(vertex.instance_index) / max(field.blades_per_chunk, 1.0);
    let blade_end = blade_fade_end(rank);
    let coverage = 1.0 - smoothstep(max(field.fade_start, blade_end - BLADE_FADE_M), blade_end, distance);
    let jitter = (rand(id, 23u) - 0.5) * 2.0 * LOD_JITTER_M;
    let in_band = (vertex.uv.x <= 0.0 || distance >= vertex.uv.x + jitter)
        && distance < vertex.uv.y + jitter;
    let pixel_m = distance * 2.0 / (view.clip_from_view[1][1] * view.viewport.w);
    let widen = max(1.0, 1.2 * pixel_m / GOT_MAX_WIDTH);
    let alive = select(0.0, coverage, in_band && ground_normal.y >= DIRT_SLOPE_NORMAL_Y
        && within_field(base_xz) && rand(id, 19u) * widen < 1.0);
    if (alive <= 0.0) {
        return culled_vertex();
    }

    // A way worn across the meadow takes the sward with it. A chance per blade
    // rather than a shrinking of them all, so the track's edge is ragged with
    // stragglers rather than mown to a line; what holds on near it is shorter.
    let bared = way_wear(base_xz);
    if (rand(id, 41u) < smoothstep(0.08, 0.45, bared)) {
        return culled_vertex();
    }

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
    let seed = rand(id, 9u);
    let gust = blade_leans(base_xz, globals.time, field.wind, seed);
    let downwind = vec3<f32>(gust.x, 0.0, gust.y);
    let leans = gust.zw;
    let sway_mid = downwind * leans.x * 0.3;
    let sway_tip = downwind * leans.y;

    // Control points relative to the root. Wheels lay blades along their
    // roll with some scatter, face up; the stiffest spring back first as the
    // track recovers.
    let pressed = sample_trample(base_xz);
    let flat = clamp(pressed.x, 0.0, 1.0);
    let roll = scatter_roll(wheel_roll(pressed), (rand(id, 37u) - 0.5) * 0.6);
    let up = vec3<f32>(0.0, 1.0, 0.0);
    let press = smoothstep(0.0, mix(0.4, 1.0, rand(id, 36u)), flat) * field.wheels.bend;
    let tip = mix((up + lean + sway_tip * (1.0 - flat)) * height, (roll * 0.92 + up * 0.06) * height, press);
    let mid = mix((up * 0.6 + lean * 0.2 + sway_mid * (1.0 - flat)) * height, (roll * 0.5 + up * 0.07) * height, press);

    let t = vertex.position.y;
    let side = vertex.position.x;
    let axis = normalize(bezier_tangent(mid, tip, t) + up * 1e-4);
    let stand_right = cross(axis, vec3<f32>(facing.x, 0.0, facing.y));
    let lay = cross(up, roll);
    let lay_right = select(-lay, lay, dot(lay, stand_right) >= 0.0);
    let right = normalize(mix(stand_right, lay_right, press) + vec3<f32>(1e-5, 0.0, 0.0));
    let normal = normalize(cross(right, axis));
    // Wide at the root, tapering fast to the tip.
    let half_w = width * 0.5 * ease_out(1.0 - t, 2.0) * side;
    let p = root + bezier(mid, tip, t) + right * half_w;

    // View-space thickening (talk, 14:05): a blade edge-on to the view is
    // widened along the screen, easing off right at edge-on.
    let to_eye = normalize(view.world_position - p);
    let face_xz = normalize(vec2<f32>(normal.x, normal.z) + vec2<f32>(1e-5, 0.0));
    let eye_xz = normalize(vec2<f32>(to_eye.x, to_eye.z) + vec2<f32>(1e-5, 0.0));
    let facing_eye = abs(dot(face_xz, eye_xz));
    let thicken = ease_out(1.0 - facing_eye, 4.0) * smoothstep(0.0, 0.2, facing_eye);
    var view_pos = view.view_from_world * vec4<f32>(p, 1.0);
    let right_view = (view.view_from_world * vec4<f32>(right, 0.0)).x;
    view_pos.x += thicken * select(-1.0, 1.0, right_view >= 0.0) * half_w;

    // Normals: tilted outward per half for a rounded blade, mostly sky
    // facing so blades light like the sward, settling onto the ground
    // normal with distance so far grass does not glitter.
    let rounded = normalize(normal + right * side * 0.6);
    let sky = normalize(mix(ground_normal, rounded, 0.35));
    let settle = smoothstep(8.0, 40.0, distance);

    // Dark roots to bright tips, the tip colour set by the clump.
    let tone = mix(0.9, 1.1, rand(id, 5u)) * mix(0.85, 1.12, rand(clump.id, 8u));
    // Patches a few metres across and tufts under a metre shade the sward
    // from above, where single blades are too small to read.
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
    let occlusion = mix(0.5, 1.0, t * t);
    let ramp = mix(base_colour, tip_colour, t * t) * tone * mottle * occlusion;
    // Past a few metres a blade settles onto its average colour: a lone bright
    // tip on a sub-pixel blade would otherwise flicker like a firefly.
    let average = mix(base_colour, tip_colour, 0.35) * tone * mottle * 0.7;
    let colour = mix(ramp, average, smoothstep(4.0, 20.0, distance));

    var out: VertexOutput;
    out.world_position = vec4<f32>(p, 1.0);
    out.clip_position = view.clip_from_view * view_pos;
    out.world_normal = normalize(mix(mix(sky, ground_normal, settle * 0.8), ground_normal, flat));
    out.ground_normal = ground_normal;
    out.canopy_uv = vec3<f32>(side, t, 0.0);
    out.color = vec4<f32>(colour * (1.0 - field.wheels.darkening * flat), 1.0);
    return out;
}

// Flower patches: each meadow cell of about 14 m blooms with one kind, thick
// where noise opens a patch, a rare stray elsewhere. position.z picks the
// part: stem 10, petals 11-18, leaves 30-31.
struct Bloom {
    height: f32,
    petals: u32,
    petal_len: f32,
    petal_w: f32,
    cup: f32,
    // Above zero the petals stack up a seed head this tall instead.
    spike: f32,
    petal: vec3<f32>,
    centre: vec3<f32>,
};

fn bloom_kind(kind: u32, pick: f32) -> Bloom {
    if (kind == 0u) { // daisy
        return Bloom(mix(0.14, 0.20, pick), 8u, 0.013, 0.005, 0.1, 0.0, vec3<f32>(0.92, 0.92, 0.86), vec3<f32>(0.88, 0.66, 0.10));
    }
    if (kind == 1u) { // buttercup
        return Bloom(mix(0.25, 0.38, pick), 5u, 0.010, 0.009, 0.9, 0.0, vec3<f32>(0.96, 0.80, 0.08), vec3<f32>(0.82, 0.60, 0.05));
    }
    if (kind == 2u) { // red clover
        return Bloom(mix(0.12, 0.18, pick), 8u, 0.009, 0.006, 1.8, 0.0, vec3<f32>(0.80, 0.34, 0.55), vec3<f32>(0.62, 0.22, 0.40));
    }
    if (kind == 3u) { // cornflower
        return Bloom(mix(0.35, 0.50, pick), 8u, 0.015, 0.006, 0.3, 0.0, vec3<f32>(0.24, 0.36, 0.86), vec3<f32>(0.18, 0.14, 0.45));
    }
    if (kind == 4u) { // poppy
        return Bloom(mix(0.35, 0.50, pick), 4u, 0.026, 0.024, 0.7, 0.0, vec3<f32>(0.86, 0.10, 0.05), vec3<f32>(0.16, 0.12, 0.08));
    }
    if (kind == 5u) { // dandelion
        return Bloom(mix(0.15, 0.25, pick), 8u, 0.014, 0.005, 0.5, 0.0, vec3<f32>(0.96, 0.82, 0.10), vec3<f32>(0.90, 0.70, 0.08));
    }
    // timothy: a tall grass with a cylindrical seed head
    return Bloom(mix(0.45, 0.70, pick), 8u, 0.006, 0.006, 0.0, 0.08, vec3<f32>(0.36, 0.44, 0.20), vec3<f32>(0.30, 0.38, 0.16));
}

// A stem point `t` up a plant: upright with a downwind bow, laid along the
// roll where a wheel pressed it.
fn flower_stem(t: f32, height: f32, bow: vec3<f32>, lay: vec3<f32>, flat: f32) -> vec3<f32> {
    let upright = vec3<f32>(0.0, height * t, 0.0) + bow * (height * t * t);
    return mix(upright, lay * (height * t) + vec3<f32>(0.0, 0.01 * t, 0.0), flat);
}

fn flower(vertex: Vertex) -> VertexOutput {
    let chunk_seed = pcg(bitcast<u32>(i32(field.corner.x)) * 73856093u
        ^ bitcast<u32>(i32(field.corner.y)) * 19349663u);
    let id = pcg(vertex.instance_index ^ chunk_seed ^ 0x5F3759DFu);
    let base = field.corner + vec2<f32>(rand(id, 1u), rand(id, 2u)) * field.chunk_size;
    let sampled = sample_field(base);
    let ground = vec3<f32>(base.x, sampled.x, base.y);
    let ground_normal = normalize(vec3<f32>(sampled.y, 1.0, sampled.z));
    let distance = length(ground - view.world_position);
    let rank = f32(vertex.instance_index) / max(field.blades_per_chunk, 1.0);
    let end = blade_fade_end(rank);
    // One kind per warped cell; noise opens the patches inside it.
    let warp = vec2<f32>(noise(base * 0.04 + vec2<f32>(5.1, -3.3)),
        noise(base * 0.04 + vec2<f32>(-7.7, 2.9))) - vec2<f32>(0.5);
    let cell = floor((base + warp * 24.0) / 14.0);
    let kind = pcg(bitcast<u32>(i32(cell.x)) * 2654435761u ^ bitcast<u32>(i32(cell.y)) * 40503u) % 7u;
    // Little blooms where the ground is parched.
    let opening = smoothstep(0.52, 0.72, noise(base * 0.11 + cell * 3.7))
        * mix(0.22, 1.0, meadow_share(base));
    let kept = rand(id, 3u) < opening + 0.004;
    let coverage = (1.0 - smoothstep(max(field.fade_start, end - BLADE_FADE_M), end, distance))
        * select(0.0, 1.0, kept && ground_normal.y >= DIRT_SLOPE_NORMAL_Y && within_field(base));
    if (coverage <= 0.0) {
        return culled_vertex();
    }
    let b = bloom_kind(kind, rand(id, 4u));
    // Heads grow a little with distance so a patch still reads as colour.
    let head = clamp(distance / 8.0, 1.0, 3.0);
    let part = vertex.position.z;
    let t = vertex.position.y;
    let side = vertex.position.x;
    let yaw = rand(id, 5u) * 6.2831853;
    let up = vec3<f32>(0.0, 1.0, 0.0);
    let pressed = sample_trample(base);
    let flat = clamp(pressed.x, 0.0, 1.0) * field.wheels.bend;
    let roll = wheel_roll(pressed);
    let tilt = vec2<f32>(cos(yaw), sin(yaw)) * 0.12 * rand(id, 6u);
    let bow = vec3<f32>(tilt.x, 0.0, tilt.y)
        + plant_lean(base, globals.time, field.wind, rand(id, 9u), 1.0) * 1.1
            * (1.0 - clamp(pressed.x, 0.0, 1.0));
    let top = flower_stem(1.0, b.height, bow, roll, flat);
    var offset: vec3<f32>;
    var normal: vec3<f32>;
    var color: vec3<f32>;
    if (part < 10.5) {
        let across = vec3<f32>(-sin(yaw), 0.0, cos(yaw));
        offset = flower_stem(t, b.height, bow, roll, flat) + across * (side * 0.0012 * (1.4 - t));
        color = mix(vec3<f32>(0.07, 0.16, 0.04), vec3<f32>(0.20, 0.34, 0.10), t);
        normal = normalize(ground_normal + across * side * 0.5);
    } else if (part < 19.5) {
        let k = u32(part - 10.5);
        let angle = yaw + f32(k) / f32(b.petals) * 6.2831853;
        let out_dir = vec3<f32>(cos(angle), 0.0, sin(angle));
        let across = vec3<f32>(-sin(angle), 0.0, cos(angle));
        let width = b.petal_w * head * sin(3.14159265 * min(t * 0.85 + 0.15, 1.0)) * side * 0.5;
        var petal: vec3<f32>;
        if (b.spike > 0.0) {
            petal = top + up * ((f32(k) + t * 1.5) / 8.0 * b.spike)
                + out_dir * 0.003 * head + across * width;
        } else {
            petal = top + out_dir * (t * b.petal_len * head)
                + up * (b.cup * t * t * b.petal_len * head) + across * width;
        }
        offset = select(top, petal, k < b.petals);
        color = mix(b.centre, b.petal, smoothstep(0.2, 0.4, t)) * mix(1.0, 0.65, smoothstep(8.0, 30.0, distance));
        normal = normalize(up + out_dir * (0.3 - b.cup * 0.3));
    } else {
        let leaf = part - 30.0;
        let angle = yaw + 2.2 + leaf * 3.1;
        let out_dir = vec3<f32>(cos(angle), 0.0, sin(angle));
        let across = vec3<f32>(-sin(angle), 0.0, cos(angle));
        let leaf_len = b.height * 0.35;
        offset = flower_stem(0.25 + 0.2 * leaf, b.height, bow, roll, flat)
            + out_dir * (t * leaf_len) + up * (leaf_len * (0.4 * t - 0.5 * t * t))
            + across * (side * 0.004 * sin(3.14159265 * t));
        color = vec3<f32>(0.10, 0.22, 0.06) * (0.9 + 0.2 * t);
        normal = normalize(up + out_dir * 0.4);
    }
    let tone = 0.9 + 0.2 * rand(id, 7u);
    var out: VertexOutput;
    out.world_position = vec4<f32>(ground + offset * coverage, 1.0);
    out.clip_position = view.clip_from_world * out.world_position;
    // Far plants take the ground's normal: one tilted away from a low sun
    // would otherwise drop out black against lit ground.
    let settle = smoothstep(6.0, 30.0, distance) * 0.9;
    out.world_normal = normalize(mix(normal, ground_normal, max(flat, settle)));
    out.ground_normal = ground_normal;
    out.canopy_uv = vec3<f32>(side, t, 0.0);
    out.color = vec4<f32>(color * tone * (1.0 - field.wheels.darkening * clamp(pressed.x, 0.0, 1.0)), 1.0);
    return out;
}

// A vertex of an instance culled before any shaping: outside the clip volume.
fn culled_vertex() -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(0.0, 0.0, -2.0, 1.0);
    return out;
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    if (vertex.position.z > 9.5) {
        return flower(vertex);
    }
    if (vertex.position.z > 0.5) {
        return meadow_detail(vertex);
    }
    if (vertex.position.z > -0.5) {
        return got_blade(vertex);
    }
    let corner = field.corner;
    let chunk_seed = pcg(bitcast<u32>(i32(corner.x)) * 73856093u ^ bitcast<u32>(i32(corner.y)) * 19349663u);
    let id = vertex.instance_index ^ chunk_seed;

    // Deterministic blade placement inside the chunk.
    let r0 = rand(id, 1u);
    let r1 = rand(id, 2u);
    let seed = rand(id, 3u);
    let local_xz = vec2<f32>(r0, r1) * field.chunk_size;
    let base_xz = corner + local_xz;
    let sampled = sample_field(base_xz);
    let ground_y = sampled.x;
    let ground_normal = normalize(vec3<f32>(sampled.y, 1.0, sampled.z));

    // Instance rank sets each blade's radial fade interval.
    let distance = length(vec3<f32>(base_xz.x, ground_y, base_xz.y) - view.world_position);
    let rank = f32(vertex.instance_index) / max(field.blades_per_chunk, 1.0);
    let blade_end = blade_fade_end(rank);
    let fade_span = select(BLADE_FADE_M, max(BLADE_FADE_M, blade_end * 0.12), vertex.position.z < -0.5);
    let blade_start = max(field.fade_start, blade_end - fade_span);
    let coverage = 1.0 - smoothstep(blade_start, blade_end, distance);
    let alive = select(0.0, coverage, ground_normal.y >= DIRT_SLOPE_NORMAL_Y && within_field(base_xz));

    let t = vertex.position.y;
    let side = vertex.position.x;

    // Wheel pressure bends vegetation along the rolling direction.
    let pressed = sample_trample(base_xz);
    if (vertex.position.z < -0.5) {
        let cluster = canopy_vertex(vertex.position, vec3<f32>(base_xz.x, ground_y, base_xz.y),
            ground_normal, seed, distance, alive, pressed, field.wheels.bend, false, field.wind);
        var out: VertexOutput;
        out.world_position = vec4<f32>(cluster.position, 1.0);
        out.clip_position = view.clip_from_world * out.world_position;
        out.world_normal = cluster.normal;
        out.ground_normal = ground_normal;
        out.canopy_uv = cluster.uv;
        out.color = vec4<f32>(vec3<f32>(0.19, 0.31, 0.075) * species_tint(grass_species(base_xz)) * dry_country(base_xz)
            * mix(0.88, 1.08, t)
            * (0.94 + seed * 0.12) * (1.0 - pressed.x * field.wheels.darkening), 1.0);
        return out;
    }
    return got_blade(vertex);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let alpha = select(1.0, canopy_alpha(in.canopy_uv, false), in.canopy_uv.z > 0.5);
    if (alpha < 0.001) { discard; }
    var pbr_input = pbr_input_new();
    let tint = meadow_tint(meadow_pattern(in.world_position.xz));
    // Blades: matte, the edges darker than the midrib; translucency grows
    // towards the tip.
    let blade = in.canopy_uv.z < 0.5;
    let across = abs(in.canopy_uv.x);
    let rib = select(1.0, mix(1.0, 0.85, smoothstep(0.1, 0.9, across)), blade);
    pbr_input.material.base_color = vec4<f32>(in.color.rgb * tint * rib, in.color.a);
    pbr_input.material.perceptual_roughness = select(0.98, 0.92, blade);
    pbr_input.material.metallic = 0.0;
    // No direct specular: on a steep blade normal at a grazing sun the GGX
    // term blows one pixel out to white for a frame.
    pbr_input.material.reflectance = vec3<f32>(0.0);
    pbr_input.specular_occlusion = 0.0;
    pbr_input.material.diffuse_transmission = select(0.35, mix(0.3, 0.5, in.canopy_uv.y), blade);
    pbr_input.material.thickness = 0.0002;
    pbr_input.material.flags = pbr_input.material.flags | STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT;
    pbr_input.frag_coord = in.clip_position;
    pbr_input.world_position = in.world_position;
    pbr_input.world_normal = normalize(in.ground_normal);
    pbr_input.V = calculate_view(in.world_position, false);
    pbr_input.N = foliage_normal(in.world_normal, pbr_input.world_normal, pbr_input.V);
    pbr_input.flags = MESH_FLAGS_SHADOW_RECEIVER_BIT;

    var color = apply_pbr_lighting(pbr_input);
    if (in.canopy_uv.z > 0.5) {
        color = surface_lighting(pbr_input, 0.65);
    }
    color = main_pass_post_lighting_processing(pbr_input, color);
    color.a = alpha;
    return color;
}
