// Weeds rooted in the slab joints. Every instance is moved onto its nearest
// joint line, so none is spent on bare concrete, and it only survives
// where the yard has gone to seed.

#import bevy_pbr::{
    mesh_view_bindings::{view, globals},
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    pbr_types::{pbr_input_new, STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing, calculate_view},
}

#import "embedded://gearbox_fields/shaders/wind.wgsl"::{blade_leans}
#import "embedded://gearbox_fields/shaders/surface_detail.wgsl"::foliage_normal
#import "embedded://gearbox_fields/shaders/interaction.wgsl"::{WheelMapParams, sample_wheels, wheel_roll, scatter_roll}
#import "embedded://gearbox_fields/concrete/shaders/yard.wgsl"::{SLAB_M, JOINT_M, yard_noise, joint_distances, yard_weedy}

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
};

@group(3) @binding(0) var heightmap: texture_2d<f32>;
@group(3) @binding(1) var heightmap_sampler: sampler;
@group(3) @binding(2) var<uniform> field: VegetationParams;
@group(3) @binding(3) var trample: texture_2d<u32>;

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

fn pcg(input: u32) -> u32 {
    let state = input * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn rand(seed: u32, salt: u32) -> f32 {
    return f32(pcg(seed ^ (salt * 0x9E3779B9u))) / 4294967295.0;
}

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

fn within_field(world_xz: vec2<f32>) -> bool {
    return all(world_xz >= field.bounds.xy) && all(world_xz < field.bounds.zw);
}

fn culled_vertex() -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(0.0, 0.0, -2.0, 1.0);
    return out;
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    let chunk_seed = pcg(bitcast<u32>(i32(field.corner.x)) * 73856093u
        ^ bitcast<u32>(i32(field.corner.y)) * 19349663u);
    let id = pcg(vertex.instance_index ^ chunk_seed ^ 0x51EDu);
    let scattered = field.corner + vec2<f32>(rand(id, 1u), rand(id, 2u)) * field.chunk_size;

    // Onto the nearest joint line, a little off its centre.
    let reach = joint_distances(scattered);
    let line = round(scattered / SLAB_M) * SLAB_M;
    let off = (rand(id, 3u) - 0.5) * JOINT_M * 0.7;
    let base = select(vec2<f32>(scattered.x, line.y + off), vec2<f32>(line.x + off, scattered.y), reach.x < reach.y);

    // Weeds come in runs along a joint, with bare stretches between.
    let run = smoothstep(0.35, 0.65, yard_noise(base * 1.3 + vec2<f32>(3.0, 8.0)));
    let kept = rand(id, 4u) < yard_weedy(base) * run;
    let sampled = sample_field(base);
    let ground = vec3<f32>(base.x, sampled.x, base.y);
    let ground_normal = normalize(vec3<f32>(sampled.y, 1.0, sampled.z));
    let distance = length(ground - view.world_position);
    let rank = f32(vertex.instance_index) / max(field.blades_per_chunk, 1.0);
    let end = mix(field.fade_end, field.fade_start, sqrt(rank));
    let coverage = (1.0 - smoothstep(max(field.fade_start, end - 4.0), end, distance))
        * select(0.0, 1.0, kept && within_field(base));
    if (coverage <= 0.0) {
        return culled_vertex();
    }

    let t = vertex.position.y;
    let side = vertex.position.x;
    let leaf = u32(vertex.position.z);
    let seed = rand(id, 9u + leaf);
    let bearing = rand(id, 12u + leaf) * 6.2831853;
    let out_dir = vec3<f32>(cos(bearing), 0.0, sin(bearing));
    // Mostly low tufts, now and then a tall seeding stem.
    let tall = step(0.9, rand(id, 6u)) * step(f32(leaf), 0.5);
    let height = mix(mix(0.05, 0.14, rand(id, 20u + leaf)), 0.32, tall) * coverage;
    let pixel_m = distance * 2.0 / (view.clip_from_view[1][1] * view.viewport.w);
    let base_w = mix(0.006, 0.011, rand(id, 30u + leaf)) * mix(1.0, 0.55, tall);
    let width = base_w * max(1.0, 1.2 * pixel_m / base_w) * coverage;
    let arch = mix(0.4 + 0.45 * rand(id, 40u + leaf), 0.12, tall);

    let pressed = sample_wheels(trample, field.wheels, base);
    let flat = clamp(pressed.x, 0.0, 1.0);
    let roll = scatter_roll(wheel_roll(pressed), (rand(id, 37u + leaf) - 0.5) * 0.6);
    let press = smoothstep(0.0, mix(0.4, 1.0, rand(id, 36u + leaf)), flat) * field.wheels.bend;
    let gust = blade_leans(base, globals.time, field.wind, seed);
    let downwind = vec3<f32>(gust.x, 0.0, gust.y);
    let up = vec3<f32>(0.0, 1.0, 0.0);

    // A leaf arched out of the gap; a tyre lays it along its roll.
    let tip = mix((up * (1.0 - 0.35 * arch) + out_dir * arch + downwind * gust.w) * height,
        (roll * 0.9 + up * 0.05) * height, press);
    let mid = mix((up * 0.58 + out_dir * (0.3 * arch) + downwind * (gust.z * 0.3)) * height,
        (roll * 0.45 + up * 0.06) * height, press);
    let curve = mid * (2.0 * t * (1.0 - t)) + tip * (t * t);
    let tangent = normalize(mix(mid * 2.0, (tip - mid) * 2.0, t) + up * 1e-4);
    let across = normalize(cross(tangent, out_dir) + vec3<f32>(1e-5, 0.0, 0.0));
    let p = ground + curve + across * (width * 0.5 * (1.0 - t * t) * side);

    // Coarse yard weeds: dusty roots, dull green to straw tips.
    let dry = rand(id, 11u);
    let root = vec3<f32>(0.06, 0.08, 0.04);
    let top = mix(vec3<f32>(0.17, 0.31, 0.09), vec3<f32>(0.36, 0.36, 0.15), dry * dry);
    let colour = mix(root, mix(top, vec3<f32>(0.42, 0.36, 0.18), tall), t * t) * mix(0.85, 1.15, rand(id, 5u));

    var out: VertexOutput;
    out.world_position = vec4<f32>(p, 1.0);
    out.clip_position = view.clip_from_world * out.world_position;
    out.world_normal = normalize(mix(normalize(cross(across, tangent)), ground_normal, 0.35 + 0.6 * flat));
    out.ground_normal = ground_normal;
    out.canopy_uv = vec3<f32>(side, t, 0.0);
    out.color = vec4<f32>(colour * (1.0 - field.wheels.darkening * flat), 1.0);
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var pbr_input = pbr_input_new();
    let rib = mix(1.0, 0.86, smoothstep(0.1, 0.9, abs(in.canopy_uv.x)));
    pbr_input.material.base_color = vec4<f32>(in.color.rgb * rib, 1.0);
    pbr_input.material.perceptual_roughness = 0.94;
    pbr_input.material.metallic = 0.0;
    // No direct specular: on a steep blade normal at a grazing sun the GGX
    // term blows one pixel out to white for a frame.
    pbr_input.material.reflectance = vec3<f32>(0.0);
    pbr_input.specular_occlusion = 0.0;
    pbr_input.material.diffuse_transmission = mix(0.2, 0.42, in.canopy_uv.y);
    pbr_input.material.thickness = 0.0002;
    pbr_input.material.flags = pbr_input.material.flags | STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT;
    pbr_input.frag_coord = in.clip_position;
    pbr_input.world_position = in.world_position;
    pbr_input.world_normal = normalize(in.ground_normal);
    pbr_input.V = calculate_view(in.world_position, false);
    pbr_input.N = foliage_normal(in.world_normal, pbr_input.world_normal, pbr_input.V);
    pbr_input.flags = MESH_FLAGS_SHADOW_RECEIVER_BIT;

    var color = apply_pbr_lighting(pbr_input);
    color = main_pass_post_lighting_processing(pbr_input, color);
    color.a = 1.0;
    return color;
}
