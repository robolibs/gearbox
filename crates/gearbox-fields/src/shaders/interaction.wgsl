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
    let angle = f32(texel.g >> 8u) / 255.0 * 6.2831853 - 3.1415927;
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

// A tyre mark on a tread map: how fresh it is, how hard the tyre scrubbed,
// and where this point lies in the tread — lug pitches rolled along the
// track, and -1..1 across the tyre.
struct WheelMark {
    press: f32,
    scrub: f32,
    along: f32,
    across: f32,
}

const TREAD_PITCH_M: f32 = 0.192;

// Press and scrub blend between texels for a soft edge; the tread
// coordinates come from the nearest texel alone, carried to this point
// along its own roll, so every lug of one pass lands in step.
fn sample_wheel_mark(tex: texture_2d<u32>, params: WheelMapParams, world_xz: vec2<f32>) -> WheelMark {
    let press = sample_wheels(tex, params, world_xz).x;
    let t = (world_xz - params.origin) * params.texels_per_metre;
    let last = vec2<i32>(i32(params.width), i32(params.height)) - vec2<i32>(1);
    let nearest = clamp(vec2<i32>(round(t)), vec2<i32>(0), last);
    let texel = textureLoad(tex, nearest, 0);
    if (press < 0.001 || texel.g == 0u) { return WheelMark(0.0, 0.0, 0.0, 0.0); }
    let angle = f32(texel.g >> 8u) / 255.0 * 6.2831853 - 3.1415927;
    let roll = vec2<f32>(cos(angle), sin(angle));
    let axle = vec2<f32>(-roll.y, roll.x);
    let offset = (t - vec2<f32>(nearest)) / params.texels_per_metre;
    let along = f32(texel.b) * 0.004 + dot(offset, roll);
    let across = (f32(texel.a >> 8u) - 128.0) * 0.005 + dot(offset, axle);
    let half_width = max(f32(texel.a & 255u) * 0.005, 0.01);
    return WheelMark(press, f32(texel.g & 15u) / 15.0, along / TREAD_PITCH_M, across / half_width);
}
