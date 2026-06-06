#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
}

#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100)
var terrain_albedo: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101)
var terrain_albedo_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(102)
var terrain_height: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(103)
var terrain_height_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(104)
var terrain_detail_albedo: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(105)
var terrain_detail_albedo_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(106)
var terrain_detail_height: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(107)
var terrain_detail_height_sampler: sampler;

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
    local: vec2<f32>,
    cell: vec2<f32>,
    seed: f32,
) -> vec4<f32> {
    let r = hash21(cell + vec2<f32>(seed, seed * 1.37));
    var p = local;
    if (r < 0.25) {
        p = local;
    } else if (r < 0.5) {
        p = vec2<f32>(local.y, 1.0 - local.x);
    } else if (r < 0.75) {
        p = vec2<f32>(1.0 - local.x, 1.0 - local.y);
    } else {
        p = vec2<f32>(1.0 - local.y, local.x);
    }
    if (hash21(cell + vec2<f32>(seed * 2.1, 19.7)) > 0.5) {
        p.x = 1.0 - p.x;
    }
    let offset = vec2<f32>(
        hash21(cell + vec2<f32>(41.3 + seed, 7.1)),
        hash21(cell + vec2<f32>(13.9, 67.7 + seed))
    );
    return textureSample(tex, smp, fract(p + offset));
}

fn scatter_sample(tex: texture_2d<f32>, smp: sampler, uv: vec2<f32>, seed: f32) -> vec4<f32> {
    let cell = floor(uv);
    let local = fract(uv);
    let f = smooth2(local);
    let c00 = sample_variant(tex, smp, local, cell, seed);
    let c10 = sample_variant(tex, smp, local, cell + vec2<f32>(1.0, 0.0), seed);
    let c01 = sample_variant(tex, smp, local, cell + vec2<f32>(0.0, 1.0), seed);
    let c11 = sample_variant(tex, smp, local, cell + vec2<f32>(1.0, 1.0), seed);
    return mix(mix(c00, c10, f.x), mix(c01, c11, f.x), f.y);
}

fn cut_hay_mask(world_xz: vec2<f32>) -> f32 {
    let spacing = 7.3;
    let waviness = sin(world_xz.x * 0.030 + 3.0 * fbm(world_xz * 0.010)) * 1.4;
    let row = fract((world_xz.y + waviness + 1000.0) / spacing) * spacing;
    let dist = abs(row - spacing * 0.5);
    let core = pow(saturate(1.0 - dist / 2.1), 1.7);
    let clumps = fbm(world_xz * vec2<f32>(0.055, 0.035) + vec2<f32>(17.0, -23.0));
    let broken = smooth01((clumps - 0.22) / 0.78);
    let flecks = pow(fbm(world_xz * vec2<f32>(0.35, 0.22) + vec2<f32>(-8.0, 14.0)), 5.0) * 0.28;
    return saturate(core * (0.45 + 0.55 * broken) + flecks * sqrt(core));
}

fn terrain_color(world_xz: vec2<f32>) -> vec4<f32> {
    // GPU "virtual high resolution": source textures are small, but
    // sampled at short world scales and layered like games do. This
    // gives sharp close-up texture without baking a giant image.
    let base = scatter_sample(terrain_albedo, terrain_albedo_sampler, world_xz / 3.6, 11.0);
    let base_h = scatter_sample(terrain_height, terrain_height_sampler, world_xz / 3.6 + vec2<f32>(3.1, -1.7), 23.0).r;
    let base_micro = scatter_sample(terrain_albedo, terrain_albedo_sampler, world_xz / 1.25 + vec2<f32>(61.0, -37.0), 59.0);
    let base_micro_h = scatter_sample(terrain_height, terrain_height_sampler, world_xz / 1.25 + vec2<f32>(7.7, -5.3), 61.0).r;
    let detail = scatter_sample(terrain_detail_albedo, terrain_detail_albedo_sampler, world_xz / 2.2 + vec2<f32>(19.3, -7.1), 37.0);
    let detail_h = scatter_sample(terrain_detail_height, terrain_detail_height_sampler, world_xz / 2.2 + vec2<f32>(-2.9, 4.7), 41.0).r;
    let detail_micro = scatter_sample(terrain_detail_albedo, terrain_detail_albedo_sampler, world_xz / 0.85 + vec2<f32>(-43.0, 91.0), 83.0);
    let detail_micro_h = scatter_sample(terrain_detail_height, terrain_detail_height_sampler, world_xz / 0.85 + vec2<f32>(12.0, 31.0), 89.0).r;

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
    // Continuous high-frequency grain. Do NOT floor world space here:
    // that makes a visible checker/cell board when magnified.
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
    c = mix(c, broad_field, broad_mask * 0.34);
    c = mix(c, mid_brown, mid_mask * 0.54);
    c = vec4<f32>(
        clamp(c.rgb * vec3<f32>(height_shade, height_shade, height_shade * 0.90), vec3<f32>(0.0), vec3<f32>(1.0)),
        c.a
    );

    let hay = cut_hay_mask(world_xz);
    let hay_luma = 0.85 + straw_n * 0.15;
    let hay_color = vec4<f32>(0.92 * hay_luma, 0.78 * hay_luma, 0.42 * hay_luma, 1.0);
    c = mix(c, hay_color, hay * 0.62);

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
    c = mix(c, hard_detail, hard_mask * 0.46);

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

    // Extra contrast modulation so the fine pattern is not washed out by
    // PBR lighting or the creamy wheat tint.
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
    return vec4<f32>(clamp(c.rgb, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    var color = terrain_color(in.world_position.xz);
#ifdef VERTEX_COLORS
    color = color * in.color;
#endif
    pbr_input.material.base_color = alpha_discard(pbr_input.material, color);
    pbr_input.material.perceptual_roughness = 0.98;
    pbr_input.material.metallic = 0.0;

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
