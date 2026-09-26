// Draws the meadow's sown blades: one instance per record from
// `blade_cull.wgsl`. The vertex shader only bends the strip; everything that
// is the same along a blade was worked out once when it was sown.

#import bevy_pbr::{
    mesh_view_bindings::view,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    pbr_types::{pbr_input_new, STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT},
    pbr_functions::calculate_view,
}
#import "embedded://gearbox_fields/shaders/surface_detail.wgsl"::{foliage_normal, plant_lighting}

@group(3) @binding(0) var<storage, read> records: array<u32>;

const RECORD_WORDS: u32 = 16u;

// The strip: position.x is the side (-1, 0, 1), position.y the row's height
// fraction at the finest detail level.
struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) @invariant clip_position: vec4<f32>,
    @location(0) @interpolate(perspective, centroid) world_position: vec4<f32>,
    @location(1) @interpolate(perspective, centroid) world_normal: vec3<f32>,
    @location(2) @interpolate(perspective, centroid) color: vec4<f32>,
    @location(3) @interpolate(perspective, centroid) blade_uv: vec2<f32>,
    @location(4) @interpolate(perspective, centroid) ground_normal: vec3<f32>,
};

fn ease_out(x: f32, power: f32) -> f32 {
    return 1.0 - pow(1.0 - x, power);
}

// Quadratic Bezier from the root (origin) through `mid` to `tip`.
fn bezier(mid: vec3<f32>, tip: vec3<f32>, t: f32) -> vec3<f32> {
    return 2.0 * (1.0 - t) * t * mid + t * t * tip;
}

fn bezier_tangent(mid: vec3<f32>, tip: vec3<f32>, t: f32) -> vec3<f32> {
    return 2.0 * (1.0 - t) * mid + 2.0 * t * (tip - mid);
}

fn half2(at: u32) -> vec2<f32> {
    return unpack2x16float(records[at]);
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    let o = vertex.instance_index * RECORD_WORDS;
    let root = vec3<f32>(bitcast<f32>(records[o]), bitcast<f32>(records[o + 1u]), bitcast<f32>(records[o + 2u]));
    let width = bitcast<f32>(records[o + 3u]);
    let m = half2(o + 4u);
    let mt = half2(o + 5u);
    let tp = half2(o + 6u);
    let mid = vec3<f32>(m.x, m.y, mt.x);
    let tip = vec3<f32>(mt.y, tp.x, tp.y);
    let facing = half2(o + 7u);
    let roll_xz = half2(o + 8u);
    let pf = half2(o + 9u);
    let slope = half2(o + 10u);
    let c0 = half2(o + 11u);
    let c1 = half2(o + 12u);
    let c2 = half2(o + 13u);
    let segments = f32(max(records[o + 14u], 1u));
    let roll = vec3<f32>(roll_xz.x, 0.0, roll_xz.y);
    let press = pf.x;
    let flat = pf.y;
    let ground_normal = normalize(vec3<f32>(slope.x, 1.0, slope.y));
    let base_colour = vec3<f32>(c0.x, c0.y, c1.x);
    let tip_colour = vec3<f32>(c1.y, c2.x, c2.y);

    // Rows collapse onto the blade's own segments.
    let row = round(vertex.position.y * 4.0);
    let t = floor(row * segments / 4.0) / segments;
    let side = vertex.position.x;
    let up = vec3<f32>(0.0, 1.0, 0.0);
    let distance = length(root - view.world_position);

    let axis = normalize(bezier_tangent(mid, tip, t) + up * 1e-4);
    let stand_right = cross(axis, vec3<f32>(facing.x, 0.0, facing.y));
    let lay = cross(up, roll);
    let lay_right = select(-lay, lay, dot(lay, stand_right) >= 0.0);
    let right = normalize(mix(stand_right, lay_right, press) + vec3<f32>(1e-5, 0.0, 0.0));
    let normal = normalize(cross(right, axis));
    // Wide at the root, tapering fast to the tip.
    let half_w = width * 0.5 * ease_out(1.0 - t, 2.0) * side;
    let p = root + bezier(mid, tip, t) + right * half_w;

    // View-space thickening: a blade edge-on to the view is widened along
    // the screen, easing off right at edge-on.
    let to_eye = normalize(view.world_position - p);
    let face_xz = normalize(vec2<f32>(normal.x, normal.z) + vec2<f32>(1e-5, 0.0));
    let eye_xz = normalize(vec2<f32>(to_eye.x, to_eye.z) + vec2<f32>(1e-5, 0.0));
    let facing_eye = abs(dot(face_xz, eye_xz));
    let thicken = ease_out(1.0 - facing_eye, 4.0) * smoothstep(0.0, 0.2, facing_eye);
    var view_pos = view.view_from_world * vec4<f32>(p, 1.0);
    let right_view = (view.view_from_world * vec4<f32>(right, 0.0)).x;
    view_pos.x += thicken * select(-1.0, 1.0, right_view >= 0.0) * half_w;

    // Normals: tilted outward per half for a rounded blade, mostly sky
    // facing, settling onto the ground normal with distance.
    let rounded = normalize(normal + right * side * 0.6);
    let sky = normalize(mix(ground_normal, rounded, 0.35));
    let settle = smoothstep(8.0, 40.0, distance);

    // Dark roots to bright tips; past a few metres a blade settles onto its
    // average colour.
    let occlusion = mix(0.5, 1.0, t * t);
    let ramp = mix(base_colour, tip_colour, t * t) * occlusion;
    let average = mix(base_colour, tip_colour, 0.35) * 0.7;
    let colour = mix(ramp, average, smoothstep(4.0, 20.0, distance));

    var out: VertexOutput;
    out.world_position = vec4<f32>(p, 1.0);
    out.clip_position = view.clip_from_view * view_pos;
    out.world_normal = normalize(mix(mix(sky, ground_normal, settle * 0.8), ground_normal, flat));
    out.ground_normal = ground_normal;
    out.blade_uv = vec2<f32>(side, t);
    out.color = vec4<f32>(colour, 1.0);
    return out;
}

// The depth-only draw ahead of the lit one: colour writes are masked off.
@fragment
fn depth_only() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var pbr_input = pbr_input_new();
    // Matte, the edges darker than the midrib; translucency grows towards
    // the tip.
    let rib = mix(1.0, 0.85, smoothstep(0.1, 0.9, abs(in.blade_uv.x)));
    pbr_input.material.base_color = vec4<f32>(in.color.rgb * rib, 1.0);
    pbr_input.material.perceptual_roughness = 0.92;
    pbr_input.material.metallic = 0.0;
    pbr_input.material.reflectance = vec3<f32>(0.0);
    pbr_input.specular_occlusion = 0.0;
    pbr_input.material.diffuse_transmission = mix(0.3, 0.5, in.blade_uv.y);
    pbr_input.material.thickness = 0.0002;
    pbr_input.material.flags = pbr_input.material.flags | STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT;
    pbr_input.frag_coord = in.clip_position;
    pbr_input.world_position = in.world_position;
    pbr_input.world_normal = normalize(in.ground_normal);
    pbr_input.V = calculate_view(in.world_position, false);
    pbr_input.N = foliage_normal(in.world_normal, pbr_input.world_normal, pbr_input.V);
    pbr_input.flags = MESH_FLAGS_SHADOW_RECEIVER_BIT;
    var color = plant_lighting(pbr_input);
    color.a = 1.0;
    return color;
}
