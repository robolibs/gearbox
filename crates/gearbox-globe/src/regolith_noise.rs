use bevy::math::DVec3;

pub fn hash(i: i64, j: i64, k: i64) -> f32 {
    let mut h = (i as u32).wrapping_mul(1597334677)
        ^ (j as u32).wrapping_mul(3812015801)
        ^ (k as u32).wrapping_mul(2798796415);
    h ^= h >> 16;
    h = h.wrapping_mul(2246822519);
    h ^= h >> 13;
    h = h.wrapping_mul(3266489917);
    h ^= h >> 16;
    (h >> 8) as f32 / 16777216.0
}

pub fn value(p: DVec3) -> f32 {
    let cell = p.floor();
    let (i, j, k) = (cell.x as i64, cell.y as i64, cell.z as i64);
    let f = (p - cell).as_vec3();
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(hash(i,j,k), hash(i+1,j,k), u.x);
    let x10 = lerp(hash(i,j+1,k), hash(i+1,j+1,k), u.x);
    let x01 = lerp(hash(i,j,k+1), hash(i+1,j,k+1), u.x);
    let x11 = lerp(hash(i,j+1,k+1), hash(i+1,j+1,k+1), u.x);
    lerp(lerp(x00,x10,u.y),lerp(x01,x11,u.y),u.z)
}

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
