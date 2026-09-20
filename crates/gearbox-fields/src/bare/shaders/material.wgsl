// Bare soil, made rather than scanned, so there is no tile to find in it at any
// range. Anything finer than the pixel looking at it is left out rather than
// drawn, or the ground crawls with static.

#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
}
#import "embedded://gearbox_fields/shaders/interaction.wgsl"::{WheelMapParams, sample_wheels, wheel_scar, sample_wheel_mark, wheel_edge}
#import "embedded://gearbox_fields/shaders/surface_detail.wgsl"::{SurfaceGeometryParams, surface_geometry_normal}
#import "embedded://gearbox_fields/bare/shaders/cover.wgsl"::{pcg, rand, lattice, taken, clump_edge, clump_frame, worn, washed_into, height_blend, settled, verge_damp, inside_field, rut_of, earth_mottle, way_read, tyre_bars, DrivenTrail}

@group(#{MATERIAL_BIND_GROUP}) @binding(105) var tracks: texture_2d<u32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var<uniform> wheels: WheelMapParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var surface_heightmap: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(108) var<uniform> geometry: SurfaceGeometryParams;

struct BareGround {
    tint: vec4<f32>,
    grain: vec4<f32>,
    grass: vec4<f32>,
    extent: vec4<f32>,
    tread: vec4<f32>,
    stony: vec4<f32>,
    west: vec4<f32>,
    east: vec4<f32>,
    south: vec4<f32>,
    north: vec4<f32>,
    reach: vec4<f32>,
    way: mat4x4<f32>,
    way_more: mat4x4<f32>,
    way_shape: vec4<f32>,
    bar: vec4<f32>,
    bar_more: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(109) var<uniform> ground: BareGround;
@group(#{MATERIAL_BIND_GROUP}) @binding(110) var<uniform> driven: DrivenTrail;

// How deep a wheel presses the ground it rolls over, at its worst. Shading and
// not geometry, for the same reason the authored ways' ruts are.
const RUT_DEEP_M: f32 = 0.022;

// What a wheel left here: the tyre's bars, and the trough it pressed the
// ground into. Both come off one walk of the line it drove.
struct Driven {
    // How much of a bar's print, its relief's slope in world x and z, and the
    // shading of the bar's own edges.
    bars: vec4<f32>,
    // How deep the trough is here — nought outside it — and the slope of its
    // sides in world x and z. A wheel does not paint a stripe, it presses a
    // hollow and pushes what it displaces up into a shoulder either side, and
    // without that the mark reads as a dark band laid on flat ground however
    // well the tread on it is drawn.
    rut: vec3<f32>,
}

// The print of a tyre on the line it drove, found by walking that line rather
// than by reading a raster. Each point holds where the wheel's middle was, how
// far along the line it lies, and half the tyre's width — negative where a run
// begins, so a wheel set down somewhere new is not joined to where it was.
//
// This is what the whole tread rests on. Against a line, where a point stands
// is exact: the nearest place on it gives the distance across, and that place's
// own distance along gives the phase. Neither has a grid to step on, neither
// needs four neighbours blended, and a turn is no different from a straight —
// the line simply curves. What it replaced could do none of that.
//
// Walked here and not in the shared cover file: naga_oil cannot carry a pointer
// into a uniform across a module boundary, and declaring the binding in the
// shared file would force it on every shader that imports it — blades and
// clumps included, which have no such binding.
fn trail_print(place: vec2<f32>, footprint: f32, pressed: f32) -> Driven {
    // Only ground a wheel has actually been over pays for the walk, which is
    // what lets the line be long enough to be worth keeping.
    if (pressed <= 0.0) {
        return Driven(vec4<f32>(0.0), vec3<f32>(0.0));
    }
    let count = i32(driven.count.x);
    var best_gap = 1e30;
    var best_across = 0.0;
    var best_along = 0.0;
    var best_half = 0.0;
    var best_heading = vec2<f32>(1.0, 0.0);
    for (var i = 1; i < count; i = i + 1) {
        let to = driven.points[i];
        // A run's first point begins a line rather than continuing one.
        if (to.w < 0.0) {
            continue;
        }
        let back = driven.points[i - 1];
        let leg = to.xy - back.xy;
        let length_of = max(length(leg), 1e-4);
        let heading = leg / length_of;
        let at = clamp(dot(place - back.xy, heading), 0.0, length_of);
        let near = back.xy + heading * at;
        let gap = distance(place, near);
        if (gap < best_gap) {
            best_gap = gap;
            best_across = dot(place - near, vec2<f32>(-heading.y, heading.x));
            best_along = abs(back.z) + at;
            best_half = abs(to.w);
            best_heading = heading;
        }
    }
    if (best_gap > best_half * 1.8 || best_half <= 0.0) {
        return Driven(vec4<f32>(0.0), vec3<f32>(0.0));
    }
    // Fading by how far out of the tyre's own width the point lies, so the
    // print ends where the tyre did and not where any grid happened to fall.
    let within = 1.0 - smoothstep(0.80, 1.0, abs(best_across) / best_half);
    let bars = tyre_bars(best_along, best_across, best_heading,
        vec2<f32>(-best_heading.y, best_heading.x), driven.bar, within, footprint);

    // The trough, across the tyre: a rounded floor out to the shoulder, then
    // the spoil standing proud just outside it. How deep goes with how hard the
    // ground was worked here, so a wheel that merely passed leaves a crease and
    // one that has been over a dozen times leaves a rut.
    let axle = vec2<f32>(-best_heading.y, best_heading.x);
    let share = best_across / best_half;
    let deep = RUT_DEEP_M * clamp(pressed, 0.0, 1.0);
    let floor_of = 1.0 - smoothstep(0.0, 1.0, abs(share));
    let shoulder = (1.0 - smoothstep(0.0, 0.55, abs(abs(share) - 1.22))) * 0.42;
    let depth = -deep * floor_of + deep * shoulder;
    // The slope of that profile, taken in closed form and laid straight into
    // the normal: the trough is a hand's breadth across and the terrain carries
    // a metre to the cell, so it can never be dug into the mesh.
    let falls = deep * 1.9 * share * (1.0 - smoothstep(0.7, 1.35, abs(share)));
    return Driven(bars, vec3<f32>(depth, falls * axle.x / best_half, falls * axle.y / best_half));
}


// `fract(sin(dot(..)))` is not a hash at these coordinates: its own periods
// beat against the lattice and draw whorls across the ground.
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

// The ground washed into whatever lies across each side: a field that stops its
// own colour dead on its boundary shows a ruled line however softly the plants
// either side interleave.
fn washed(place: vec2<f32>, colour: vec3<f32>) -> vec3<f32> {
    return washed_into(place, colour, ground.extent,
        mat4x4<f32>(ground.west, ground.east, ground.south, ground.north), ground.reach);
}

// Distance to the nearest clump as a share of its own reach: under one is
// inside it. Two cells out, because an elongated clump overruns its own.
fn under_clumps(place: vec2<f32>) -> f32 {
    let cell_m = 0.85;
    let cell = floor(place / cell_m);
    var nearest = 1000.0;
    for (var dy = -2; dy <= 2; dy = dy + 1) {
        for (var dx = -2; dx <= 2; dx = dx + 1) {
            let c = cell + vec2<f32>(f32(dx), f32(dy));
            let seed = pcg(bitcast<u32>(i32(c.x)) * 2654435761u ^ bitcast<u32>(i32(c.y)) * 40503u);
            if (rand(seed, 3u) < 0.30) {
                continue;
            }
            let middle = (c + vec2<f32>(rand(seed, 1u), rand(seed, 2u))) * cell_m;
            let spread = mix(0.1, 0.3, rand(seed, 4u));
            let frame = clump_frame(c, cell_m, seed);
            let off = place - middle;
            let local = vec2<f32>(dot(off, frame.xy) / frame.z,
                dot(off, vec2<f32>(-frame.y, frame.x))) / spread;
            let about = atan2(local.y, local.x);
            nearest = min(nearest, length(local) / clump_edge(about, seed));
        }
    }
    return nearest;
}

// The comb across bare ground: a plough's furrows, or the wind's ripples.
// Steering it by the lie of the land curls the furrows into whorls.
struct Comb {
    crest: f32,
    depth: f32,
    worked: f32,
};

fn comb(place: vec2<f32>, spacing: f32) -> Comb {
    // One heading to a field, taken from the field itself. Hashing a grid of
    // plots instead turns the furrows through a new angle wherever a field
    // happens to cross a plot line, which no plough has ever done.
    let span = ground.extent.zw - ground.extent.xy;
    let worked_field = span.x > 1.0 && span.y > 1.0;
    let plot = select(floor(place / 128.0), floor(ground.extent.xy / 8.0), worked_field);
    let heading = hash21(plot) * 3.1415927;
    let lean = vec2<f32>(cos(heading), sin(heading));
    let along = dot(place, lean);
    // Three waves that do not divide into one another, so the line never comes
    // back round. A plough is steered and runs near straight; the wind is not,
    // so how far the line strays goes by how fine the comb is.
    let strays = 1.0 + 2.6 * smoothstep(0.8, 0.15, spacing);
    let wander = (sin(along * 0.11 + hash21(plot + 3.0) * 6.28) * 0.22
        + sin(along * 0.037 + 1.9) * 0.3
        + sin(along * 0.0143 + hash21(plot + 11.0) * 6.28) * 0.5) * strays;
    let across = dot(place, vec2<f32>(-lean.y, lean.x)) / max(spacing, 0.05) + wander;
    let furrow = floor(across);
    var comb: Comb;
    // A turned slice, not a wave: steep where it was laid from, falling away
    // long on the other side. Nought at both edges, or the step between two
    // furrows of different depth draws a hairline crack down the field.
    let within_furrow = across - furrow;
    comb.crest = sin(3.1415927 * pow(within_furrow, 0.68));
    let run = along / max(spacing * 18.0, 2.0);
    let stretch = floor(run);
    let into = smooth2(vec2<f32>(run - stretch)).x;
    let strength = mix(
        hash21(vec2<f32>(furrow * 3.0 + 0.5, stretch)),
        hash21(vec2<f32>(furrow * 3.0 + 0.5, stretch + 1.0)),
        into);
    comb.depth = mix(0.72, 1.28, hash21(vec2<f32>(furrow, plot.x + plot.y * 7.0)))
        * mix(0.45, 1.2, strength);
    // The headland a plough turns on. Without it the furrows run at full depth
    // into whatever is next door and stop mid-stride on the line.
    var boundary = 1.0;
    if (worked_field) {
        let to_edge = min(
            min(place.x - ground.extent.x, ground.extent.z - place.x),
            min(place.y - ground.extent.y, ground.extent.w - place.y));
            boundary = smoothstep(0.0, 5.5, to_edge + (lattice(place, 7.0) - 0.5) * 2.2);
    }
    comb.worked = boundary;
    return comb;
}

// Gradient noise: value noise on a square lattice lays its own grid through
// everything stacked on it, biased to 45 and 90 degrees. Octaves are turned
// against one another for the same reason.
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

// A clod is a cell, not a hump of noise: the gap between the two nearest seeds
// gives one that falls to nothing where it meets its neighbours.
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
    let gap = clamp(next - nearest, 0.0, 1.0);
    let lump = gap * gap * (3.0 - 2.0 * gap);
    // Scaled by the owning seed, or the cell borders make one unbroken net and
    // the ground reads as crazy paving.
    return lump * mix(0.3, 1.0, hash21(owner + vec2<f32>(7.7, 1.3)));
}


fn made_relief(place: vec2<f32>, close: f32, pixel_m: f32) -> f32 {
    let clod = max(ground.grain.x, 0.05);
    // Furrows narrower than their own pixel beat against the pixel grid and
    // the field moirés from the air.
    let combed_seen = 1.0 - smoothstep(ground.grain.w * 0.18, ground.grain.w * 0.9, pixel_m);
    let swell = ground_fbm(place / 26.0, 3);
    let coarseness = ground_fbm(place / 5.5 + vec2<f32>(31.0, 17.0), 2);
    let slabs = clods(place / (clod * 3.2));
    let lumps = clods(place / (clod * 0.9) + vec2<f32>(17.0, 4.0));
    let crumb = mix(0.5, ground_fbm(place / (clod * 0.22), 2), close);
    let grit = mix(0.5, clods(place / (clod * 0.25) + vec2<f32>(3.0, 29.0)), close);
    let drawn = comb(place, ground.grain.w);
    let combed = drawn.crest * drawn.depth * drawn.worked;
    let broken = 1.0 + (1.0 - drawn.worked) * 1.4;
    // Driving presses the ground into hollows and crushes its clods flat. This
    // is the relief the normal is taken from, not the mesh, so a rut shades but
    // never breaks a silhouette.
    let pressed_in =
        worn(place, ground.extent, ground.tread, ground.way, ground.way_more, ground.way_shape);
    let crushed = 1.0 - pressed_in * 0.55;
    // A furrow does not survive being driven over. The ridges go down first and
    // the whole comb after them, so a lane across a ploughed field cuts the
    // furrows instead of lying on top of them — which is what made it read as
    // paint: the plough ran through the road without a break in it.
    let flattened = 1.0 - pressed_in * 0.95;
    return swell * 0.08
        - pressed_in * 0.09
        // How deep a comb cuts goes with how far apart its teeth are.
        + combed * ground.grain.z * min(ground.grain.w * 0.5, 0.62) * combed_seen * flattened
        + slabs * ground.grain.y * mix(0.04, 0.12, coarseness) * broken * crushed
        + lumps * ground.grain.y * mix(0.12, 0.04, coarseness) * broken * crushed
        + crumb * ground.grain.y * 0.05 * crushed
        + grit * ground.grain.y * 0.045 * crushed;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let place = in.world_position.xz;
    // A field owns the ground up to its *strayed* edge, not to the rectangle
    // it was authored as. The mesh reaches past that edge so there is ground
    // to own; this is what gives it back, and what the neighbour keeps.
    if (inside_field(place, ground.extent, ground.reach) < 0.0) {
        discard;
    }

    let land = surface_geometry_normal(surface_heightmap, geometry, place, in.world_normal);
    let slope = vec2<f32>(land.x, land.z);

    var colour = vec3<f32>(0.0);
    let pixel_m = max(fwidth(place.x), fwidth(place.y));
    let close = 1.0 - smoothstep(0.09, 0.42, pixel_m);
    let clod = max(ground.grain.x, 0.05);
    let patchy = ground_fbm(place / 7.0 + vec2<f32>(3.3, 8.8), 3);
    let speck = mix(0.5, ground_fbm(place / (clod * 0.3), 2), close);
    let shade = comb(place, ground.grain.w);
    let stripes_seen = 1.0 - smoothstep(ground.grain.w * 0.18, ground.grain.w * 0.9, pixel_m);
    let crest = mix(0.5, shade.crest * shade.worked, stripes_seen);
    // Where the wheels have worn the ground, no grass holds it — whether that
    // is a road laid out as worn, or a way beaten across a field by driving it.
    let edge = wheel_edge(tracks, wheels, place);
    let rolled_now = wheel_scar(tracks, wheels, place) * edge;
    // One walk of the way answers all of it: how worn, how the wheels sweep it,
    // where water will stand and the print of the tyre bars.
    let read = way_read(place, ground.extent, ground.tread, ground.way, ground.way_more, ground.way_shape);
    // The print of the tyre bars, and the one thing on this ground that is
    // relief before it is colour: the chevron is a shape the sun finds, so it
    // goes into the normal and takes only a little shade of its own for the
    // ground lying in the bottom of it.
    // Only a wheel that has actually rolled here leaves a chevron; a way the
    // layout laid out is ground worn down by years of traffic. The tread comes
    // off the line that wheel drove, not out of the wheel map.
    let drove = trail_print(place, pixel_m, max(rolled_now, wheel_scar(tracks, wheels, place)));
    let print = drove.bars;
    let laid = read.x;
    let bared = max(laid, rolled_now * 0.8);
    let damp_patch = ground_fbm(place / 2.7 + vec2<f32>(-13.0, 41.0), 3);
    // Ground changes over tens of metres as well as over inches — drainage,
    // the lie of the land, where the subsoil comes nearer the surface. Without
    // it a field is one flat tone however much crumb is drawn on it.
    let country = ground_fbm(place / 34.0 + vec2<f32>(7.0, -29.0), 3);
    // A way is not simply its field with the grass off it. Driving takes the
    // fines away and brings up what was under them, so the ground it wears to
    // is a surface of its own.
    // Only a *way* comes down to the stone under it. A wheel passing over a
    // field presses its soil and takes what grows in it, and reaching for the
    // hardcore by the press alone laid a pale grey pair of tracks across a
    // ploughed field — road metal where nothing had made a road.
    let pool = vec3<f32>(read.y, read.z * smoothstep(0.46, 0.86, lattice(place, 7.5) * 0.62 + lattice(place + 29.0, 1.9) * 0.38), read.w);
    let earth = mix(ground.tint.rgb, ground.stony.rgb, settled(laid) * ground.stony.w)
        * earth_mottle(place, pixel_m) * mix(1.0, 0.58, pool.y)
        * mix(1.0, pool.z, 1.0 - smoothstep(0.05, 0.22, pixel_m));
    colour = earth * mix(0.78, 1.24, country)
        * mix(0.82, 1.12, patchy) * mix(0.74, 1.16, damp_patch) * mix(0.88, 1.14, speck)
        // The stripe the comb shades with goes out with the comb itself, or the
        // furrows keep showing as colour across a lane that has no furrows left.
        * mix(1.0, mix(0.72, 1.24, crest), ground.grain.z * (1.0 - laid * 0.95));

    var grass_share = 0.0;
    var shade_of_clumps = 1.0;
    if (ground.grass.w > 0.001) {
        let grown = taken(place);
        // On a driven surface the patches do not decide it: what is not worn
        // is green, and the ruts are what make the pattern.
        let patchy_cover = smoothstep(0.42, 0.70, grown);
        let grassy = select(patchy_cover, mix(patchy_cover, 1.0, 0.85), ground.tread.z > 0.0);
        // Near to, each clump is its own; far off they are smaller than a pixel
        // and drawing them only speckles the ground, so a wash is left instead.
        let spread_out = grassy * ground.grass.w * (1.0 - bared);
        let away = under_clumps(place);
        let took = max((1.0 - smoothstep(0.72, 1.06, away)) * spread_out * close,
            spread_out * 0.5 * (1.0 - close));
        // Nothing here casts a shadow, so the ground darkens itself round the
        // foot of a clump, or the clump looks stuck on rather than grown.
        shade_of_clumps = mix(1.0, 0.66, (1.0 - smoothstep(0.9, 1.8, away)) * spread_out);
        let tufts = ground_fbm(place / 0.38 + vec2<f32>(8.0, 14.0), 2);
        let blades = mix(0.5, ground_fbm(place / 0.09, 2), close);
        let grass_height = tufts * 0.7 + blades * 0.3;
        let thirst = 1.0 - smoothstep(0.40, 0.88, grown);
        let straw = vec3<f32>(0.115, 0.088, 0.030);
        // The tufts standing in this are mostly straw-headed, and past their
        // reach the wash is all that is left: kept as green as the grass tint
        // it starts from, the patches read as painted-on emerald from the air.
        let green = mix(ground.grass.rgb, straw, clamp(0.46 + thirst * 0.5, 0.0, 1.0))
            * mix(0.72, 1.22, tufts) * mix(0.9, 1.1, blades);
        let soil_height = made_relief(place, close, pixel_m) * 2.0;
        let met = height_blend(colour, soil_height, 1.0 - took, green, grass_height, took);
        colour = met.rgb;
        grass_share = met.a;
    }

    colour *= shade_of_clumps * mix(1.0, 0.74, verge_damp(bared)) * mix(1.0, 0.72, print.x) * (1.0 + print.w * 0.40);
    // The floor of a trough sees less of the sky than the ground beside it, and
    // the spoil on its shoulders catches what the floor misses. Read off the
    // depth itself, so it holds whatever the sun is doing — which is what makes
    // the mark read as pressed into the ground rather than painted onto it.
    colour *= clamp(1.0 + drove.rut.x * 3.2, 0.82, 1.09);

    let pressed = sample_wheels(tracks, wheels, place);
    let rolled = clamp(pressed.x * edge, 0.0, 1.0);
    colour *= 1.0 - rolled * wheels.darkening;

    // The normal from the height itself, stepped no finer than the pixel can
    // see so the relief fades with distance instead of boiling.
    let step = max(max(clod * 0.07, 0.014), pixel_m * 0.7);
    let pressed_down = 1.0 - rolled * 0.75;
    let here = made_relief(place, close, pixel_m) * pressed_down;
    let east = made_relief(place + vec2<f32>(step, 0.0), close, pixel_m) * pressed_down;
    let north = made_relief(place + vec2<f32>(0.0, step), close, pixel_m) * pressed_down;
    let bumps = normalize(vec3<f32>((here - east) * 0.5, step, (here - north) * 0.5));
    let shaped = normalize(land + bumps - vec3<f32>(0.0, 1.0, 0.0)
        + vec3<f32>(print.y + drove.rut.y, 0.0, print.z + drove.rut.z));

    let pit = clamp(0.82 + here * 1.1, 0.7, 1.22);

    pbr_input.material.base_color = vec4<f32>(washed(place, colour) * pit, 1.0);
    let glint = (ground_noise(place / (clod * 0.08)) - 0.5) * 0.18 * close;
    pbr_input.material.perceptual_roughness = clamp(mix(0.95, 0.78, rolled) + glint, 0.35, 1.0);
    // Left at the default, the sheen off the soil is worth more than the earth's
    // own colour and the ground comes out grey whatever it is tinted.
    pbr_input.material.reflectance = vec3<f32>(0.03);
    pbr_input.world_normal = shaped;
    pbr_input.N = shaped;
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
