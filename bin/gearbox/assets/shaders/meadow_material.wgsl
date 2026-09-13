#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
}

#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100)
var grass_albedo: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(101)
var grass_albedo_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(102)
var dirt_albedo: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(103)
var dirt_albedo_sampler: sampler;

fn luma(c: vec4<f32>) -> f32 {
    return dot(c.rgb, vec3<f32>(0.30, 0.59, 0.11));
}

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
    for (var i = 0; i < 3; i = i + 1) {
        sum = sum + amp * noise(p * freq);
        norm = norm + amp;
        amp = amp * 0.5;
        freq = freq * 2.03;
    }
    return sum / norm;
}

// One texture sample per tile with a per-tile rotation about the tile
// centre. No offset: shifted tiles differ in low-frequency brightness and
// the blend between them reads as a checkerboard.
fn sample_variant(
    tex: texture_2d<f32>,
    smp: sampler,
    local: vec2<f32>,
    cell: vec2<f32>,
    seed: f32,
) -> vec4<f32> {
    let r = hash21(cell + vec2<f32>(seed, seed * 1.37));
    var p = local;
    let quarter = floor(r * 4.0);
    if (quarter == 1.0) {
        p = vec2<f32>(1.0 - p.y, p.x);
    } else if (quarter == 2.0) {
        p = vec2<f32>(1.0 - p.x, 1.0 - p.y);
    } else if (quarter == 3.0) {
        p = vec2<f32>(p.y, 1.0 - p.x);
    }
    return textureSample(tex, smp, fract(p));
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

fn meadow_color(world_xz: vec2<f32>, normal: vec3<f32>) -> vec4<f32> {
    let grass = scatter_sample(grass_albedo, grass_albedo_sampler, world_xz / 2.6, 11.0);
    let grass_micro = textureSample(grass_albedo, grass_albedo_sampler, fract(world_xz / 0.8));
    let grass_h = luma(grass) * 2.2;
    var g = mix(grass, grass_micro, 0.3);

    // Broad dry and lush patches, then a mid-scale shade so the field is
    // not one flat green.
    let macro_n = fbm(world_xz * 0.012 + vec2<f32>(5.0, -11.0));
    let dry = smooth01((macro_n - 0.50) / 0.30);
    g = mix(g, g * vec4<f32>(1.18, 1.06, 0.66, 1.0), dry * 0.55);
    let lush = smooth01((0.42 - macro_n) / 0.25);
    g = mix(g, g * vec4<f32>(0.82, 1.02, 0.78, 1.0), lush * 0.5);
    let shade = 0.70 + fbm(world_xz * 0.05 + vec2<f32>(-31.0, 19.0)) * 0.42;
    g = vec4<f32>(g.rgb * vec3<f32>(0.62, 0.70, 0.50) * shade * (0.88 + grass_h * 0.16), 1.0);

    let dirt = scatter_sample(dirt_albedo, dirt_albedo_sampler, world_xz / 2.2 + vec2<f32>(19.3, -7.1), 37.0);
    let dirt_h = luma(dirt) * 1.6;
    let d = vec4<f32>(dirt.rgb * (0.75 + dirt_h * 0.4), 1.0);

    // Bare ground on steep slopes; the height maps sharpen the edge so
    // grass tufts break into the dirt instead of fading.
    let steep = 1.0 - saturate((normal.y - 0.80) / 0.12);
    let worn = smooth01((fbm(world_xz * 0.02 + vec2<f32>(71.0, -113.0)) - 0.66) / 0.10) * 0.35;
    let mask = smooth01((steep + worn - (grass_h - dirt_h) * 0.5 - 0.25) / 0.35);
    return mix(g, d, mask);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let color = meadow_color(in.world_position.xz, normalize(in.world_normal));
    pbr_input.material.base_color = alpha_discard(pbr_input.material, color);
    pbr_input.material.perceptual_roughness = 0.95;
    pbr_input.material.metallic = 0.0;

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
