// One wind for every plant layer, after Ghost of Tsushima: 2D Perlin noise
// scrolled downwind gives soft-edged gusts over a steady breeze, and each
// plant bobs on its own phase, so neighbouring layers never move apart.

const WIND_DIR: vec3<f32> = vec3<f32>(0.8, 0.0, 0.6);
// Metres a second the gusts travel downwind.
const GUST_SPEED: f32 = 3.0;

fn wind_gradient(cell: vec2<f32>) -> vec2<f32> {
    let h = fract(sin(vec2<f32>(dot(cell, vec2<f32>(127.1, 311.7)),
        dot(cell, vec2<f32>(269.5, 183.3)))) * 43758.5453);
    return h * 2.0 - 1.0;
}

fn wind_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let a = dot(wind_gradient(i), f);
    let b = dot(wind_gradient(i + vec2<f32>(1.0, 0.0)), f - vec2<f32>(1.0, 0.0));
    let c = dot(wind_gradient(i + vec2<f32>(0.0, 1.0)), f - vec2<f32>(0.0, 1.0));
    let d = dot(wind_gradient(i + vec2<f32>(1.0, 1.0)), f - vec2<f32>(1.0, 1.0));
    return 0.5 + mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Gust strength at a spot: 0.2 in the lulls, 1 in a gust. Two scrolled
// octaves, smoothstepped so a gust fades in and out with no edge.
fn wind_strength(xz: vec2<f32>, time: f32) -> f32 {
    let downwind = WIND_DIR.xz * time * GUST_SPEED;
    let broad = wind_noise((xz - downwind) * 0.07);
    let fine = wind_noise((xz - downwind * 1.4) * 0.23 + vec2<f32>(17.3, -9.1));
    return mix(0.2, 1.0, smoothstep(0.3, 0.75, broad * 0.75 + fine * 0.25));
}

// A plant's own bob in -1..1: a slow sway with a quicker flutter, its phase
// running along the plant by `along` so it bends rather than pivots.
fn wind_bob(time: f32, seed: f32, along: f32) -> f32 {
    let phase = time * 2.1 + seed * 6.2831853 - along * 1.2;
    return sin(phase) * 0.7 + sin(phase * 2.3 + 1.7) * 0.3;
}
