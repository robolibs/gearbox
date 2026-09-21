#import bevy_pbr::{pbr_types::PbrInput, pbr_functions::apply_pbr_lighting, mesh_view_bindings::lights}

struct SurfaceGeometryParams {
    origin: vec2<f32>,
    texels_per_metre: f32,
    texel_count: f32,
}

fn foliage_normal(normal: vec3<f32>, ground: vec3<f32>, view: vec3<f32>) -> vec3<f32> {
    // A degenerate or NaN blade normal falls back to the ground's.
    let own = select(ground, normalize(normal), dot(normal, normal) > 1e-8);
    // Under a raking sun a blade leaning into the light and its neighbour
    // leaning away differ by their whole brightness, which reads as grain;
    // plants take the ground's normal as the sun drops and darken together.
    var sun_height = 1.0;
    if (lights.n_directional_lights > 0u) {
        sun_height = max(lights.directional_lights[0].direction_to_light.y, 0.0);
    }
    let raking = 1.0 - smoothstep(0.1, 0.7, sun_height);
    let n = normalize(mix(own, ground, raking * 0.85));
    let vertical = dot(n, ground);
    var lateral = n - ground * vertical;
    lateral *= select(-1.0, 1.0, dot(lateral, view) >= 0.0);
    return normalize(lateral + ground * max(abs(vertical), 0.55));
}

fn surface_geometry_normal(heightmap: texture_2d<f32>, params: SurfaceGeometryParams,
    world_xz: vec2<f32>, fallback: vec3<f32>) -> vec3<f32> {
    let p = (world_xz - params.origin) * params.texels_per_metre;
    if (params.texel_count < 2.0 || any(p < vec2<f32>(0.0)) || any(p > vec2<f32>(params.texel_count - 1.0))) {
        return normalize(fallback);
    }
    let cell = clamp(vec2<i32>(floor(p)), vec2<i32>(0), vec2<i32>(i32(params.texel_count) - 2));
    let f = clamp(p - vec2<f32>(cell), vec2<f32>(0.0), vec2<f32>(1.0));
    let a = textureLoad(heightmap, cell, 0).yz;
    let b = textureLoad(heightmap, cell + vec2<i32>(1, 0), 0).yz;
    let c = textureLoad(heightmap, cell + vec2<i32>(0, 1), 0).yz;
    let d = textureLoad(heightmap, cell + vec2<i32>(1, 1), 0).yz;
    let n = mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
    return normalize(vec3<f32>(n.x, 1.0, n.y));
}

fn surface_lighting(input: PbrInput, canopy: f32) -> vec4<f32> {
    // The side lobes stand for lit blade flanks. A low sun rakes across a
    // sward and its blades shade the ground between them, so the lobes fade
    // with the sun and the ground darkens in step with the grass on it.
    var sun_height = 1.0;
    if (lights.n_directional_lights > 0u) {
        sun_height = max(lights.directional_lights[0].direction_to_light.y, 0.0);
    }
    let lobes = canopy * smoothstep(0.1, 0.7, sun_height);
    var result = apply_pbr_lighting(input) * (1.0 - lobes);
    let n = input.N;
    let tangent = normalize(cross(vec3<f32>(0.0, 0.0, 1.0), n));
    let bitangent = cross(n, tangent);
    let directions = array<vec3<f32>, 4>(tangent, -tangent, bitangent, -bitangent);
    for (var i = 0u; i < 4u; i += 1u) {
        var leaf = input;
        leaf.N = normalize(n * 0.7 + directions[i]);
        leaf.world_normal = leaf.N;
        result += apply_pbr_lighting(leaf) * (lobes * 0.25);
    }
    return result;
}

fn surface_hash(p: vec2<f32>) -> f32 {
    var q = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    q += dot(q, q.yzx + vec3<f32>(33.33));
    return fract((q.x + q.y) * q.z);
}

fn surface_noise(p: vec2<f32>) -> f32 {
    let cell = floor(p);
    let f = fract(p);
    let u = f * f * (vec2<f32>(3.0) - 2.0 * f);
    return mix(
        mix(surface_hash(cell), surface_hash(cell + vec2<f32>(1.0, 0.0)), u.x),
        mix(surface_hash(cell + vec2<f32>(0.0, 1.0)), surface_hash(cell + vec2<f32>(1.0)), u.x), u.y);
}

fn surface_footprint(world_xz: vec2<f32>) -> f32 {
    return max(length(dpdx(world_xz)), length(dpdy(world_xz)));
}

fn filtered_clumps(world_xz: vec2<f32>, size: f32, footprint: f32, seed: f32) -> f32 {
    let resolved = 1.0 - smoothstep(0.3, 1.2, footprint / size);
    return mix(0.5, surface_noise(world_xz / size + vec2<f32>(seed, seed * 1.73)), resolved);
}

fn surface_relief(world_xz: vec2<f32>, normal: vec3<f32>, footprint: f32) -> vec3<f32> {
    let cell = 0.22;
    let p = world_xz / cell;
    let dx = surface_noise(p + vec2<f32>(0.1, 0.0)) - surface_noise(p - vec2<f32>(0.1, 0.0));
    let dz = surface_noise(p + vec2<f32>(0.0, 0.1)) - surface_noise(p - vec2<f32>(0.0, 0.1));
    let resolved = 1.0 - smoothstep(0.03, 0.22, footprint);
    // Bumps a raking sun would light as grain flatten as it drops.
    var sun_height = 1.0;
    if (lights.n_directional_lights > 0u) {
        sun_height = max(lights.directional_lights[0].direction_to_light.y, 0.0);
    }
    let raked = mix(0.35, 1.0, smoothstep(0.1, 0.7, sun_height));
    return normalize(normal + vec3<f32>(-dx, 0.0, -dz) * (0.7 * resolved * raked));
}

fn fiber_stamp(world_xz: vec2<f32>, size: f32, width: f32, footprint: f32, seed: f32) -> f32 {
    let p = world_xz / size + vec2<f32>(seed, seed * 1.73);
    let cell = floor(p);
    let random = surface_hash(cell + vec2<f32>(seed));
    let angle = random * 6.2831853;
    let axis = vec2<f32>(cos(angle), sin(angle));
    let center = vec2<f32>(0.5) + (vec2<f32>(surface_hash(cell + vec2<f32>(7.1, 3.7)),
        surface_hash(cell + vec2<f32>(29.2, 13.4))) - vec2<f32>(0.5)) * 0.2;
    let local = fract(p) - center;
    let along = dot(local, axis);
    let across = dot(local, vec2<f32>(-axis.y, axis.x));
    let half_length = mix(0.19, 0.34, random);
    let distance = length(vec2<f32>(max(abs(along) - half_length, 0.0), across)) - width;
    let aa = max(0.6 * footprint / size, 0.001);
    let stamp = 1.0 - smoothstep(-aa, aa, distance);
    let mean_coverage = 4.0 * 0.265 * width + 3.14159265 * width * width;
    let resolved = 1.0 - smoothstep(0.18, 0.65, footprint / size);
    return mix(mean_coverage, stamp, resolved);
}
