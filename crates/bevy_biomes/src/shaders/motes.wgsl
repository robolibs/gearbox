// A box of air that follows the view. Each mote is nothing but its number:
// where it started, how the wind has carried it since, and which biome it
// finds under itself when it gets there.

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

const DUST: u32 = 0u;
const POLLEN: u32 = 1u;
const PETAL: u32 = 2u;
const CHAFF: u32 = 3u;
const SEED: u32 = 4u;

struct Vertex {
    @location(0) position: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) corner: vec2<f32>,
    @location(1) world_position: vec3<f32>,
    @location(2) alpha: f32,
};

fn pcg(input: u32) -> u32 {
    let state = input * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn rand(mote: u32, salt: u32) -> f32 {
    return f32(pcg(mote * 0x9E3779B9u ^ salt * 0x85EBCA6Bu ^ bitcast<u32>(field.seed))) / 4294967295.0;
}

fn smooth_band(low: f32, high: f32, value: f32) -> f32 {
    return smoothstep(low, high, value);
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
        let share = 1.0 - smooth_band(-half_band, half_band, distance_to(field.regions[i], place));
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
    let height = field.ground_m
        + fract(start.y + (field.rise_mps * time) / field.ceiling_m) * field.ceiling_m
        + stirred.y * 0.1;
    let world = vec3<f32>(ground_xz.x, height, ground_xz.y);

    // Only where this kind of mote belongs, and thinner where it is scarcer.
    let here = air_at(world.xz);
    if (rand(mote, 7u) * field.most_per_hectare >= here) {
        return culled();
    }

    let from_view = world - view.world_position;
    let distance = length(from_view);
    let near = smooth_band(0.5, 1.6, distance);
    let far = 1.0 - smooth_band(field.reach_m * 0.6, field.reach_m, distance);
    // Nothing is seen to wrap: they are already gone at the wall of the box.
    let from_wall = 1.0 - max(abs(from_view.x), abs(from_view.z)) / (extent * 0.5);
    let wall = smooth_band(0.0, 0.25, from_wall);
    let rising = select(1.0, 1.0 - smooth_band(0.75, 1.0, (height - field.ground_m) / field.ceiling_m), field.rise_mps > 0.0);
    let alpha = near * far * wall * rising;
    if (alpha <= 0.002) {
        return culled();
    }

    // Turning as it goes, and broadside on only now and then.
    let tumbles = field.kind == PETAL || field.kind == CHAFF;
    let turn = time * (0.5 + rand(mote, 5u) * 1.7) * select(0.15, 3.0, tumbles) + rand(mote, 6u) * 6.2831853;
    let flip = abs(cos(turn * 0.63 + rand(mote, 8u) * 3.14159));
    var shape = vec2<f32>(1.0, 1.0);
    if (field.kind == CHAFF) {
        shape = vec2<f32>(1.0, 0.16);
    } else if (field.kind == PETAL) {
        shape = vec2<f32>(1.0, 0.7);
    }
    shape.y = shape.y * mix(0.25, 1.0, select(1.0, flip, tumbles));

    let size = mix(field.size_m.x, field.size_m.y, rand(mote, 4u));
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
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // Nothing has an edge to it: a mote is a smudge that fades to nothing,
    // or it reads as a speck of glitter rather than something in the air.
    let radius = length(in.corner);
    var shape = pow(max(1.0 - radius, 0.0), 1.8);
    if (field.kind == CHAFF) {
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
    let into_sun = max(dot(-to_view, sun), 0.0);
    let glow = 1.0 + 0.7 * pow(into_sun, 4.0);
    let day = clamp(sun.y * 4.0, 0.06, 1.0);
    // Lit by the whole sky rather than the sun: dust does not glint.
    let sky = mix(vec3<f32>(0.62, 0.68, 0.78), sunlight, 0.35);
    let colour = field.colour.rgb * sky * glow * day;
    let strength = alpha * field.colour.a;
    return vec4<f32>(colour * strength, strength);
}
