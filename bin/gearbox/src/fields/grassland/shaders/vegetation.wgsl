#import bevy_pbr::{
    mesh_view_bindings::{view, globals},
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    pbr_types::{pbr_input_new, STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing, calculate_view},
}

#import "embedded://gearbox_sim/fields/grassland/shaders/palette.wgsl"::{meadow_pattern, meadow_tint, grass_species, species_tint}
#import "embedded://gearbox_sim/fields/shaders/canopy.wgsl"::{canopy_vertex, canopy_alpha}
#import "embedded://gearbox_sim/fields/shaders/surface_detail.wgsl"::{surface_lighting, foliage_normal}

#import "embedded://gearbox_sim/fields/shaders/interaction.wgsl"::{WheelMapParams, sample_wheels}

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
};

@group(3) @binding(0) var heightmap: texture_2d<f32>;
@group(3) @binding(1) var heightmap_sampler: sampler;
@group(3) @binding(2) var<uniform> field: VegetationParams;
@group(3) @binding(3) var trample: texture_2d<u32>;

fn sample_trample(world_xz: vec2<f32>) -> vec3<f32> {
    return sample_wheels(trample, field.wheels, world_xz);
}

fn within_field(world_xz: vec2<f32>) -> bool {
    return all(world_xz >= field.bounds.xy) && all(world_xz < field.bounds.zw);
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
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) canopy_uv: vec3<f32>,
    @location(4) ground_normal: vec3<f32>,
};

const WIND_DIR: vec3<f32> = vec3<f32>(0.8, 0.0, 0.6);
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
        let leaf_width = 0.022 * pow(max(sin(t * 3.14159265), 0.0), 0.65);
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
        let angle = yaw + f32(leaf) * 2.3999632 + rand(id, 12u + leaf) * 0.3;
        let forward = vec3<f32>(cos(angle), 0.0, sin(angle));
        let right = vec3<f32>(-sin(angle), 0.0, cos(angle));
        let species = grass_species(base);
        let variety = rand(id, 10u);
        let fine = variety < 0.25 + 0.5 * species.x;
        let broad = !fine && variety > 0.8 - 0.5 * species.y;
        let height = mix(0.105, 0.165, rand(id, 20u + leaf)) * select(1.0, 0.9, broad);
        let width = mix(0.003, 0.0045, rand(id, 30u + leaf))
            * select(select(1.0, 2.0, broad), 0.65, fine);
        let curve = select(select(0.4, 0.65, broad), 0.2, fine) + 0.25 * rand(id, 40u + leaf);
        let bend = height * curve * t * t;
        offset = forward * (0.007 + bend)
            + vec3<f32>(0.0, height * (t - 0.22 * t * t), 0.0)
            + right * (side * width * 0.5 * (1.0 - t * t));
        let blade_normal = normalize(-forward * (1.0 - 0.44 * t)
            + vec3<f32>(0.0, 2.0 * curve * t, 0.0));
        normal = normalize(ground_normal * 0.8 + blade_normal * 0.2 + right * side * 0.12);
        let tip = select(select(vec3<f32>(0.18, 0.36, 0.075), vec3<f32>(0.12, 0.28, 0.09), broad),
            vec3<f32>(0.28, 0.38, 0.12), fine);
        color = mix(vec3<f32>(0.11, 0.20, 0.045), tip, smoothstep(0.0, 0.7, t)) * tone;
    }

    let pressed = sample_trample(base);
    let flat = clamp(pressed.x, 0.0, 1.0);
    offset += vec3<f32>(pressed.y, 0.0, pressed.z) * offset.y * 0.9;
    offset.y *= 1.0 - flat * field.wheels.bend;
    let gust = sin(globals.time * 1.6 + base.x * 0.7 + base.y * 0.5);
    offset += WIND_DIR * (gust * 0.09 * offset.y * t * t);
    let p = ground + offset * coverage;
    var out: VertexOutput;
    out.world_position = vec4<f32>(p, 1.0);
    out.clip_position = view.clip_from_world * out.world_position;
    out.world_normal = normalize(mix(normal, ground_normal, flat));
    out.ground_normal = ground_normal;
    out.color = vec4<f32>(color * (1.0 - field.wheels.darkening * flat), 1.0);
    return out;
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    if (vertex.position.z > 0.5) {
        return meadow_detail(vertex);
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
            ground_normal, seed, distance, alive, pressed, field.wheels.bend, false);
        var out: VertexOutput;
        out.world_position = vec4<f32>(cluster.position, 1.0);
        out.clip_position = view.clip_from_world * out.world_position;
        out.world_normal = cluster.normal;
        out.ground_normal = ground_normal;
        out.canopy_uv = cluster.uv;
        out.color = vec4<f32>(vec3<f32>(0.19, 0.31, 0.075) * species_tint(grass_species(base_xz))
            * mix(0.88, 1.08, t)
            * (0.94 + seed * 0.12) * (1.0 - pressed.x * field.wheels.darkening), 1.0);
        return out;
    }
    let flat = clamp(pressed.x, 0.0, 1.0);
    let roll = vec3<f32>(pressed.y, 0.0, pressed.z);
    let yaw = seed * 6.2831853;
    // Fescue and ryegrass take over by patch, mixed blade by blade at the edges.
    let species = grass_species(base_xz);
    let pick = rand(id, 13u);
    let fescue = pick < species.x;
    let rye = !fescue && pick < species.x + species.y;
    let height = (BLADE_MIN_HEIGHT + (BLADE_MAX_HEIGHT - BLADE_MIN_HEIGHT) * rand(id, 6u))
        * select(1.0, 0.85, fescue) * alive;
    let width = (BLADE_MIN_WIDTH + (BLADE_MAX_WIDTH - BLADE_MIN_WIDTH) * rand(id, 7u))
        * select(select(1.0, 1.7, rye), 0.6, fescue);
    let dry = select(0.0, 0.3 + 0.7 * hash11(seed * 9.1), hash11(seed * 4.4 + 3.0) < 0.18);

    let right = vec3<f32>(cos(yaw), 0.0, sin(yaw));
    let lean_angle = hash11(seed * 6.1 + 4.0) * 6.2831853;
    let lean_dir = vec3<f32>(sin(lean_angle), 0.0, cos(lean_angle));
    // Every blade tilts 10-35 degrees so the sward still reads from above.
    let lean = lean_dir * (select(0.18, 0.28, fescue) + 0.45 * hash11(seed * 7.13 + 5.0));

    let phase = base_xz.x * 0.31 + base_xz.y * 0.23 + seed * 2.0;
    let gust = sin(globals.time * 1.6 + phase) * 0.6
        + sin(globals.time * 4.3 + phase * 2.1) * 0.25
        + sin(globals.time * 0.37 + phase * 0.11) * 0.15;
    let bend = t * t * WIND_SWAY * height * (0.55 + gust) * alive;

    var p = vec3<f32>(base_xz.x, ground_y, base_xz.y)
        + vec3<f32>(0.0, 1.0, 0.0) * (height * t * (1.0 - field.wheels.bend * flat))
        + roll * (height * t * field.wheels.bend)
        + lean * (height * t * t)
        + right * (width * 0.5 * side * (1.0 - t * 0.85) * alive)
        + WIND_DIR * bend;
    p.y = p.y - abs(bend) * 0.15;

    // Roots grade into green or dry tips.
    let tone = 0.65 + 0.7 * rand(id, 5u);
    var root = vec3<f32>(0.11, 0.20, 0.045) * tone;
    var tip = mix(vec3<f32>(0.22, 0.42, 0.09), vec3<f32>(0.40, 0.34, 0.10), dry) * tone;
    if (fescue) {
        root = vec3<f32>(0.08, 0.17, 0.09) * tone;
        tip = vec3<f32>(0.20, 0.35, 0.24) * tone;
    } else if (rye) {
        root = vec3<f32>(0.05, 0.16, 0.03) * tone;
        tip = vec3<f32>(0.13, 0.40, 0.05) * tone;
    }

    var out: VertexOutput;
    out.world_position = vec4<f32>(p, 1.0);
    out.clip_position = view.clip_from_world * vec4<f32>(p, 1.0);
    let blade_normal = normalize(cross(right, vec3<f32>(0.0, 1.0, 0.0) + lean * (2.0 * t)));
    out.world_normal = normalize(mix(blade_normal + ground_normal * 0.7, ground_normal, flat));
    out.ground_normal = ground_normal;
    out.color = vec4<f32>(mix(root, tip, smoothstep(0.0, 0.7, t)) * (1.0 - field.wheels.darkening * flat), 1.0);
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let alpha = select(1.0, canopy_alpha(in.canopy_uv, false), in.canopy_uv.z > 0.5);
    if (alpha < 0.001) { discard; }
    var pbr_input = pbr_input_new();
    let tint = meadow_tint(meadow_pattern(in.world_position.xz));
    pbr_input.material.base_color = vec4<f32>(in.color.rgb * tint, in.color.a);
    pbr_input.material.perceptual_roughness = 0.98;
    pbr_input.material.metallic = 0.0;
    pbr_input.material.reflectance = vec3<f32>(0.04);
    pbr_input.specular_occlusion = 0.0;
    pbr_input.material.diffuse_transmission = select(0.45, 0.35, in.canopy_uv.z > 0.5);
    pbr_input.material.thickness = 0.0002;
    pbr_input.material.flags = pbr_input.material.flags | STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT;
    pbr_input.frag_coord = in.clip_position;
    pbr_input.world_position = in.world_position;
    pbr_input.world_normal = normalize(in.ground_normal);
    pbr_input.V = calculate_view(in.world_position, false);
    pbr_input.N = foliage_normal(normalize(in.world_normal), pbr_input.world_normal, pbr_input.V);
    pbr_input.flags = MESH_FLAGS_SHADOW_RECEIVER_BIT;

    var color = apply_pbr_lighting(pbr_input);
    if (in.canopy_uv.z > 0.5) {
        color = surface_lighting(pbr_input, 0.65);
    }
    color = main_pass_post_lighting_processing(pbr_input, color);
    color.a = alpha;
    return color;
}
