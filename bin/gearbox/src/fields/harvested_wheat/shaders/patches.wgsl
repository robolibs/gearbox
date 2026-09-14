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
