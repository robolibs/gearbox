#import bevy_pbr::mesh_view_bindings::view
#import gearbox::draw_common::{srgb_to_linear, display_to_scene}
#import gearbox::draw_frost::background_frost

// Compact instance records: an occupancy cube per record (default), an oriented
// surfel disc per record (OCTREE_DISCS), or three vertex records per mesh
// triangle drawn without instancing (OCTREE_TRIANGLES).
struct OctreeCell {
    center: vec3<f32>,
    half: f32,
    rgba: u32,
    code: u32,
    probability: f32,
    spare: u32,
};

struct OctreeUniform {
    world_from_local: mat4x4<f32>,
    // x opacity, y glow, z style (0 solid, 1 surface, 2 wire), w edge width px.
    params: vec4<f32>,
    // x blur, y 1 when colours come from `colors` rather than the records.
    extra: vec4<f32>,
};

@group(2) @binding(0) var<storage, read> cells: array<OctreeCell>;
@group(2) @binding(1) var<storage, read> colors: array<u32>;
@group(3) @binding(0) var<uniform> octree: OctreeUniform;
@group(3) @binding(1) var lut_texture: texture_3d<f32>;
@group(3) @binding(2) var lut_sampler: sampler;

const AGGREGATE: u32 = 65536u;
const IMPLICIT: u32 = 131072u;
const NORMAL_MISSING: u32 = 262144u;
const ARROW: u32 = 524288u;
// Six bits from here: cube faces a same-state neighbour covers, dropped outside the clip volume.
const HIDDEN_SHIFT: u32 = 23u;
const FREE: u32 = 1u;

struct OctreeOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec3<f32>,
    @location(1) @interpolate(flat) rgba: vec4<f32>,
    @location(2) @interpolate(flat) face: u32,
    @location(3) @interpolate(flat) code: u32,
    @location(4) normal: vec3<f32>,
    @location(5) world: vec3<f32>,
};

fn face_normal(face: u32) -> vec3<f32> {
    let axis = face / 2u;
    let sign = select(1.0, -1.0, (face & 1u) == 1u);
    var n = vec3(0.0);
    n[axis] = sign;
    return n;
}

fn cube_corner(vertex_index: u32) -> vec3<f32> {
    let face = vertex_index / 6u;
    let corner = vertex_index % 6u;
    let uv = array(vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0),
                   vec2(-1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0))[corner];
    let axis = face / 2u;
    let n = face_normal(face);
    var p = n;
    p[(axis + 1u) % 3u] = uv.x;
    p[(axis + 2u) % 3u] = uv.y;
    return p;
}

fn unpack_rgba(value: u32) -> vec4<f32> {
    return vec4(f32(value & 255u), f32((value >> 8u) & 255u), f32((value >> 16u) & 255u),
                f32((value >> 24u) & 255u)) / 255.0;
}

fn sign_of(v: f32) -> f32 {
    return select(-1.0, 1.0, v >= 0.0);
}

fn oct_decode(value: u32) -> vec3<f32> {
    let x = f32(value & 65535u) / 65535.0 * 2.0 - 1.0;
    let y = f32(value >> 16u) / 65535.0 * 2.0 - 1.0;
    let z = 1.0 - abs(x) - abs(y);
    let t = max(-z, 0.0);
    return normalize(vec3(x - t * sign_of(x), y - t * sign_of(y), z));
}

fn scene_color(value: u32) -> vec4<f32> {
    let color = unpack_rgba(value);
    return vec4(display_to_scene(srgb_to_linear(color.rgb), lut_texture, lut_sampler), color.a);
}

@vertex
fn vertex(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) instance_index: u32) -> OctreeOutput {
    var out: OctreeOutput;
#ifdef OCTREE_TRIANGLES
    let cell = cells[vertex_index];
    let world = octree.world_from_local * vec4(cell.center, 1.0);
    out.local = array(vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0))[vertex_index % 3u];
    out.normal = normalize((octree.world_from_local * vec4(oct_decode(cell.spare), 0.0)).xyz);
#else
#ifdef OCTREE_DISCS
    let cell = cells[instance_index];
    let center = (octree.world_from_local * vec4(cell.center, 1.0)).xyz;
    var n = normalize((octree.world_from_local * vec4(oct_decode(cell.spare), 0.0)).xyz);
    if (cell.code & NORMAL_MISSING) != 0u {
        n = normalize(view.world_position - center);
    }
    let helper = select(vec3(0.0, 1.0, 0.0), vec3(1.0, 0.0, 0.0), abs(n.y) > 0.9);
    let t1 = normalize(cross(n, helper));
    let t2 = cross(n, t1);
    let corner = array(vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0),
                       vec2(-1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0))[vertex_index];
    let scale = length(octree.world_from_local[0].xyz);
    let world = vec4(center + (t1 * corner.x + t2 * corner.y) * cell.half * scale, 1.0);
    out.local = vec3(corner, 0.0);
    out.normal = n;
#else
    let cell = cells[instance_index];
    if ((cell.code >> HIDDEN_SHIFT) & (1u << (vertex_index / 6u))) != 0u {
        out.position = vec4(0.0, 0.0, 2.0, 1.0);
        return out;
    }
    let local = cube_corner(vertex_index);
    let world = octree.world_from_local * vec4(cell.center + local * cell.half, 1.0);
    out.local = local;
    out.normal = normalize((octree.world_from_local * vec4(face_normal(vertex_index / 6u), 0.0)).xyz);
#endif
#endif
    out.position = view.clip_from_world * world;
    out.world = world.xyz;
    out.face = vertex_index / 6u;
    out.code = cell.code;
    var rgba = cell.rgba;
    if octree.extra.y > 0.5 {
#ifdef OCTREE_TRIANGLES
        rgba = colors[vertex_index];
#else
        rgba = colors[instance_index];
#endif
    }
    out.rgba = scene_color(rgba);
    return out;
}

// Second-largest of |local|: 1 on a cube edge, smaller inside a face.
fn edge_coordinate(local: vec3<f32>) -> f32 {
    let a = abs(local);
    let hi = max(a.x, max(a.y, a.z));
    let lo = min(a.x, min(a.y, a.z));
    return a.x + a.y + a.z - hi - lo;
}

/// Two-sided light from above with a little side fill.
fn lighting(n: vec3<f32>) -> f32 {
    return 0.58 + 0.30 * abs(n.y) + 0.12 * abs(n.x);
}

@fragment
fn fragment(in: OctreeOutput) -> @location(0) vec4<f32> {
    let style = octree.params.z;
    var alpha = in.rgba.a * octree.params.x;
    var rgb = in.rgba.rgb;
    var edge = 0.0;
    var free = false;
#ifdef OCTREE_DISCS
    let r = length(in.local.xy);
    if r > 1.0 { discard; }
    let width = max(fwidth(r), 1e-5) * octree.params.w;
    edge = smoothstep(1.0 - width, 1.0, r);
    rgb = rgb * lighting(in.normal);
    if style > 1.5 {
        alpha = alpha * edge;
        rgb = in.rgba.rgb;
    } else if style > 0.5 {
        rgb = mix(rgb, in.rgba.rgb * 1.15, edge * 0.6);
    }
#else
#ifdef OCTREE_TRIANGLES
    let low = min(in.local.x, min(in.local.y, in.local.z));
    let width = max(fwidth(low), 1e-5) * octree.params.w;
    edge = 1.0 - smoothstep(0.0, width, low);
    if (in.code & ARROW) == 0u {
        var n = in.normal;
        if (in.code & NORMAL_MISSING) != 0u {
            n = normalize(cross(dpdx(in.world), dpdy(in.world)));
        }
        rgb = rgb * lighting(n);
        if style > 1.5 {
            alpha = alpha * edge;
            rgb = in.rgba.rgb;
        } else if style > 0.5 {
            rgb = mix(rgb, in.rgba.rgb * 1.15, edge * 0.6);
        }
    }
#else
    let m = edge_coordinate(in.local);
    let width = max(fwidth(m), 1e-5) * octree.params.w;
    edge = smoothstep(1.0 - width, 1.0, m);
    rgb = rgb * (0.58 + 0.30 * max(in.normal.y, 0.0) + 0.12 * abs(in.normal.x));
    free = (in.code & 15u) == FREE;
    if style > 1.5 || free {
        // Wire: only cell edges are drawn.
        alpha = alpha * edge;
        rgb = in.rgba.rgb;
    } else if style > 0.5 {
        // Surface: faces with a thin lighter rim.
        rgb = mix(rgb, in.rgba.rgb * 1.15, edge * 0.6);
    }
    if (in.code & IMPLICIT) != 0u {
        // Implicit unknown space: faint fill, visible bounds.
        alpha = alpha * mix(0.18, 1.0, edge);
    }
#endif
#endif
    rgb = min(rgb * (1.0 + octree.params.y * 0.12 * edge), vec3(65504.0));
#ifdef OCTREE_OPAQUE
    if free && edge < 0.5 { discard; }
    return vec4(rgb, 1.0);
#else
    if alpha <= 0.002 { discard; }
    return background_frost(vec4(rgb * alpha, alpha), in.position.xy, octree.extra.x);
#endif
}
