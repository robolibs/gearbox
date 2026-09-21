// CDLOD cube-sphere terrain: vertex stage morphs the shared 129x129 grid
// between LOD levels and displaces it by the baked heightmap atlas; fragment
// stage computes finite-difference normals and slope/altitude PBR coloring.

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
    origin: vec2<f32>,       // node min corner in face uv [-1,1]
    scale: f32,              // node extent in face uv
    face_lod: u32,           // face | (lod << 3)
    morph_consts: vec2<f32>, // (morph start distance, 1/(end-start))
    atlas_layer: u32,
    morph_floor: f32, // reveal blend: freshly swapped-in nodes start at 1
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> tg: TerrainGlobals;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var heightmaps: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var heightmap_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var<storage, read> nodes: array<TerrainNode>;

const GRID_F: f32 = 128.0;
const TILE_TEXELS: f32 = 132.0;
const BORDER: f32 = 1.0;

const DEBUG_GRID: u32 = 1u;
const DEBUG_LOD_TINT: u32 = 2u;
const DEBUG_MORPH_HEAT: u32 = 4u;
const DEBUG_STATIC_TIME: u32 = 8u;

/// Animation clock — frozen under DEBUG_STATIC_TIME so verification captures
/// are reproducible.
fn anim_time() -> f32 {
    if (tg.debug_flags & DEBUG_STATIC_TIME) != 0u {
        return 0.0;
    }
    return globals.time;
}

fn face_basis(face: u32) -> mat3x3<f32> {
    // columns: tangent, bitangent, normal — must match tile_bake.wgsl / Rust.
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

// Grid coord (0..128, fractional ok) -> atlas uv. Texel centers coincide with
// integer grid coords, so integer g fetches exact baked values.
fn sample_height(layer: u32, g: vec2<f32>) -> f32 {
    let uv = (g + BORDER + 0.5) / TILE_TEXELS;
    return textureSampleLevel(heightmaps, heightmap_sampler, uv, layer, 0.0).r;
}

// Height -> displaced world position. Negative heights are real seabed;
// the translucent water surface at radius R is a separate mesh (water.wgsl).
fn terrain_pos(dir: vec3<f32>, h: f32) -> vec3<f32> {
    return dir * (tg.radius + h * tg.height_amp);
}

// Tiny 2D value noise for caustics/foam wobble (self-contained, cheap).
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

    var g = in.position.xy; // integer grid coords 0..128
    let skirt = in.position.z; // 1 on the skirt ring, 0 on the grid

    // CDLOD morph factor from the unmorphed vertex's camera distance.
    let uv_f = node.origin + (g / GRID_F) * node.scale;
    let dir_f = face_dir(basis, uv_f);
    let h_f = sample_height(node.atlas_layer, g);
    let p_f = terrain_pos(dir_f, h_f);
    let dist = distance(view.world_position, p_f);
    let morph = max(
        clamp((dist - node.morph_consts.x) * node.morph_consts.y, 0.0, 1.0),
        node.morph_floor,
    );

    // Snap odd vertices toward the even (parent-lattice) position.
    g = g - fract(g * 0.5) * 2.0 * morph;

    let uv = node.origin + (g / GRID_F) * node.scale;
    let dir = face_dir(basis, uv);
    let h = sample_height(node.atlas_layer, g);
    var p = terrain_pos(dir, h);
    // Skirt: drop the duplicated edge ring toward the planet center, deep
    // enough to cover boundary gaps from transient >1-LOD neighbors. Capped:
    // gaps are bounded by height variance (tens of meters), and uncapped
    // skirts on coarse tiles formed 100m+ walls visible edge-on at the
    // planet's limb — dark tile-shaped notches flickering at the ocean
    // horizon as LOD rings shifted.
    p -= dir * skirt * min(tg.radius * node.scale * 0.05 + 2.0, 15.0);

    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4(p, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
    // `dir` is planet-local; lighting needs it in world space (planets rotate
    // and sit far from the origin).
    out.world_normal = mesh_functions::mesh_normal_local_to_world(dir, in.instance_index);
#ifdef VERTEX_UVS_A
    out.uv = g / GRID_F; // morphed node-local coords, matches geometry
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = in.instance_index;
#endif
    return out;
}

fn lod_tint(lod: u32) -> vec3<f32> {
    let t = f32(lod) * 0.61803;
    return 0.55 + 0.45 * cos(6.2831 * (vec3(t, t, t) + vec3(0.0, 0.33, 0.67)));
}

/// How much of the sky's light reaches ground the sun does not. Small, but
/// never nought: a shadow is darker ground, not a hole.
const SKY_FILL: f32 = 0.22;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    let tag = mesh_functions::get_tag(in.instance_index);
    let node = nodes[tag];
    let face = node.face_lod & 7u;
    let lod = node.face_lod >> 3u;
    let basis = face_basis(face);

    let g = in.uv * GRID_F;
    let layer = node.atlas_layer;

    // Finite-difference normal (1-texel step; border texels exist for edges).
    let h_c = sample_height(layer, g);
    let h_x = sample_height(layer, g + vec2(1.0, 0.0));
    let h_y = sample_height(layer, g + vec2(0.0, 1.0));
    let uv_c = node.origin + (g / GRID_F) * node.scale;
    let step = node.scale / GRID_F;
    let dir_c = face_dir(basis, uv_c);
    let p_c = terrain_pos(dir_c, h_c);
    let p_x = terrain_pos(face_dir(basis, uv_c + vec2(step, 0.0)), h_x);
    let p_y = terrain_pos(face_dir(basis, uv_c + vec2(0.0, step)), h_y);
    var n_local = normalize(cross(p_x - p_c, p_y - p_c));
    if dot(n_local, dir_c) < 0.0 {
        n_local = -n_local;
    }
    // Biome math stays planet-local (rotation-invariant); lighting needs world.
    let n = normalize(mesh_functions::mesh_normal_local_to_world(n_local, in.instance_index));

    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.world_normal = n;
    pbr_input.N = n;

    // Slope/altitude coloring.
    let h = h_c;                      // raw height, [-1,1]-ish
    let slope = dot(n_local, dir_c);  // 1 flat .. 0 cliff
    // Snow caps the highest ground and the poles, not every hill: at 0.55 it
    // began below the height ordinary land reaches and the planet went white.
    let snow_line = 0.80 + 0.2 * dot(dir_c, vec3(0.0, 1.0, 0.0));

    let sand = vec3(0.55, 0.5, 0.34);
    let grass = vec3(0.1, 0.28, 0.06);
    let rock = vec3(0.28, 0.24, 0.21);
    let snow = vec3(0.9, 0.92, 0.95);
    let sediment = vec3(0.16, 0.17, 0.12);

    // Caustic/foam noise domain in PLANET-LOCAL space: stable regardless of
    // where the planet sits (and it rotates with the planet, as it should).
    let p2 = p_c.xy + p_c.z * vec2(0.53, 0.71);
    let t = anim_time();

    // One ground everywhere, the tone the ground square is seen as from a
    // distance. No sea, no snow, no rock line: the globe is the same surface
    // all the way round until something authors otherwise.
    // The same green the distant meadow backdrop was measured at from 60 km,
    // so the globe and the drawn ground it borders are one colour rather than
    // land running into bare soil at the join. Linear, in the range the covers
    // work in: set from an sRGB swatch it was three times too bright and every
    // lit face clipped to white.
    var color = vec3<f32>(0.058, 0.108, 0.015);

    // Debug overlays.
    if (tg.debug_flags & DEBUG_LOD_TINT) != 0u {
        color = mix(color, lod_tint(lod), 0.65);
    }
    if (tg.debug_flags & DEBUG_MORPH_HEAT) != 0u {
        let dist = distance(view.world_position, in.world_position.xyz);
        let m = clamp((dist - node.morph_consts.x) * node.morph_consts.y, 0.0, 1.0);
        color = mix(color, vec3(m, 0.1, 1.0 - m), 0.6);
    }
    if (tg.debug_flags & DEBUG_GRID) != 0u {
        let cell = abs(fract(g + 0.5) - 0.5);
        let w = fwidth(g) * 0.9;
        let line = 1.0 - min(min(cell.x / max(w.x, 1e-5), cell.y / max(w.y, 1e-5)), 1.0);
        color = mix(color, vec3(0.0, 0.0, 0.0), line * 0.75);
    }

    pbr_input.material.base_color = vec4(color, 1.0);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

    var out: FragmentOutput;
    // What the sun does not reach, the sky still does. Out here there is no air
    // between the eye and the ground to fill a shadow in, so a cloud's shadow
    // fell to nothing and read as a hole cut in the planet.
    out.color = apply_pbr_lighting(pbr_input) + vec4<f32>(color * SKY_FILL, 0.0);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
