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
    radius: f32,
    height_amp: f32,
    surface_kind: u32,
    terrain_home: vec4<f32>,
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

// The prepared pad at home off Earth: level within PAD_FLAT_M, full relief by
// PAD_RELIEF_M. The same as gearbox-globe's terrain, which a test holds it to.
const PAD_FLAT_M: f32 = 320.0;
const PAD_RELIEF_M: f32 = 480.0;

// Deterministic height in roughly [-1, 1] from a unit sphere direction.
// IMPORTANT: octave counts are constant across all LODs — that is what makes
// tiles of different LODs agree at shared texels.
fn terrain_height(dir: vec3<f32>, seed: vec3<f32>, freq: f32) -> f32 {
    if params.surface_kind == 0u { return 0.0; }
    let p = dir * params.radius;
    let fade = smoothstep(PAD_FLAT_M, PAD_RELIEF_M, distance(p, params.terrain_home.xyz));
    return (regolith_height(p, params.surface_kind) - params.terrain_home.w) * fade / params.height_amp;
}

fn regolith_hash(cell: vec3<i32>) -> f32 {
    let c = bitcast<vec3<u32>>(cell);
    var h = (c.x * 1597334677u) ^ (c.y * 3812015801u) ^ (c.z * 2798796415u);
    h ^= h >> 16u;
    h *= 2246822519u;
    h ^= h >> 13u;
    h *= 3266489917u;
    h ^= h >> 16u;
    return f32(h >> 8u) / 16777216.0;
}

fn regolith_noise(p: vec3<f32>) -> f32 {
    let i = vec3<i32>(floor(p));
    let f = fract(p);
    let u = f*f*f*(f*(f*6.0-15.0)+10.0);
    let x00 = mix(regolith_hash(i), regolith_hash(i+vec3(1,0,0)), u.x);
    let x10 = mix(regolith_hash(i+vec3(0,1,0)), regolith_hash(i+vec3(1,1,0)), u.x);
    let x01 = mix(regolith_hash(i+vec3(0,0,1)), regolith_hash(i+vec3(1,0,1)), u.x);
    let x11 = mix(regolith_hash(i+vec3(0,1,1)), regolith_hash(i+vec3(1,1,1)), u.x);
    return mix(mix(x00,x10,u.y),mix(x01,x11,u.y),u.z);
}

fn regolith_fbm(p: vec3<f32>, octaves: u32) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var total = 0.0;
    var q = p;
    for (var n = 0u; n < octaves; n++) {
        sum += amp * regolith_noise(q);
        total += amp;
        amp *= 0.5;
        q = q * 2.03 + vec3(17.13,3.71,9.27);
    }
    return sum / total;
}

fn crater_profile(q: f32) -> f32 {
    if q >= 1.6 { return 0.0; }
    let bowl = select(0.0, -0.19 * (1.0-q*q)*(1.0-q*q), q < 1.0);
    let crest = (q-1.0)/0.14;
    let rim = 0.048 * exp(-crest*crest);
    let t = clamp((1.6-q)/0.25, 0.0, 1.0);
    return bowl + rim*t*t*(3.0-2.0*t);
}

fn regolith_craters(p: vec3<f32>, cell_m: f32, density: f32) -> f32 {
    let cell = vec3<i32>(floor(p / cell_m));
    let radius = length(p);
    var height = 0.0;
    for (var x = -1; x <= 1; x++) {
        for (var y = -1; y <= 1; y++) {
            for (var z = -1; z <= 1; z++) {
                let c = cell + vec3(x,y,z);
                let seed = regolith_hash(c);
                if seed > density { continue; }
                let jitter = vec3(seed, regolith_hash(c+vec3(913,0,0)), regolith_hash(c+vec3(0,719,0)));
                let raw_centre = (vec3<f32>(c)+jitter*0.7+vec3(0.15))*cell_m;
                if abs(length(raw_centre)-radius) > cell_m*0.45 { continue; }
                let centre = normalize(raw_centre)*radius;
                let r = cell_m*(0.12+0.20*regolith_hash(c+vec3(0,0,331)));
                height += crater_profile(distance(p,centre)/r)*r;
            }
        }
    }
    return height;
}

fn regolith_height(p: vec3<f32>, kind: u32) -> f32 {
    let broad = (regolith_fbm(p*0.000055,4u)-0.5)*420.0;
    let hills = (regolith_fbm(p*0.0013+vec3(71.0),4u)-0.5)*28.0;
    let grit = (regolith_fbm(p*0.06,3u)-0.5)*0.18;
    if kind == 1u {
        let wind = dot(p,vec3(0.53,0.21,0.82));
        let dunes = sin(wind*0.30+2.0*regolith_noise(p*0.02))*0.32;
        return broad+hills+grit+dunes+regolith_craters(p,95.0,0.35)+regolith_craters(p,700.0,0.55)+regolith_craters(p,9000.0,0.6)
            +regolith_craters(p,180000.0,0.55)*0.25+regolith_craters(p,900000.0,0.4)*0.1
            +(regolith_fbm(p*0.000001,4u)-0.5)*3600.0;
    }
    return broad*0.75+hills*0.55+grit+regolith_craters(p,65.0,0.8)+regolith_craters(p,450.0,0.9)+regolith_craters(p,6500.0,0.95)
        +regolith_craters(p,120000.0,0.85)*0.7+regolith_craters(p,550000.0,0.75)*0.3
        +(regolith_fbm(p*0.0000016,4u)-0.5)*1200.0;
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
