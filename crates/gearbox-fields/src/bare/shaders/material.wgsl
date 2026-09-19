// Bare soil: no blades, so all of it has to come from the ground itself.
//
// Two readings of one scan at unrelated scales, each taken from a different
// corner of it per cell so the eye never finds the tile; a height built from
// the same readings, and the normal taken from that height rather than from
// a scan, so clods catch the light from whichever side it comes; and, where
// the wind has combed the ground, ripples across the slope.

#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
}
#import "embedded://gearbox_fields/shaders/interaction.wgsl"::{WheelMapParams, sample_wheels}
#import "embedded://gearbox_fields/shaders/surface_detail.wgsl"::{SurfaceGeometryParams, surface_geometry_normal}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var soil_albedo: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var soil_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var soil_height: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var grit_albedo: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var grit_height: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var tracks: texture_2d<u32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var<uniform> wheels: WheelMapParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var surface_heightmap: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(108) var<uniform> geometry: SurfaceGeometryParams;

struct BareGround {
    tint: vec4<f32>,
    grain: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(109) var<uniform> ground: BareGround;

fn hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453123);
}

fn smooth2(v: vec2<f32>) -> vec2<f32> {
    return v * v * (3.0 - 2.0 * v);
}

// One cell's reading of a scan, taken from its own corner of it and turned
// by its own quarter, so neighbouring cells never show the same piece twice.
fn cell_read(tex: texture_2d<f32>, uv: vec2<f32>, cell: vec2<f32>, seed: f32) -> vec4<f32> {
    let pick = hash21(cell + seed);
    let turn = floor(pick * 4.0) * 1.5707963;
    let offset = vec2<f32>(hash21(cell + seed + 3.7), hash21(cell + seed + 9.1));
    let centred = uv - cell - vec2<f32>(0.5);
    let turned = vec2<f32>(
        centred.x * cos(turn) - centred.y * sin(turn),
        centred.x * sin(turn) + centred.y * cos(turn),
    );
    return textureSample(tex, soil_sampler, turned + offset);
}

// The scan read without its tile: four cells' readings blended across.
fn scattered(tex: texture_2d<f32>, uv: vec2<f32>, seed: f32) -> vec4<f32> {
    let cell = floor(uv);
    let f = smooth2(fract(uv));
    let a = cell_read(tex, uv, cell, seed);
    let b = cell_read(tex, uv, cell + vec2<f32>(1.0, 0.0), seed);
    let c = cell_read(tex, uv, cell + vec2<f32>(0.0, 1.0), seed);
    let d = cell_read(tex, uv, cell + vec2<f32>(1.0, 1.0), seed);
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// Ripples the wind has combed across the ground, running with the slope and
// only where the ground is near enough level to hold them.
fn ripples(place: vec2<f32>, slope: vec2<f32>, spacing: f32) -> f32 {
    // Ripples that run dead straight read as corduroy. Their line wanders
    // with the ground, their spacing drifts, and they break up along their
    // length, as combed sand does.
    let along = normalize(slope * 3.0 + vec2<f32>(0.31, -0.19));
    let turn = sin(place.x * 0.031) * 0.3 + sin(place.y * 0.027 + 1.7) * 0.3;
    let lean = vec2<f32>(along.x * cos(turn) - along.y * sin(turn), along.x * sin(turn) + along.y * cos(turn));
    let across = dot(place, vec2<f32>(-lean.y, lean.x)) / max(spacing, 0.05);
    let wander = sin(dot(place, lean) * 0.9) * 0.4 + sin(dot(place, lean) * 0.23 + 2.1) * 0.6;
    let crest = sin((across + wander) * 6.2831853) * 0.5 + 0.5;
    let broken = 0.55 + 0.45 * sin(dot(place, lean) * 1.7 + 0.6);
    return crest * broken;
}

// The height of the ground under a place, from the two readings and the
// ripples; the normal is the slope of this, not of a scan.
fn relief(place: vec2<f32>, slope: vec2<f32>) -> f32 {
    let clods = scattered(soil_height, place / max(ground.grain.x, 0.05), 23.0).r;
    let grit = scattered(grit_height, place / max(ground.grain.x * 0.24, 0.02) + vec2<f32>(7.7, -5.3), 61.0).r;
    let combed = ripples(place, slope, ground.grain.w);
    return mix(clods, grit, 0.35) * ground.grain.y + combed * ground.grain.z * 0.12;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let place = in.world_position.xz;

    // The ground's own shape, so the clods lie along the land and not across it.
    let land = surface_geometry_normal(surface_heightmap, geometry, place, in.world_normal);
    let slope = vec2<f32>(land.x, land.z);

    let soil = scattered(soil_albedo, place / max(ground.grain.x, 0.05), 11.0).rgb;
    let grit = scattered(grit_albedo, place / max(ground.grain.x * 0.24, 0.02) + vec2<f32>(61.0, -37.0), 59.0).rgb;
    var colour = mix(soil, grit, 0.22) * ground.tint.rgb;

    // Where a wheel has passed the ground is pressed flat and darkened.
    let pressed = sample_wheels(tracks, wheels, place);
    let rolled = clamp(pressed.x, 0.0, 1.0);
    colour *= 1.0 - rolled * wheels.darkening;

    // The normal from the height itself: a step to each side, and the cross
    // of what that leaves. Clods lit from the side read as clods.
    let step = max(ground.grain.x * 0.06, 0.01);
    let here = relief(place, slope) * (1.0 - rolled * 0.75);
    let east = relief(place + vec2<f32>(step, 0.0), slope) * (1.0 - rolled * 0.75);
    let north = relief(place + vec2<f32>(0.0, step), slope) * (1.0 - rolled * 0.75);
    let bumps = normalize(vec3<f32>(here - east, step * 2.2, here - north));
    let shaped = normalize(land + bumps - vec3<f32>(0.0, 1.0, 0.0));

    // Hollows between the clods keep the light out of them.
    let pit = clamp(1.0 - (here - 0.35) * 0.9, 0.55, 1.15);

    pbr_input.material.base_color = vec4<f32>(colour * pit, 1.0);
    pbr_input.material.perceptual_roughness = mix(0.95, 0.78, rolled);
    pbr_input.world_normal = shaped;
    pbr_input.N = shaped;
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
