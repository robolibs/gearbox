#import bevy_pbr::{
    pbr_types::{PbrInput, STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT},
    pbr_functions::{apply_pbr_lighting, calculate_diffuse_color, calculate_F0_dielectric, main_pass_post_lighting_processing},
    mesh_view_bindings::lights,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    mesh_view_types::{DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT, FOG_MODE_LINEAR, FOG_MODE_EXPONENTIAL, FOG_MODE_EXPONENTIAL_SQUARED, FOG_MODE_ATMOSPHERIC},
    shadows::{fetch_directional_shadow, get_cascade_index, world_to_directional_light_local},
    shadow_sampling::sample_shadow_map_hardware,
    fog::{linear_fog, exponential_fog, exponential_squared_fog, atmospheric_fog},
    lighting,
}
#import bevy_pbr::mesh_view_bindings as view_bindings

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

const MOST_SUNS: u32 = 4u;

// The shadow each directional light casts on this point, fetched once.
fn sun_shadows(input: PbrInput) -> array<f32, 4> {
    var shadows = array<f32, 4>(1.0, 1.0, 1.0, 1.0);
    if ((input.flags & MESH_FLAGS_SHADOW_RECEIVER_BIT) == 0u) {
        return shadows;
    }
    let from_world = view_bindings::view.view_from_world;
    let view_z = dot(vec4<f32>(from_world[0].z, from_world[1].z, from_world[2].z, from_world[3].z),
        input.world_position);
    for (var i = 0u; i < min(lights.n_directional_lights, MOST_SUNS); i += 1u) {
        if ((lights.directional_lights[i].flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u) {
            shadows[i] = fetch_directional_shadow(i, input.world_position, input.world_normal,
                view_z, input.frag_coord.xy);
        }
    }
    return shadows;
}

// The direct light the suns' shadows take away from `input`, lit unshadowed.
fn shaded_out(input: PbrInput, shadows: array<f32, 4>) -> vec3<f32> {
    let perceptual_roughness = input.material.perceptual_roughness;
    let NdotV = max(dot(input.N, input.V), 0.0001);
    var li: lighting::LightingInput;
    li.layers[lighting::LAYER_BASE].NdotV = NdotV;
    li.layers[lighting::LAYER_BASE].N = input.N;
    li.layers[lighting::LAYER_BASE].R = reflect(-input.V, input.N);
    li.layers[lighting::LAYER_BASE].perceptual_roughness = perceptual_roughness;
    li.layers[lighting::LAYER_BASE].roughness = lighting::perceptualRoughnessToRoughness(perceptual_roughness);
    li.P = input.world_position.xyz;
    li.V = input.V;
    li.diffuse_color = calculate_diffuse_color(input.material.base_color.rgb, input.material.metallic,
        input.material.specular_transmission, input.material.diffuse_transmission);
    li.metallic = input.material.metallic;
    li.F0_dielectric = calculate_F0_dielectric(input.material.reflectance);
    li.F0_metallic = input.material.base_color.rgb;
    li.F_ab = lighting::F_AB(perceptual_roughness, NdotV);
#ifdef STANDARD_MATERIAL_CLEARCOAT
    li.layers[lighting::LAYER_CLEARCOAT].NdotV = max(dot(input.clearcoat_N, input.V), 0.0001);
    li.layers[lighting::LAYER_CLEARCOAT].N = input.clearcoat_N;
    li.layers[lighting::LAYER_CLEARCOAT].R = reflect(-input.V, input.clearcoat_N);
    li.layers[lighting::LAYER_CLEARCOAT].perceptual_roughness = input.material.clearcoat_perceptual_roughness;
    li.layers[lighting::LAYER_CLEARCOAT].roughness =
        lighting::perceptualRoughnessToRoughness(input.material.clearcoat_perceptual_roughness);
    li.clearcoat_strength = input.material.clearcoat;
#endif
#ifdef STANDARD_MATERIAL_ANISOTROPY
    li.anisotropy = input.anisotropy_strength;
    li.Ta = input.anisotropy_T;
    li.Ba = input.anisotropy_B;
#endif
    var lost = vec3<f32>(0.0);
    for (var i = 0u; i < min(lights.n_directional_lights, MOST_SUNS); i += 1u) {
        lost += lighting::directional_light(i, &li, true) * (1.0 - shadows[i]);
    }
    return lost;
}

// The suns' shadows on a plant: the view's own filter in the nearest cascade,
// one hardware comparison tap past it.
fn plant_shadows(input: PbrInput) -> array<f32, 4> {
    var shadows = array<f32, 4>(1.0, 1.0, 1.0, 1.0);
    if ((input.flags & MESH_FLAGS_SHADOW_RECEIVER_BIT) == 0u) {
        return shadows;
    }
    let from_world = view_bindings::view.view_from_world;
    let view_z = dot(vec4<f32>(from_world[0].z, from_world[1].z, from_world[2].z, from_world[3].z),
        input.world_position);
    for (var i = 0u; i < min(lights.n_directional_lights, MOST_SUNS); i += 1u) {
        let light = &lights.directional_lights[i];
        if (((*light).flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) == 0u) {
            continue;
        }
        let cascade = get_cascade_index(i, view_z);
        if (cascade == 0u) {
            shadows[i] = fetch_directional_shadow(i, input.world_position, input.world_normal,
                view_z, input.frag_coord.xy);
            continue;
        }
        if (cascade >= (*light).num_cascades) {
            continue;
        }
        let normal_offset = (*light).shadow_normal_bias * (*light).cascades[cascade].texel_size
            * input.world_normal;
        let depth_offset = (*light).shadow_depth_bias * (*light).direction_to_light;
        let local = world_to_directional_light_local(i, cascade,
            vec4<f32>(input.world_position.xyz + normal_offset + depth_offset, input.world_position.w));
        if (local.w > 0.0) {
            shadows[i] = sample_shadow_map_hardware(local.xy, local.z,
                i32((*light).depth_texture_base_index + cascade));
        }
    }
    return shadows;
}

// The main pass's post-lighting for a plant: distance fog lit by the suns
// through the shadows already fetched, then the rest as the main pass does it.
fn plant_post(input: PbrInput, color: vec4<f32>, shadows: array<f32, 4>) -> vec4<f32> {
    var output = color;
#ifdef DISTANCE_FOG
    if ((input.material.flags & STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT) != 0u) {
        let params = view_bindings::fog;
        let to_frag = input.world_position.xyz - view_bindings::view.world_position.xyz;
        let distance = length(to_frag);
        var scattering = vec3<f32>(0.0);
        if (params.directional_light_color.a > 0.0) {
            let along = to_frag / distance;
            for (var i = 0u; i < lights.n_directional_lights; i += 1u) {
                let light = &lights.directional_lights[i];
                var shadow = 1.0;
                if (i < MOST_SUNS) {
                    shadow = shadows[i];
                }
                scattering += pow(max(dot(along, (*light).direction_to_light), 0.0),
                    params.directional_light_exponent) * (*light).color.rgb * view_bindings::view.exposure
                    * shadow;
            }
        }
        if (params.mode == FOG_MODE_LINEAR) {
            output = linear_fog(params, output, distance, scattering);
        } else if (params.mode == FOG_MODE_EXPONENTIAL) {
            output = exponential_fog(params, output, distance, scattering);
        } else if (params.mode == FOG_MODE_EXPONENTIAL_SQUARED) {
            output = exponential_squared_fog(params, output, distance, scattering);
        } else if (params.mode == FOG_MODE_ATMOSPHERIC) {
            output = atmospheric_fog(params, output, distance, scattering);
        }
    }
#endif
    var rest = input;
    rest.material.flags = input.material.flags & ~STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT;
    return main_pass_post_lighting_processing(rest, output);
}

// A plant lit and post-processed with one shadow lookup per sun.
fn plant_lighting(input: PbrInput) -> vec4<f32> {
    let shadows = plant_shadows(input);
    return plant_post(input, lit_under(input, shadows) + back_light(input), shadows);
}

// Light through a thin leaf from behind, unshadowed: Bevy's diffuse
// transmission, or with GRASS_CHEAP_BACKLIGHT the view-dependent lobe of
// Barré-Brisebois 2011 around the light's direction.
fn back_light(input: PbrInput) -> vec4<f32> {
#ifdef GRASS_CHEAP_BACKLIGHT
    let transmit = input.material.base_color.rgb * input.material.diffuse_transmission;
    var through = vec3<f32>(0.0);
    for (var i = 0u; i < lights.n_directional_lights; i += 1u) {
        let light = &lights.directional_lights[i];
        let bent = normalize((*light).direction_to_light + input.N * 0.3);
        through += pow(clamp(dot(input.V, -bent), 0.0, 1.0), 4.0) * (*light).color.rgb;
    }
    return vec4<f32>(transmit * through * view_bindings::view.exposure, 0.0);
#else
    return vec4<f32>(0.0);
#endif
}

// `apply_pbr_lighting` with shadows the caller fetched: lit unshadowed, less
// what the shadows take.
fn lit_under(input: PbrInput, shadows: array<f32, 4>) -> vec4<f32> {
    var open = input;
    open.flags = input.flags & ~MESH_FLAGS_SHADOW_RECEIVER_BIT;
    return apply_pbr_lighting(open)
        - vec4<f32>(shaded_out(input, shadows) * view_bindings::view.exposure, 0.0);
}

fn surface_lighting(input: PbrInput, canopy: f32) -> vec4<f32> {
    return surface_lighting_under(input, canopy, sun_shadows(input));
}

// A canopy of plants lit and post-processed with one shadow lookup per sun.
fn canopy_lighting(input: PbrInput, canopy: f32) -> vec4<f32> {
    let shadows = plant_shadows(input);
    return plant_post(input, surface_lighting_under(input, canopy, shadows) + back_light(input), shadows);
}

fn surface_lighting_under(input: PbrInput, canopy: f32, shadows: array<f32, 4>) -> vec4<f32> {
    // The side lobes stand for lit blade flanks. A low sun rakes across a
    // sward and its blades shade the ground between them, so the lobes fade
    // with the sun and the ground darkens in step with the grass on it.
    var sun_height = 1.0;
    if (lights.n_directional_lights > 0u) {
        sun_height = max(lights.directional_lights[0].direction_to_light.y, 0.0);
    }
    let lobes = canopy * smoothstep(0.1, 0.7, sun_height);
    // One shadow lookup serves the base and all four lobes.
    var result = lit_under(input, shadows) * (1.0 - lobes);
    if (lobes <= 0.0) {
        return result;
    }
    let n = input.N;
    let tangent = normalize(cross(vec3<f32>(0.0, 0.0, 1.0), n));
    let bitangent = cross(n, tangent);
    let directions = array<vec3<f32>, 4>(tangent, -tangent, bitangent, -bitangent);
    for (var i = 0u; i < 4u; i += 1u) {
        var leaf = input;
        leaf.N = normalize(n * 0.7 + directions[i]);
        leaf.world_normal = leaf.N;
        result += lit_under(leaf, shadows) * (lobes * 0.25);
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
