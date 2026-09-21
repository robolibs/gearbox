// Translucent ocean surface: the same CDLOD-morphed node grid as the terrain,
// but pinned to sea level (radius R). Fragment shading tints by water depth
// (sampled from the shared heightmap atlas), animates a wave normal, and uses
// the standard PBR epilogue so distance fog applies underwater.

#import bevy_pbr::{
    forward_io::{Vertex, VertexOutput, FragmentOutput},
    mesh_functions,
    mesh_view_bindings::{view, globals},
    view_transformations::position_world_to_clip,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
}

struct TerrainGlobals {
    radius: f32,
    height_amp: f32,
    debug_flags: u32,
    _pad: u32,
}

struct TerrainNode {
    origin: vec2<f32>,
    scale: f32,
    face_lod: u32,
    morph_consts: vec2<f32>,
    atlas_layer: u32,
    morph_floor: f32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> tg: TerrainGlobals;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var heightmaps: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var heightmap_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var<storage, read> nodes: array<TerrainNode>;

const DEBUG_STATIC_TIME: u32 = 8u;

/// Animation clock — frozen under DEBUG_STATIC_TIME so verification captures
/// are reproducible.
fn anim_time() -> f32 {
    if (tg.debug_flags & DEBUG_STATIC_TIME) != 0u {
        return 0.0;
    }
    return globals.time;
}

const GRID_F: f32 = 128.0;
const TILE_TEXELS: f32 = 132.0;
const BORDER: f32 = 1.0;

fn face_basis(face: u32) -> mat3x3<f32> {
    // Must match terrain.wgsl / tile_bake.wgsl / cube_sphere.rs.
    switch face {
        case 0u: { return mat3x3(vec3(0.0, 0.0, -1.0), vec3(0.0, 1.0, 0.0), vec3(1.0, 0.0, 0.0)); }
        case 1u: { return mat3x3(vec3(0.0, 0.0, 1.0), vec3(0.0, 1.0, 0.0), vec3(-1.0, 0.0, 0.0)); }
        case 2u: { return mat3x3(vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, -1.0), vec3(0.0, 1.0, 0.0)); }
        case 3u: { return mat3x3(vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0), vec3(0.0, -1.0, 0.0)); }
        case 4u: { return mat3x3(vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)); }
        default: { return mat3x3(vec3(-1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, -1.0)); }
    }
}

fn face_dir(basis: mat3x3<f32>, uv: vec2<f32>) -> vec3<f32> {
    return normalize(basis[2] + uv.x * basis[0] + uv.y * basis[1]);
}

fn sample_height(layer: u32, g: vec2<f32>) -> f32 {
    let uv = (g + BORDER + 0.5) / TILE_TEXELS;
    return textureSampleLevel(heightmaps, heightmap_sampler, uv, layer, 0.0).r;
}

fn hash21(p: vec2<f32>) -> f32 {
    var q = fract(p * vec2(123.34, 456.21));
    q += dot(q, q + 45.32);
    return fract(q.x * q.y);
}

fn vnoise2(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(hash21(i), hash21(i + vec2(1.0, 0.0)), u.x),
        mix(hash21(i + vec2(0.0, 1.0)), hash21(i + vec2(1.0, 1.0)), u.x),
        u.y,
    );
}

@vertex
fn vertex(in: Vertex) -> VertexOutput {
    let tag = mesh_functions::get_tag(in.instance_index);
    let node = nodes[tag];
    let face = node.face_lod & 7u;
    let basis = face_basis(face);

    var g = in.position.xy;
    // Skirt verts collapse onto the surface (no drop): a smooth sphere needs
    // no skirts, and dropped translucent walls would show underwater.

    // Same CDLOD morph as the terrain so adjacent water tiles share edge
    // vertices exactly; height is identically zero.
    let uv_f = node.origin + (g / GRID_F) * node.scale;
    let dir_f = face_dir(basis, uv_f);
    let p_f = dir_f * tg.radius;
    let dist = distance(view.world_position, p_f);
    let morph = max(
        clamp((dist - node.morph_consts.x) * node.morph_consts.y, 0.0, 1.0),
        node.morph_floor,
    );
    g = g - fract(g * 0.5) * 2.0 * morph;

    let uv = node.origin + (g / GRID_F) * node.scale;
    let dir = face_dir(basis, uv);
    let p = dir * tg.radius;

    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4(p, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
    out.world_normal = mesh_functions::mesh_normal_local_to_world(dir, in.instance_index);
#ifdef VERTEX_UVS_A
    out.uv = g / GRID_F;
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = in.instance_index;
#endif
    return out;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    let tag = mesh_functions::get_tag(in.instance_index);
    let node = nodes[tag];
    let basis = face_basis(node.face_lod & 7u);

    let g = in.uv * GRID_F;
    let h = sample_height(node.atlas_layer, g);
    let depth_m = max(-h, 0.0) * tg.height_amp;

    // Reconstruct the sphere normal from the cube face (planet-local) rather
    // than normalizing the world position — the planet is not at the origin.
    let uv_c = node.origin + (g / GRID_F) * node.scale;
    let dir_c = face_dir(basis, uv_c);
    let p_local = dir_c * tg.radius;
    let sphere_n = normalize(mesh_functions::mesh_normal_local_to_world(dir_c, in.instance_index));
    let cam_dist = distance(view.world_position, in.world_position.xyz);

    // Animated wave normal: four directional sine waves (smooth everywhere,
    // unlike cell noise) perturbing the sphere normal in the local tangent
    // frame, faded out with distance.
    let p2 = p_local.xy + p_local.z * vec2(0.53, 0.71);
    let t = anim_time();
    let d1 = vec2(0.80, 0.60);
    let d2 = vec2(-0.55, 0.83);
    let d3 = vec2(0.30, -0.95);
    let d4 = vec2(-0.90, -0.44);
    let ph1 = dot(p2, d1) * 0.55 + t * 1.4;
    let ph2 = dot(p2, d2) * 0.90 - t * 1.1;
    let ph3 = dot(p2, d3) * 1.70 + t * 2.0;
    let ph4 = dot(p2, d4) * 3.10 + t * 2.6;
    let slope2 = d1 * cos(ph1) * 0.45 + d2 * cos(ph2) * 0.30
        + d3 * cos(ph3) * 0.18 + d4 * cos(ph4) * 0.10;
    let wave_h = sin(ph1) * 0.45 + sin(ph2) * 0.30 + sin(ph3) * 0.18 + sin(ph4) * 0.10;
    let wave_amp = 0.35 * (1.0 - smoothstep(200.0, 4000.0, cam_dist));
    // Wave frame built in planet-local space, then rotated to world.
    let tangent = normalize(cross(vec3(0.0, 1.0, 0.001), dir_c));
    let bitangent = cross(dir_c, tangent);
    let n_local = normalize(dir_c + (tangent * slope2.x + bitangent * slope2.y) * wave_amp);
    var n = normalize(mesh_functions::mesh_normal_local_to_world(n_local, in.instance_index));
    if !is_front {
        // Overwriting pbr_input.N bypasses the automatic double-sided flip.
        n = -n;
    }

    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.world_normal = n;
    pbr_input.N = n;

    // Depth-based color and opacity: turquoise shallows -> dark deep blue.
    let shallow = vec3(0.1, 0.42, 0.45);
    let deep = vec3(0.012, 0.09, 0.18);
    var color = mix(shallow, deep, smoothstep(0.0, 60.0, depth_m));
    var alpha = mix(0.35, 0.85, smoothstep(0.0, 45.0, depth_m));
    // Fresnel: grazing angles reflect more (more opaque).
    let view_dir = normalize(view.world_position - in.world_position.xyz);
    let fresnel = pow(1.0 - abs(dot(view_dir, n)), 3.0);
    alpha = clamp(alpha + fresnel * 0.35, 0.0, 0.92);
    if is_front {
        // Fake sky reflection at grazing angles (there is no environment map
        // to reflect; without this the horizon water mirrors black space).
        color = mix(color, vec3(0.4, 0.58, 0.72), fresnel * 0.55);
    }
    if !is_front {
        // Seen from below: the surface reads darker and more mirror-like.
        alpha = clamp(alpha + 0.08, 0.0, 0.95);
        // Fake transmitted daylight (Snell window): brightest straight
        // overhead, shimmering with the wave noise. Emissive so it survives
        // the dim underwater lighting; fog still extinguishes it at range.
        // Scaled for the HDR pipeline, where lit surfaces sit in the
        // thousands (cd/m^2-ish), not 0..1.
        let up = clamp(dot(normalize(in.world_position.xyz - view.world_position), sphere_n), 0.0, 1.0);
        let shimmer = 0.75 + 0.55 * wave_h;
        pbr_input.material.emissive += vec4(
            vec3(0.2, 0.42, 0.48) * 8.0 * (0.1 + 0.9 * pow(up, 2.0)) * shimmer,
            0.0,
        );
    }

    // Debug (F1 grid bit): unmistakable opaque magenta — shows exactly where
    // water fragments are actually rasterized.
    if (tg.debug_flags & 1u) != 0u {
        color = vec3(1.0, 0.0, 1.0);
        alpha = 1.0;
    }

    pbr_input.material.base_color = vec4(color, alpha);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
