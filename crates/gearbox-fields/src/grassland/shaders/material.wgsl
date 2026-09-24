#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
}

#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    mesh_view_bindings::{globals, lights},
}

#import "embedded://gearbox_fields/grassland/shaders/palette.wgsl"::{noise, meadow_pattern, meadow_canopy, meadow_tint, meadow_pocket_blend, grass_species, species_tint, dry_country}
#import "embedded://gearbox_fields/shaders/surface_detail.wgsl"::{surface_footprint, filtered_clumps, fiber_stamp}
#import "embedded://gearbox_fields/shaders/surface_detail.wgsl"::{SurfaceGeometryParams, surface_geometry_normal, surface_relief, surface_lighting}

@group(#{MATERIAL_BIND_GROUP}) @binding(106) var surface_heightmap: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var<uniform> geometry: SurfaceGeometryParams;

struct MeadowEdges {
    extent: vec4<f32>,
    west: vec4<f32>,
    east: vec4<f32>,
    south: vec4<f32>,
    north: vec4<f32>,
    reach: vec4<f32>,
    tread: vec4<f32>,
    way: mat4x4<f32>,
    way_more: mat4x4<f32>,
    way_shape: vec4<f32>,
    bar: vec4<f32>,
    bar_more: vec4<f32>,
    soil: vec4<f32>,
    stony: vec4<f32>,
};
@group(#{MATERIAL_BIND_GROUP}) @binding(108) var<uniform> edges: MeadowEdges;





// The same wear the bare grounds read, so a track crossing a meadow is worn by
// one rule and not by a second one that has to be kept in step with it.
#import "embedded://gearbox_fields/bare/shaders/cover.wgsl"::{worn, washed_into, height_blend, settled, verge_damp, inside_field, rut_of, earth_mottle, way_read, tyre_bars, DrivenTrail, TREAD_SHOW, UNDER_SHOW, SEPARATE_M, TREAD_DEBUG, tread_checker, lattice}

@group(#{MATERIAL_BIND_GROUP}) @binding(109) var<uniform> driven: DrivenTrail;

// How deep a wheel presses the ground it rolls over, at its worst. Shading and
// not geometry, for the same reason the authored ways' ruts are.
const RUT_DEEP_M: f32 = 0.022;

// The ground pressure a rut of `RUT_DEEP_M` belongs to: what a field tyre at
// working pressure bears, about a bar. Heavier or narrower than that and the
// wheel digs in further; lighter or wider and it hardly marks.
const RUT_AT_KPA: f32 = 100.0;

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
    // How much of the tyre's width covers this point, straight off the line:
    // one under the middle of it, nought past its shoulder, and softened by
    // no more than the pixel can see. The map cannot answer this without a
    // staircase — it is a grid, and a wheel goes where it likes.
    cover: f32,
    // Whether a line runs near enough here for its word to stand over the
    // wheel map's. Inside this the map is not consulted at all: its stamp is
    // a rectangle of whole texels that overruns the tyre, and taken together
    // with the line by whichever is the greater, that overrun survives as a
    // blocky fringe outside the clean edge the line drew.
    near: f32,
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
        return Driven(vec4<f32>(0.0), vec3<f32>(0.0), 0.0, 0.0);
    }
    let count = i32(driven.count.x);
    var best_gap = 1e30;
    var best_across = 0.0;
    var best_along = 0.0;
    var best_half = 0.0;
    var best_heading = vec2<f32>(1.0, 0.0);
    var best_hard = 0.0;
    var best_fade = 1.0;
    // The nearest line that *crosses* the nearest one, kept apart from it so
    // that where two sets of wheelings meet both are drawn — the older pressed
    // over rather than wiped out.
    var under_gap = 1e30;
    var under_across = 0.0;
    var under_along = 0.0;
    var under_half = 0.0;
    var under_heading = vec2<f32>(1.0, 0.0);
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
        let across = dot(place - near, vec2<f32>(-heading.y, heading.x));
        let along = abs(back.z) + at;
        // A different pass, told apart by how far along its own run it is.
        let crosses = abs(along - best_along) > SEPARATE_M;
        if (gap < best_gap) {
            // What was nearest drops underneath, if it runs across this one.
            if (crosses && best_gap < under_gap) {
                under_gap = best_gap;
                under_across = best_across;
                under_along = best_along;
                under_half = best_half;
                under_heading = best_heading;
            }
            best_gap = gap;
            best_across = across;
            best_along = along;
            best_half = abs(to.w);
            best_heading = heading;
            best_hard = driven.hard[i].x;
            best_fade = driven.hard[i].y;
        } else if (crosses && gap < under_gap) {
            under_gap = gap;
            under_across = across;
            under_along = along;
            under_half = abs(to.w);
            under_heading = heading;
        }
    }
    if (best_half <= 0.0 || best_gap > best_half * 3.0) {
        return Driven(vec4<f32>(0.0), vec3<f32>(0.0), 0.0, 0.0);
    }
    // Near enough to speak for this ground, even where it says the tyre did
    // not reach: that nought is an answer, and a better one than the map's.
    if (best_gap > best_half * 1.8) {
        return Driven(vec4<f32>(0.0), vec3<f32>(0.0), 0.0, 1.0);
    }
    // Fading by how far out of the tyre's own width the point lies, so the
    // print ends where the tyre did and not where any grid happened to fall.
    let within = 1.0 - smoothstep(0.80, 1.0, abs(best_across) / best_half);
    var top = tyre_bars(best_along, best_across, best_heading,
        vec2<f32>(-best_heading.y, best_heading.x), driven.bar, within, footprint);
    if (TREAD_DEBUG) {
        top = vec4<f32>(tread_checker(best_along, best_across) * within, 0.0, 0.0, 0.0);
    }
    // The line already here keeps its share of the print, and what the new
    // lugs do not cover of it still shows through. Driving over a track does
    // not sweep it away.
    var bars = top * best_fade;
    var under_rut = 0.0;
    if (under_half > 0.0 && under_gap <= under_half * 1.8) {
        let under_within = 1.0 - smoothstep(0.80, 1.0, abs(under_across) / under_half);
        var older = tyre_bars(under_along, under_across, under_heading,
            vec2<f32>(-under_heading.y, under_heading.x), driven.bar, under_within, footprint);
        if (TREAD_DEBUG) {
            older = vec4<f32>(tread_checker(under_along + 0.11, under_across + 0.11) * under_within, 0.0, 0.0, 0.0);
        }
        bars = top + older * UNDER_SHOW * (1.0 - top.x);
        under_rut = UNDER_SHOW;
    }

    // The trough, across the tyre: a rounded floor out to the shoulder, then
    // the spoil standing proud just outside it. How deep goes with how hard the
    // ground was worked here, so a wheel that merely passed leaves a crease and
    // one that has been over a dozen times leaves a rut.
    let axle = vec2<f32>(-best_heading.y, best_heading.x);
    let share = best_across / best_half;
    // How deep the wheel pressed goes with how hard it bore on the ground —
    // the load it carried over the patch it carried it on — and not with how
    // lately it came past, which is what `pressed` is and says nothing about
    // weight. A laden trailer on narrow tyres cuts in where an empty tractor
    // on flotation tyres barely marks. `RUT_AT_KPA` is what a field tyre at
    // working pressure does, so that case is unchanged and everything heavier
    // or narrower now tells itself apart from it.
    let bearing = clamp(best_hard / RUT_AT_KPA, 0.0, 2.5);
    let deep = RUT_DEEP_M * clamp(pressed, 0.0, 1.0) * bearing;
    let floor_of = 1.0 - smoothstep(0.0, 1.0, abs(share));
    let shoulder = (1.0 - smoothstep(0.0, 0.55, abs(abs(share) - 1.22))) * 0.42;
    let depth = -deep * floor_of + deep * shoulder;
    // The slope of that profile, taken in closed form and laid straight into
    // the normal: the trough is a hand's breadth across and the terrain carries
    // a metre to the cell, so it can never be dug into the mesh.
    let falls = deep * 1.9 * share * (1.0 - smoothstep(0.7, 1.35, abs(share)));
    var rut = vec3<f32>(depth, falls * axle.x / best_half, falls * axle.y / best_half);
    // The crossing line's trough is pressed in as well, so ground driven over
    // twice is dug a little deeper rather than having one of its ruts filled
    // back in.
    if (under_rut > 0.0) {
        let under_axle = vec2<f32>(-under_heading.y, under_heading.x);
        let under_share = under_across / under_half;
        let under_floor = 1.0 - smoothstep(0.0, 1.0, abs(under_share));
        let under_shoulder = (1.0 - smoothstep(0.0, 0.55, abs(abs(under_share) - 1.22))) * 0.42;
        let under_depth = -deep * under_floor + deep * under_shoulder;
        let under_falls = deep * 1.9 * under_share * (1.0 - smoothstep(0.7, 1.35, abs(under_share)));
        rut += vec3<f32>(under_depth,
            under_falls * under_axle.x / under_half,
            under_falls * under_axle.y / under_half) * under_rut;
    }
    // Soft by a pixel's width, so the edge is smooth however close the eye
    // gets and however the track lies against the world's axes.
    let soft = clamp(footprint / max(best_half, 0.01), 0.02, 0.5);
    let cover = (1.0 - smoothstep(1.0 - soft, 1.0 + soft, abs(best_across) / best_half))
        * clamp(bearing, 0.35, 1.0) * best_fade;
    return Driven(bars, rut, cover, 1.0);
}


fn meadow_worn(place: vec2<f32>) -> f32 {
    return worn(place, edges.extent, edges.tread, edges.way, edges.way_more, edges.way_shape);
}

// The sward washed into whatever lies across each side, so a meadow meets a
// track from its own side too and the two blends meet in the middle.
fn washed(place: vec2<f32>, colour: vec3<f32>) -> vec3<f32> {
    return washed_into(place, colour, edges.extent,
        mat4x4<f32>(edges.west, edges.east, edges.south, edges.north), edges.reach);
}

#import "embedded://gearbox_fields/shaders/interaction.wgsl"::{WheelMapParams, sample_wheels, wheel_scar}

@group(#{MATERIAL_BIND_GROUP}) @binding(100)
var grass_albedo: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101)
var grass_albedo_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(102)
var dirt_albedo: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(103)
var dirt_albedo_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(104)
var trample: texture_2d<u32>;
// Per-field wheel-map coordinates and response.
@group(#{MATERIAL_BIND_GROUP}) @binding(105)
var<uniform> trample_params: WheelMapParams;

// Whether anything has been over this ground at all. The map answers that and
// nothing else now: it is what decides whether walking the driven line is worth
// paying for, and a gate is never seen. Held wide on purpose — the stamp
// overruns the tyre, and a gate narrower than the line it guards would cut the
// line's own edge off.
fn trample_pressed(world_xz: vec2<f32>) -> f32 {
    return sample_wheels(trample, trample_params, world_xz).x;
}

fn luma(c: vec4<f32>) -> f32 {
    return dot(c.rgb, vec3<f32>(0.30, 0.59, 0.11));
}

fn saturate(v: f32) -> f32 {
    return clamp(v, 0.0, 1.0);
}

fn smooth2(v: vec2<f32>) -> vec2<f32> {
    return v * v * (vec2<f32>(3.0) - 2.0 * v);
}

fn hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453123);
}

// One sample per tile, rotated around its centre without translation.
fn sample_variant(
    tex: texture_2d<f32>,
    smp: sampler,
    uv: vec2<f32>,
    cell: vec2<f32>,
    seed: f32,
) -> vec4<f32> {
    let r = hash21(cell + vec2<f32>(seed, seed * 1.37));
    var p = fract(uv);
    var dx = dpdx(uv);
    var dy = dpdy(uv);
    let quarter = floor(r * 4.0);
    if (quarter == 1.0) {
        p = vec2<f32>(1.0 - p.y, p.x);
        dx = vec2<f32>(-dx.y, dx.x);
        dy = vec2<f32>(-dy.y, dy.x);
    } else if (quarter == 2.0) {
        p = vec2<f32>(1.0 - p.x, 1.0 - p.y);
        dx = -dx;
        dy = -dy;
    } else if (quarter == 3.0) {
        p = vec2<f32>(p.y, 1.0 - p.x);
        dx = vec2<f32>(dx.y, -dx.x);
        dy = vec2<f32>(dy.y, -dy.x);
    }
    return textureSampleGrad(tex, smp, fract(p), dx, dy);
}

fn scatter_sample(tex: texture_2d<f32>, smp: sampler, uv: vec2<f32>, seed: f32) -> vec4<f32> {
    let cell = floor(uv);
    let local = fract(uv);
    let f = smooth2(local);
    let c00 = sample_variant(tex, smp, uv, cell, seed);
    let c10 = sample_variant(tex, smp, uv, cell + vec2<f32>(1.0, 0.0), seed);
    let c01 = sample_variant(tex, smp, uv, cell + vec2<f32>(0.0, 1.0), seed);
    let c11 = sample_variant(tex, smp, uv, cell + vec2<f32>(1.0, 1.0), seed);
    return mix(mix(c00, c10, f.x), mix(c01, c11, f.x), f.y);
}

struct MeadowSurface {
    color: vec4<f32>,
    roughness: f32,
}

fn meadow_surface(world_xz: vec2<f32>, normal: vec3<f32>, cover: f32, near: f32) -> MeadowSurface {
    let grass = scatter_sample(grass_albedo, grass_albedo_sampler, world_xz / 2.6, 11.0);
    let grass_micro = textureSampleGrad(grass_albedo, grass_albedo_sampler, fract(world_xz / 0.8), dpdx(world_xz / 0.8), dpdy(world_xz / 0.8));
    let g = mix(grass, grass_micro, 0.3);
    let dirt = scatter_sample(dirt_albedo, dirt_albedo_sampler, world_xz / 2.2 + vec2<f32>(19.3, -7.1), 37.0);

    let pattern = meadow_pattern(world_xz);
    let edge = noise(world_xz * 1.7 + vec2<f32>(-31.0, 19.0)) - 0.5;
    let dry = pattern.x;
    let damp = pattern.y;
    let grass_detail = luma(g);
    let dirt_detail = luma(dirt);

    // Albedo luminance breaks up layer edges; it is not a displacement map.
    let breakup = edge * 0.025 + (dirt_detail - grass_detail) * 0.06;
    let soil = meadow_pocket_blend(pattern.z + dry * 0.055 + breakup);
    let steep = 1.0 - saturate((normal.y - 0.80) / 0.12);
    // Driving bares a meadow as surely as a way laid across it: the two are
    // taken together and the harder wins, as on the bare grounds. Gentler than
    // there, because one pass over turf presses the sward down into the soil
    // rather than stripping it — the blades are not culled for this, they lie
    // flattened over what shows through, which is what a fresh tyre mark is.
    // The map's own edge, and over it the coverage the driven line gives:
    // the map is a grid and stairsteps a track laid across it, the line is
    // walked and does not.
    // One drawing of a trail and one only: the line the wheel drove. The
    // wheel map is not consulted for it at all. Two drawings of the same mark
    // means a handoff, and a handoff means the trail behind a machine stops
    // being what it was and becomes something coarser, somewhere back down the
    // field. It eases out with the line that holds it and then it is gone.
    let scar = cover;
    // One walk of the way for all of it, rather than one for the wear, one for
    // the same wear again and a third for the rut.
    let read = way_read(world_xz, edges.extent, edges.tread, edges.way, edges.way_more, edges.way_shape);
    let driven = max(read.x, scar * 0.5);
    let bared = clamp(driven + breakup * 1.6, 0.0, 1.0);
    let exposed = max(max(soil, bared), smoothstep(0.15, 0.8, steep + breakup));
    let footprint = surface_footprint(world_xz);
    let texture_visibility = 1.0 - smoothstep(0.015, 0.12, footprint);
    let texture_detail = mix(1.0, clamp(0.78 + mix(grass_detail, dirt_detail, exposed * 0.45) * 1.2, 0.8, 1.2), texture_visibility);
    let tufts = filtered_clumps(world_xz, 0.55, footprint, 19.3);
    let litter = filtered_clumps(world_xz, 1.6, footprint, 61.7);
    let fine = fiber_stamp(world_xz, 0.10, 0.045, footprint, 11.7);
    let blades = fiber_stamp(world_xz, 0.18, 0.055, footprint, 39.1);
    let mat = fiber_stamp(world_xz, 0.27, 0.032, footprint, 83.6);
    var ground = meadow_canopy(pattern) * texture_detail
        * mix(0.72, 1.20, tufts) * mix(0.90, 1.08, litter);
    let blade_color = mix(vec3<f32>(0.16, 0.27, 0.066), vec3<f32>(0.21, 0.24, 0.074), dry * 0.5)
        * meadow_tint(pattern) * dry_country(world_xz);
    ground = mix(ground, blade_color,
        clamp(fine * 0.55 + blades * 0.50 + mat * 0.30, 0.0, 0.8) * (1.0 - bared));
    ground *= species_tint(grass_species(world_xz)) * dry_country(world_xz);
    // The earth under the turf, lit by the dirt map so the way is not a flat
    // band of colour laid over the field.
    let pool = vec3<f32>(read.y, read.z * smoothstep(0.46, 0.86, lattice(world_xz, 7.5) * 0.62 + lattice(world_xz + 29.0, 1.9) * 0.38), read.w);
    let earth = mix(edges.soil.rgb, edges.stony.rgb, settled(read.x)) * (0.74 + dirt_detail * 0.86)
        * earth_mottle(world_xz, footprint) * mix(1.0, 0.58, pool.y)
        * mix(1.0, pool.z, 1.0 - smoothstep(0.02, 0.10, footprint));
    // Not a fade between the two: each brings its own relief and the taller
    // takes the pixel, so the earth comes up first through the hollows of the
    // sward and the last of the grass holds on the high ground. A plain mix
    // here is what made a road look painted on rather than worn through.
    ground = height_blend(ground, grass_detail, 1.0 - bared, earth, dirt_detail, bared).rgb;
    ground *= mix(1.0, 0.74, verge_damp(driven));
    let pressed = cover;
    return MeadowSurface(
        vec4<f32>(ground * (1.0 - trample_params.darkening * 1.2 * pressed), 1.0),
        mix(mix(0.96, 1.0, dry), 0.94, damp * exposed),
    );
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    if (inside_field(in.world_position.xz, edges.extent, edges.reach) < 0.0) {
        discard;
    }
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let normal = surface_geometry_normal(surface_heightmap, geometry, in.world_position.xz, in.world_normal);
    // The print of the tyre bars, tilting the normal rather than tinting the
    // ground: a chevron is a shape, and a shape wants the sun to find it.
    // Only a wheel that has rolled here leaves one. A way the layout laid out
    // is ground worn down by years of traffic, not one tyre's chevron held in it.
    let footprint_here = surface_footprint(in.world_position.xz);
    // The tread comes off the line the wheel drove, not out of the wheel map:
    // a lug is finer than a texel, so the raster could never hold it steady.
    let drove = trail_print(in.world_position.xz, footprint_here,
        max(wheel_scar(trample, trample_params, in.world_position.xz),
            trample_pressed(in.world_position.xz)));
    // The tread is the line's alone. The wheel map can hold that a wheel was
    // here and how hard, but it cannot hold a lug: it is a grid of an eighth
    // of a metre and a lug is a fifth of one. Drawn from it anyway, the trail
    // behind a machine did not fade as the line let go of it — it *changed*,
    // from the walked drawing to a coarse one, somewhere back down the field.
    // Now the line eases its own tread out along its oldest stretch and there
    // is nothing waiting to take it over.
    let fresh = drove.bars;
    let older = vec4<f32>(0.0);
    let print = fresh * TREAD_SHOW;
    // The same composite, undimmed, for the debug view: black is ground no
    // wheel has touched, and every pass is its own checkerboard over it.
    // Coverage, not the tread: how much of the tyre's width is on this point.
    // Black is ground no wheel has touched. This is the thing to look at for
    // borders, seams and stairsteps; the lugs fade with distance by design and
    // hide all of it.
    let shown = clamp(drove.cover, 0.0, 1.0);
    pbr_input.N = normalize(
        surface_relief(in.world_position.xz, normal, surface_footprint(in.world_position.xz))
        + vec3<f32>(print.y, 0.0, print.z));
    pbr_input.world_normal = normal;
    let surface = meadow_surface(in.world_position.xz, normal, drove.cover, drove.near);
    // A low sun is caught by the blades before it reaches the soil between
    // them, so the ground loses light faster than the grass standing on it.
    var sun_height = 1.0;
    if (lights.n_directional_lights > 0u) {
        sun_height = max(lights.directional_lights[0].direction_to_light.y, 0.0);
    }
    let shaded = mix(0.72, 1.0, smoothstep(0.1, 0.7, sun_height));
    pbr_input.material.base_color = alpha_discard(pbr_input.material,
        vec4<f32>(washed(in.world_position.xz, surface.color.rgb) * shaded * mix(1.0, 0.40, print.x) * (1.0 + print.w * 0.85),
            surface.color.a));
    pbr_input.material.perceptual_roughness = surface.roughness;
    pbr_input.material.metallic = 0.0;
    pbr_input.material.reflectance = vec3<f32>(0.04);
    pbr_input.specular_occlusion = 0.0;

    var out: FragmentOutput;
    out.color = surface_lighting(pbr_input,
        0.65 * (1.0 - drove.cover));
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    if (TREAD_DEBUG) {
        out.color = vec4<f32>(vec3<f32>(shown), 1.0);
    }
    return out;
}
