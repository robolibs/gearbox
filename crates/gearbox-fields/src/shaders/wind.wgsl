// One wind for every plant layer, after Ghost of Tsushima: a global wind
// (direction, speed, gustiness) set in the environment, its strength and a
// small turn of its direction read from a baked multi-octave Perlin gust map
// scrolled downwind at the wind's own speed, and each plant bobbing on its
// own phase along its height.

@group(3) @binding(6) var wind_map: texture_2d<f32>;
@group(3) @binding(7) var wind_sampler: sampler;

// Metres one tile of the gust map covers.
const WIND_TILE: f32 = 256.0;

struct Gust {
    dir: vec3<f32>,
    strength: f32,
}

// The wind arrives packed as (downwind x, downwind z, speed m/s, gustiness).
fn wind_dir(wind: vec4<f32>) -> vec3<f32> {
    return vec3<f32>(wind.x, 0.0, wind.y);
}

// Push at a spot, as a fraction of a grass blade's height, and the local
// downwind direction. Broad 32-64 m gust patches ride downwind at the wind's
// speed, finer 8-16 m detail a little quicker; the push grows with the speed
// and dips to (1 - gustiness) of itself between gusts.
fn wind_at(xz: vec2<f32>, time: f32, wind: vec4<f32>) -> Gust {
    let downwind = wind.xy * time * max(wind.z, 0.5);
    let broad = textureSampleLevel(wind_map, wind_sampler, (xz - downwind) / WIND_TILE, 0.0);
    let detail = textureSampleLevel(wind_map, wind_sampler,
        (xz - downwind * 1.35) / WIND_TILE + vec2<f32>(0.37, 0.61), 0.0).g;
    let gust = smoothstep(0.35, 0.72, broad.r * 0.7 + detail * 0.3);
    let push = clamp(wind.z / 6.0, 0.0, 1.5);
    let turn = (broad.b - 0.5) * 0.9;
    let c = cos(turn);
    let s = sin(turn);
    let dir = vec3<f32>(wind.x * c - wind.y * s, 0.0, wind.x * s + wind.y * c);
    return Gust(dir, push * mix(1.0 - clamp(wind.w, 0.0, 1.0), 1.0, gust));
}

// A plant's own bob in -1..1, quicker in stronger wind, its phase running
// along the plant by `along` so it bends rather than pivots; still air
// barely moves it.
fn wind_bob(time: f32, seed: f32, along: f32, wind: vec4<f32>) -> f32 {
    let phase = time * (1.6 + 0.1 * wind.z) + seed * 6.2831853 - along * 1.2;
    let calm = clamp(sqrt(max(wind.z, 0.0)) * 0.5, 0.0, 1.0);
    return (sin(phase) * 0.7 + sin(phase * 2.3 + 1.7) * 0.3) * calm;
}

// How far a point `along` 0..1 up a plant leans, as a fraction of the
// plant's height along the local downwind: the push plus a flutter.
fn plant_lean(xz: vec2<f32>, time: f32, wind: vec4<f32>, seed: f32, along: f32) -> vec3<f32> {
    let gust = wind_at(xz, time, wind);
    return gust.dir * (gust.strength * 0.45 + wind_bob(time, seed, along, wind) * 0.08 * (0.5 + gust.strength));
}

// Local downwind (x, z) and the leans at a blade's middle and tip, from one
// read of the gust map.
fn blade_leans(xz: vec2<f32>, time: f32, wind: vec4<f32>, seed: f32) -> vec4<f32> {
    let gust = wind_at(xz, time, wind);
    let flutter = 0.08 * (0.5 + gust.strength);
    return vec4<f32>(gust.dir.x, gust.dir.z,
        gust.strength * 0.45 + wind_bob(time, seed, 0.5, wind) * flutter,
        gust.strength * 0.45 + wind_bob(time, seed, 1.0, wind) * flutter);
}
