// The ground's own damp, for anything that needs to agree with the lands:
// the blades standing in it, the motes over it, the soil between them. It is
// the same reading the crate makes on the processor, written the same way, so
// nothing has to be told where the country turns dry.

fn land_gradient(cell: vec2<f32>) -> vec2<f32> {
    let hash = fract(sin(dot(cell, vec2<f32>(127.1, 311.7))) * 43758.5453123);
    let angle = hash * 6.2831853;
    return vec2<f32>(cos(angle), sin(angle));
}

fn land_noise(place: vec2<f32>) -> f32 {
    let corner = floor(place);
    let inside = fract(place);
    let ease = inside * inside * inside * (inside * (inside * 6.0 - 15.0) + 10.0);
    let a = dot(land_gradient(corner), inside);
    let b = dot(land_gradient(corner + vec2<f32>(1.0, 0.0)), inside - vec2<f32>(1.0, 0.0));
    let c = dot(land_gradient(corner + vec2<f32>(0.0, 1.0)), inside - vec2<f32>(0.0, 1.0));
    let d = dot(land_gradient(corner + vec2<f32>(1.0, 1.0)), inside - vec2<f32>(1.0, 1.0));
    return 0.5 + 0.7 * mix(mix(a, b, ease.x), mix(c, d, ease.x), ease.y);
}

// Nought parched, one sodden.
fn land_damp(place: vec2<f32>) -> f32 {
    let broad = land_noise(place / 420.0 + vec2<f32>(13.7, -4.1));
    let fine = land_noise(place / 95.0 + vec2<f32>(-27.3, 8.9));
    return clamp(broad * 0.72 + fine * 0.28, 0.0, 1.0);
}

// The share of the damp land at a place; the rest is the dry one.
fn land_damp_share(place: vec2<f32>, parched: f32, lush: f32) -> f32 {
    return smoothstep(parched, lush, land_damp(place));
}
