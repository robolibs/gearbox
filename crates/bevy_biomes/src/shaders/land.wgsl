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
    // Noise on a lattice shows its lattice: diamonds, and edges along the
    // axes. Folding the place through a coarse reading of itself, and turning
    // each finer reading against the last, leaves nothing straight to see.
    let turn = mat2x2<f32>(0.80, 0.60, -0.60, 0.80);
    let warp = vec2<f32>(land_noise(place / 610.0 + vec2<f32>(5.2, 1.3)),
        land_noise(place / 610.0 + vec2<f32>(-3.1, 7.7))) - vec2<f32>(0.5);
    let folded = place + warp * 260.0;
    let broad = land_noise(folded / 430.0 + vec2<f32>(13.7, -4.1));
    let middle = land_noise(turn * folded / 170.0 + vec2<f32>(-27.3, 8.9));
    let fine = land_noise(turn * turn * folded / 68.0 + vec2<f32>(41.0, -9.3));
    return clamp(broad * 0.54 + middle * 0.31 + fine * 0.15, 0.0, 1.0);
}

// How much of each land there is at a place: parched sand, dry grass, damp
// meadow. They add up to one. `bands` holds the damp at which the sand has
// wholly given way, where it begins to, and the same for the meadow.
fn land_shares(place: vec2<f32>, bands: vec4<f32>) -> vec3<f32> {
    let wet = land_damp(place);
    let parched = 1.0 - smoothstep(bands.x, bands.y, wet);
    let meadow = smoothstep(bands.z, bands.w, wet);
    return vec3<f32>(parched, max(1.0 - parched - meadow, 0.0), meadow);
}
