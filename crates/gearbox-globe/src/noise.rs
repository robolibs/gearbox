//! Value noise over double-precision space: the lattice is addressed in
//! integers, so it is as fine at the far side of a planet as at its centre.

use bevy::math::DVec3;

fn hash(i: i64, j: i64, k: i64) -> f32 {
    let mut h = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (j as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (k as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 31;
    h = h.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    h ^= h >> 29;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Smooth noise in 0..1.
pub fn value(p: DVec3) -> f32 {
    let cell = p.floor();
    let (i, j, k) = (cell.x as i64, cell.y as i64, cell.z as i64);
    let f = (p - cell).as_vec3();
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(hash(i, j, k), hash(i + 1, j, k), u.x);
    let x10 = lerp(hash(i, j + 1, k), hash(i + 1, j + 1, k), u.x);
    let x01 = lerp(hash(i, j, k + 1), hash(i + 1, j, k + 1), u.x);
    let x11 = lerp(hash(i, j + 1, k + 1), hash(i + 1, j + 1, k + 1), u.x);
    lerp(lerp(x00, x10, u.y), lerp(x01, x11, u.y), u.z)
}

/// Octaves of [`value`], each twice as fine and half as strong; 0..1.
pub fn fbm(p: DVec3, octaves: u32) -> f32 {
    let (mut sum, mut amplitude, mut total, mut q) = (0.0, 0.5, 0.0, p);
    for _ in 0..octaves {
        sum += amplitude * value(q);
        total += amplitude;
        amplitude *= 0.5;
        q = q * 2.03 + DVec3::new(17.13, 3.71, 9.27);
    }
    sum / total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_as_smooth_far_out_as_at_the_centre() {
        for base in [DVec3::ZERO, DVec3::new(2.9e4, -1.4e4, 6.1e3)] {
            let step = (value(base + DVec3::X * 0.5) - value(base + DVec3::X * 0.5001)).abs();
            assert!(step < 1e-3, "{step}");
        }
    }

    #[test]
    fn fbm_stays_in_range() {
        for n in 0..200 {
            let v = fbm(DVec3::new(n as f64 * 0.37, n as f64 * -1.91, 5.0), 5);
            assert!((0.0..=1.0).contains(&v));
        }
    }
}
