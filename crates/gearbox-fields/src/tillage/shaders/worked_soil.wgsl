// Worked soil raised as geometry: one patch a square metre per instance,
// laid edge to edge over the chunk and lifted into loosened, ridged, cloddy
// soil wherever the work map says a tool cut the ground. Patches with nothing
// worked under them, or outside their own detail band, are dropped whole.

#import bevy_pbr::{
    mesh_view_bindings::view,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    pbr_types::{pbr_input_new, STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing, calculate_view},
}
#import "embedded://gearbox_fields/shaders/interaction.wgsl"::WheelMapParams
#import "embedded://gearbox_fields/tillage/shaders/work.wgsl"::{Work, work_at, soil_relief, soil_colour, ridge_slope, crumb_slope}

struct VegetationParams {
    corner: vec2<f32>,
    origin: vec2<f32>,
    texels_per_metre: f32,
    chunk_size: f32,
    fade_start: f32,
    fade_end: f32,
    blades_per_chunk: f32,
    texel_count: f32,
    inverse_square_thinning: u32,
    bounds: vec4<f32>,
    wheels: WheelMapParams,
    wind: vec4<f32>,
    follow_grass: f32,
    tread: vec4<f32>,
    soft_border: f32,
    way: mat4x4<f32>,
    way_more: mat4x4<f32>,
    way_shape: vec4<f32>,
};

@group(3) @binding(0) var heightmap: texture_2d<f32>;
@group(3) @binding(1) var heightmap_sampler: sampler;
@group(3) @binding(2) var<uniform> field: VegetationParams;
@group(3) @binding(4) var soil_albedo: texture_2d<f32>;
@group(3) @binding(5) var soil_sampler: sampler;
@group(3) @binding(8) var work_map: texture_2d<u32>;

// The patch template: x and z run 0..1 across its square metre; uv holds the
// camera band this level of detail draws in.
struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    // The way the tool went, and where across it this point lay.
    @location(2) along_across: vec3<f32>,
    // Height of the relief above the cut ground, how much of it is ridge, and
    // how worked the point is.
    @location(3) relief: vec3<f32>,
};

// Below the ground a patch's unworked edge sinks to, out of sight under it.
const BURIED_M: f32 = 0.04;

fn culled() -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(0.0, 0.0, -2.0, 1.0);
    return out;
}

// Bilinear read of (height, normal.x, normal.z) at a world XZ position.
fn ground_at(world_xz: vec2<f32>) -> vec3<f32> {
    let t = (world_xz - field.origin) * field.texels_per_metre;
    let max_index = i32(field.texel_count) - 1;
    let i = clamp(vec2<i32>(floor(t)), vec2<i32>(0), vec2<i32>(max_index - 1));
    let f = clamp(t - vec2<f32>(i), vec2<f32>(0.0), vec2<f32>(1.0));
    let s00 = textureLoad(heightmap, i, 0).xyz;
    let s10 = textureLoad(heightmap, i + vec2<i32>(1, 0), 0).xyz;
    let s01 = textureLoad(heightmap, i + vec2<i32>(0, 1), 0).xyz;
    let s11 = textureLoad(heightmap, i + vec2<i32>(1, 1), 0).xyz;
    return mix(mix(s00, s10, f.x), mix(s01, s11, f.x), f.y);
}

// Whether any tool worked the texel nearest a point.
fn worked_near(world_xz: vec2<f32>) -> bool {
    let size = vec2<i32>(textureDimensions(work_map));
    let t = vec2<i32>(round((world_xz - field.wheels.origin) * field.wheels.texels_per_metre));
    if (any(t < vec2<i32>(0)) || any(t >= size)) {
        return false;
    }
    return (textureLoad(work_map, t, 0).x >> 12u) != 0u;
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    let side = max(u32(round(field.chunk_size)), 1u);
    let cell = vec2<f32>(f32(vertex.instance_index % side), f32(vertex.instance_index / side));
    let cell_min = field.corner + cell;
    let middle = cell_min + vec2<f32>(0.5);
    if (any(middle < field.bounds.xy) || any(middle > field.bounds.zw)) {
        return culled();
    }
    // Decided per patch, from its middle, so every vertex of one patch agrees
    // and no triangle is left half dropped.
    let middle_ground = ground_at(middle);
    let eye = view.world_position;
    let patch_distance = length(vec3<f32>(middle.x, middle_ground.x, middle.y) - eye);
    if (patch_distance < vertex.uv.x || patch_distance >= vertex.uv.y) {
        return culled();
    }
    var any_worked = false;
    for (var y = 0; y <= 2; y++) {
        for (var x = 0; x <= 2; x++) {
            any_worked = any_worked || worked_near(cell_min + vec2<f32>(f32(x), f32(y)) * 0.5);
        }
    }
    if (!any_worked) {
        return culled();
    }

    let world_xz = cell_min + vertex.position.xz;
    let ground = ground_at(world_xz);
    let work = work_at(work_map, field.wheels, world_xz);
    let near = vertex.uv.x <= 0.0;
    let detail = select(0.35, 1.0, near);
    let show = smoothstep(0.0, 0.6, work.amount);
    let relief = soil_relief(work, world_xz);
    // The far level holds the loosened lift and only some of the ridging:
    // its grid is too coarse to carry a clod.
    let lift = mix(0.055, relief.x, detail);
    let height = ground.x + mix(-BURIED_M, lift, show);

    // The ground's normal tilted by the ridges; the clods' own faces are
    // found per pixel, from the triangles they shape.
    let slope = ridge_slope(work) * show * detail;
    let ground_normal = normalize(vec3<f32>(ground.y, 1.0, ground.z));
    let normal = normalize(ground_normal + vec3<f32>(-slope.x, 0.0, -slope.y));

    var out: VertexOutput;
    out.world_position = vec4<f32>(world_xz.x, height, world_xz.y, 1.0);
    out.clip_position = view.clip_from_world * out.world_position;
    out.world_normal = normal;
    out.along_across = vec3<f32>(work.along, work.across);
    out.relief = vec3<f32>(lift, relief.y, work.amount);
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let world_xz = in.world_position.xz;
    var work: Work;
    work.amount = in.relief.z;
    work.kind = 1u;
    work.along = normalize(in.along_across.xy + vec2<f32>(1.0e-5, 0.0));
    work.across = in.along_across.z;
    work.depth = 0.0;
    // The soil the stubble stood in, tiled every metre and a half and broken
    // up by a second, turned read of itself.
    let uv = world_xz / 1.5;
    let base = mix(
        textureSample(soil_albedo, soil_sampler, uv).rgb,
        textureSample(soil_albedo, soil_sampler, vec2<f32>(-uv.y, uv.x) * 0.61 + 0.37).rgb,
        0.4);
    let colour = soil_colour(base, work, world_xz, in.relief.xy);

    // Near to, each clod's faces turn to the sun as the triangles shaping it
    // do, and crumb finer than any vertex roughens them.
    let eye_distance = length(in.world_position.xyz - view.world_position);
    let near = 1.0 - smoothstep(6.0, 16.0, eye_distance);
    var facet = normalize(cross(dpdy(in.world_position.xyz), dpdx(in.world_position.xyz)));
    facet = select(facet, -facet, facet.y < 0.0);
    var normal = normalize(mix(normalize(in.world_normal), facet, 0.65 * near));
    let crumb = crumb_slope(world_xz) * near;
    normal = normalize(normal + vec3<f32>(-crumb.x, 0.0, -crumb.y));

    var pbr_input = pbr_input_new();
    pbr_input.material.base_color = vec4<f32>(colour, 1.0);
    pbr_input.material.perceptual_roughness = 0.96;
    pbr_input.material.metallic = 0.0;
    pbr_input.material.reflectance = vec3<f32>(0.02);
    pbr_input.material.flags = pbr_input.material.flags | STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT;
    pbr_input.frag_coord = in.clip_position;
    pbr_input.world_position = in.world_position;
    pbr_input.world_normal = normalize(in.world_normal);
    pbr_input.N = normal;
    pbr_input.V = calculate_view(in.world_position, false);
    pbr_input.flags = MESH_FLAGS_SHADOW_RECEIVER_BIT;
    var color = apply_pbr_lighting(pbr_input);
    color = main_pass_post_lighting_processing(pbr_input, color);
    return vec4<f32>(color.rgb, 1.0);
}
