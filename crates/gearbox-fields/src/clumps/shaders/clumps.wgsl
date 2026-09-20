#import bevy_pbr::{
    mesh_view_bindings::{view, globals},
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    pbr_types::{pbr_input_new, STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing, calculate_view},
}

#import "embedded://gearbox_fields/shaders/interaction.wgsl"::{WheelMapParams, sample_wheels, wheel_roll, scatter_roll}
#import "embedded://gearbox_fields/shaders/surface_detail.wgsl"::foliage_normal
#import "embedded://gearbox_fields/shaders/wind.wgsl"::{plant_lean}
// The patches a bare ground thins its own grass by, so a weed standing in
// one comes up where that grass does and not in a patch of its own.
#import "embedded://gearbox_fields/bare/shaders/cover.wgsl"::{taken, worn}

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
    way_shape: vec4<f32>,
};

@group(3) @binding(0) var heightmap: texture_2d<f32>;
@group(3) @binding(1) var heightmap_sampler: sampler;
@group(3) @binding(2) var<uniform> field: VegetationParams;
@group(3) @binding(3) var trample: texture_2d<u32>;
@group(3) @binding(4) var albedo: texture_2d<f32>;
@group(3) @binding(5) var albedo_sampler: sampler;

// A packed clump vertex; colour is (variant, patch salt, sparse share, gain).
struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(5) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) @interpolate(perspective, centroid) world_position: vec4<f32>,
    @location(1) @interpolate(perspective, centroid) world_normal: vec3<f32>,
    @location(2) @interpolate(perspective, centroid) uv: vec2<f32>,
    @location(3) @interpolate(perspective, centroid) shade: vec3<f32>,
    @location(4) @interpolate(perspective, centroid) ground_normal: vec3<f32>,
};

const FADE_M: f32 = 3.0;
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

fn blade_fade_end(rank: f32) -> f32 {
    if field.inverse_square_thinning == 0u {
        return mix(field.fade_end, field.fade_start, sqrt(rank));
    }
    return field.fade_start * field.fade_end
        / (field.fade_start + (field.fade_end - field.fade_start) * sqrt(rank));
}

fn patch_noise(p: vec2<f32>) -> f32 {
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

// The nine-metre grass patches of a bare ground, hashed exactly as that
// ground's own shader hashes them. A different hash here would put the weeds
// in patches of their own that have nothing to do with where the grass is.
// A vertex of an instance culled before any shaping: outside the clip volume.
fn culled_vertex() -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(0.0, 0.0, -2.0, 1.0);
    return out;
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    // The draw's first instance carries the variant count (bits 28-31) and
    // this variant (24-27); plants interleave so each id keeps its variant.
    let count = max(vertex.instance_index >> 28u, 1u);
    let variant = (vertex.instance_index >> 24u) & 15u;
    let index = (vertex.instance_index & 0xFFFFFFu) * count + variant;
    let chunk_seed = pcg(bitcast<u32>(i32(field.corner.x)) * 73856093u
        ^ bitcast<u32>(i32(field.corner.y)) * 19349663u);
    let salt = vertex.color.y;
    let id = pcg(index ^ chunk_seed ^ (u32(salt) * 0x68E31DA4u));
    let base = field.corner + vec2<f32>(rand(id, 1u), rand(id, 2u)) * field.chunk_size;
    let sampled = sample_field(base);
    let ground = vec3<f32>(base.x, sampled.x, base.y);
    let ground_normal = normalize(vec3<f32>(sampled.y, 1.0, sampled.z));
    let distance = length(ground - view.world_position);
    let end = blade_fade_end(f32(index) / max(field.blades_per_chunk, 1.0));

    // Thick inside this pack's patches, a sparse share of plants elsewhere.
    var patchiness = smoothstep(0.55, 0.78, patch_noise(base * 0.08 + vec2<f32>(salt * 37.1, -salt * 19.7)));
    var loose = vertex.color.z;
    if (field.follow_grass > 0.0) {
        // On bare ground a weed comes up where the grass has, not where this
        // pack would have put it: the same nine-metre patches the tufts of that
        // ground are thinned by, and only the given share out on the bare.
        patchiness = smoothstep(0.42, 0.70, taken(base));
        loose = field.follow_grass;
    }
    // Nothing broad-leaved survives where a way is worn; a few stragglers hold
    // on at its edge, which is where the wear is already fading.
    let bared = worn(base, field.bounds, field.tread, field.way, field.way_shape);
    let kept = rand(id, 9u) < mix(loose, 1.0, patchiness) * (1.0 - bared);
    let coverage = (1.0 - smoothstep(max(field.fade_start, end - FADE_M), end, distance))
        * select(0.0, 1.0, kept && ground_normal.y >= DIRT_SLOPE_NORMAL_Y && within_field(base));
    if (coverage <= 0.0) {
        return culled_vertex();
    }

    let yaw = rand(id, 4u) * 6.2831853;
    let c = cos(yaw);
    let s = sin(yaw);
    let scale = (0.8 + 0.4 * rand(id, 5u)) * coverage;
    var local = vec3<f32>(c * vertex.position.x - s * vertex.position.z, vertex.position.y,
        s * vertex.position.x + c * vertex.position.z) * scale;
    let normal = vec3<f32>(c * vertex.normal.x - s * vertex.normal.z, vertex.normal.y,
        s * vertex.normal.x + c * vertex.normal.z);

    // Wheels lay the clump down along the roll; wind sways its top.
    let pressed = sample_wheels(trample, field.wheels, base);
    let flat = clamp(pressed.x, 0.0, 1.0);
    local += wheel_roll(pressed) * (flat * local.y * field.wheels.bend);
    local.y *= 1.0 - flat * field.wheels.bend;
    let rise = max(local.y, 0.0);
    local += plant_lean(base, globals.time, field.wind, rand(id, 9u), 1.0)
        * (rise * min(rise / 0.08, 1.0) * 0.9 * (1.0 - flat));

    var out: VertexOutput;
    out.world_position = vec4<f32>(ground + local, 1.0);
    out.clip_position = view.clip_from_world * out.world_position;
    // Far clumps take the ground's normal: one tilted away from a low sun
    // would otherwise drop out black against lit ground.
    let settle = smoothstep(6.0, 30.0, distance) * 0.9;
    out.world_normal = normalize(mix(normal, ground_normal, max(flat * 0.7, settle)));
    out.uv = vertex.uv;
    // What of a plant is down among its own leaves and the sward around it
    // sees little of the sky. Nothing here casts a shadow, so a plant that is
    // lit as brightly at its root as at its crown reads as stuck on the ground
    // rather than growing out of it.
    let rooted = mix(0.42, 1.0, smoothstep(0.0, 0.2, rise));
    out.shade = vec3<f32>((0.85 + 0.3 * rand(id, 6u)) * rooted, flat, vertex.color.w);
    out.ground_normal = ground_normal;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // Cut-out alpha from a mip no coarser than 1, boosted by the true mip so
    // thin blades keep their coverage at a distance instead of dissolving.
    let size = vec2<f32>(textureDimensions(albedo));
    let dx = dpdx(in.uv * size);
    let dy = dpdy(in.uv * size);
    let lod = max(0.5 * log2(max(dot(dx, dx), dot(dy, dy))), 0.0);
    let texel = textureSample(albedo, albedo_sampler, in.uv);
    let cutout = textureSampleLevel(albedo, albedo_sampler, in.uv, min(lod, 1.0)).a
        * (1.0 + max(lod - 1.0, 0.0) * 0.35);
    if (cutout < 0.5) { discard; }
    var pbr_input = pbr_input_new();
    pbr_input.material.base_color = vec4<f32>(
        texel.rgb * in.shade.x * in.shade.z * (1.0 - field.wheels.darkening * in.shade.y), 1.0);
    pbr_input.material.perceptual_roughness = 0.9;
    pbr_input.material.metallic = 0.0;
    // No direct specular: on a steep blade normal at a grazing sun the GGX
    // term blows one pixel out to white for a frame.
    pbr_input.material.reflectance = vec3<f32>(0.0);
    pbr_input.specular_occlusion = 0.0;
    pbr_input.material.diffuse_transmission = 0.4;
    pbr_input.material.thickness = 0.0002;
    pbr_input.material.flags = pbr_input.material.flags | STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT;
    pbr_input.frag_coord = in.clip_position;
    pbr_input.world_position = in.world_position;
    pbr_input.world_normal = normalize(in.ground_normal);
    pbr_input.V = calculate_view(in.world_position, false);
    // A grass blade stands on edge, so its face points sideways and wants the
    // flattening that keeps a sward from shimmering. A broad leaf is held out
    // flat, its face points at the sky, and flattening it too leaves the plant
    // reading as a paper cut-out. How far the face is from upright decides it.
    let face = normalize(in.world_normal);
    let lit = select(-face, face, dot(face, pbr_input.V) > 0.0);
    pbr_input.N = normalize(mix(
        foliage_normal(in.world_normal, pbr_input.world_normal, pbr_input.V),
        lit,
        smoothstep(0.35, 0.8, abs(face.y))));
    pbr_input.flags = MESH_FLAGS_SHADOW_RECEIVER_BIT;
    var color = apply_pbr_lighting(pbr_input);
    color = main_pass_post_lighting_processing(pbr_input, color);
    color.a = 1.0;
    return color;
}
