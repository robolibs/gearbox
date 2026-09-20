#import bevy_pbr::{
    mesh_view_bindings::{view, globals},
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    pbr_types::{pbr_input_new, STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing, calculate_view},
}


#import "embedded://gearbox_fields/shaders/interaction.wgsl"::{WheelMapParams, sample_wheels, wheel_roll, scatter_roll}
#import "embedded://gearbox_fields/shaders/canopy.wgsl"::{canopy_vertex, canopy_alpha}
#import "embedded://gearbox_fields/shaders/wind.wgsl"::{plant_lean}
#import "embedded://gearbox_fields/shaders/surface_detail.wgsl"::{surface_lighting, foliage_normal}
#import "embedded://gearbox_fields/harvested_wheat/shaders/patches.wgsl"::{regrowth, row_drift, row_wobble, plant_jog}
#import "embedded://gearbox_fields/bare/shaders/cover.wgsl"::{worn}

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
    let inside = min(
        min(world_xz.x - field.bounds.x, field.bounds.z - world_xz.x),
        min(world_xz.y - field.bounds.y, field.bounds.w - world_xz.y));
    if (field.soft_border <= 0.0) {
        return inside >= 0.0;
    }
    let past = max(-inside, 0.0);
    return edge_roll(world_xz) < 1.0 - smoothstep(0.0, field.soft_border, past);
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

const WIND_DIR: vec3<f32> = vec3<f32>(0.8, 0.0, 0.6);
// Sway is a fraction of the blade height.
const WIND_SWAY: f32 = 0.004;
const BLADE_MIN_HEIGHT: f32 = 0.05;
const BLADE_MAX_HEIGHT: f32 = 0.15;
const BLADE_MIN_WIDTH: f32 = 0.004;
const BLADE_MAX_WIDTH: f32 = 0.006;
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

fn blade_fade_end(rank: f32) -> f32 {
    if field.inverse_square_thinning == 0u {
        return mix(field.fade_end, field.fade_start, sqrt(rank));
    }
    return field.fade_start * field.fade_end
        / (field.fade_start + (field.fade_end - field.fade_start) * sqrt(rank));
}

// Clover and rosettes the cutter bar passed over: thick in regrowth
// patches, scattered elsewhere. position.z selects the leaf.
// How far a way has worn the stubble away here. Read by all three stalk paths
// and by the stubble material, so nothing is left standing in the road.
fn way_wear(place: vec2<f32>) -> f32 {
    return worn(place, field.bounds, field.tread, field.way, field.way_more, field.way_shape);
}

fn stubble_detail(vertex: Vertex) -> VertexOutput {
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
    let kept = rand(id, 9u) < (0.10 + 0.90 * regrowth(base)) * (1.0 - way_wear(base));
    let coverage = (1.0 - smoothstep(max(field.fade_start, end - BLADE_FADE_M), end, distance))
        * select(0.0, 1.0, kept && ground_normal.y >= DIRT_SLOPE_NORMAL_Y && within_field(base));
    if (coverage <= 0.0) {
        return culled_vertex();
    }
    let clover = rand(id, 17u) < 0.8;
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
        let leaf_width = 0.022 * pow(max(sin(t * 3.14159265), 0.0), 0.65);
        let cup = 0.003 * side * side * sin(t * 3.14159265);
        offset = stem_dir * 0.035 + vec3<f32>(0.0, 0.105 + 0.015 * f32(head), 0.0)
            + forward * (0.002 + t * 0.031 * size)
            + right * (side * leaf_width * 0.5 * size)
            + vec3<f32>(0.0, 0.009 * t - 0.006 * t * t + cup, 0.0);
        let band = 1.0 - smoothstep(0.035, 0.105, abs(t - (0.54 + 0.16 * abs(side))));
        color = mix(vec3<f32>(0.055, 0.18, 0.065), vec3<f32>(0.24, 0.36, 0.17), band * 0.7) * tone;
        normal = normalize(ground_normal - forward * 0.15 + right * side * 0.25);
    } else {
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
    }

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

const ROW_M: f32 = 0.125;
const PLANT_M: f32 = 0.05;
const CLUMP_CELL_M: f32 = 0.6;
const LOD_JITTER_M: f32 = 1.5;

struct Clump {
    centre: vec2<f32>,
    id: u32,
};

// Nearest Voronoi clump centre: patches of the field share height and tone.
fn clump_of(p: vec2<f32>) -> Clump {
    let cell = vec2<i32>(floor(p / CLUMP_CELL_M));
    var best = Clump(p, 0u);
    var best_d = 1e9;
    for (var dz = -1; dz <= 1; dz = dz + 1) {
        for (var dx = -1; dx <= 1; dx = dx + 1) {
            let c = cell + vec2<i32>(dx, dz);
            let h = pcg(bitcast<u32>(c.x) * 73856093u ^ bitcast<u32>(c.y) * 19349663u ^ 0x7E57A1Bu);
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

fn ease_out(x: f32, power: f32) -> f32 {
    return 1.0 - pow(1.0 - x, power);
}

// A cut stalk after the Ghost of Tsushima grass, without the wind: stalks
// stand in drill rows along the harvest direction, tillered into plants and
// folded into twins, thickened along the screen when edge-on, with rounded
// waxy shading from a shaded base to a pale top. vertex.uv is the distance
// band of this detail level.
fn got_stalk(vertex: Vertex) -> VertexOutput {
    let chunk_seed = pcg(bitcast<u32>(i32(field.corner.x)) * 73856093u
        ^ bitcast<u32>(i32(field.corner.y)) * 19349663u);
    let id = pcg(vertex.instance_index ^ chunk_seed);
    let spot = field.corner + r2(vertex.instance_index, chunk_seed) * field.chunk_size;
    // Drill rows run along x; along each row the stalks gather into plants.
    let drift = row_drift(spot);
    let row = floor((spot.y - drift) / ROW_M);
    let row_hash = pcg(bitcast<u32>(i32(row)) ^ 0x51ED27u);
    let wobble = drift + row_wobble(row, spot.x);
    let plant = floor(spot.x / PLANT_M);
    let plant_id = pcg(bitcast<u32>(i32(plant)) * 2654435761u ^ row_hash);
    let plant_centre = vec2<f32>((plant + 0.5 + (rand(plant_id, 1u) - 0.5) * 0.6) * PLANT_M,
        (row + 0.5) * ROW_M + wobble + plant_jog(row, plant));
    let twin = vertex.normal.x > 0.5;
    let own_yaw = rand(id, 4u) * 6.2831853 + select(0.0, mix(0.6, 1.4, rand(id, 30u)), twin);
    var base_xz = vec2<f32>(mix(spot.x, plant_centre.x, 0.7),
        plant_centre.y + (rand(id, 2u) - 0.5) * 0.02);
    if (twin) {
        base_xz += vec2<f32>(cos(own_yaw), sin(own_yaw)) * mix(0.006, 0.015, rand(id, 31u));
    }
    let clump = clump_of(base_xz);
    let sampled = sample_field(base_xz);
    let root = vec3<f32>(base_xz.x, sampled.x, base_xz.y);
    let ground_normal = normalize(vec3<f32>(sampled.y, 1.0, sampled.z));
    let distance = length(root - view.world_position);

    // Density fades by rank; each stalk swaps detail level at a jittered
    // band edge; stalks under ~1.2 px wide are widened and thinned alike.
    let rank = f32(vertex.instance_index) / max(field.blades_per_chunk, 1.0);
    let blade_end = blade_fade_end(rank);
    let coverage = 1.0 - smoothstep(max(field.fade_start, blade_end - BLADE_FADE_M), blade_end, distance);
    let jitter = (rand(id, 23u) - 0.5) * 2.0 * LOD_JITTER_M;
    let in_band = (vertex.uv.x <= 0.0 || distance >= vertex.uv.x + jitter)
        && distance < vertex.uv.y + jitter;
    let pixel_m = distance * 2.0 / (view.clip_from_view[1][1] * view.viewport.w);
    let widen = max(1.0, 1.2 * pixel_m / BLADE_MAX_WIDTH);
    let alive = select(0.0, coverage, in_band && ground_normal.y >= DIRT_SLOPE_NORMAL_Y
        && within_field(base_xz) && rand(id, 19u) * widen < 1.0);
    if (alive <= 0.0 || rand(id, 41u) < smoothstep(0.08, 0.45, way_wear(base_xz))) {
        return culled_vertex();
    }

    let height = mix(BLADE_MIN_HEIGHT, BLADE_MAX_HEIGHT, rand(id, 6u))
        * mix(0.8, 1.2, rand(plant_id, 3u)) * select(1.0, 0.0, rand(plant_id, 7u) < 0.12) * mix(0.85, 1.15, rand(clump.id, 3u))
        * select(1.0, mix(0.8, 0.95, rand(id, 32u)), twin) * alive;
    let width = mix(BLADE_MIN_WIDTH, BLADE_MAX_WIDTH, rand(id, 7u)) * widen * alive;

    // Stalks fan out from the plant centre; the header snapped some over,
    // mostly along its direction of travel.
    let fan = base_xz - plant_centre + vec2<f32>(cos(own_yaw), sin(own_yaw)) * 1e-3;
    let lean_xz = normalize(fan);
    let lean = vec3<f32>(lean_xz.x, 0.0, lean_xz.y) * mix(0.05, 0.3, rand(id, 8u));
    let pressed = sample_trample(base_xz);
    let flat = clamp(pressed.x, 0.0, 1.0);
    let up = vec3<f32>(0.0, 1.0, 0.0);
    // Wheels snap stalks over along their roll, scattered; the stiffest stand
    // back up first as the track recovers.
    let crush = smoothstep(0.0, mix(0.5, 1.0, rand(id, 36u)), flat) * field.wheels.bend;
    let roll = scatter_roll(wheel_roll(pressed), (rand(id, 37u) - 0.5) * 1.0);
    let t = vertex.position.y;
    let side = vertex.position.x;
    var axis_point = up * (height * t * (1.0 - crush))
        + roll * (height * t * crush) + lean * (height * t * (1.0 - crush));
    let knee = max(t - 0.5, 0.0) * 2.0;
    let snapped = rand(id, 14u) < 0.35;
    let kink = select(0.1, 0.4, snapped) + select(0.15, 0.6, snapped) * rand(id, 15u);
    // Alternate combine passes cut in opposite directions, so each pass
    // snaps its stalks its own way and reads as a band from above.
    let cut_pass = floor((base_xz.y + 2.0 * sin(base_xz.x * 0.03)) / 4.0);
    let heading = select(-1.0, 1.0, fract(cut_pass * 0.5) < 0.25);
    let travel = heading * select(-1.0, 1.0, rand(id, 16u) < 0.8);
    let kink_dir = normalize(mix(vec3<f32>(travel, 0.0, 0.0), vec3<f32>(lean_xz.x, 0.0, lean_xz.y), 0.4));
    axis_point += (kink_dir * sin(kink) - up * (1.0 - cos(kink))) * (0.5 * height * knee * (1.0 - crush));
    // Stalks are stiff, a tenth of the grass's sway; green shoots sway fully.
    let green = regrowth(base_xz) * step(rand(id, 9u), 0.5);
    let give = mix(0.28, 1.0, green) * (1.0 - crush);
    axis_point += plant_lean(root.xz, globals.time, field.wind, rand(id, 9u), t)
        * (give * height * t);

    let stand_right = vec3<f32>(cos(own_yaw), 0.0, sin(own_yaw));
    let lay = cross(up, roll);
    let lay_right = select(-lay, lay, dot(lay, stand_right) >= 0.0);
    let right = normalize(mix(stand_right, lay_right, crush) + vec3<f32>(1e-5, 0.0, 0.0));
    let axis = normalize(mix(up + lean + kink_dir * knee * sin(kink), roll + up * 0.05, crush));
    let normal = normalize(cross(right, axis));
    let half_w = width * 0.5 * side;
    let p = root + axis_point + right * half_w;

    // View-space thickening (talk, 14:05): a stalk edge-on to the view is
    // widened along the screen, easing off right at edge-on.
    let to_eye = normalize(view.world_position - p);
    let face_xz = normalize(vec2<f32>(normal.x, normal.z) + vec2<f32>(1e-5, 0.0));
    let eye_xz = normalize(vec2<f32>(to_eye.x, to_eye.z) + vec2<f32>(1e-5, 0.0));
    let facing_eye = abs(dot(face_xz, eye_xz));
    let thicken = ease_out(1.0 - facing_eye, 4.0) * smoothstep(0.0, 0.2, facing_eye);
    var view_pos = view.view_from_world * vec4<f32>(p, 1.0);
    let right_view = (view.view_from_world * vec4<f32>(right, 0.0)).x;
    view_pos.x += thicken * select(-1.0, 1.0, right_view >= 0.0) * half_w;

    // A hollow stalk shades like a tube: normals roll strongly across it,
    // leaning to the sky, settling onto the ground normal with distance.
    let rounded = normalize(normal + right * side * 0.9);
    let sky = normalize(mix(ground_normal, rounded, 0.45));
    let settle = smoothstep(8.0, 40.0, distance);

    // Shaded grey-brown bases to pale golden tops; green volunteer shoots
    // in regrowth patches.
    let tone = mix(0.9, 1.1, rand(id, 5u)) * mix(0.8, 1.15, rand(plant_id, 5u))
        * mix(0.85, 1.1, rand(clump.id, 6u)) * select(0.94, 1.05, heading > 0.0);
    let dry = rand(id, 11u);
    let base_colour = mix(vec3<f32>(0.10, 0.075, 0.035), vec3<f32>(0.14, 0.10, 0.05), dry);
    let top_colour = mix(vec3<f32>(0.62, 0.48, 0.22), vec3<f32>(0.76, 0.62, 0.33), dry);
    let shoot = regrowth(base_xz) * step(rand(id, 9u), 0.5);
    let root_colour = mix(base_colour, vec3<f32>(0.03, 0.08, 0.015), shoot);
    let tip_colour = mix(top_colour, vec3<f32>(0.30, 0.52, 0.14), shoot);
    let occlusion = mix(0.3, 1.0, t * t);
    let ramp = mix(root_colour, tip_colour, pow(t, 1.5)) * tone * occlusion;
    // Past a few metres a stalk settles onto its average colour so pale tops
    // under a pixel do not flicker like fireflies.
    let average = mix(root_colour, tip_colour, 0.45) * tone * 0.75;
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

// Chopped straw lying on the stubble, the part of a cut field that reads from
// above. position.y runs along the piece, position.x across it.
fn lying_straw(vertex: Vertex) -> VertexOutput {
    let chunk_seed = pcg(bitcast<u32>(i32(field.corner.x)) * 73856093u
        ^ bitcast<u32>(i32(field.corner.y)) * 19349663u);
    let id = pcg(vertex.instance_index ^ chunk_seed ^ 0x5A17C3E1u);
    let base = field.corner + r2(vertex.instance_index, chunk_seed ^ 0x5A17C3E1u) * field.chunk_size;
    let sampled = sample_field(base);
    let ground = vec3<f32>(base.x, sampled.x, base.y);
    let ground_normal = normalize(vec3<f32>(sampled.y, 1.0, sampled.z));
    let distance = length(ground - view.world_position);
    let end = blade_fade_end(f32(vertex.instance_index) / max(field.blades_per_chunk, 1.0));
    let coverage = (1.0 - smoothstep(max(field.fade_start, end - BLADE_FADE_M), end, distance))
        * select(0.0, 1.0, ground_normal.y >= DIRT_SLOPE_NORMAL_Y && within_field(base));
    if (coverage <= 0.0 || rand(id, 41u) < smoothstep(0.08, 0.45, way_wear(base))) {
        return culled_vertex();
    }
    // Kept at least ~1.2 px wide, thinned by the same share, like the stalks.
    let pixel_m = distance * 2.0 / (view.clip_from_view[1][1] * view.viewport.w);
    let width = mix(0.003, 0.005, rand(id, 7u));
    let widen = max(1.0, 1.2 * pixel_m / width);
    // Swaths: straw lies thick in the combine's bands along x, sparse between.
    let swath = 1.0 - smoothstep(0.3, 0.8, abs(fract((base.y + 2.0 * sin(base.x * 0.03)) / 4.0) - 0.5) * 2.0);
    let kept = select(0.0, 1.0, rand(id, 19u) * widen < mix(0.35, 1.0, swath));
    // Pieces lie mostly along the harvest direction.
    let yaw = (rand(id, 4u) - 0.5) * 1.1 + select(0.0, 3.14159265, rand(id, 24u) < 0.5);
    let along = vec3<f32>(cos(yaw), 0.0, sin(yaw));
    let across = vec3<f32>(-sin(yaw), 0.0, cos(yaw));
    let piece = mix(0.05, 0.18, rand(id, 6u));
    let lift = rand(id, 8u) * 0.25;
    let t = vertex.position.y - 0.5;
    let side = vertex.position.x;
    // At a grazing view a lying piece tips its near edge up towards the
    // camera so it keeps its width on screen (the talk's thickening).
    let to_eye = normalize(view.world_position - ground);
    let grazing = 1.0 - abs(dot(ground_normal, to_eye));
    let half_w = width * widen * 0.5;
    let offset = along * (t * piece)
        + across * (side * half_w)
        + ground_normal * (0.008 + max(t, 0.0) * piece * lift + side * half_w * grazing);
    let flat = clamp(sample_trample(base).x, 0.0, 1.0);
    // A straw is a tube: normals roll across it, leaning to the sky.
    let rounded = normalize(ground_normal + across * side * 0.9);
    let sky = normalize(mix(ground_normal, rounded, 0.45));
    // Where a piece rests on the stubble it sits in shade.
    let contact = mix(0.7, 1.0, smoothstep(0.0, 0.02, max(t, 0.0) * piece * lift));
    let tone = 0.75 + 0.45 * rand(id, 5u);
    let fresh = mix(vec3<f32>(0.62, 0.46, 0.20), vec3<f32>(0.80, 0.66, 0.36), rand(id, 9u)) * tone * contact;
    // Far off a piece settles onto the stubble's straw tone so pale pieces
    // under a pixel do not sparkle.
    let color = mix(fresh, vec3<f32>(0.55, 0.42, 0.19), smoothstep(6.0, 30.0, distance));
    var out: VertexOutput;
    out.world_position = vec4<f32>(ground + offset * coverage * kept, 1.0);
    out.clip_position = view.clip_from_world * out.world_position;
    out.world_normal = sky;
    out.ground_normal = ground_normal;
    out.canopy_uv = vec3<f32>(vertex.position.x, vertex.position.y, 0.0);
    out.color = vec4<f32>(color * (1.0 - field.wheels.darkening * flat), 1.0);
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
    if (vertex.position.z > 0.1 && vertex.position.z < 0.5) {
        return lying_straw(vertex);
    }
    if (vertex.position.z > 0.5) {
        return stubble_detail(vertex);
    }
    if (vertex.position.z > -0.5) {
        return got_stalk(vertex);
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
    let green = regrowth(base_xz);
    if (vertex.position.z < -0.5) {
        let cluster = canopy_vertex(vertex.position, vec3<f32>(base_xz.x, ground_y, base_xz.y),
            ground_normal, seed, distance, alive, pressed, field.wheels.bend, true, field.wind);
        var out: VertexOutput;
        out.world_position = vec4<f32>(cluster.position, 1.0);
        out.clip_position = view.clip_from_world * out.world_position;
        out.world_normal = cluster.normal;
        out.ground_normal = ground_normal;
        out.canopy_uv = cluster.uv;
        out.color = vec4<f32>(mix(vec3<f32>(0.43, 0.30, 0.115), vec3<f32>(0.20, 0.30, 0.08), green * 0.7)
            * mix(0.88, 1.08, t)
            * (0.94 + seed * 0.12) * (1.0 - pressed.x * field.wheels.darkening), 1.0);
        return out;
    }
    return got_stalk(vertex);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let alpha = select(1.0, canopy_alpha(in.canopy_uv, true), in.canopy_uv.z > 0.5);
    if (alpha < 0.001) { discard; }
    var pbr_input = pbr_input_new();
    // Stalks and straw: matte, edges darker than the midrib, little
    // translucency.
    let stalk = in.canopy_uv.z < 0.5;
    let across = abs(in.canopy_uv.x);
    let rib = select(1.0, mix(1.0, 0.82, smoothstep(0.1, 0.9, across)), stalk);
    pbr_input.material.base_color = vec4<f32>(in.color.rgb * rib, in.color.a);
    pbr_input.material.perceptual_roughness = select(1.0, 0.92, stalk);
    pbr_input.material.metallic = 0.0;
    // No direct specular: on a steep blade normal at a grazing sun the GGX
    // term blows one pixel out to white for a frame.
    pbr_input.material.reflectance = vec3<f32>(0.0);
    pbr_input.specular_occlusion = 0.0;
    pbr_input.material.diffuse_transmission = select(0.15, 0.15, in.canopy_uv.z > 0.5);
    pbr_input.material.thickness = 0.0006;
    pbr_input.material.flags = pbr_input.material.flags | STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT;
    pbr_input.frag_coord = in.clip_position;
    pbr_input.world_position = in.world_position;
    pbr_input.world_normal = normalize(in.ground_normal);
    pbr_input.V = calculate_view(in.world_position, false);
    pbr_input.N = foliage_normal(in.world_normal, pbr_input.world_normal, pbr_input.V);
    pbr_input.flags = MESH_FLAGS_SHADOW_RECEIVER_BIT;

    var color = apply_pbr_lighting(pbr_input);
    if (in.canopy_uv.z > 0.5) {
        color = surface_lighting(pbr_input, 0.45);
    }
    color = main_pass_post_lighting_processing(pbr_input, color);
    color.a = alpha;
    return color;
}
