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
    var base = field.corner + vec2<f32>(rand(id, 1u), rand(id, 2u)) * field.chunk_size;
    // Grass grows in clumps with bare ground between them, so a tuft is drawn
    // most of the way to the middle of the clump it belongs to.
    let is_stone = vertex.uv.x < 0.5;
    var clump_seed = 0u;
    var out_of_clump = 0.0;
    if (!is_stone) {
        let cell_m = 0.85;
        let cell = floor(base / cell_m);
        clump_seed = pcg(bitcast<u32>(i32(cell.x)) * 2654435761u ^ bitcast<u32>(i32(cell.y)) * 40503u);
        // The clump sits anywhere in its cell, not at the middle of it, or the
        // grass comes up in rows like a crop.
        let middle = (cell + vec2<f32>(rand(clump_seed, 1u), rand(clump_seed, 2u))) * cell_m;
        // Some ground has no clump at all, and no two clumps are as tight.
        if (rand(clump_seed, 3u) < 0.38) {
            return culled_vertex();
        }
        let spread = mix(0.1, 0.3, rand(clump_seed, 4u));
        let about = rand(id, 20u) * 6.2831853;
        let out_by = sqrt(rand(id, 21u)) * spread;
        base = middle + vec2<f32>(cos(about), sin(about)) * out_by;
        out_of_clump = out_by / max(spread, 0.001);
    }
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

    let green = taken(base);
    // Whether one stands here is a chance weighted by the patch, so its edge
    // is ragged with stragglers rather than cut with a knife.
    let luck = rand(id, 12u);
    let grassy = smoothstep(0.42, 0.70, green);
    if (is_stone && luck < grassy * 0.85) {
        return culled_vertex();
    }
    if (!is_stone && luck > grassy) {
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
        // Mostly grit, a few pebbles, and now and then a stone worth kicking:
        // the size is drawn from a tail, not from a range.
        // A tyre presses what it rolls over down into the soil, so a stone in
        // a wheel mark is a stone half gone.
        let size = mix(0.003, 0.085, pow(rand(id, 6u), 5.5)) * alive * (1.0 - flat * 0.35);
        let squat = mix(0.4, 0.72, rand(id, 7u));
        let local = vertex.position * vec3<f32>(size, size * squat, size);
        let spun = turn * local.xz;
        // Well down into the soil: a stone sitting on top of the ground reads
        // as dropped there rather than turned up out of it.
        let sunk = size * squat * mix(mix(0.3, 0.62, rand(id, 8u)), 0.95, flat);
        out.world_position = vec4<f32>(ground + vec3<f32>(spun.x, local.y - sunk, spun.y), 1.0);
        let spun_n = turn * vertex.normal.xz;
        out.world_normal = normalize(vec3<f32>(spun_n.x, vertex.normal.y, spun_n.y));
        // No two stones are the same stone: some flint, some sandstone. All of
        // them warm, though — a stone lying in soil is stained by it, and a
        // cold grey one reads as a pebble washed up on a beach.
        let kind = rand(id, 10u);
        var rock = vec3<f32>(0.088, 0.078, 0.066);
        if (kind < 0.34) {
            rock = vec3<f32>(0.042, 0.039, 0.036);
        } else if (kind < 0.64) {
            rock = vec3<f32>(0.112, 0.081, 0.047);
        } else if (kind > 0.94) {
            rock = vec3<f32>(0.104, 0.094, 0.080);
        }
        // Stones lying in sand are the sand's own: sun-bleached, never flint.
        var spread_of_tone = vec2<f32>(0.6, 1.2);
        if (vertex.uv.y > 0.5) {
            rock = mix(vec3<f32>(0.262, 0.216, 0.150), vec3<f32>(0.178, 0.152, 0.114), kind);
            spread_of_tone = vec2<f32>(0.82, 1.14);
        }
        let stone = rock * mix(spread_of_tone.x, spread_of_tone.y, rand(id, 9u));
        // Dust settles on whatever faces the sky, so the top of a stone is
        // nearer the colour of the ground than the stone's own.
        let dust = vec3<f32>(0.105, 0.072, 0.044);
        out.color = vec4<f32>(mix(stone, dust, 0.4 * rand(id, 11u)), 1.0);
        out.shape = vec3<f32>(0.0, vertex.position.y, 0.0);
    } else {
        // A tuft leans downwind and lies flat where a wheel has been over it.
        // The clump has its own stature, and every leaf in it its own.
        let stature = mix(0.55, 1.35, rand(clump_seed, 5u));
        let domed = 1.0 - out_of_clump * out_of_clump * 0.55;
        let height = mix(0.11, 0.26, rand(id, 6u)) * stature * domed * alive;
        let width = mix(0.005, 0.011, rand(id, 7u));
        let t = vertex.position.y;
        let leaf = vertex.position.z - 1.0;
        // Each leaf of the tuft stands on its own bearing and bows outward,
        // and every one tapers from its base to a point.
        let bearing = yaw + leaf * 2.3999632 + rand(id, 13u) * 0.6;
        let out_of = vec2<f32>(cos(bearing), sin(bearing));
        let taper = width * (1.0 - t * t * 0.94);
        let across = vec2<f32>(-out_of.y, out_of.x) * vertex.position.x * taper;
        let bow = out_of * t * t * height * mix(0.18, 0.42, rand(id, 14u));
        let leans = blade_leans(base, globals.time, field.wind, rand(id, 3u));
        let lean = vec3<f32>(leans.x, 0.0, leans.y) * leans.w * t * t;
        let roll = wheel_roll(pressed) * flat * field.wheels.bend;
        let stand = vec3<f32>(across.x + bow.x, t * height * (1.0 - flat * 0.7), across.y + bow.y)
            + (lean + roll) * height;
        out.world_position = vec4<f32>(ground + stand, 1.0);
        out.world_normal = ground_normal;
        // Darker at the root, and no two tufts the same green.
        let blade = mix(0.5, 1.15, rand(id, 9u)) * mix(0.5, 1.0, t)
            * (1.0 - flat * field.wheels.darkening);
        // A clump at the thin edge of a patch is a clump short of water, and
        // it goes over to straw before the ones in the thick of it do.
        let thirst = 1.0 - smoothstep(0.40, 0.88, green);
        let dry = clamp(rand(clump_seed, 6u) * 0.5 + thirst * 0.55, 0.0, 1.0);
        let fresh = vec3<f32>(0.026, 0.082, 0.018);
        let straw = vec3<f32>(0.115, 0.088, 0.030);
        // The tips go first, so a drying clump is straw-headed and green-footed.
        let hue = mix(fresh, straw, clamp(dry * mix(0.55, 1.15, t), 0.0, 1.0));
        out.color = vec4<f32>(hue * blade * mix(0.82, 1.2, rand(clump_seed, 7u))
            * mix(0.92, 1.08, rand(id, 15u)), 1.0);
        out.shape = vec3<f32>(1.0, t, vertex.position.x);
    }
    out.clip_position = view.clip_from_world * out.world_position;
    out.ground_normal = ground_normal;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // A tuft is cut out of its quad into blades; a stone is solid.
    // A leaf is already tapered in its shape; the edges of it are only
    // softened so it does not end in a hard line of pixels.
    var alpha = 1.0;
    if (in.shape.x > 0.5) {
        let across = abs(in.shape.z);
        alpha = 1.0 - smoothstep(0.72, 1.0, across);
    }
    if (alpha < 0.02) {
        discard;
    }
    var pbr_input = pbr_input_new();
    pbr_input.material.base_color = in.color;
    pbr_input.material.perceptual_roughness = select(0.94, 0.95, in.shape.x > 0.5);
    pbr_input.material.metallic = 0.0;
    pbr_input.material.reflectance = vec3<f32>(0.0);
    pbr_input.specular_occlusion = 0.0;
    pbr_input.material.flags = pbr_input.material.flags | STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT;
    pbr_input.frag_coord = in.clip_position;
    pbr_input.world_position = in.world_position;
    pbr_input.V = calculate_view(in.world_position, false);
    if (in.shape.x > 0.5) {
        // A leaf is a sheet: it is lit by the ground it stands in, turned a
        // little towards whoever is looking at it.
        pbr_input.world_normal = normalize(in.ground_normal);
        pbr_input.N = foliage_normal(in.world_normal, pbr_input.world_normal, pbr_input.V);
    } else {
        // A stone is a solid, and has to be lit by its own faces, or it reads
        // as a flat disc painted on the soil. The face it is actually on beats
        // the rounded normal it was given: a stone is chipped, not turned.
        let rounded = normalize(in.world_normal);
        let across = cross(dpdx(in.world_position.xyz), dpdy(in.world_position.xyz));
        var facet = rounded;
        if (length(across) > 1e-12) {
            facet = normalize(across) * select(-1.0, 1.0, dot(across, rounded) > 0.0);
        }
        pbr_input.world_normal = normalize(mix(rounded, facet, 0.6));
        pbr_input.N = pbr_input.world_normal;
        // What of a stone is down in the soil sees little of the sky, which is
        // what stops it reading as a stone dropped on top of the ground.
        pbr_input.diffuse_occlusion = vec3<f32>(mix(0.55, 1.0, smoothstep(-1.0, 0.25, in.shape.y)));
    }
    pbr_input.flags = MESH_FLAGS_SHADOW_RECEIVER_BIT;
    var colour = apply_pbr_lighting(pbr_input);
    colour = main_pass_post_lighting_processing(pbr_input, colour);
    colour.a = alpha;
    return colour;
}
