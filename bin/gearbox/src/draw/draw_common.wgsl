// Colour helpers shared by the draw shaders: sRGB decoding and the scene
// colour the AgX view transform shows as a wanted display colour.
#define_import_path gearbox::draw_common

// Depth-only lift above short grass; xy and perspective interpolation stay fixed.
fn ground_clip(at_eye: vec4<f32>, projection: mat4x4<f32>) -> vec4<f32> {
    let clip = projection * at_eye;
    let distance = max(length(at_eye.xyz), 1e-4);
    let pull = min(0.3, 0.1 * distance);
    var lifted = vec4(at_eye.xyz * (1.0 - pull / distance), 1.0);
    if projection[3][3] == 1.0 { lifted = at_eye + vec4(0.0, 0.0, pull, 0.0); }
    let depth = projection * lifted;
    if abs(depth.w) < 1e-6 { return clip; }
    return vec4(clip.xy, depth.z / depth.w * clip.w, clip.w);
}

// Brightest scene value for neutral and for fully saturated colours.
const SCENE_MAX_NEUTRAL: f32 = 4.0;
const SCENE_MAX_SATURATED: f32 = 1.6;
const RULE_PX: f32 = 1.0;
const CORNER_PX: f32 = 6.0;
const STRUCTURE_CORNER_GAIN: f32 = 1.75;
const BRACKET_FRACTION: f32 = 0.18;
const HALO_MAX_PX: f32 = 4.0;
const HALO_BASE_PX: f32 = 3.6;
const HALO_RADIUS_GAIN: f32 = 0.08;
const HALO_RADIUS_CAP: f32 = 10.0;
const HALO_GAIN: f32 = 0.42;

fn detail_visibility(footprint: f32) -> f32 {
    return 1.0 - smoothstep(0.22, 0.65, abs(footprint));
}

fn technical_line(distance: f32, width: f32, footprint: f32) -> f32 {
    let aa = max(abs(footprint), 1e-5);
    return 1.0 - smoothstep(max(width - 0.5 * aa, 0.0), width + 0.5 * aa, abs(distance));
}

fn ink_highlight(base: vec3<f32>) -> vec3<f32> {
    return base * 1.08;
}

fn energy_glass_shade(normal: vec3<f32>, facing: f32) -> f32 {
    return 0.45 + 0.35 * max(dot(normal, normalize(vec3(0.35, 0.8, 0.55))), 0.0) + 0.20 * facing;
}

fn energy_coat(base: vec3<f32>, opacity: f32, shade: f32, coverage: f32, rim: f32, structure: f32, activity: f32, wake: f32, glow: f32) -> vec4<f32> {
    let detail = clamp(structure, 0.0, 1.0);
    let hot = ink_highlight(base);
    let alpha = clamp(coverage, 0.0, 1.0) * opacity;
    let body = mix(base * shade, hot, clamp(max(0.55 * detail, activity), 0.0, 1.0));
    let light = (base * (0.025 * clamp(coverage / 0.16, 0.0, 1.0) + 0.08 * wake)
        + hot * (0.55 * detail + 0.55 * rim + 0.60 * activity)) * max(glow, 0.0) * opacity;
    return vec4(body * alpha + light, alpha);
}

fn energy_glass(base: vec3<f32>, opacity: f32, shade: f32, rim: f32, structure: f32, activity: f32, wake: f32, glow: f32) -> vec4<f32> {
    let coverage = 0.16 + 0.32 * clamp(structure, 0.0, 1.0) + 0.08 * rim + 0.16 * activity + 0.04 * wake;
    return energy_coat(base, opacity, shade, coverage, rim, structure, activity, wake, glow);
}

fn energy_packet(phase: f32, width: f32, footprint: f32, hz: f32) -> f32 {
    let distance = abs(fract(phase + 0.5) - 0.5);
    return technical_line(distance, width, footprint) * detail_visibility(footprint) * select(0.0, 1.0, hz > 0.0);
}

fn compact_halo_px(radius: f32, glow: f32) -> f32 {
    let reach = (HALO_BASE_PX + HALO_RADIUS_GAIN * clamp(radius, 0.0, HALO_RADIUS_CAP)) * sqrt(max(glow, 0.0));
    return select(0.0, min(HALO_MAX_PX, reach), glow > 0.0);
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3(2.4));
    return select(hi, lo, c <= vec3(0.04045));
}

// Bevy's AgX: inset matrix, log2 encoding over [-10, 6.5] EV around 0.18.
fn agx_log(color: vec3<f32>) -> vec3<f32> {
    let c = max(vec3(0.0), color);
    let r = dot(c, vec3(0.84247906, 0.0784336, 0.07922375));
    let g = dot(c, vec3(0.04232824, 0.87846864, 0.07916613));
    let b = dot(c, vec3(0.04237565, 0.0784336, 0.87914297));
    var n = vec3(r, g, b);
    n = select(n, 0.00001525878 + n, n < vec3(0.00003051757));
    n = clamp(log2(n / 0.18), vec3(-10.0), vec3(6.5));
    return clamp((n + 10.0) / 16.5, vec3(0.0), vec3(1.0));
}

fn agx_display(color: vec3<f32>, lut: texture_3d<f32>, lut_sampler: sampler) -> vec3<f32> {
    let block = 32.0;
    let uvw = agx_log(color) * ((block - 1.0) / block) + 0.5 / block;
    return textureSampleLevel(lut, lut_sampler, uvw, 0.0).rgb;
}

// Fixed-point search in log space for the scene colour AgX maps to `shown_as`.
fn display_to_scene(shown_as: vec3<f32>, lut: texture_3d<f32>, lut_sampler: sampler) -> vec3<f32> {
    let wanted = max(shown_as, vec3(0.0));
    let hi = max(max(wanted.r, wanted.g), wanted.b);
    let lo = min(min(wanted.r, wanted.g), wanted.b);
    let saturation = select(0.0, (hi - lo) / hi, hi > 1e-4);
    let cap = mix(SCENE_MAX_NEUTRAL, SCENE_MAX_SATURATED, saturation);
    var x = max(wanted, vec3(0.0005));
    for (var i = 0; i < 10; i = i + 1) {
        let shown = agx_display(x, lut, lut_sampler);
        x = x * clamp(wanted / max(shown, vec3(0.00002)), vec3(0.25), vec3(4.0));
        x = min(x, vec3(cap));
    }
    return x;
}
