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

// A hash of whole numbers, mixed bit by bit. The usual `fract(sin(dot(..)))`
// is not a hash at all at these coordinates: its own periods beat against the
// lattice and draw rings and whorls across the ground, which is what put
// fingerprints in the soil.
fn hash21(p: vec2<f32>) -> f32 {
    let i = vec2<i32>(floor(p + 0.5));
    var h = u32(i.x) * 0x9E3779B9u ^ u32(i.y) * 0x85EBCA6Bu;
    h = h ^ (h >> 15u);
    h = h * 0x2C1B3C6Du;
    h = h ^ (h >> 12u);
    h = h * 0x297A2D39u;
    h = h ^ (h >> 15u);
    return f32(h) / 4294967295.0;
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
// The comb across bare ground: a plough's furrows, or the wind's ripples.
//
// It runs dead straight, on one heading for each plot of ground, because
// steering it by the lie of the land is what curls furrows into fingerprints:
// warping a coordinate by noise is precisely how one draws a whorl.
fn comb(place: vec2<f32>, spacing: f32, plot_m: f32) -> f32 {
    let plot = floor(place / plot_m);
    let heading = hash21(plot) * 3.1415927;
    let lean = vec2<f32>(cos(heading), sin(heading));
    let across = dot(place, vec2<f32>(-lean.y, lean.x)) / max(spacing, 0.05);
    return sin(across * 6.2831853) * 0.5 + 0.5;
}

// Gradient noise, not value noise: value noise on a square lattice lays its
// own grid through everything stacked on it, and the bias shows at 45 and 90
// degrees. Each octave is turned against the last for the same reason.
fn slope_of(cell: vec2<f32>) -> vec2<f32> {
    let angle = hash21(cell) * 6.2831853;
    return vec2<f32>(cos(angle), sin(angle));
}

fn ground_noise(p: vec2<f32>) -> f32 {
    let corner = floor(p);
    let f = p - corner;
    let ease = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let a = dot(slope_of(corner), f);
    let b = dot(slope_of(corner + vec2<f32>(1.0, 0.0)), f - vec2<f32>(1.0, 0.0));
    let c = dot(slope_of(corner + vec2<f32>(0.0, 1.0)), f - vec2<f32>(0.0, 1.0));
    let d = dot(slope_of(corner + vec2<f32>(1.0, 1.0)), f - vec2<f32>(1.0, 1.0));
    return clamp(0.5 + 0.7 * mix(mix(a, b, ease.x), mix(c, d, ease.x), ease.y), 0.0, 1.0);
}

fn ground_fbm(p: vec2<f32>, octaves: i32) -> f32 {
    let turn = mat2x2<f32>(0.80, 0.60, -0.60, 0.80);
    var sum = 0.0;
    var weight = 0.0;
    var amplitude = 0.5;
    var q = p;
    for (var i = 0; i < octaves; i = i + 1) {
        sum = sum + amplitude * ground_noise(q);
        weight = weight + amplitude;
        amplitude = amplitude * 0.5;
        q = turn * q * 2.03 + vec2<f32>(11.3, 5.7);
    }
    return sum / weight;
}

// A clod is a lump of earth, which is a cell and not a hump of noise: each
// belongs to the seed nearest it, and it stands highest at its middle and
// falls to nothing where it meets its neighbours. The difference between the
// two nearest seeds gives that shape directly.
fn clods(p: vec2<f32>) -> f32 {
    let cell = floor(p);
    let f = p - cell;
    var nearest = 8.0;
    var next = 8.0;
    for (var dy = -1; dy <= 1; dy = dy + 1) {
        for (var dx = -1; dx <= 1; dx = dx + 1) {
            let step = vec2<f32>(f32(dx), f32(dy));
            let seed = cell + step;
            let at = step + vec2<f32>(hash21(seed + vec2<f32>(0.3, 0.7)), hash21(seed + vec2<f32>(5.1, 2.3)));
            let away = length(at - f);
            if (away < nearest) {
                next = nearest;
                nearest = away;
            } else if (away < next) {
                next = away;
            }
        }
    }
    // Rounded, not peaked: the raw gap between the two nearest seeds leaves a
    // ridge at every border and the ground reads as faceted glass.
    let gap = clamp(next - nearest, 0.0, 1.0);
    return gap * gap * (3.0 - 2.0 * gap);
}


// Bare ground: the swell of it, the clods lying on it in two sizes, the crumb
// between them, and the comb of the plough or the wind across the lot.
fn made_relief(place: vec2<f32>, close: f32) -> f32 {
    let clod = max(ground.grain.x, 0.05);
    let swell = ground_fbm(place / 26.0, 3);
    let coarseness = ground_fbm(place / 5.5 + vec2<f32>(31.0, 17.0), 2);
    // Far apart in size, and not the same amount of each everywhere.
    let slabs = clods(place / (clod * 5.5));
    let lumps = clods(place / (clod * 0.9) + vec2<f32>(17.0, 4.0));
    let crumb = mix(0.5, ground_fbm(place / (clod * 0.22), 2), close);
    let combed = comb(place, ground.grain.w, 128.0);
    return swell * 0.08
        + slabs * ground.grain.y * mix(0.08, 0.34, coarseness)
        + lumps * ground.grain.y * mix(0.22, 0.05, coarseness)
        + crumb * ground.grain.y * 0.05
        + combed * ground.grain.z * 0.10;
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
    let patchy = ground_fbm(place / 7.0 + vec2<f32>(3.3, 8.8), 3);
    let speck = mix(0.5, ground_fbm(place / (clod * 0.3), 2), close);
    let furrow_shade = comb(place, ground.grain.w, 128.0);
    colour = ground.tint.rgb * mix(0.74, 1.2, patchy) * mix(0.9, 1.12, speck)
        * mix(1.0, 0.94 + 0.12 * furrow_shade, ground.grain.z);

    // Where a wheel has passed the ground is pressed flat and darkened.
    let pressed = sample_wheels(tracks, wheels, place);
    let rolled = clamp(pressed.x, 0.0, 1.0);
    colour *= 1.0 - rolled * wheels.darkening;

    // The normal from the height itself: a step to each side, and the cross
    // of what that leaves. Clods lit from the side read as clods.
    let step = max(clod * 0.12, 0.02);
    let pressed_down = 1.0 - rolled * 0.75;
    let here = made_relief(place, close) * pressed_down;
    let east = made_relief(place + vec2<f32>(step, 0.0), close) * pressed_down;
    let north = made_relief(place + vec2<f32>(0.0, step), close) * pressed_down;
    // The slope of that height, taken across a step of it.
    let bumps = normalize(vec3<f32>((here - east) * 0.4, step, (here - north) * 0.4));
    let shaped = normalize(land + bumps - vec3<f32>(0.0, 1.0, 0.0));

    // Hollows between the clods keep the light out of them.
    // What stands proud catches the light; the hollows between keep it out.
    let pit = clamp(0.82 + here * 1.1, 0.7, 1.22);

    pbr_input.material.base_color = vec4<f32>(colour * pit, 1.0);
    // Sand glitters a little where its grains face the light.
    let glint = (ground_noise(place / (clod * 0.08)) - 0.5) * 0.18 * close;
    pbr_input.material.perceptual_roughness = clamp(mix(0.95, 0.78, rolled) + glint, 0.35, 1.0);
    pbr_input.world_normal = shaped;
    pbr_input.N = shaped;
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
