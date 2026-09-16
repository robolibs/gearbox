#import bevy_pbr::mesh_view_bindings::globals

struct WheelMapParams {
    origin: vec2<f32>,
    texels_per_metre: f32,
    width: f32,
    height: f32,
    recovery_seconds: f32,
    bend: f32,
    darkening: f32,
}

fn wheel_texel(tex: texture_2d<u32>, index: vec2<i32>, recovery: f32) -> vec3<f32> {
    let texel = textureLoad(tex, index, 0);
    if (texel.g == 0u) { return vec3<f32>(0.0); }
    let stamped = f32(texel.r) / 65535.0 * 3600.0;
    // A stamp a moment ahead of this frame is fresh, not a period old.
    let delta = globals.time - stamped;
    let age = select(delta, max(delta + 3600.0, 0.0), delta < -8.0);
    let press = 1.0 - clamp(age / max(recovery, 0.001), 0.0, 1.0);
    let angle = f32(texel.g - 1u) / 65534.0 * 6.2831853 - 3.1415927;
    return vec3<f32>(press, cos(angle) * press, sin(angle) * press);
}

fn sample_wheels(tex: texture_2d<u32>, params: WheelMapParams, world_xz: vec2<f32>) -> vec3<f32> {
    let t = (world_xz - params.origin) * params.texels_per_metre;
    let last = vec2<i32>(i32(params.width), i32(params.height)) - vec2<i32>(1);
    if (any(t < vec2<f32>(0.0)) || any(t > vec2<f32>(last))) { return vec3<f32>(0.0); }
    let i = clamp(vec2<i32>(floor(t)), vec2<i32>(0), last - vec2<i32>(1));
    let f = clamp(t - vec2<f32>(i), vec2<f32>(0.0), vec2<f32>(1.0));
    let a = wheel_texel(tex, i, params.recovery_seconds);
    let b = wheel_texel(tex, i + vec2<i32>(1, 0), params.recovery_seconds);
    let c = wheel_texel(tex, i + vec2<i32>(0, 1), params.recovery_seconds);
    let d = wheel_texel(tex, i + vec2<i32>(1, 1), params.recovery_seconds);
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// Unit roll direction of a wheel sample; zero where nothing has rolled.
fn wheel_roll(pressed: vec3<f32>) -> vec3<f32> {
    let size = length(pressed.yz);
    if (size < 1e-4) { return vec3<f32>(0.0); }
    return vec3<f32>(pressed.y, 0.0, pressed.z) / size;
}

// The roll direction turned by `angle` radians about the vertical.
fn scatter_roll(roll: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec3<f32>(roll.x * c - roll.z * s, 0.0, roll.x * s + roll.z * c);
}
