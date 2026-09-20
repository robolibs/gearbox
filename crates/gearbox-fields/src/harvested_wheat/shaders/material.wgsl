#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
}

#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}

#import "embedded://gearbox_fields/shaders/interaction.wgsl"::{WheelMapParams, sample_wheels, wheel_scar}
#import "embedded://gearbox_fields/harvested_wheat/shaders/patches.wgsl"::{regrowth, row_drift, row_wobble, plant_jog}
#import "embedded://gearbox_fields/shaders/surface_detail.wgsl"::{surface_footprint, filtered_clumps, fiber_stamp}
#import "embedded://gearbox_fields/shaders/surface_detail.wgsl"::{SurfaceGeometryParams, surface_geometry_normal, surface_relief, surface_lighting}

@group(#{MATERIAL_BIND_GROUP}) @binding(111) var surface_heightmap: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(112) var<uniform> geometry: SurfaceGeometryParams;

@group(#{MATERIAL_BIND_GROUP}) @binding(109) var tracks: texture_2d<u32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(110) var<uniform> wheels: WheelMapParams;

struct WornStubble {
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
    soil: vec4<f32>,
    stony: vec4<f32>,
};
@group(#{MATERIAL_BIND_GROUP}) @binding(113) var<uniform> worn_ground: WornStubble;

// The same wear the bare grounds and the meadow read, so a road crossing from
// one field to the next does not change shape or colour on the boundary.
#import "embedded://gearbox_fields/bare/shaders/cover.wgsl"::{worn, washed_into, height_blend, settled, verge_damp, inside_field, rut_of, earth_mottle}

fn stubble_worn(place: vec2<f32>) -> f32 {
    return worn(place, worn_ground.extent, worn_ground.tread,
        worn_ground.way, worn_ground.way_more, worn_ground.way_shape);
}

// A stubble field meets its neighbours from its own side too. Without this the
// blend was one-sided: the meadow next door washed towards the stubble's colour
// and the stubble washed towards nothing, so the join fell on a line.
fn washed(place: vec2<f32>, colour: vec3<f32>) -> vec3<f32> {
    return washed_into(place, colour, worn_ground.extent,
        mat4x4<f32>(worn_ground.west, worn_ground.east, worn_ground.south, worn_ground.north),
        worn_ground.reach);
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100)
var terrain_albedo: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101)
var terrain_albedo_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(102)
var terrain_height: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104)
var terrain_detail_albedo: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106)
var terrain_detail_height: texture_2d<f32>;
// rgb multiplies the field colour, a scales the cut-hay rows.
@group(#{MATERIAL_BIND_GROUP}) @binding(108)
var<uniform> terrain_tint: vec4<f32>;

fn saturate(v: f32) -> f32 {
    return clamp(v, 0.0, 1.0);
}

fn smooth01(v: f32) -> f32 {
    let t = saturate(v);
    return t * t * (3.0 - 2.0 * t);
}

fn smooth2(v: vec2<f32>) -> vec2<f32> {
    return v * v * (vec2<f32>(3.0) - 2.0 * v);
}

fn hash21(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453123);
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = smooth2(fract(p));
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var freq = 1.0;
    var norm = 0.0;
    for (var i = 0; i < 5; i = i + 1) {
        sum = sum + amp * noise(p * freq);
        norm = norm + amp;
        amp = amp * 0.5;
        freq = freq * 2.03;
    }
    return sum / norm;
}

fn sample_variant(
    tex: texture_2d<f32>,
    smp: sampler,
    uv: vec2<f32>,
    cell: vec2<f32>,
    seed: f32,
) -> vec4<f32> {
    let r = hash21(cell + vec2<f32>(seed, seed * 1.37));
    let local = fract(uv);
    var p = local;
    var dx = dpdx(uv);
    var dy = dpdy(uv);
    if (r < 0.25) {
        p = local;
    } else if (r < 0.5) {
        p = vec2<f32>(local.y, 1.0 - local.x);
        dx = vec2<f32>(dx.y, -dx.x);
        dy = vec2<f32>(dy.y, -dy.x);
    } else if (r < 0.75) {
        p = vec2<f32>(1.0 - local.x, 1.0 - local.y);
        dx = -dx;
        dy = -dy;
    } else {
        p = vec2<f32>(1.0 - local.y, local.x);
        dx = vec2<f32>(-dx.y, dx.x);
        dy = vec2<f32>(-dy.y, dy.x);
    }
    if (hash21(cell + vec2<f32>(seed * 2.1, 19.7)) > 0.5) {
        p.x = 1.0 - p.x;
        dx.x = -dx.x;
        dy.x = -dy.x;
    }
    let offset = vec2<f32>(
        hash21(cell + vec2<f32>(41.3 + seed, 7.1)),
        hash21(cell + vec2<f32>(13.9, 67.7 + seed))
    );
    return textureSampleGrad(tex, smp, fract(p + offset), dx, dy);
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

const HARVEST_PASS_WIDTH_M: f32 = 4.0;
const HARVEST_RESIDUE_WIDTH_M: f32 = 1.0;
const HARVEST_EDGE_BLEND_M: f32 = 0.4;

fn cut_hay_mask(world_xz: vec2<f32>) -> f32 {
    let spacing = HARVEST_PASS_WIDTH_M;
    let waviness = sin(world_xz.x * 0.030 + 3.0 * fbm(world_xz * 0.010)) * 1.4;
    let row = fract((world_xz.y + waviness + 1000.0) / spacing) * spacing;
    let dist = abs(row - spacing * 0.5);
    let half_width = HARVEST_RESIDUE_WIDTH_M * 0.5;
    let half_blend = HARVEST_EDGE_BLEND_M * 0.5;
    let core = 1.0 - smoothstep(half_width - half_blend, half_width + half_blend, dist);
    let clumps = fbm(world_xz * vec2<f32>(0.055, 0.035) + vec2<f32>(17.0, -23.0));
    let broken = smooth01((clumps - 0.22) / 0.78);
    let flecks = pow(fbm(world_xz * vec2<f32>(0.35, 0.22) + vec2<f32>(-8.0, 14.0)), 5.0) * 0.28;
    return saturate(core * (0.45 + 0.55 * broken) + flecks * sqrt(core));
}

fn terrain_color(world_xz: vec2<f32>) -> vec4<f32> {
    // Layer ground textures at coarse and fine world-space scales.
    let base = scatter_sample(terrain_albedo, terrain_albedo_sampler, world_xz / 3.6, 11.0);
    let base_h = scatter_sample(terrain_height, terrain_albedo_sampler, world_xz / 3.6 + vec2<f32>(3.1, -1.7), 23.0).r;
    let base_micro = scatter_sample(terrain_albedo, terrain_albedo_sampler, world_xz / 1.25 + vec2<f32>(61.0, -37.0), 59.0);
    let base_micro_h = scatter_sample(terrain_height, terrain_albedo_sampler, world_xz / 1.25 + vec2<f32>(7.7, -5.3), 61.0).r;
    let detail = scatter_sample(terrain_detail_albedo, terrain_albedo_sampler, world_xz / 2.2 + vec2<f32>(19.3, -7.1), 37.0);
    let detail_h = scatter_sample(terrain_detail_height, terrain_albedo_sampler, world_xz / 2.2 + vec2<f32>(-2.9, 4.7), 41.0).r;
    let detail_micro = scatter_sample(terrain_detail_albedo, terrain_albedo_sampler, world_xz / 0.85 + vec2<f32>(-43.0, 91.0), 83.0);
    let detail_micro_h = scatter_sample(terrain_detail_height, terrain_albedo_sampler, world_xz / 0.85 + vec2<f32>(12.0, 31.0), 89.0).r;

    var c = mix(base, base_micro, 0.22);
    let macro_n = fbm(world_xz * 0.0018 + vec2<f32>(5.0, -11.0));
    let straw_n = fbm(world_xz * 0.0045 + vec2<f32>(-31.0, 19.0));
    let luma = saturate(dot(c.rgb, vec3<f32>(0.30, 0.59, 0.11)));
    let cut_wheat = vec4<f32>(
        0.68 + luma * 0.24,
        0.55 + luma * 0.22,
        0.29 + luma * 0.16,
        1.0
    );
    c = mix(c, cut_wheat, 0.72);
    let shade = 0.90 + macro_n * 0.18 + straw_n * 0.04;
    c = vec4<f32>(
        clamp(c.rgb * vec3<f32>(1.06, 1.00, 0.82) * shade, vec3<f32>(0.0), vec3<f32>(1.0)),
        c.a
    );

    // Three independent patch scales: broad field color zones, middle
    // dirt/stubble patches, and very fine chopped-straw/soil speckle.
    let broad_noise = fbm(world_xz * 0.0018 + vec2<f32>(71.0, -113.0));
    let mid_noise = fbm(world_xz * 0.010 + vec2<f32>(-31.0, 57.0));
    let fine_noise = fbm(world_xz * 0.18 + vec2<f32>(129.0, -203.0));
    // Continuous high-frequency grain.
    let fine_grain = fbm(world_xz * 1.35 + vec2<f32>(401.0, -277.0));
    let broad_mask = smooth01((broad_noise - 0.16) / 0.62);
    let mid_mask = smooth01((mid_noise - 0.12) / 0.52);
    let fine_mask = saturate(
        smooth01((fine_noise - 0.18) / 0.36) * 0.44
        + smooth01((fine_grain - 0.46) / 0.48) * 0.30
    );
    let patch_mask = saturate(broad_mask * 0.48 + mid_mask * 0.70);
    let detail_mix = mix(detail, detail_micro, 0.35);
    let detail_luma = saturate(dot(detail_mix.rgb, vec3<f32>(0.30, 0.59, 0.11)));
    let detail_height_mix = saturate(detail_h * 0.68 + detail_micro_h * 0.32);
    let broad_field = vec4<f32>(
        0.48 + detail_luma * 0.30 + detail_height_mix * 0.10,
        0.37 + detail_luma * 0.25 + detail_height_mix * 0.06,
        0.16 + detail_luma * 0.14 + detail_height_mix * 0.03,
        1.0
    );
    let mid_brown = vec4<f32>(
        0.25 + detail_luma * 0.46 + detail_height_mix * 0.22,
        0.18 + detail_luma * 0.33 + detail_height_mix * 0.12,
        0.07 + detail_luma * 0.19 + detail_height_mix * 0.04,
        1.0
    );
    let height_shade = 0.66 + (base_h * 0.25 + base_micro_h * 0.20 + detail_height_mix * 0.55) * 0.62;
    c = mix(c, broad_field, broad_mask * 0.10);
    c = mix(c, mid_brown, mid_mask * 0.16);
    c = vec4<f32>(
        clamp(c.rgb * vec3<f32>(height_shade, height_shade, height_shade * 0.90), vec3<f32>(0.0), vec3<f32>(1.0)),
        c.a
    );

    let hard_noise = fbm(world_xz * vec2<f32>(0.026, 0.021) + vec2<f32>(-137.0, 53.0));
    let hard_mask = saturate(
        smooth01((hard_noise - 0.22) / 0.44) * (0.34 + detail_height_mix * 0.48)
        + fine_mask * 0.46
    );
    let hard_detail = vec4<f32>(
        0.18 + detail_luma * 0.60 + detail_height_mix * 0.16,
        0.12 + detail_luma * 0.42 + detail_height_mix * 0.09,
        0.045 + detail_luma * 0.22 + detail_height_mix * 0.03,
        1.0
    );
    c = mix(c, hard_detail, hard_mask * 0.14);

    let fine_stubble = vec4<f32>(
        0.40 + detail_luma * 0.38,
        0.27 + detail_luma * 0.28,
        0.085 + detail_luma * 0.14,
        1.0
    );
    let fine_bright_straw = vec4<f32>(0.92, 0.74, 0.32, 1.0);
    let fine_dark_stubble = vec4<f32>(0.13, 0.085, 0.035, 1.0);
    c = mix(c, fine_stubble, fine_mask * 0.26);
    c = mix(c, fine_bright_straw, fine_mask * smooth01((fine_grain - 0.70) / 0.20) * 0.26);
    c = mix(c, fine_dark_stubble, fine_mask * smooth01((0.24 - fine_grain) / 0.24) * 0.32);

    // Modulate fine-pattern contrast.
    let fine_contrast = 0.84 + fine_mask * (0.18 + fine_grain * 0.14);
    c = vec4<f32>(
        clamp(c.rgb * vec3<f32>(fine_contrast * 1.04, fine_contrast, fine_contrast * 0.90), vec3<f32>(0.0), vec3<f32>(1.0)),
        c.a
    );

    // Final close-up stubble/grain pass: visible only as small-scale
    // contrast, not as a broad color patch.
    let micro_luma = saturate(dot(base_micro.rgb * 0.45 + detail_micro.rgb * 0.55, vec3<f32>(0.30, 0.59, 0.11)));
    let micro_shade = 0.82 + micro_luma * 0.24 + (base_micro_h * 0.4 + detail_micro_h * 0.6) * 0.16;
    c = vec4<f32>(
        clamp(c.rgb * vec3<f32>(micro_shade * 1.03, micro_shade, micro_shade * 0.92), vec3<f32>(0.0), vec3<f32>(1.0)),
        c.a
    );
    let footprint = surface_footprint(world_xz);
    let clumps = filtered_clumps(world_xz, 0.65, footprint, 71.3);
    let litter = filtered_clumps(world_xz, 1.9, footprint, 23.7);
    let chopped = fiber_stamp(world_xz, 0.14, 0.028, footprint, 31.4);
    let straw = fiber_stamp(world_xz, 0.28, 0.035, footprint, 57.8);
    let mat = fiber_stamp(world_xz, 0.32, 0.018, footprint, 91.2);
    let straw_color = mix(vec3<f32>(0.43, 0.28, 0.095), vec3<f32>(0.67, 0.47, 0.20), litter);
    let broken_soil = c.rgb * mix(0.72, 1.12, clumps);
    c = vec4<f32>(mix(broken_soil, straw_color, clamp(chopped * 0.75 + straw * 0.65 + mat * 0.40, 0.0, 0.9)), c.a);
    // Drill rows of cut stubs, a pale top per plant and soil between; rows
    // settle to their average once one spans only a few pixels.
    let row_y = world_xz.y - row_drift(world_xz);
    let row_index = floor(row_y / 0.125);
    let plant_cell = floor(world_xz.x / 0.05);
    let row_d = abs(row_y - (row_index + 0.5) * 0.125 - row_wobble(row_index, world_xz.x)
        - plant_jog(row_index, plant_cell));
    let plant_x = (plant_cell + 0.5 + (hash21(vec2<f32>(plant_cell, row_index)) - 0.5) * 0.6) * 0.05;
    let gap = step(0.12, hash21(vec2<f32>(plant_cell + 0.5, row_index * 1.7)));
    let stubs = (1.0 - smoothstep(0.012, 0.03 + footprint, row_d))
        * mix(0.55, 1.0, noise(vec2<f32>(world_xz.x * 1.5, row_index * 3.1)));
    let dots = stubs * gap * (1.0 - smoothstep(0.008, 0.016 + footprint, abs(world_xz.x - plant_x)));
    let resolved_rows = 1.0 - smoothstep(0.03, 0.08, footprint);
    let rows = mix(0.35, stubs, resolved_rows);
    let tops = mix(0.12, dots, resolved_rows);
    c = vec4<f32>(mix(c.rgb * mix(0.85, 1.0, rows), vec3<f32>(0.50, 0.38, 0.17), rows * 0.45), 1.0);
    c = vec4<f32>(mix(c.rgb, vec3<f32>(0.78, 0.64, 0.34), tops * 0.5), 1.0);
    // Alternate combine passes lean their stubble opposite ways.
    let cut_pass = floor((world_xz.y + 2.0 * sin(world_xz.x * 0.03)) / 4.0);
    c = vec4<f32>(c.rgb * select(0.94, 1.05, fract(cut_pass * 0.5) < 0.25), 1.0);
    // Regrowth patches: olive-green cover over the straw.
    let green_luma = saturate(dot(c.rgb, vec3<f32>(0.30, 0.59, 0.11)));
    let regrowth_color = vec3<f32>(0.20, 0.29, 0.075) * (0.75 + green_luma * 0.6) * mix(0.9, 1.1, litter);
    c = vec4<f32>(mix(c.rgb, regrowth_color, regrowth(world_xz) * 0.6), c.a);
    let residue = cut_hay_mask(world_xz) * terrain_tint.a;
    let residue_tint = mix(vec3<f32>(1.0), vec3<f32>(0.94, 0.925, 0.91), residue);
    return vec4<f32>(clamp(c.rgb * residue_tint * terrain_tint.rgb, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    if (inside_field(in.world_position.xz, worn_ground.extent, worn_ground.reach) < 0.0) {
        discard;
    }
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let normal = surface_geometry_normal(surface_heightmap, geometry, in.world_position.xz, in.world_normal);
    pbr_input.N = surface_relief(in.world_position.xz, normal, surface_footprint(in.world_position.xz));
    pbr_input.world_normal = normal;
    var color = terrain_color(in.world_position.xz);
    let pressed = sample_wheels(tracks, wheels, in.world_position.xz).x;
    color = vec4<f32>(color.rgb * (1.0 - wheels.darkening * pressed), 1.0);
    // A way worn across the stubble: the rows go and the earth under them shows.
    let bared = max(
        stubble_worn(in.world_position.xz),
        wheel_scar(tracks, wheels, in.world_position.xz) * 0.6,
    );
    let grit = fbm(in.world_position.xz * 1.6);
    let pool = rut_of(in.world_position.xz, worn_ground.extent, worn_ground.tread, worn_ground.way, worn_ground.way_more, worn_ground.way_shape);
    let earth = mix(worn_ground.soil.rgb, worn_ground.stony.rgb, settled(stubble_worn(in.world_position.xz))) * (0.8 + grit * 0.5)
        * earth_mottle(in.world_position.xz) * mix(1.0, 0.58, pool.y)
        * mix(1.0, pool.z, 1.0 - smoothstep(0.02, 0.10, surface_footprint(in.world_position.xz)));
    // The taller of the two takes the pixel rather than the two being faded
    // together: the earth rises through the stubble's own hollows instead of
    // being washed over it, which is the difference between worn and painted.
    let met = height_blend(color.rgb, saturate(dot(color.rgb, vec3<f32>(0.3, 0.59, 0.11))),
        1.0 - bared, earth, grit, bared);
    color = vec4<f32>(washed(in.world_position.xz, met.rgb * mix(1.0, 0.74, verge_damp(bared))), 1.0);
#ifdef VERTEX_COLORS
    color = color * in.color;
#endif
    pbr_input.material.base_color = alpha_discard(pbr_input.material, color);
    pbr_input.material.perceptual_roughness = 0.98;
    pbr_input.material.reflectance = vec3<f32>(0.04);
    pbr_input.specular_occlusion = 0.0;
    pbr_input.material.metallic = 0.0;

    var out: FragmentOutput;
    out.color = surface_lighting(pbr_input, 0.45 * (1.0 - pressed));
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
