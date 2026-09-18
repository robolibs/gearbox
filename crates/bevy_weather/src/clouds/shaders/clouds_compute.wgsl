#import bevy_weather::clouds::common

const EPSILON = 0.000001;
const MAX_DISTANCE = 1.0e9;
const WORLEY_RESOLUTION = 32;
const WORLEY_RESOLUTION_F32 = 32.0;

struct Config {
    clouds_base_scale: f32,
    clouds_raymarch_steps_count: u32,
    clouds_bottom_height: f32,
    clouds_top_height: f32,
    clouds_coverage: f32,
    clouds_density: f32,
    clouds_detail_scale: f32,
    clouds_detail_strength: f32,
    clouds_base_edge_softness: f32,
    clouds_bottom_softness: f32,
    clouds_shadow_raymarch_steps_count: u32,
    clouds_shadow_raymarch_step_size: f32,
    clouds_shadow_raymarch_step_multiply: f32,
    clouds_ambient_color_top: vec4f,
    clouds_ambient_color_bottom: vec4f,
    clouds_min_transmittance: f32,
    planet_radius: f32,
    forward_scattering_g: f32,
    backward_scattering_g: f32,
    scattering_lerp: f32,
    sun_dir: vec4f,
    sun_color: vec4f,
    sky_zenith_color: vec4f,
    sky_horizon_color: vec4f,
    camera_translation: vec3f,
    time: f32,
    reprojection_strength: f32,
    render_resolution: vec2f,
    inverse_camera_view: mat4x4f,
    inverse_camera_projection: mat4x4f,
    wind_displacement: vec3f,
    shadow_right: vec3f,
    shadow_half_extent: f32,
    shadow_up: vec3f,
    shadow_opacity: f32,
    shadow_center: vec3f,
};

@group(0) @binding(0) var<uniform> config: Config;

@group(1) @binding(0) var clouds_render_texture: texture_storage_2d<rgba16float, read_write>;
@group(1) @binding(1) var clouds_atlas_texture: texture_storage_2d<rgba16float, read_write>;
@group(1) @binding(2) var clouds_worley_texture: texture_storage_3d<rgba16float, read_write>;
@group(1) @binding(3) var sky_texture: texture_storage_2d<rgba16float, read_write>;
@group(1) @binding(4) var history_texture: texture_2d<f32>;
@group(1) @binding(5) var ground_shadow_texture: texture_storage_2d<rgba16float, read_write>;

struct Ray {
    step_distance: f32,
    dir_length: f32,
    start: f32
}

struct RaymarchResult {
    dist: f32,
    color: vec4f,
}

fn atlas_texel(index: vec2i) -> vec4f {
    let size = vec2i(textureDimensions(clouds_atlas_texture));
    return textureLoad(clouds_atlas_texture, (index % size + size) % size);
}

fn sample_atlas(position: vec2f) -> vec4f {
    let size = vec2f(textureDimensions(clouds_atlas_texture));
    let uv = fract(position) * size - vec2f(0.5);
    let cell = vec2i(floor(uv));
    let f = fract(uv);
    return mix(
        mix(atlas_texel(cell), atlas_texel(cell + vec2i(1, 0)), f.x),
        mix(atlas_texel(cell + vec2i(0, 1)), atlas_texel(cell + vec2i(1, 1)), f.x), f.y);
}

// Cloud shape and weather over a flat map `q` of the ground.
fn cloud_map_planar(q: vec2f, normalized_height: f32) -> vec2f {
    let rotate = mat2x2f(0.8, 0.6, -0.6, 0.8);
    let weather_a = sample_atlas(q / 160000.0 + vec2f(0.13, 0.39)).a;
    let weather_b = sample_atlas(rotate * q / 57000.0 + vec2f(0.63, 0.17)).a;
    let weather = smoothstep(0.24, 0.73, weather_a * 0.65 + weather_b * 0.35);
    let warp = vec2f(weather_a - 0.5, weather_b - 0.5) * 9000.0;
    let uv = (q + warp) * (0.00005 * config.clouds_base_scale);
    let cloud = mix(sample_atlas(uv), sample_atlas(rotate * uv * 1.713 + vec2f(0.37, 0.61)), 0.28);
    let height = normalized_height / mix(0.65, 1.1, weather);
    let n = height * height * cloud.b + pow(1.0 - normalized_height, 16.0);
    return vec2f(common::remap(cloud.r - n, cloud.g, 1.0), weather);
}

// A flat map cannot wrap a globe: it smears where the ground turns away from
// it. So the map is laid from three sides and each place takes the side it
// faces. `p` is measured from the pole under the field and `up` points away
// from the planet's centre; around the field only the map from above counts,
// and the clouds there are what a single flat map gave.
fn cloud_map_base(p: vec3f, up: vec3f, normalized_height: f32) -> vec2f {
    var facing = pow(abs(up), vec3f(8.0));
    facing /= facing.x + facing.y + facing.z;
    var shape = vec2f(0.0);
    if facing.y > 0.004 { shape += facing.y * cloud_map_planar(p.xz, normalized_height); }
    if facing.x > 0.004 { shape += facing.x * cloud_map_planar(p.zy, normalized_height); }
    if facing.z > 0.004 { shape += facing.z * cloud_map_planar(p.xy, normalized_height); }
    return shape;
}

fn worley_texel(index: vec3i) -> f32 {
    let size = vec3i(textureDimensions(clouds_worley_texture));
    return textureLoad(clouds_worley_texture, (index % size + size) % size).r;
}

// The Worley volume read at a point, blended between its eight texels.
fn worley_at(p: vec3f) -> f32 {
    let cell = vec3i(floor(p));
    let f = fract(p);
    let lower = mix(
        mix(worley_texel(cell), worley_texel(cell + vec3i(1, 0, 0)), f.x),
        mix(worley_texel(cell + vec3i(0, 1, 0)), worley_texel(cell + vec3i(1, 1, 0)), f.x), f.y);
    let upper = mix(
        mix(worley_texel(cell + vec3i(0, 0, 1)), worley_texel(cell + vec3i(1, 0, 1)), f.x),
        mix(worley_texel(cell + vec3i(0, 1, 1)), worley_texel(cell + vec3i(1, 1, 1)), f.x), f.y);
    return mix(lower, upper, f.z);
}

// The volume is small and tiles, so one reading of it is a regular lattice of
// puffs, and a lattice seen in perspective draws rows that converge on the
// horizon. A second reading, turned in all three axes and scaled by the
// golden ratio, never lines up with the first, so their blend has no rows.
fn cloud_map_detail(position: vec3f) -> f32 {
    let p = position * (0.0016 * config.clouds_base_scale * config.clouds_detail_scale);
    let turned = mat3x3f(0.80, 0.36, -0.48, -0.36, 0.93, 0.10, 0.48, 0.10, 0.87);
    return mix(worley_at(p), worley_at(turned * p * 1.618034 + vec3f(11.3, 5.7, 17.1)), 0.5);
}

// Erode a bit from the clouds_bottom_height and clouds_top_height of the cloud layer
fn cloud_gradient(normalized_height: f32) -> f32 {
    return (
        common::linearstep(0.0, 0.1, normalized_height) -
        common::linearstep(0.8, 1.2, normalized_height)
    );
}

fn get_cloud_map_density(pos: vec3f, normalized_height: f32, sample_span: f32) -> f32 {
    if config.clouds_coverage <= 0.0 {
        return 0.0;
    }
    let ps = pos - vec3f(0.0, config.planet_radius, 0.0) - config.wind_displacement;
    let shape_position = ps + vec3f(normalized_height * 180.0, 0.0, normalized_height * 90.0);
    let base = cloud_map_base(shape_position, normalize(pos), normalized_height);
    var m = base.x * cloud_gradient(normalized_height);

	let clouds_detail_strength = (1.0 - smoothstep(0.5, 1.0, m));

    // Erode with detail
    if clouds_detail_strength > 0.0 {
        // Detail the march cannot resolve is dropped, not sampled: its finest
        // puffs are some 57 m across, and read at coarser steps than that they
        // alias into a ripple across the sky.
        let resolved_detail = 1.0 - smoothstep(16.0, 48.0, sample_span);
        var detail = 0.35;
        if resolved_detail > 0.0 {
            let evolution = config.time * vec3f(0.3, -0.2, 0.15);
            detail = mix(detail, cloud_map_detail(ps + evolution), resolved_detail);
        }
        m -= detail * clouds_detail_strength * config.clouds_detail_strength;
    }

	let weather_variation = 4.0 * config.clouds_coverage * (1.0 - config.clouds_coverage);
	let local_coverage = clamp(config.clouds_coverage + (base.y - 0.5) * 0.55 * weather_variation, 0.0, 1.0);
	m = smoothstep(0.0, config.clouds_base_edge_softness, m + local_coverage - 1.0);
    m *= common::linearstep0(config.clouds_bottom_softness, normalized_height);

    return clamp(m * config.clouds_density, 0.0, 1.0);
}

fn get_normalized_height(pos: vec3f) -> f32 {
    let clouds_height = config.clouds_top_height - config.clouds_bottom_height;
    return (length(pos) - (config.planet_radius + config.clouds_bottom_height)) / clouds_height;
}

fn volumetric_shadow(origin: vec3f, ray_dot_sun: f32) -> f32 {
    var ray_step_size = config.clouds_shadow_raymarch_step_size;
    var distance_along_ray = ray_step_size * 0.5;
    var transmittance = 1.0;

    for (var step: u32 = 0; step < config.clouds_shadow_raymarch_steps_count; step++) {
        let pos = origin + config.sun_dir.xyz * distance_along_ray;
        let normalized_height = get_normalized_height(pos);

        if (normalized_height > 1.0) { return transmittance; };

        let clouds_density = get_cloud_map_density(pos, normalized_height, ray_step_size);
        transmittance *= exp(-clouds_density * ray_step_size);

        ray_step_size *= config.clouds_shadow_raymarch_step_multiply;
        distance_along_ray += ray_step_size;
    }

    return transmittance;
}

fn intersect_planet_sphere(origin: vec3f, ray_dir: vec3f, height: f32) -> f32 {
    let radius = length(origin);
    let b = dot(origin, ray_dir);
    let c = (radius - config.planet_radius - height) * (radius + config.planet_radius + height);
    return -b + sqrt(max(b * b - c, 0.0));
}

fn henyey_greenstein(ray_dot_sun: f32, g: f32) -> f32 {
    let g_squared = g * g;
    return (1.0 - g_squared) / pow(1.0 + g_squared - 2.0 * g * ray_dot_sun, 1.5);
}

// Where a ray meets the sphere `height` above the ground: the near and far
// distances along it, the far one negative when it misses.
fn sphere_span(origin: vec3f, ray_dir: vec3f, height: f32) -> vec2f {
    let radius = length(origin);
    let b = dot(origin, ray_dir);
    let c = (radius - config.planet_radius - height) * (radius + config.planet_radius + height);
    let disc = b * b - c;
    if disc < 0.0 { return vec2f(1.0, -1.0); }
    let root = sqrt(disc);
    return vec2f(-b - root, -b + root);
}

// The stretch of a view ray inside the cloud shell, from an eye under it,
// within it or above it, out to orbit. The ground ends a ray that meets it.
fn get_ray(ray_origin: vec3f, ray_dir: vec3f, max_dist: f32) -> Ray {
    let none = Ray(0.0, max_dist + 1.0, max_dist + 1.0);
    let top = sphere_span(ray_origin, ray_dir, config.clouds_top_height);
    if top.y <= 0.0 { return none; }
    let base = sphere_span(ray_origin, ray_dir, config.clouds_bottom_height);
    let eye_height = length(ray_origin) - config.planet_radius;
    var start = 0.0;
    var end = top.y;
    if eye_height < config.clouds_bottom_height {
        let ground = sphere_span(ray_origin, ray_dir, 0.0);
        if ground.y > 0.0 && ground.x > 0.0 { return none; }
        start = base.y;
    } else {
        // Looking down it leaves through the base; otherwise through the top.
        if base.y > 0.0 && base.x > 0.0 { end = base.x; }
        if eye_height > config.clouds_top_height { start = top.x; }
    }
    start = max(start, 0.0);
    end = min(end, max_dist);
    let step_distance = max(end - start, 0.0) / f32(max(config.clouds_raymarch_steps_count, 1u));
    return Ray(step_distance, start + step_distance * 0.5, start);
}

fn raymarch(ray_origin: vec3f, ray_dir: vec3f, max_dist: f32) -> RaymarchResult {
    let ray = get_ray(ray_origin, ray_dir, max_dist);

    if (ray.start > max_dist) {
        return RaymarchResult(max_dist, vec4f(0.0, 0.0, 0.0, 1.0));
    }

    // Frostbite: dual-lobe phase function
    let ray_dot_sun = dot(ray_dir, config.sun_dir.xyz);
    let scattering = mix(
        henyey_greenstein(ray_dot_sun, config.forward_scattering_g),
        henyey_greenstein(ray_dot_sun, config.backward_scattering_g),
        config.scattering_lerp
    );

    var dir_length = ray.dir_length;
    var dist = max_dist;
    var scattered_light = vec3f(0.0, 0.0, 0.0);
    var transmittance = 1.0;

    for (var step: u32 = 0; step < config.clouds_raymarch_steps_count; step++) {
        let world_position = ray_origin + dir_length * ray_dir;

        let normalized_height = clamp(get_normalized_height(world_position), 0.0, 1.0);
        let pixel_span = dir_length * 2.0 * abs(config.inverse_camera_projection[1][1]) / max(config.render_resolution.y, 1.0);
        let clouds_density_sampled = get_cloud_map_density(world_position, normalized_height, max(ray.step_distance, pixel_span));

        if (clouds_density_sampled > 0.0) {
            dist = min(dist, dir_length);

            let ambient_light = mix(
                config.clouds_ambient_color_bottom,
                config.clouds_ambient_color_top,
                normalized_height
            );

            // Across a globe the sun stands lower than over the field, down to
            // night: light there falls off, and the planet shadows its far side.
            let local_sun = dot(normalize(world_position), config.sun_dir.xyz);
            let day = min(smoothstep(-0.1, 0.15, local_sun) / max(smoothstep(-0.1, 0.15, config.sun_dir.y), 0.001), 1.0);
            let toward = dot(world_position, config.sun_dir.xyz);
            let clearance = (length(world_position) - config.planet_radius) * (length(world_position) + config.planet_radius);
            let eclipsed = toward < 0.0 && toward * toward > clearance;
            let sunlight = select(volumetric_shadow(world_position, ray_dot_sun), 0.0, eclipsed);

            // Frostbite energy-conversing integration
            let S = clouds_density_sampled * day * (
                ambient_light.rgb +
                config.sun_color.rgb * scattering * sunlight
            );
            let delta_transmittance = exp(-clouds_density_sampled * ray.step_distance);
            let integrated_scattering = S * (1.0 - delta_transmittance) / clouds_density_sampled;

            scattered_light += transmittance * integrated_scattering;
            transmittance *= delta_transmittance;
        }

        if transmittance <= config.clouds_min_transmittance { break; }

        dir_length += ray.step_distance;
    }

    return RaymarchResult(dist, vec4f(scattered_light, transmittance));
}

fn get_sky_color(ray_dir: vec3f) -> vec3f {
    let mu = clamp(dot(ray_dir, config.sun_dir.xyz), 0.0, 1.0);
    let height = clamp(ray_dir.y, 0.0, 1.0);
    let horizon = pow(1.0 - height, 4.0);
    var col = mix(config.sky_zenith_color.rgb, config.sky_horizon_color.rgb, horizon);
    col += config.sun_color.rgb * (0.08 * pow(mu, 6.0) + 0.15 * pow(mu, 64.0) + 0.25 * pow(mu, 512.0));
    return col;
}

fn render_clouds_atlas(frag_coord: vec2f) -> vec4f {
    let v_uv = frag_coord / vec2f(textureDimensions(clouds_atlas_texture));
    let coord = vec3f(v_uv, 0.5);

    let mfbm = 0.9;
    let mvor = 0.7;

    return vec4f(
        mix(1.0, common::tilable_perlin_fbm(coord, 5, 4), mfbm) *
            mix(1.0, common::tilable_voronoi(coord, 4, 9.0), mvor),
        0.625 * common::tilable_voronoi(coord, 3, 15.0) +
            0.250 * common::tilable_voronoi(coord, 3, 19.0) +
            0.125 * common::tilable_voronoi(coord, 3, 23.0) -
            1.0,
        1.0 - common::tilable_voronoi(coord + 0.5, 4, 9.0),
        common::tilable_perlin_fbm(coord + vec3f(0.31, 0.17, 0.41), 4, 5.0)
    );
}

fn render_clouds_worley(coord: vec3f) -> vec4f {
    let r = common::tilable_voronoi(coord, 3, 3.0);
    let g = common::tilable_voronoi(coord, 1, 8.0);
    let b = common::tilable_voronoi(coord, 1, 12.0);

    let c = max(0.0, 1.0 - (r + g * 0.5 + b * 0.25) / 1.75);

    return vec4f(c);
}

fn get_clouds_color(pixel: vec2u, ray_dir: vec3f, ray_origin: vec3f) -> vec4f {
    let result = raymarch(ray_origin, ray_dir, MAX_DISTANCE);
    let transmittance = result.color.a;
    // Distant clouds fade into the air between; from above it there is little.
    let eye_height = max(length(ray_origin) - config.planet_radius, 0.0);
    let fog_factor = clamp(0.8 - exp(-4.0e-5 * result.dist), 0.0, 0.8) * exp(-eye_height / 6000.0);
    let color = vec4f(mix(result.color.rgb,
        get_sky_color(ray_dir) * (1.0 - transmittance), fog_factor), transmittance);
    let previous = textureLoad(history_texture, vec2i(pixel), 0);
    return mix(color, previous, config.reprojection_strength);
}

fn get_ray_origin() -> vec3f {
    return (
        config.camera_translation +
        vec3f(0.0, config.planet_radius, 0.0)
    );
}

fn get_ray_direction(frag_coord: vec2f) -> vec3f {
    // inverse_camera_projection is also called view_from_clip
    // inverse_camera_view is also called world_from_view
    let rect_relative = frag_coord / config.render_resolution;

    // Flip the Y co-ordinate from the clouds_top_height to the clouds_bottom_height to enter NDC.
    let ndc_xy = (rect_relative * 2.0 - vec2f(1.0, 1.0)) * vec2f(1.0, -1.0);

    let ray_clip = vec4f(ndc_xy.xy, 1.0, 1.0);
    let ray_eye = config.inverse_camera_projection * ray_clip;
    let ray_world = config.inverse_camera_view * vec4f(ray_eye.xyz, 0.0);

    return normalize(ray_world.xyz);
}

@compute @workgroup_size(8, 8, 1)
fn init(@builtin(global_invocation_id) id: vec3u) {
    if all(id.xy < textureDimensions(clouds_atlas_texture)) {
        textureStore(clouds_atlas_texture, id.xy, render_clouds_atlas(vec2f(id.xy) + vec2f(0.5)));
    }
    if id.x < 256u && id.y < 128u {
        let linear = id.x + id.y * 256u;
        let xyz = vec3u(linear % 32u, (linear / 32u) % 32u, linear / 1024u);
        textureStore(clouds_worley_texture, xyz, render_clouds_worley((vec3f(xyz) + vec3f(0.5)) / 32.0));
    }
}

@compute @workgroup_size(8, 8, 1)
fn update(@builtin(global_invocation_id) id: vec3u) {
    if any(id.xy >= textureDimensions(clouds_render_texture)) { return; }
    let pixel = vec2f(id.xy) + vec2f(0.5);
    let ray_origin = get_ray_origin();
    let ray_dir = get_ray_direction(pixel);
    let color = get_clouds_color(id.xy, ray_dir, ray_origin);
    textureStore(clouds_render_texture, id.xy, color);
    textureStore(sky_texture, id.xy, vec4f(get_sky_color(ray_dir), 1.0));
}

// The clouds' shadow as the sun sees it, for the sun's light texture. Each
// texel is one sun ray through a square that faces the sun and follows the
// camera, laid out as a directional light reads its texture. It marches the
// density the sky is drawn from, so a shadow lies under its cloud and drifts
// with it, and it takes the clouds' extinction as it is (Beer-Lambert): a
// cloud that hides the sun casts a full shadow and a gap casts none. Opacity
// is the artist's hand, applied after the physics.
@compute @workgroup_size(8, 8, 1)
fn ground_shadow(@builtin(global_invocation_id) id: vec3u) {
    let size = textureDimensions(ground_shadow_texture);
    if any(id.xy >= size) { return; }
    let uv = (vec2f(id.xy) + vec2f(0.5)) / vec2f(size);
    let across = (config.shadow_right * (1.0 - 2.0 * uv.x) + config.shadow_up * (2.0 * uv.y - 1.0))
        * config.shadow_half_extent;
    let origin = config.shadow_center + across + vec3f(0.0, config.planet_radius, 0.0);
    let sun = config.sun_dir.xyz;
    var optical_depth = 0.0;
    if sun.y > 0.02 {
        // Signed distances along the ray, so a ray whose texel lies above the
        // clouds is still marched through the layer behind it.
        let enter = intersect_planet_sphere(origin, sun, config.clouds_bottom_height);
        let leave = intersect_planet_sphere(origin, sun, config.clouds_top_height);
        let steps = 12u;
        let span = (leave - enter) / f32(steps);
        for (var step = 0u; step < steps; step++) {
            let pos = origin + sun * (enter + span * (f32(step) + 0.5));
            optical_depth += get_cloud_map_density(pos, get_normalized_height(pos), span) * span;
        }
    }
    // Towards the map's rim the shadows fade to full sun: past it the texture
    // repeats, and a shadow cut off at the seam would draw a straight line.
    let rim = smoothstep(0.0, 0.1, min(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y)));
    let shadow = 1.0 - config.shadow_opacity * rim * (1.0 - exp(-optical_depth));
    textureStore(ground_shadow_texture, vec2i(id.xy), vec4f(shadow, 0.0, 0.0, 1.0));
}
