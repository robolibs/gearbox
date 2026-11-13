// Colour helpers shared by the draw shaders: sRGB decoding and the scene
// colour the AgX view transform shows as a wanted display colour.

// Brightest scene value for neutral and for fully saturated colours.
const SCENE_MAX_NEUTRAL: f32 = 4.0;
const SCENE_MAX_SATURATED: f32 = 1.6;

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
