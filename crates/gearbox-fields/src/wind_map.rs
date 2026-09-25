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

/// The gust map on the CPU, for physics: the same wind the vegetation
/// shaders draw.
pub struct Gusts {
    texels: Vec<u8>,
}

impl Default for Gusts {
    fn default() -> Self {
        Self { texels: bake() }
    }
}

/// Metres one tile of the gust map covers.
const TILE: f32 = SIZE as f32;

fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Gusts {
    // Bilinear, repeating read of the RGB channels at uv, like the shaders'
    // sampler.
    fn sample(&self, u: f32, v: f32) -> [f32; 3] {
        let n = SIZE as usize;
        let (x, y) = (u * SIZE as f32 - 0.5, v * SIZE as f32 - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let wrap = |i: f32| (i as i64).rem_euclid(n as i64) as usize;
        let texel = |ix: usize, iy: usize, c: usize| self.texels[(iy * n + ix) * 4 + c] as f32 / 255.0;
        let (ix0, iy0, ix1, iy1) = (wrap(x0), wrap(y0), wrap(x0 + 1.0), wrap(y0 + 1.0));
        std::array::from_fn(|c| {
            let bottom = texel(ix0, iy0, c) + (texel(ix1, iy0, c) - texel(ix0, iy0, c)) * fx;
            let top = texel(ix0, iy1, c) + (texel(ix1, iy1, c) - texel(ix0, iy1, c)) * fx;
            bottom + (top - bottom) * fy
        })
    }

    /// Wind velocity (m/s, x and z) at ground spot `xz` and `time`, for the
    /// shaders' packed `wind` (downwind x, downwind z, speed, gustiness):
    /// full speed inside a gust, `1 − gustiness` of it between, its heading
    /// turned by the slow field. Mirrors `wind_at` in `shaders/wind.wgsl`.
    pub fn at(&self, xz: [f32; 2], time: f32, wind: [f32; 4]) -> [f32; 2] {
        let [dx, dz, speed, gustiness] = wind;
        let travel = time * speed.max(0.5);
        let (sx, sz) = (dx * travel, dz * travel);
        let broad = self.sample((xz[0] - sx) / TILE, (xz[1] - sz) / TILE);
        let detail = self.sample((xz[0] - sx * 1.35) / TILE + 0.37, (xz[1] - sz * 1.35) / TILE + 0.61)[1];
        let gust = smoothstep(0.35, 0.72, broad[0] * 0.7 + detail * 0.3);
        let (s, c) = ((broad[2] - 0.5) * 0.9).sin_cos();
        let strength = speed.max(0.0) * (1.0 - gustiness.clamp(0.0, 1.0) * (1.0 - gust));
        [(dx * c - dz * s) * strength, (dx * s + dz * c) * strength]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gusts_blow_downwind_between_the_lull_and_full_speed() {
        let gusts = Gusts::default();
        let wind = [0.6, 0.8, 5.0, 0.7];
        let speeds: Vec<f32> = (0..400)
            .map(|i| {
                let [x, z] = gusts.at([i as f32 * 3.1, i as f32 * 1.7], i as f32 * 0.25, wind);
                assert!(x * 0.6 + z * 0.8 > 0.0, "the wind turns less than 90° off downwind");
                (x * x + z * z).sqrt()
            })
            .collect();
        let (lo, hi) = speeds.iter().fold((f32::MAX, 0.0f32), |(lo, hi), &s| (lo.min(s), hi.max(s)));
        assert!(lo >= 5.0 * 0.3 - 1e-4 && hi <= 5.0 + 1e-4, "speeds {lo}..{hi}");
        assert!(lo < 2.0 && hi > 4.5, "gusts and lulls both pass: {lo}..{hi}");
        let [x, z] = gusts.at([10.0, 20.0], 3.0, [0.6, 0.8, 5.0, 0.0]);
        assert!(((x * x + z * z).sqrt() - 5.0).abs() < 1e-4, "no gustiness is a steady wind");
    }
}
