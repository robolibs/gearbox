// A box of air that follows the view. Each mote is nothing but its number:
// where it started, how the wind has carried it since, and what it finds
// under itself when it gets there. What it looks like is not its own: a petal
// takes the colour of the flowers it came off, a sliver the colour of the
// grass, and neither is anywhere the thing it came from is not.

#import bevy_pbr::mesh_view_bindings::{view, globals, lights}

struct MoteField {
    colour: vec4<f32>,
    wind: vec4<f32>,
    background: vec4<f32>,
    size_m: vec2<f32>,
    rise_mps: f32,
    drag: f32,
    ceiling_m: f32,
    reach_m: f32,
    extent_m: f32,
    ground_m: f32,
    count: u32,
    kind: u32,
    region_count: u32,
    most_per_hectare: f32,
    seed: f32,
    pad: f32,
    regions: array<vec4<f32>, 16>,
    region_air: array<vec4<f32>, 16>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> field: MoteField;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var gust_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var gust_sampler: sampler;

// Metres one tile of the gust map covers; it is the plants' own map, so the
// air surges where they lean.
const WIND_TILE: f32 = 256.0;

struct Gust {
    dir: vec3<f32>,
    strength: f32,
};

fn wind_at(place: vec2<f32>, time: f32) -> Gust {
    let downwind = field.wind.xy * time * max(field.wind.z, 0.5);
    let broad = textureSampleLevel(gust_map, gust_sampler, (place - downwind) / WIND_TILE, 0.0);
    let detail = textureSampleLevel(gust_map, gust_sampler,
        (place - downwind * 1.35) / WIND_TILE + vec2<f32>(0.37, 0.61), 0.0).g;
    let gust = smoothstep(0.35, 0.72, broad.r * 0.7 + detail * 0.3);
    let push = clamp(field.wind.z / 6.0, 0.0, 1.5);
    let turn = (broad.b - 0.5) * 0.9;
    let dir = vec3<f32>(field.wind.x * cos(turn) - field.wind.y * sin(turn), 0.0,
        field.wind.x * sin(turn) + field.wind.y * cos(turn));
    return Gust(dir, push * mix(1.0 - clamp(field.wind.w, 0.0, 1.0), 1.0, gust));
}

const DUST: u32 = 0u;
const POLLEN: u32 = 1u;
const PETAL: u32 = 2u;
const CHAFF: u32 = 3u;
const SEED: u32 = 4u;
const LEAF: u32 = 5u;

struct Vertex {
    @location(0) position: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) corner: vec2<f32>,
    @location(1) world_position: vec3<f32>,
    @location(2) alpha: f32,
    @location(3) tint: vec3<f32>,
    @location(4) shade: f32,
};

fn pcg(input: u32) -> u32 {
    let state = input * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn rand(mote: u32, salt: u32) -> f32 {
    return f32(pcg(mote * 0x9E3779B9u ^ salt * 0x85EBCA6Bu ^ bitcast<u32>(field.seed))) / 4294967295.0;
}

// The ground's own noise, as the plants standing in it use: a mote must find
// the same patches they were placed by, or it will drift over the wrong ones.
fn gradient(cell: vec2<f32>) -> vec2<f32> {
    let hash = fract(sin(dot(cell, vec2<f32>(127.1, 311.7))) * 43758.5453123);
    let angle = hash * 6.2831853;
    return vec2<f32>(cos(angle), sin(angle));
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let a = dot(gradient(i), f);
    let b = dot(gradient(i + vec2<f32>(1.0, 0.0)), f - vec2<f32>(1.0, 0.0));
    let c = dot(gradient(i + vec2<f32>(0.0, 1.0)), f - vec2<f32>(0.0, 1.0));
    let d = dot(gradient(i + vec2<f32>(1.0, 1.0)), f - vec2<f32>(1.0, 1.0));
    return 0.5 + 0.7 * mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// The flower patch under a place: how much of it is in bloom, and which
// flower it is. One kind to a patch, as they grow.
fn flower_patch(place: vec2<f32>) -> vec2<f32> {
    let warp = vec2<f32>(noise(place * 0.04 + vec2<f32>(5.1, -3.3)),
        noise(place * 0.04 + vec2<f32>(-7.7, 2.9))) - vec2<f32>(0.5);
    let cell = floor((place + warp * 24.0) / 14.0);
    let kind = pcg(bitcast<u32>(i32(cell.x)) * 2654435761u ^ bitcast<u32>(i32(cell.y)) * 40503u) % 7u;
    let opening = smoothstep(0.52, 0.72, noise(place * 0.11 + cell * 3.7));
    return vec2<f32>(opening, f32(kind));
}

fn bloom_colour(kind: u32) -> vec3<f32> {
    if (kind == 0u) { return vec3<f32>(0.92, 0.92, 0.86); }
    if (kind == 1u) { return vec3<f32>(0.96, 0.80, 0.08); }
    if (kind == 2u) { return vec3<f32>(0.80, 0.34, 0.55); }
    if (kind == 3u) { return vec3<f32>(0.24, 0.36, 0.86); }
    if (kind == 4u) { return vec3<f32>(0.86, 0.10, 0.05); }
    if (kind == 5u) { return vec3<f32>(0.96, 0.82, 0.10); }
    return vec3<f32>(0.36, 0.44, 0.20);
}

// The grass under a place, by the species patches it grows in.
fn grass_colour(place: vec2<f32>) -> vec3<f32> {
    let warp = vec2<f32>(noise(place * 0.035 + vec2<f32>(3.7, -8.1)),
        noise(place * 0.035 + vec2<f32>(-5.9, 2.3))) - vec2<f32>(0.5);
    let p = place + warp * 16.0;
    let fescue = smoothstep(0.56, 0.74, noise(p * 0.07 + vec2<f32>(41.0, 7.0)));
    let rye = smoothstep(0.56, 0.74, noise(p * 0.055 + vec2<f32>(-19.0, 63.0)));
    let species = vec2<f32>(fescue, rye) / max(fescue + rye, 1.0);
    let tint = vec3<f32>(1.0) + species.x * vec3<f32>(-0.12, 0.02, 0.22)
        + species.y * vec3<f32>(-0.25, 0.18, -0.30);
    // A blade off the plant has dried a little: paler and browner than one on it.
    return vec3<f32>(0.19, 0.31, 0.075) * tint * 1.15 + vec3<f32>(0.05, 0.04, 0.015);
}

// How far a place is outside a rectangle; negative inside it.
fn distance_to(bounds: vec4<f32>, place: vec2<f32>) -> f32 {
    let outside = max(bounds.xy - place, place - bounds.zw);
    return length(max(outside, vec2<f32>(0.0))) + min(max(outside.x, outside.y), 0.0);
}

// Motes a hectare where the mote is: the same mixing the biomes do, so a
// hard border stops them dead and a soft one thins them out across it.
fn air_at(place: vec2<f32>) -> f32 {
    for (var i = 0u; i < field.region_count; i = i + 1u) {
        let air = field.region_air[i];
        if (air.z > 0.5 && distance_to(field.regions[i], place) < 0.0) {
            return air.x;
        }
    }
    var here = 0.0;
    var taken = 0.0;
    for (var i = 0u; i < field.region_count; i = i + 1u) {
        let air = field.region_air[i];
        if (air.z > 0.5) {
            continue;
        }
        let half_band = max(air.y * 0.5, 0.0001);
        let share = 1.0 - smoothstep(-half_band, half_band, distance_to(field.regions[i], place));
        here = here + share * air.x;
        taken = taken + share;
    }
    return here + max(1.0 - taken, 0.0) * field.background.x;
}

// Enough stirring that the air is never a conveyor belt; it displaces rather
// than drifts, so nothing is carried away by it.
fn stirring(place: vec3<f32>, time: f32) -> vec3<f32> {
    let across = sin(place.x * 0.21 + time * 0.7) + sin(place.z * 0.17 - time * 0.53);
    let lift = sin(place.y * 0.35 + time * 0.9) + sin(place.x * 0.13 + time * 0.31);
    let along = sin(place.z * 0.23 + time * 0.61) + sin(place.y * 0.19 - time * 0.44);
    return vec3<f32>(across, lift * 0.45, along);
}

fn culled() -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(0.0, 0.0, -2.0, 1.0);
    return out;
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    let mote = u32(vertex.position.z);
    if (mote >= field.count) {
        return culled();
    }
    let extent = field.extent_m;
    let time = globals.time;

    // Where it would be by now, had it started at its own corner of the box.
    let start = vec3<f32>(rand(mote, 1u), rand(mote, 2u), rand(mote, 3u));
    let carried = vec3<f32>(field.wind.x, 0.0, field.wind.y) * field.wind.z * field.drag * time;
    let stirred = stirring(start * extent, time) * (0.3 + 0.25 * field.wind.z) * field.drag;

    // The box is always around the view, and a mote leaving one side comes
    // back at the other: within the box its place in the world holds still.
    let floor_of_box = vec2<f32>(view.world_position.x, view.world_position.z) - vec2<f32>(extent * 0.5);
    let along = start.xz * extent + carried.xz + stirred.xz;
    let ground_xz = floor_of_box + fract((along - floor_of_box) / extent) * extent;
    // The gust the plants here are leaning in carries the mote with them.
    let gust = wind_at(ground_xz, time);
    let surge = gust.dir.xz * gust.strength * 2.2;
    let height = field.ground_m
        + fract(start.y + (field.rise_mps * time) / field.ceiling_m) * field.ceiling_m
        + stirred.y * 0.1
        + gust.strength * 0.35;
    let world = vec3<f32>(ground_xz.x + surge.x, height, ground_xz.y + surge.y);

    // Only where this kind of mote belongs, and thinner where it is scarcer.
    let here = air_at(world.xz);
    if (rand(mote, 7u) * field.most_per_hectare >= here) {
        return culled();
    }

    // What it came off, which is also whether there was anything to come off.
    var tint = field.colour.rgb;
    var size_scale = 1.0;
    if (field.kind == PETAL) {
        let bloom = flower_patch(world.xz);
        if (rand(mote, 11u) > bloom.x) {
            return culled();
        }
        let bloom_tint = bloom_colour(u32(bloom.y));
        // Held to the brightness of straw, but by the whole colour at once: a
        // cornflower petal that lost its blue would only be another white speck.
        let brightest = max(max(bloom_tint.r, bloom_tint.g), bloom_tint.b);
        let held = bloom_tint * min(1.0, 0.66 / max(brightest, 0.001));
        let dull = vec3<f32>(dot(held, vec3<f32>(0.3, 0.6, 0.1)));
        tint = mix(held, dull, 0.18);
        size_scale = select(1.0, 1.7, u32(bloom.y) == 4u);
    } else if (field.kind == LEAF) {
        tint = grass_colour(world.xz);
    } else {
        // Dust and chaff are the ground they came off: paler where it has
        // dried out, darker over the damp, and never the same twice.
        let dry = noise(world.xz * 0.045 + vec2<f32>(19.0, -7.0));
        let damp = noise(world.xz * 0.012 + vec2<f32>(-31.0, 53.0));
        tint = tint * mix(0.72, 1.18, dry) * mix(0.88, 1.06, damp);
    }

    // Never against the sky: a scrap that small, seen against something that
    // bright, is a white speck whatever colour it is. Below the eye it is
    // seen against the ground, where its own colour shows.
    if (world.y > view.world_position.y - 0.25) {
        return culled();
    }
    let from_view = world - view.world_position;
    let distance = length(from_view);
    let near = smoothstep(0.5, 1.6, distance);
    let far = 1.0 - smoothstep(field.reach_m * 0.6, field.reach_m, distance);
    // Nothing is seen to wrap: they are already gone at the wall of the box.
    let from_wall = 1.0 - max(abs(from_view.x), abs(from_view.z)) / (extent * 0.5);
    let wall = smoothstep(0.0, 0.25, from_wall);
    let rising = select(1.0, 1.0 - smoothstep(0.75, 1.0, (height - field.ground_m) / field.ceiling_m), field.rise_mps > 0.0);
    let alpha = near * far * wall * rising;
    if (alpha <= 0.002) {
        return culled();
    }

    // Turning as it goes, and broadside on only now and then.
    let tumbles = field.kind == PETAL || field.kind == CHAFF || field.kind == LEAF;
    let turn = time * (0.5 + rand(mote, 5u) * 1.7) * select(0.15, 2.4, tumbles) + rand(mote, 6u) * 6.2831853;
    let flip = abs(cos(turn * 0.63 + rand(mote, 8u) * 3.14159));
    var shape = vec2<f32>(1.0, 1.0);
    if (field.kind == CHAFF) {
        shape = vec2<f32>(1.0, 0.16);
    } else if (field.kind == LEAF) {
        shape = vec2<f32>(1.0, 0.22);
    } else if (field.kind == PETAL) {
        shape = vec2<f32>(1.0, 0.7);
    }
    shape.y = shape.y * mix(0.25, 1.0, select(1.0, flip, tumbles));

    let size = mix(field.size_m.x, field.size_m.y, rand(mote, 4u)) * size_scale;
    let spun = vec2<f32>(
        vertex.position.x * cos(turn) - vertex.position.y * sin(turn),
        vertex.position.x * sin(turn) + vertex.position.y * cos(turn),
    );
    let right = normalize(view.world_from_view[0].xyz);
    let up = normalize(view.world_from_view[1].xyz);
    let offset = right * spun.x * size * shape.x + up * spun.y * size * shape.y;

    var out: VertexOutput;
    out.world_position = world + offset;
    out.clip_position = view.clip_from_world * vec4<f32>(out.world_position, 1.0);
    out.corner = vertex.position.xy;
    out.alpha = alpha;
    out.tint = tint;
    // Edge on it catches almost nothing, broadside the whole sun.
    out.shade = mix(0.35, 1.15, flip) * mix(0.8, 1.2, rand(mote, 12u));
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // Nothing has an edge to it: a mote is a smudge that fades to nothing,
    // or it reads as a speck of glitter rather than something in the air.
    let radius = length(in.corner);
    var shape = pow(max(1.0 - radius, 0.0), 1.8);
    if (field.kind == CHAFF || field.kind == LEAF) {
        shape = pow(max(1.0 - abs(in.corner.x), 0.0), 0.9) * pow(max(1.0 - abs(in.corner.y), 0.0), 1.5);
    }
    let alpha = in.alpha * shape;
    if (alpha <= 0.003) {
        discard;
    }

    // Dust is mostly seen against the light: it is the sun through it, not on
    // it, and there is nothing to see of it after dark.
    var sun = vec3<f32>(0.0, 1.0, 0.0);
    var sunlight = vec3<f32>(1.0);
    if (lights.n_directional_lights > 0u) {
        sun = lights.directional_lights[0].direction_to_light;
        sunlight = lights.directional_lights[0].color.rgb;
    }
    let to_view = normalize(view.world_position - in.world_position);
    let day = clamp(sun.y * 4.0, 0.06, 1.0);
    // Lit as the thing it broke off would be: some sky on it, some sun, and
    // darker as it turns edge on. Nothing here makes its own light.
    // A light's colour here carries its illuminance with it, tens of thousands
    // of lux, so only its hue is any use: multiplied by the rest a mote would
    // come out white whatever colour it was given. A scrap lying in a field is
    // about as dark as the field.
    let sun_hue = sunlight / max(max(sunlight.r, max(sunlight.g, sunlight.b)), 0.0001);
    let sky = vec3<f32>(0.10, 0.11, 0.13);
    let colour = in.tint * (sky + sun_hue * day * 0.55 * in.shade);
    let strength = alpha * field.colour.a;
    return vec4<f32>(colour * strength, strength);
}
