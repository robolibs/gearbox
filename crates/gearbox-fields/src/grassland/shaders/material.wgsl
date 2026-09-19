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

#import "embedded://gearbox_fields/shaders/interaction.wgsl"::{WheelMapParams, sample_wheels}

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

fn meadow_surface(world_xz: vec2<f32>, normal: vec3<f32>) -> MeadowSurface {
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
    let exposed = max(soil, smoothstep(0.15, 0.8, steep + breakup));
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
    ground = mix(ground, blade_color, clamp(fine * 0.55 + blades * 0.50 + mat * 0.30, 0.0, 0.8));
    ground *= species_tint(grass_species(world_xz)) * dry_country(world_xz);
    let pressed = trample_pressed(world_xz);
    return MeadowSurface(
        vec4<f32>(ground * (1.0 - trample_params.darkening * 1.2 * pressed), 1.0),
        mix(mix(0.96, 1.0, dry), 0.94, damp * exposed),
    );
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let normal = surface_geometry_normal(surface_heightmap, geometry, in.world_position.xz, in.world_normal);
    pbr_input.N = surface_relief(in.world_position.xz, normal, surface_footprint(in.world_position.xz));
    pbr_input.world_normal = normal;
    let surface = meadow_surface(in.world_position.xz, normal);
    // A low sun is caught by the blades before it reaches the soil between
    // them, so the ground loses light faster than the grass standing on it.
    var sun_height = 1.0;
    if (lights.n_directional_lights > 0u) {
        sun_height = max(lights.directional_lights[0].direction_to_light.y, 0.0);
    }
    let shaded = mix(0.72, 1.0, smoothstep(0.1, 0.7, sun_height));
    pbr_input.material.base_color = alpha_discard(pbr_input.material,
        vec4<f32>(surface.color.rgb * shaded, surface.color.a));
    pbr_input.material.perceptual_roughness = surface.roughness;
    pbr_input.material.metallic = 0.0;
    pbr_input.material.reflectance = vec3<f32>(0.04);
    pbr_input.specular_occlusion = 0.0;

    var out: FragmentOutput;
    out.color = surface_lighting(pbr_input, 0.65 * (1.0 - trample_pressed(in.world_position.xz)));
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
