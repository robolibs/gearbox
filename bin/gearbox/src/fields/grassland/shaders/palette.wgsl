fn gradient(cell: vec2<f32>) -> vec2<f32> {
    let hash = fract(sin(dot(cell, vec2<f32>(127.1, 311.7))) * 43758.5453123);
    let angle = hash * 6.2831853;
    return vec2<f32>(cos(angle), sin(angle));
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let a = dot(gradient(i), f);
    let b = dot(gradient(i + vec2<f32>(1.0, 0.0)), f - vec2<f32>(1.0, 0.0));
    let c = dot(gradient(i + vec2<f32>(0.0, 1.0)), f - vec2<f32>(0.0, 1.0));
    let d = dot(gradient(i + vec2<f32>(1.0, 1.0)), f - vec2<f32>(1.0, 1.0));
    return 0.5 + 0.7 * mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    let rot = mat2x2<f32>(0.80, 0.60, -0.60, 0.80);
    var q = p;
    var sum = 0.0;
    var amp = 0.5;
    var norm = 0.0;
    for (var i = 0; i < 3; i = i + 1) {
        sum += amp * noise(q);
        norm += amp;
        amp *= 0.5;
        q = rot * q * 2.03 + vec2<f32>(17.3, -9.1);
    }
    return sum / norm;
}

// Dry, damp and pocket weights shared by ground and vegetation.
fn meadow_pattern(world_xz: vec2<f32>) -> vec3<f32> {
    let pattern_xz = world_xz / 1.5;
    let warp = vec2<f32>(
        noise(pattern_xz * 0.025 + vec2<f32>(17.0, -43.0)),
        noise(pattern_xz * 0.025 + vec2<f32>(-61.0, 29.0)),
    ) - vec2<f32>(0.5);
    let terrain_uv = pattern_xz + warp * 22.0;
    let moisture = fbm(terrain_uv * 0.018 + vec2<f32>(5.0, -11.0));
    let pockets = fbm(terrain_uv * 0.11 + vec2<f32>(71.0, -113.0));
    let dry = smoothstep(0.38, 0.68, moisture + (pockets - 0.5) * 0.3);
    let damp = 1.0 - smoothstep(0.27, 0.54, moisture);
    return vec3<f32>(dry, damp, pockets);
}

fn meadow_pocket_blend(value: f32) -> f32 {
    return smoothstep(0.44, 0.725, value);
}

fn meadow_tint(pattern: vec3<f32>) -> vec3<f32> {
    let olive = mix(vec3<f32>(1.0), vec3<f32>(1.12, 0.94, 0.86), pattern.x);
    let lush = mix(olive, olive * vec3<f32>(0.80, 0.91, 0.87), pattern.y);
    return lush * mix(1.0, 0.90, meadow_pocket_blend(pattern.z));
}

fn meadow_canopy(pattern: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(0.13, 0.215, 0.052) * meadow_tint(pattern);
}

// Fescue (x) and ryegrass (y) patch weights; meadow grass fills the rest.
fn grass_species(world_xz: vec2<f32>) -> vec2<f32> {
    let warp = vec2<f32>(noise(world_xz * 0.035 + vec2<f32>(3.7, -8.1)),
        noise(world_xz * 0.035 + vec2<f32>(-5.9, 2.3))) - vec2<f32>(0.5);
    let p = world_xz + warp * 16.0;
    let fescue = smoothstep(0.56, 0.74, noise(p * 0.07 + vec2<f32>(41.0, 7.0)));
    let rye = smoothstep(0.56, 0.74, noise(p * 0.055 + vec2<f32>(-19.0, 63.0)));
    return vec2<f32>(fescue, rye) / max(fescue + rye, 1.0);
}

// Colour shift a species patch gives the ground and distant clumps.
fn species_tint(species: vec2<f32>) -> vec3<f32> {
    return vec3<f32>(1.0) + species.x * vec3<f32>(-0.12, 0.02, 0.22)
        + species.y * vec3<f32>(-0.25, 0.18, -0.30);
}
