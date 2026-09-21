// Green regrowth patches in the stubble, shared by ground and vegetation.
fn patch_hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453123);
}

fn patch_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (vec2<f32>(3.0) - 2.0 * f);
    let a = patch_hash(i);
    let b = patch_hash(i + vec2<f32>(1.0, 0.0));
    let c = patch_hash(i + vec2<f32>(0.0, 1.0));
    let d = patch_hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// 0 in bare stubble, 1 inside a patch of clover, weeds and volunteer shoots.
fn regrowth(world_xz: vec2<f32>) -> f32 {
    let warp = vec2<f32>(
        patch_noise(world_xz * 0.021 + vec2<f32>(13.0, -7.0)),
        patch_noise(world_xz * 0.021 + vec2<f32>(-41.0, 23.0)),
    ) - vec2<f32>(0.5);
    let p = world_xz + warp * 28.0;
    let blobs = patch_noise(p * 0.045) * 0.65 + patch_noise(p * 0.13 + vec2<f32>(5.3, 1.7)) * 0.35;
    return smoothstep(0.56, 0.74, blobs);
}

// Drill rows wander: each 3 m drill pass sways on its own over tens of
// metres, so rows bunch and part at the seams; each row also wobbles.
// Shared so stalks and the painted ground rows stay together.
fn row_drift(p: vec2<f32>) -> f32 {
    let drill_pass = floor(p.y / 3.0);
    let phase = patch_hash(vec2<f32>(drill_pass, 3.1)) * 6.2831853;
    let sway = mix(0.08, 0.2, patch_hash(vec2<f32>(drill_pass, 5.9)));
    return sway * sin(p.x * 0.22 + phase) + 0.06 * sin(p.x * 0.05 + 1.3 * sin(p.y * 0.021));
}

fn row_wobble(row: f32, x: f32) -> f32 {
    let phase = patch_hash(vec2<f32>(row, 7.3));
    return (sin(x * 0.35 + phase * 6.2831853) + 0.5 * sin(x * 1.1 + phase * 17.0)) * 0.02;
}

// Most plants sit a few millimetres to 4 cm off their row line, either side.
fn plant_jog(row: f32, plant: f32) -> f32 {
    let size = patch_hash(vec2<f32>(plant * 0.37 + 11.0, row * 1.93));
    let side = select(-1.0, 1.0, patch_hash(vec2<f32>(plant, row + 41.0)) < 0.5);
    let moved = step(0.3, patch_hash(vec2<f32>(row * 0.71, plant + 5.0)));
    return side * mix(0.001, 0.04, size * size) * moved;
}
