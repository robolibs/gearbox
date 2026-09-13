#import bevy_pbr::{
    mesh_view_bindings::{view, globals},
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    pbr_types::{pbr_input_new, STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing, calculate_view},
}

struct GrassParams {
    corner: vec2<f32>,
    origin: vec2<f32>,
    texels_per_metre: f32,
    chunk_size: f32,
    fade_start: f32,
    fade_end: f32,
    blades_per_chunk: f32,
    texel_count: f32,
    trample_texels_per_metre: f32,
    trample_texel_count: f32,
};

@group(3) @binding(0) var heightmap: texture_2d<f32>;
@group(3) @binding(1) var heightmap_sampler: sampler;
@group(3) @binding(2) var<uniform> field: GrassParams;
@group(3) @binding(3) var trample: texture_2d<u32>;

const TRAMPLE_CLOCK_S: f32 = 3600.0;
const TRAMPLE_RECOVER_S: f32 = 300.0;

// One trample texel: how pressed (0..1, recovering with age) and the roll
// direction scaled by it. Angle 0 marks a texel no wheel ever touched.
fn trample_texel(index: vec2<i32>) -> vec3<f32> {
    let texel = textureLoad(trample, index, 0);
    if (texel.g == 0u) {
        return vec3<f32>(0.0);
    }
    let stamped = f32(texel.r) / 65535.0 * TRAMPLE_CLOCK_S;
    let age = (globals.time - stamped + TRAMPLE_CLOCK_S) % TRAMPLE_CLOCK_S;
    let press = 1.0 - clamp(age / TRAMPLE_RECOVER_S, 0.0, 1.0);
    let angle = f32(texel.g - 1u) / 65534.0 * 6.2831853 - 3.1415927;
    return vec3<f32>(press, cos(angle) * press, sin(angle) * press);
}

// Bilinear read of the trample map at a world XZ position.
fn sample_trample(world_xz: vec2<f32>) -> vec3<f32> {
    let t = (world_xz - field.origin) * field.trample_texels_per_metre;
    let max_index = i32(field.trample_texel_count) - 1;
    let i = clamp(vec2<i32>(floor(t)), vec2<i32>(0), vec2<i32>(max_index - 1));
    let f = clamp(t - vec2<f32>(i), vec2<f32>(0.0), vec2<f32>(1.0));
    let s00 = trample_texel(i);
    let s10 = trample_texel(i + vec2<i32>(1, 0));
    let s01 = trample_texel(i + vec2<i32>(0, 1));
    let s11 = trample_texel(i + vec2<i32>(1, 1));
    return mix(mix(s00, s10, f.x), mix(s01, s11, f.x), f.y);
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
};

const WIND_DIR: vec3<f32> = vec3<f32>(0.8, 0.0, 0.6);
// Sway is a fraction of the blade height.
const WIND_SWAY: f32 = 0.3;
const BLADE_MIN_HEIGHT: f32 = 0.04;
const BLADE_MAX_HEIGHT: f32 = 0.10;
const BLADE_MIN_WIDTH: f32 = 0.006;
const BLADE_MAX_WIDTH: f32 = 0.011;
const DIRT_SLOPE_NORMAL_Y: f32 = 0.86;

// Integer hashing (PCG) so blade indices in the hundreds of thousands
// keep full randomness; float hashes fall apart at that magnitude.
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
    let keep_roll = rand(id, 4u);
    let local_xz = vec2<f32>(r0, r1) * field.chunk_size;
    let base_xz = corner + local_xz;
    let sampled = sample_field(base_xz);
    let ground_y = sampled.x;
    let ground_normal = normalize(vec3<f32>(sampled.y, 1.0, sampled.z));

    // Density fades with distance; a blade that loses the roll collapses.
    // Survivors widen so the ground stays covered as they thin out.
    let distance = length(base_xz - view.world_position.xz);
    let fade = 1.0 - clamp((distance - field.fade_start) / (field.fade_end - field.fade_start), 0.0, 1.0);
    let density = fade * fade;
    let alive = select(0.0, 1.0, keep_roll < density && ground_normal.y >= DIRT_SLOPE_NORMAL_Y);
    let widen = 1.0 + 2.5 * (1.0 - density);

    let t = vertex.position.y;
    let side = vertex.position.x;

    // A wheel that rolled over this spot lays the blade flat along its
    // direction; it springs back over TRAMPLE_RECOVER_S.
    let pressed = sample_trample(base_xz);
    let flat = clamp(pressed.x, 0.0, 1.0);
    let roll = vec3<f32>(pressed.y, 0.0, pressed.z);
    let yaw = seed * 6.2831853;
    let height = (BLADE_MIN_HEIGHT + (BLADE_MAX_HEIGHT - BLADE_MIN_HEIGHT) * rand(id, 6u)) * alive;
    let width = (BLADE_MIN_WIDTH + (BLADE_MAX_WIDTH - BLADE_MIN_WIDTH) * rand(id, 7u)) * widen;
    let dry = select(0.0, 0.3 + 0.7 * hash11(seed * 9.1), hash11(seed * 4.4 + 3.0) < 0.18);

    let right = vec3<f32>(cos(yaw), 0.0, sin(yaw));
    let lean_angle = hash11(seed * 6.1 + 4.0) * 6.2831853;
    let lean_dir = vec3<f32>(sin(lean_angle), 0.0, cos(lean_angle));
    let lean = lean_dir * (0.08 + 0.16 * hash11(seed * 7.13 + 5.0));

    let phase = base_xz.x * 0.31 + base_xz.y * 0.23 + seed * 2.0;
    let gust = sin(globals.time * 1.6 + phase) * 0.6
        + sin(globals.time * 4.3 + phase * 2.1) * 0.25
        + sin(globals.time * 0.37 + phase * 0.11) * 0.15;
    let bend = t * t * WIND_SWAY * height * (0.55 + gust) * alive;

    var p = vec3<f32>(base_xz.x, ground_y, base_xz.y)
        + vec3<f32>(0.0, 1.0, 0.0) * (height * t * (1.0 - 0.9 * flat))
        + roll * (height * t * 0.9)
        + lean * (height * t * t)
        + right * (width * 0.5 * side * (1.0 - t * 0.85) * alive)
        + WIND_DIR * bend;
    p.y = p.y - abs(bend) * 0.15;

    // Dense grass shows mostly tips, so the gradient stays dark: the root
    // is buried shade, the tip a mid green, dry blades go straw.
    let tone = 0.65 + 0.7 * rand(id, 5u);
    let root = vec3<f32>(0.035, 0.085, 0.015) * tone;
    let tip = mix(vec3<f32>(0.22, 0.42, 0.09), vec3<f32>(0.40, 0.34, 0.10), dry) * tone;

    var out: VertexOutput;
    out.world_position = vec4<f32>(p, 1.0);
    out.clip_position = view.clip_from_world * vec4<f32>(p, 1.0);
    out.world_normal = normalize(ground_normal + lean_dir * 0.3);
    out.color = vec4<f32>(mix(root, tip, t) * (1.0 - 0.25 * flat), 1.0);
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var pbr_input = pbr_input_new();
    pbr_input.material.base_color = in.color;
    pbr_input.material.perceptual_roughness = 0.85;
    pbr_input.material.metallic = 0.0;
    pbr_input.material.reflectance = vec3<f32>(0.12);
    pbr_input.material.flags = pbr_input.material.flags | STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT;
    pbr_input.frag_coord = in.clip_position;
    pbr_input.world_position = in.world_position;
    pbr_input.world_normal = normalize(in.world_normal);
    pbr_input.N = pbr_input.world_normal;
    pbr_input.V = calculate_view(in.world_position, false);
    pbr_input.flags = MESH_FLAGS_SHADOW_RECEIVER_BIT;

    var color = apply_pbr_lighting(pbr_input);
    color = main_pass_post_lighting_processing(pbr_input, color);
    return color;
}
