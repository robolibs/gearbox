// Bakes one CDLOD node's heightmap tile into a layer of the R32Float atlas.
//
// Texel t maps to grid coord g = t - BORDER (g in [-1, 130]; vertex texels are
// g in [0,128], the rest is border for fragment-shader finite differences).
// All face-uv math is exact dyadic arithmetic, so texels shared between tiles
// of different LODs / faces bake bit-identical heights -> crack-free.

// Field order/layout must match TileBakeRequest in bake.rs.
struct TileBakeParams {
    origin: vec2<f32>, // node min corner in face uv [-1,1]
    scale: f32,        // node extent in face uv
    face: u32,
    seed: vec3<f32>,   // per-planet noise domain offset
    layer: u32,
    freq: f32,         // per-planet noise domain frequency
}

@group(0) @binding(0) var atlas: texture_storage_2d_array<r32float, write>;
@group(0) @binding(1) var<uniform> params: TileBakeParams;

const TILE_TEXELS: u32 = 132u;
const BORDER: f32 = 1.0;
const GRID_F: f32 = 128.0;

fn face_dir(face: u32, uv: vec2<f32>) -> vec3<f32> {
    var t: vec3<f32>;
    var b: vec3<f32>;
    var n: vec3<f32>;
    switch face {
        case 0u: { t = vec3(0.0, 0.0, -1.0); b = vec3(0.0, 1.0, 0.0); n = vec3(1.0, 0.0, 0.0); }
        case 1u: { t = vec3(0.0, 0.0, 1.0);  b = vec3(0.0, 1.0, 0.0); n = vec3(-1.0, 0.0, 0.0); }
        case 2u: { t = vec3(1.0, 0.0, 0.0);  b = vec3(0.0, 0.0, -1.0); n = vec3(0.0, 1.0, 0.0); }
        case 3u: { t = vec3(1.0, 0.0, 0.0);  b = vec3(0.0, 0.0, 1.0);  n = vec3(0.0, -1.0, 0.0); }
        case 4u: { t = vec3(1.0, 0.0, 0.0);  b = vec3(0.0, 1.0, 0.0);  n = vec3(0.0, 0.0, 1.0); }
        default: { t = vec3(-1.0, 0.0, 0.0); b = vec3(0.0, 1.0, 0.0);  n = vec3(0.0, 0.0, -1.0); }
    }
    return normalize(n + uv.x * t + uv.y * b);
}

// ---- hash-based 3D gradient noise -----------------------------------------

fn pcg3d(p: vec3<u32>) -> vec3<u32> {
    var v = p * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v ^= v >> vec3<u32>(16u);
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return v;
}

fn grad3(ip: vec3<i32>) -> vec3<f32> {
    let h = pcg3d(bitcast<vec3<u32>>(ip));
    // [-1,1]^3, deliberately unnormalized (amplitude folded into fbm scaling)
    return vec3<f32>(h) * (2.0 / 4294967296.0) - 1.0;
}

fn gnoise(p: vec3<f32>) -> f32 {
    let i = vec3<i32>(floor(p));
    let f = fract(p);
    // quintic fade (C2-continuous)
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let c000 = dot(grad3(i + vec3(0, 0, 0)), f - vec3(0.0, 0.0, 0.0));
    let c100 = dot(grad3(i + vec3(1, 0, 0)), f - vec3(1.0, 0.0, 0.0));
    let c010 = dot(grad3(i + vec3(0, 1, 0)), f - vec3(0.0, 1.0, 0.0));
    let c110 = dot(grad3(i + vec3(1, 1, 0)), f - vec3(1.0, 1.0, 0.0));
    let c001 = dot(grad3(i + vec3(0, 0, 1)), f - vec3(0.0, 0.0, 1.0));
    let c101 = dot(grad3(i + vec3(1, 0, 1)), f - vec3(1.0, 0.0, 1.0));
    let c011 = dot(grad3(i + vec3(0, 1, 1)), f - vec3(0.0, 1.0, 1.0));
    let c111 = dot(grad3(i + vec3(1, 1, 1)), f - vec3(1.0, 1.0, 1.0));
    let x00 = mix(c000, c100, u.x);
    let x10 = mix(c010, c110, u.x);
    let x01 = mix(c001, c101, u.x);
    let x11 = mix(c011, c111, u.x);
    return mix(mix(x00, x10, u.y), mix(x01, x11, u.y), u.z) * 1.15;
}

fn fbm(p: vec3<f32>, octaves: i32) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var norm = 0.0;
    var q = p;
    for (var o = 0; o < octaves; o++) {
        sum += amp * gnoise(q);
        norm += amp;
        amp *= 0.5;
        q = q * 2.0 + vec3(13.7, 7.3, 3.1);
    }
    return sum / norm;
}

fn ridged(p: vec3<f32>, octaves: i32) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var norm = 0.0;
    var q = p;
    for (var o = 0; o < octaves; o++) {
        let n = 1.0 - abs(gnoise(q));
        sum += amp * n * n;
        norm += amp;
        amp *= 0.52;
        q = q * 2.05 + vec3(5.1, 11.9, 2.7);
    }
    return sum / norm;
}

// Deterministic height in roughly [-1, 1] from a unit sphere direction.
// IMPORTANT: octave counts are constant across all LODs — that is what makes
// tiles of different LODs agree at shared texels.
fn terrain_height(dir: vec3<f32>, seed: vec3<f32>, freq: f32) -> f32 {
    // One ground everywhere. The globe carries no continents, no sea and no
    // mountains of its own: it is the same surface the field stands on, all
    // the way round. What the ground actually looks like is the covers' job,
    // and terrain belongs to whatever authors it later.
    return 0.0;
}

@compute @workgroup_size(8, 8, 1)
fn bake(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= TILE_TEXELS || id.y >= TILE_TEXELS {
        return;
    }
    let g = vec2<f32>(id.xy) - BORDER;
    let uv = params.origin + (g / GRID_F) * params.scale;
    let dir = face_dir(params.face, uv);
    let h = terrain_height(dir, params.seed, params.freq);
    textureStore(atlas, vec2<i32>(id.xy), i32(params.layer), vec4<f32>(h, 0.0, 0.0, 0.0));
}
