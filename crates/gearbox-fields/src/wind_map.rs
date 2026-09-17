//! Tiling gust map for the vegetation wind, after Ghost of Tsushima: gusts
//! are big patches of multi-octave Perlin noise that the shaders scroll
//! downwind. Baked once, 1 texel per metre over a 256 m tile.
//! Red: broad gusts (64 m and 32 m octaves). Green: gust detail (16 m and
//! 8 m). Blue: a slow field (128 m and 64 m) that turns the wind locally.

pub const SIZE: u32 = 256;

const BROAD: [(u32, f32); 2] = [(4, 0.7), (8, 0.3)];
const DETAIL: [(u32, f32); 2] = [(16, 0.65), (32, 0.35)];
const TURN: [(u32, f32); 2] = [(2, 0.75), (4, 0.25)];

fn gradient(x: u32, y: u32, period: u32, seed: u32) -> (f32, f32) {
    let mut h = (x % period).wrapping_mul(0x8da6_b343)
        ^ (y % period).wrapping_mul(0xd816_3841)
        ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    let angle = h as f32 / u32::MAX as f32 * std::f32::consts::TAU;
    (angle.cos(), angle.sin())
}

// Perlin noise tiling every `period` lattice cells over u, v in 0..1.
fn perlin(u: f32, v: f32, period: u32, seed: u32) -> f32 {
    let (x, y) = (u * period as f32, v * period as f32);
    let (ix, iy) = (x.floor(), y.floor());
    let (fx, fy) = (x - ix, y - iy);
    let (ix, iy) = (ix as u32, iy as u32);
    let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let corner = |dx: u32, dy: u32| {
        let (gx, gy) = gradient(ix + dx, iy + dy, period, seed);
        gx * (fx - dx as f32) + gy * (fy - dy as f32)
    };
    let (sx, sy) = (fade(fx), fade(fy));
    let bottom = corner(0, 0) + (corner(1, 0) - corner(0, 0)) * sx;
    let top = corner(0, 1) + (corner(1, 1) - corner(0, 1)) * sx;
    bottom + (top - bottom) * sy
}

fn octaves(u: f32, v: f32, layers: &[(u32, f32)], seed: u32) -> f32 {
    layers
        .iter()
        .enumerate()
        .map(|(i, &(period, weight))| perlin(u, v, period, seed + i as u32 * 101) * weight)
        .sum()
}

// Each channel stretched to its full 0..1 range.
fn channel(layers: &[(u32, f32)], seed: u32) -> Vec<u8> {
    let values: Vec<f32> = (0..SIZE * SIZE)
        .map(|i| {
            let (u, v) = ((i % SIZE) as f32 / SIZE as f32, (i / SIZE) as f32 / SIZE as f32);
            octaves(u, v, layers, seed)
        })
        .collect();
    let (lo, hi) = values
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    values
        .iter()
        .map(|v| ((v - lo) / (hi - lo).max(1e-6) * 255.0).round() as u8)
        .collect()
}

/// RGBA8 texels of the gust map, row by row.
pub fn bake() -> Vec<u8> {
    let (broad, detail, turn) = (channel(&BROAD, 7), channel(&DETAIL, 31), channel(&TURN, 53));
    (0..(SIZE * SIZE) as usize)
        .flat_map(|i| [broad[i], detail[i], turn[i], 255])
        .collect()
}
