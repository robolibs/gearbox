#import bevy_pbr::{
    mesh_view_bindings::{view, globals},
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    pbr_types::{pbr_input_new, STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing, calculate_view},
}


#import "embedded://gearbox_sim/fields/shaders/interaction.wgsl"::{WheelMapParams, sample_wheels}
#import "embedded://gearbox_sim/fields/shaders/canopy.wgsl"::{canopy_vertex, canopy_alpha}
#import "embedded://gearbox_sim/fields/shaders/surface_detail.wgsl"::{surface_lighting, foliage_normal}

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

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
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
    var blade_end = mix(field.fade_end, field.fade_start, sqrt(rank));
    if field.inverse_square_thinning != 0u {
        blade_end = field.fade_start * field.fade_end
            / (field.fade_start + (field.fade_end - field.fade_start) * sqrt(rank));
    }
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
            ground_normal, seed, distance, alive, pressed, field.wheels.bend, true);
        var out: VertexOutput;
        out.world_position = vec4<f32>(cluster.position, 1.0);
        out.clip_position = view.clip_from_world * out.world_position;
        out.world_normal = cluster.normal;
        out.ground_normal = ground_normal;
        out.canopy_uv = cluster.uv;
        out.color = vec4<f32>(vec3<f32>(0.43, 0.30, 0.115) * mix(0.88, 1.08, t)
            * (0.94 + seed * 0.12) * (1.0 - pressed.x * field.wheels.darkening), 1.0);
        return out;
    }
    let flat = clamp(pressed.x, 0.0, 1.0);
    let roll = vec3<f32>(pressed.y, 0.0, pressed.z);
    let yaw = seed * 6.2831853;
    let height = (BLADE_MIN_HEIGHT + (BLADE_MAX_HEIGHT - BLADE_MIN_HEIGHT) * rand(id, 6u)) * alive;
    let width = BLADE_MIN_WIDTH + (BLADE_MAX_WIDTH - BLADE_MIN_WIDTH) * rand(id, 7u);
    let dry = select(0.0, 0.3 + 0.7 * hash11(seed * 9.1), hash11(seed * 4.4 + 3.0) < 0.18);

    let right = vec3<f32>(cos(yaw), 0.0, sin(yaw));
    let lean_angle = hash11(seed * 6.1 + 4.0) * 6.2831853;
    let lean_dir = vec3<f32>(sin(lean_angle), 0.0, cos(lean_angle));
    let lean = lean_dir * (0.015 + 0.04 * hash11(seed * 7.13 + 5.0));

    let phase = base_xz.x * 0.31 + base_xz.y * 0.23 + seed * 2.0;
    let gust = sin(globals.time * 1.6 + phase) * 0.6
        + sin(globals.time * 4.3 + phase * 2.1) * 0.25
        + sin(globals.time * 0.37 + phase * 0.11) * 0.15;
    let bend = t * t * WIND_SWAY * height * (0.55 + gust) * alive;

    var p = vec3<f32>(base_xz.x, ground_y, base_xz.y)
        + vec3<f32>(0.0, 1.0, 0.0) * (height * t * (1.0 - field.wheels.bend * flat))
        + roll * (height * t * field.wheels.bend)
        + lean * (height * t)
        + right * (width * 0.5 * side * alive)
        + WIND_DIR * bend;
    p.y = p.y - abs(bend) * 0.15;

    // Cut stalks grade from shaded straw roots to pale dry tips.
    let tone = 0.65 + 0.7 * rand(id, 5u);
    let root = vec3<f32>(0.28, 0.18, 0.065) * tone;
    let tip = mix(vec3<f32>(0.46, 0.30, 0.10), vec3<f32>(0.60, 0.43, 0.18), dry) * tone;

    var out: VertexOutput;
    out.world_position = vec4<f32>(p, 1.0);
    out.clip_position = view.clip_from_world * vec4<f32>(p, 1.0);
    let stalk_normal = normalize(cross(right, vec3<f32>(0.0, 1.0, 0.0) + lean));
    out.world_normal = normalize(mix(stalk_normal + ground_normal * 0.7, ground_normal, flat));
    out.ground_normal = ground_normal;
    out.color = vec4<f32>(mix(root, tip, smoothstep(0.0, 0.7, t)) * (1.0 - field.wheels.darkening * flat), 1.0);
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let alpha = select(1.0, canopy_alpha(in.canopy_uv, true), in.canopy_uv.z > 0.5);
    if (alpha < 0.001) { discard; }
    var pbr_input = pbr_input_new();
    pbr_input.material.base_color = in.color;
    pbr_input.material.perceptual_roughness = 1.0;
    pbr_input.material.metallic = 0.0;
    pbr_input.material.reflectance = vec3<f32>(0.04);
    pbr_input.specular_occlusion = 0.0;
    pbr_input.material.diffuse_transmission = select(0.25, 0.15, in.canopy_uv.z > 0.5);
    pbr_input.material.thickness = 0.0006;
    pbr_input.material.flags = pbr_input.material.flags | STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT;
    pbr_input.frag_coord = in.clip_position;
    pbr_input.world_position = in.world_position;
    pbr_input.world_normal = normalize(in.ground_normal);
    pbr_input.V = calculate_view(in.world_position, false);
    pbr_input.N = foliage_normal(normalize(in.world_normal), pbr_input.world_normal, pbr_input.V);
    pbr_input.flags = MESH_FLAGS_SHADOW_RECEIVER_BIT;

    var color = apply_pbr_lighting(pbr_input);
    if (in.canopy_uv.z > 0.5) {
        color = surface_lighting(pbr_input, 0.45);
    }
    color = main_pass_post_lighting_processing(pbr_input, color);
    color.a = alpha;
    return color;
}
