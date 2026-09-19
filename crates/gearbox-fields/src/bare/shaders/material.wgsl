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

// Sand has no scan: it is dunes, the grain they are made of, and the comb
// the wind leaves across them. Value noise on a lattice, folded through
// itself so the lattice cannot show.
fn sand_noise(p: vec2<f32>) -> f32 {
    let cell = floor(p);
    let f = smooth2(fract(p));
    let a = hash21(cell);
    let b = hash21(cell + vec2<f32>(1.0, 0.0));
    let c = hash21(cell + vec2<f32>(0.0, 1.0));
    let d = hash21(cell + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

fn sand_fbm(p: vec2<f32>, octaves: i32) -> f32 {
    let turn = mat2x2<f32>(0.80, 0.60, -0.60, 0.80);
    var sum = 0.0;
    var weight = 0.0;
    var amplitude = 0.5;
    var q = p;
    for (var i = 0; i < octaves; i = i + 1) {
        sum = sum + amplitude * sand_noise(q);
        weight = weight + amplitude;
        amplitude = amplitude * 0.5;
        q = turn * q * 2.03 + vec2<f32>(11.3, 5.7);
    }
    return sum / weight;
}

// Dunes, and the smaller drifts lying over them.
fn sand_relief(place: vec2<f32>, slope: vec2<f32>) -> f32 {
    let warp = vec2<f32>(sand_fbm(place / 34.0, 2), sand_fbm(place / 34.0 + vec2<f32>(4.1, -7.3), 2)) - vec2<f32>(0.5);
    let dunes = sand_fbm((place + warp * 22.0) / 26.0, 3);
    let drifts = sand_fbm(place / 3.1 + vec2<f32>(19.0, -3.0), 3);
    let grit = sand_fbm(place / 0.35 + vec2<f32>(-7.0, 12.0), 2);
    // (the grain of it is left to the colour, which knows how far away it is)
    let combed = ripples(place, slope, ground.grain.w);
    // A field of sand is flat: what stands proud of it is centimetres, not
    // metres, or the shading of it shows the ground's own triangles.
    return dunes * 0.16 + drifts * 0.09 + grit * 0.025 + combed * ground.grain.z * 0.1;
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

    var colour = vec3<f32>(0.0);
    // Grain finer than the pixel that sees it is left out, not drawn: it
    // would only come back as a crawl of static.
    let pixel_m = max(fwidth(place.x), fwidth(place.y));
    let close = 1.0 - smoothstep(0.09, 0.42, pixel_m);
    // Patches of damper, coarser ground lie through the drier: the colour is
    // made of the same noise the shape is, so they agree.
    let clod = max(ground.grain.x, 0.05);
    let patchy = sand_fbm(place / 7.0 + vec2<f32>(3.3, 8.8), 3);
    let speck = mix(0.5, sand_fbm(place / (clod * 0.3), 2), close);
    colour = ground.tint.rgb * mix(0.74, 1.2, patchy) * mix(0.9, 1.12, speck);

    // Where a wheel has passed the ground is pressed flat and darkened.
    let pressed = sample_wheels(tracks, wheels, place);
    let rolled = clamp(pressed.x, 0.0, 1.0);
    colour *= 1.0 - rolled * wheels.darkening;

    // The normal from the height itself: a step to each side, and the cross
    // of what that leaves. Clods lit from the side read as clods.
    let step = max(clod * 0.12, 0.02);
    let pressed_down = 1.0 - rolled * 0.75;
    let here = made_relief(place, slope, close) * pressed_down;
    let east = made_relief(place + vec2<f32>(step, 0.0), slope, close) * pressed_down;
    let north = made_relief(place + vec2<f32>(0.0, step), slope, close) * pressed_down;
    // The slope of that height, taken across a step of it.
    let bumps = normalize(vec3<f32>((here - east) * 1.6, step, (here - north) * 1.6));
    let shaped = normalize(land + bumps - vec3<f32>(0.0, 1.0, 0.0));

    // Hollows between the clods keep the light out of them.
    // What stands proud catches the light; the hollows between keep it out.
    let pit = clamp(0.82 + here * 1.1, 0.7, 1.22);

    pbr_input.material.base_color = vec4<f32>(colour * pit, 1.0);
    // Sand glitters a little where its grains face the light.
    let glint = (sand_fbm(place / (clod * 0.08), 1) - 0.5) * 0.18 * close;
    pbr_input.material.perceptual_roughness = clamp(mix(0.95, 0.78, rolled) + glint, 0.35, 1.0);
    pbr_input.world_normal = shaped;
    pbr_input.N = shaped;
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
