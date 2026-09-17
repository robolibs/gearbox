// The slab grid of a concrete yard, shared by the ground and the weeds in
// its joints so both agree on where a gap is and which stretches of yard
// have gone to seed.

// Slab pitch, and the open gap between two pours.
const SLAB_M: f32 = 10.0;
const JOINT_M: f32 = 0.05;

fn yard_pcg(input: u32) -> u32 {
    let state = input * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

// A slab's own random number; integer cells hash the same in every stage.
fn slab_rand(cell: vec2<i32>, salt: u32) -> f32 {
    let h = yard_pcg(bitcast<u32>(cell.x) ^ yard_pcg(bitcast<u32>(cell.y) ^ (salt * 0x9E3779B9u)));
    return f32(h) / 4294967295.0;
}

fn yard_noise(p: vec2<f32>) -> f32 {
    let cell = vec2<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(slab_rand(cell, 41u), slab_rand(cell + vec2<i32>(1, 0), 41u), u.x),
        mix(slab_rand(cell + vec2<i32>(0, 1), 41u), slab_rand(cell + vec2<i32>(1, 1), 41u), u.x), u.y);
}

fn slab_cell(world_xz: vec2<f32>) -> vec2<i32> {
    return vec2<i32>(floor(world_xz / SLAB_M));
}

// Metres to the nearest joint line running along Z (x) and along X (y).
fn joint_distances(world_xz: vec2<f32>) -> vec2<f32> {
    return (vec2<f32>(0.5) - abs(fract(world_xz / SLAB_M) - vec2<f32>(0.5))) * SLAB_M;
}

// 0 where the yard is kept clean, 1 where weeds and moss have taken hold.
fn yard_weedy(world_xz: vec2<f32>) -> f32 {
    let broad = yard_noise(world_xz * 0.028) * 0.7 + yard_noise(world_xz * 0.085 + vec2<f32>(17.0, -9.0)) * 0.3;
    return smoothstep(0.44, 0.6, broad);
}
