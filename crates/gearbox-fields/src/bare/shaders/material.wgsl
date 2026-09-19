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
    // The grass that grows through it, and how much of the ground it holds.
    grass: vec4<f32>,
};

// Two surfaces met by their own heights, not faded into one another: the
// taller of the two wins a pixel outright, and only within a shallow band of
// each other do they mix. It is what puts soil in the gaps between tufts
// instead of a grey halo round every patch.
fn height_blend(a: vec3<f32>, a_height: f32, a_share: f32,
    b: vec3<f32>, b_height: f32, b_share: f32) -> vec4<f32> {
    let band = 0.2;
    let tallest = max(a_height + a_share, b_height + b_share) - band;
    let weight_a = max(a_height + a_share - tallest, 0.0);
    let weight_b = max(b_height + b_share - tallest, 0.0);
    let total = max(weight_a + weight_b, 0.0001);
    return vec4<f32>((a * weight_a + b * weight_b) / total, weight_b / total);
}

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
// One heading to a plot, dead straight, because steering it by the lie of the
// land curls furrows into whorls. But no two furrows are the same depth, and
// none of them runs perfectly true along its length.
struct Comb {
    crest: f32,
    // How deep this furrow is cut, against its neighbours.
    depth: f32,
    // Nought on the headland where the plough turned, one out in the field.
    worked: f32,
};

fn comb(place: vec2<f32>, spacing: f32, plot_m: f32) -> Comb {
    let plot = floor(place / plot_m);
    let heading = hash21(plot) * 3.1415927;
    let lean = vec2<f32>(cos(heading), sin(heading));
    let along = dot(place, lean);
    // The line of it wanders a little, as a tractor does — and a wind ripple
    // wanders more. Three waves that do not divide into one another, so the
    // wander never comes back round to where it started.
    let wander = sin(along * 0.11 + hash21(plot + 3.0) * 6.28) * 0.22
        + sin(along * 0.037 + 1.9) * 0.3
        + sin(along * 0.0143 + hash21(plot + 11.0) * 6.28) * 0.5;
    let across = dot(place, vec2<f32>(-lean.y, lean.x)) / max(spacing, 0.05) + wander;
    let furrow = floor(across);
    // The headland: a turning strip round the edge of the plot, worked over
    // rather than drawn through.
    let within = abs(fract(place / plot_m) - vec2<f32>(0.5)) * 2.0;
    let edge = max(within.x, within.y);
    var comb: Comb;
    comb.crest = sin(across * 6.2831853) * 0.5 + 0.5;
    // Not the same all the way along: a furrow shallows and deepens over its
    // length, and a ripple dies out altogether and picks up again further on.
    let run = along / max(spacing * 18.0, 2.0);
    let stretch = floor(run);
    let into = smooth2(vec2<f32>(run - stretch)).x;
    let strength = mix(
        hash21(vec2<f32>(furrow * 3.0 + 0.5, stretch)),
        hash21(vec2<f32>(furrow * 3.0 + 0.5, stretch + 1.0)),
        into);
    comb.depth = mix(0.72, 1.28, hash21(vec2<f32>(furrow, plot.x + plot.y * 7.0)))
        * mix(0.45, 1.2, strength);
    comb.worked = 1.0 - smoothstep(0.86, 0.99, edge);
    return comb;
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
    var owner = cell;
    for (var dy = -1; dy <= 1; dy = dy + 1) {
        for (var dx = -1; dx <= 1; dx = dx + 1) {
            let step = vec2<f32>(f32(dx), f32(dy));
            let seed = cell + step;
            let at = step + vec2<f32>(hash21(seed + vec2<f32>(0.3, 0.7)), hash21(seed + vec2<f32>(5.1, 2.3)));
            let away = length(at - f);
            if (away < nearest) {
                next = nearest;
                nearest = away;
                owner = seed;
            } else if (away < next) {
                next = away;
            }
        }
    }
    // Rounded, not peaked: the raw gap between the two nearest seeds leaves a
    // ridge at every border and the ground reads as faceted glass.
    let gap = clamp(next - nearest, 0.0, 1.0);
    let lump = gap * gap * (3.0 - 2.0 * gap);
    // The borders of the cells make one unbroken net across the ground, which
    // reads as crazy paving. Softening each cell by its own roll breaks the
    // net without losing the lumps.
    return lump * mix(0.3, 1.0, hash21(owner + vec2<f32>(7.7, 1.3)));
}


// Bare ground: the swell of it, the clods lying on it in two sizes, the crumb
// between them, and the comb of the plough or the wind across the lot.
fn made_relief(place: vec2<f32>, close: f32) -> f32 {
    let clod = max(ground.grain.x, 0.05);
    let swell = ground_fbm(place / 26.0, 3);
    let coarseness = ground_fbm(place / 5.5 + vec2<f32>(31.0, 17.0), 2);
    // Far apart in size, and not the same amount of each everywhere.
    let slabs = clods(place / (clod * 3.2));
    let lumps = clods(place / (clod * 0.9) + vec2<f32>(17.0, 4.0));
    let crumb = mix(0.5, ground_fbm(place / (clod * 0.22), 2), close);
    // Under the clods, the grit: too small to make a shape of its own, but it
    // is the whole of what the ground looks like from a step away.
    let grit = mix(0.5, clods(place / (clod * 0.25) + vec2<f32>(3.0, 29.0)), close);
    let drawn = comb(place, ground.grain.w, 128.0);
    let combed = drawn.crest * drawn.depth * drawn.worked;
    // Where the plough turned, it left broken ground instead of furrows.
    let broken = 1.0 + (1.0 - drawn.worked) * 1.4;
    return swell * 0.08
        // The furrows are the ground's shape; the clods only lie on them. How
        // deep a comb cuts goes with how far apart its teeth are, so a plough's
        // furrow is a trench and the wind's ripple is a wrinkle.
        + combed * ground.grain.z * min(ground.grain.w * 0.5, 0.62)
        + slabs * ground.grain.y * mix(0.04, 0.12, coarseness) * broken
        + lumps * ground.grain.y * mix(0.12, 0.04, coarseness) * broken
        + crumb * ground.grain.y * 0.05
        + grit * ground.grain.y * 0.045;
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
    // A crest dries pale; the trough beside it stays damp and dark.
    let shade = comb(place, ground.grain.w, 128.0);
    let crest = shade.crest * shade.worked;
    let damp_patch = ground_fbm(place / 2.7 + vec2<f32>(-13.0, 41.0), 3);
    colour = ground.tint.rgb * mix(0.82, 1.12, patchy) * mix(0.74, 1.16, damp_patch) * mix(0.88, 1.14, speck)
        * mix(1.0, mix(0.72, 1.24, crest), ground.grain.z);

    // Grass takes the ground in patches, and holds it where it is tallest.
    var grass_share = 0.0;
    if (ground.grass.w > 0.001) {
        let where_it_grows = ground_fbm(place / 9.0 + vec2<f32>(51.0, -23.0), 3);
        let tufts = ground_fbm(place / 0.38 + vec2<f32>(8.0, 14.0), 2);
        let blades = mix(0.5, ground_fbm(place / 0.09, 2), close);
        let grass_height = tufts * 0.7 + blades * 0.3;
        // The noise sits about its middle, so the level it must pass to count as
        // grass is set from the share wanted, not from the share itself.
        let level = mix(0.74, 0.26, ground.grass.w);
        let took = smoothstep(level - 0.07, level + 0.07, where_it_grows);
        let green = ground.grass.rgb * mix(0.72, 1.22, tufts) * mix(0.9, 1.1, blades);
        let soil_height = made_relief(place, close) * 2.0;
        let met = height_blend(colour, soil_height, 1.0 - took, green, grass_height, took);
        colour = met.rgb;
        grass_share = met.a;
    }

    // Where a wheel has passed the ground is pressed flat and darkened.
    let pressed = sample_wheels(tracks, wheels, place);
    let rolled = clamp(pressed.x, 0.0, 1.0);
    colour *= 1.0 - rolled * wheels.darkening;

    // The normal from the height itself: a step to each side, and the cross
    // of what that leaves. Clods lit from the side read as clods.
    // The step is taken no finer than the pixel can see, so the relief fades
    // out with distance instead of boiling.
    let step = max(max(clod * 0.07, 0.014), pixel_m * 0.7);
    let pressed_down = 1.0 - rolled * 0.75;
    let here = made_relief(place, close) * pressed_down;
    let east = made_relief(place + vec2<f32>(step, 0.0), close) * pressed_down;
    let north = made_relief(place + vec2<f32>(0.0, step), close) * pressed_down;
    // The slope of that height, taken across a step of it.
    let bumps = normalize(vec3<f32>((here - east) * 0.5, step, (here - north) * 0.5));
    let shaped = normalize(land + bumps - vec3<f32>(0.0, 1.0, 0.0));

    // Hollows between the clods keep the light out of them.
    // What stands proud catches the light; the hollows between keep it out.
    let pit = clamp(0.82 + here * 1.1, 0.7, 1.22);

    pbr_input.material.base_color = vec4<f32>(colour * pit, 1.0);
    // Sand glitters a little where its grains face the light.
    let glint = (ground_noise(place / (clod * 0.08)) - 0.5) * 0.18 * close;
    pbr_input.material.perceptual_roughness = clamp(mix(0.95, 0.78, rolled) + glint, 0.35, 1.0);
    // Soil is not a dielectric with a polished surface. Left at the default,
    // the sheen off it is worth more than the earth's own colour and the
    // ground comes out grey whatever it is tinted.
    pbr_input.material.reflectance = vec3<f32>(0.03);
    pbr_input.world_normal = shaped;
    pbr_input.N = shaped;
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
