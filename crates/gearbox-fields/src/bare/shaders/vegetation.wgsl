// What stands in bare ground: stones half buried in it, and the tufts of
// grass that take hold wherever it is left alone. Both are instanced — one
// mesh drawn many times, each one placed, turned and sized from its own
// number — so nothing is stored per stone but the number itself.

#import bevy_pbr::{
    mesh_view_bindings::{view, globals},
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    pbr_types::{pbr_input_new, STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing, calculate_view},
}
#import "embedded://gearbox_fields/shaders/wind.wgsl"::{blade_leans}
#import "embedded://gearbox_fields/shaders/surface_detail.wgsl"::foliage_normal
#import "embedded://gearbox_fields/shaders/interaction.wgsl"::{WheelMapParams, sample_wheels, wheel_roll}

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
    @location(3) @interpolate(perspective, centroid) shape: vec3<f32>,
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

// Where grass has taken the ground back. The same reading the soil under it
// makes, so the tufts stand where the soil shows green.
fn taken(place: vec2<f32>) -> f32 {
    let cell = floor(place / 9.0);
    let f = fract(place / 9.0);
    let ease = f * f * (3.0 - 2.0 * f);
    let corner = array<vec2<f32>, 4>(vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0), vec2(1.0, 1.0));
    var heights = array<f32, 4>();
    for (var i = 0; i < 4; i = i + 1) {
        let c = cell + corner[i];
        var h = u32(i32(c.x)) * 0x9E3779B9u ^ u32(i32(c.y)) * 0x85EBCA6Bu;
        h = h ^ (h >> 15u); h = h * 0x2C1B3C6Du; h = h ^ (h >> 12u);
        heights[i] = f32(h) / 4294967295.0;
    }
    let low = mix(heights[0], heights[1], ease.x);
    let high = mix(heights[2], heights[3], ease.x);
    return mix(low, high, ease.y);
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
    let base = field.corner + vec2<f32>(rand(id, 1u), rand(id, 2u)) * field.chunk_size;
    let sampled = sample_field(base);
    let ground = vec3<f32>(base.x, sampled.x, base.y);
    let ground_normal = normalize(vec3<f32>(sampled.y, 1.0, sampled.z));
    let distance = length(ground - view.world_position);
    let rank = f32(vertex.instance_index) / max(field.blades_per_chunk, 1.0);
    let end = mix(field.fade_end, field.fade_start, sqrt(rank));
    var alive = 1.0 - smoothstep(max(field.fade_start, end - 4.0), end, distance);
    if (!within_field(base)) {
        alive = 0.0;
    }
    if (alive <= 0.0) {
        return culled_vertex();
    }

    // A stone is meant for bare ground, a tuft for ground the grass has taken.
    let is_stone = vertex.position.z < 0.5;
    let green = taken(base);
    if (is_stone && green > 0.62) {
        return culled_vertex();
    }
    if (!is_stone && green < 0.58) {
        return culled_vertex();
    }

    let yaw = rand(id, 5u) * 6.2831853;
    let turn = mat2x2<f32>(cos(yaw), -sin(yaw), sin(yaw), cos(yaw));
    let pressed = sample_wheels(trample, field.wheels, base);
    let flat = clamp(pressed.x, 0.0, 1.0);

    var out: VertexOutput;
    if (is_stone) {
        // Stones are of a size, lie on their broadest face, and sit down into
        // the ground rather than on it.
        let size = mix(0.035, 0.16, pow(rand(id, 6u), 2.2)) * alive;
        let squat = mix(0.4, 0.75, rand(id, 7u));
        let local = vertex.position * vec3<f32>(size, size * squat, size);
        let spun = turn * local.xz;
        let sunk = size * squat * mix(0.25, 0.55, rand(id, 8u));
        out.world_position = vec4<f32>(ground + vec3<f32>(spun.x, local.y - sunk, spun.y), 1.0);
        let spun_n = turn * vertex.normal.xz;
        out.world_normal = normalize(vec3<f32>(spun_n.x, vertex.normal.y, spun_n.y));
        let grey = mix(0.09, 0.2, rand(id, 9u));
        out.color = vec4<f32>(vec3<f32>(grey * 1.05, grey, grey * 0.92), 1.0);
        out.shape = vec3<f32>(0.0, vertex.position.y, 0.0);
    } else {
        // A tuft leans downwind and lies flat where a wheel has been over it.
        let height = mix(0.05, 0.13, rand(id, 6u)) * alive;
        let width = mix(0.010, 0.02, rand(id, 7u));
        let t = vertex.position.y;
        let leans = blade_leans(base, globals.time, field.wind, rand(id, 3u));
        let lean = vec3<f32>(leans.x, 0.0, leans.y) * leans.w * t * t;
        let roll = wheel_roll(pressed) * flat * field.wheels.bend;
        let side = turn * vec2<f32>(vertex.position.x * width, 0.0);
        let stand = vec3<f32>(side.x, t * height, side.y) + (lean + roll) * height;
        out.world_position = vec4<f32>(ground + stand, 1.0);
        out.world_normal = ground_normal;
        let blade = mix(0.55, 1.1, rand(id, 9u)) * (1.0 - flat * field.wheels.darkening);
        out.color = vec4<f32>(vec3<f32>(0.055, 0.1, 0.028) * blade, 1.0);
        out.shape = vec3<f32>(1.0, t, vertex.position.z - 1.0);
    }
    out.clip_position = view.clip_from_world * out.world_position;
    out.ground_normal = ground_normal;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // A tuft is cut out of its quad into blades; a stone is solid.
    var alpha = 1.0;
    if (in.shape.x > 0.5) {
        let x = (in.shape.z + 0.5) * 3.0;
        let blade = floor(x);
        let random = fract(sin(blade * 37.7) * 4375.3);
        let top = mix(0.5, 1.0, random);
        let width = 0.32 * (1.0 - in.shape.y / top);
        let across = abs(fract(x) - 0.5);
        alpha = (1.0 - smoothstep(width - 0.05, width + 0.05, across))
            * (1.0 - smoothstep(top - 0.06, top + 0.06, in.shape.y));
    }
    if (alpha < 0.02) {
        discard;
    }
    var pbr_input = pbr_input_new();
    pbr_input.material.base_color = in.color;
    pbr_input.material.perceptual_roughness = select(0.72, 0.95, in.shape.x > 0.5);
    pbr_input.material.metallic = 0.0;
    pbr_input.material.reflectance = vec3<f32>(0.0);
    pbr_input.specular_occlusion = 0.0;
    pbr_input.material.flags = pbr_input.material.flags | STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT;
    pbr_input.frag_coord = in.clip_position;
    pbr_input.world_position = in.world_position;
    pbr_input.world_normal = normalize(in.ground_normal);
    pbr_input.V = calculate_view(in.world_position, false);
    pbr_input.N = foliage_normal(in.world_normal, pbr_input.world_normal, pbr_input.V);
    pbr_input.flags = MESH_FLAGS_SHADOW_RECEIVER_BIT;
    var colour = apply_pbr_lighting(pbr_input);
    colour = main_pass_post_lighting_processing(pbr_input, colour);
    colour.a = alpha;
    return colour;
}
