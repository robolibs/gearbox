import bevy_pbr::render::mesh_view_bindings::{view, globals};
import gearbox::draw::draw_common::{srgb_to_linear, display_to_scene, hud_ink, hud_rule, hud_ladder, hud_iso_distance, hud_scan, hud_hex_border, hud_compose, HUD_SCAN_AXIS, HUD_EDGE_PX, CORNER_PX};
import gearbox::draw::draw_frost::background_frost;

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
const SLICE: u32 = 1048576u;
const SLICE_UNKNOWN: u32 = 2097152u;
const TERRAIN: u32 = 4194304u;
const FRAME: u32 = 16u;
// Six bits from here: cube faces a same-state neighbour covers, dropped outside the clip volume.
const HIDDEN_SHIFT: u32 = 23u;
const UNKNOWN_STATE: u32 = 0u;
const FREE: u32 = 1u;
// Terrain: shaded relief, strata, index contours, map grid with junction squares.
const SUN: vec3<f32> = vec3(-0.4472, 0.7454, 0.4943);
const STRATUM_M: f32 = 0.05;
const STRATUM_SLOPE: f32 = 0.3;
const GRID_M: f32 = 1.0;
const JUNCTION_PX: f32 = 4.0;
// Meshes: a projected line raster crawling up the body, in spacings per second.
const RASTER_M: f32 = 0.035;
const RASTER_CRAWL: f32 = 0.5;
// Voxels: a read-out plane rising through the world, lighting each slab it crosses.
const READOUT_PERIOD_M: f32 = 6.0;
const READOUT_SPEED_MPS: f32 = 1.2;
const READOUT_AFTERGLOW_M: f32 = 0.9;

struct OctreeOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec3<f32>,
    @location(1) @interpolate(flat) rgba: vec4<f32>,
    @location(2) @interpolate(flat) face: u32,
    @location(3) @interpolate(flat) code: u32,
    @location(4) normal: vec3<f32>,
    @location(5) world: vec3<f32>,
    // Position within the cell quad a triangle belongs to, for slices and terrain.
    @location(6) quad: vec2<f32>,
    // World centre and half size of an occupancy cell.
    @location(7) @interpolate(flat) cell: vec4<f32>,
};

// Shaded body, its coverage, structural line coverage and emission weight.
struct Layers {
    body: vec3<f32>,
    fill: f32,
    line: f32,
    rim: f32,
    // 1 where glow turns the body into hologram glass, 0 for maps whose fill is data.
    glass: f32,
};

// Hologram under glow: emission per unit of glow, and the interlace rolling over glass bodies.
const HOLO_LINE_GAIN: f32 = 8.0;
const HOLO_RIM_GAIN: f32 = 6.0;
const INTERLACE_PERIOD_PX: f32 = 3.0;
const INTERLACE_ROLL_PX: f32 = 40.0;

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
    @if((OCTREE_TRIANGLES))
    let cell = cells[vertex_index];
    @if((OCTREE_TRIANGLES))
    let world = octree.world_from_local * vec4(cell.center, 1.0);
    @if((OCTREE_TRIANGLES))
    out.local = array(vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0))[vertex_index % 3u];
    @if((OCTREE_TRIANGLES))
    out.normal = normalize((octree.world_from_local * vec4(oct_decode(cell.spare), 0.0)).xyz);
    @if((OCTREE_TRIANGLES))
    out.quad = array(vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
                     vec2(0.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0))[vertex_index % 6u];
    @if((!(OCTREE_TRIANGLES)) && (OCTREE_DISCS))
    let cell = cells[instance_index];
    @if((!(OCTREE_TRIANGLES)) && (OCTREE_DISCS))
    let center = (octree.world_from_local * vec4(cell.center, 1.0)).xyz;
    @if((!(OCTREE_TRIANGLES)) && (OCTREE_DISCS))
    var n = normalize((octree.world_from_local * vec4(oct_decode(cell.spare), 0.0)).xyz);
    @if((!(OCTREE_TRIANGLES)) && (OCTREE_DISCS))
    if (cell.code & NORMAL_MISSING) != 0u {
        n = normalize(view.world_position - center);
    }
    @if((!(OCTREE_TRIANGLES)) && (OCTREE_DISCS))
    let helper = select(vec3(0.0, 1.0, 0.0), vec3(1.0, 0.0, 0.0), abs(n.y) > 0.9);
    @if((!(OCTREE_TRIANGLES)) && (OCTREE_DISCS))
    let t1 = normalize(cross(n, helper));
    @if((!(OCTREE_TRIANGLES)) && (OCTREE_DISCS))
    let t2 = cross(n, t1);
    @if((!(OCTREE_TRIANGLES)) && (OCTREE_DISCS))
    let corner = array(vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0),
                       vec2(-1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0))[vertex_index];
    @if((!(OCTREE_TRIANGLES)) && (OCTREE_DISCS))
    let scale = length(octree.world_from_local[0].xyz);
    @if((!(OCTREE_TRIANGLES)) && (OCTREE_DISCS))
    let world = vec4(center + (t1 * corner.x + t2 * corner.y) * cell.half * scale, 1.0);
    @if((!(OCTREE_TRIANGLES)) && (OCTREE_DISCS))
    out.local = vec3(corner, 0.0);
    @if((!(OCTREE_TRIANGLES)) && (OCTREE_DISCS))
    out.normal = n;
    @if((!(OCTREE_TRIANGLES)) && (!(OCTREE_DISCS)))
    let cell = cells[instance_index];
    @if((!(OCTREE_TRIANGLES)) && (!(OCTREE_DISCS)))
    if ((cell.code >> HIDDEN_SHIFT) & (1u << (vertex_index / 6u))) != 0u {
        out.position = vec4(0.0, 0.0, 2.0, 1.0);
        return out;
    }
    @if((!(OCTREE_TRIANGLES)) && (!(OCTREE_DISCS)))
    let local = cube_corner(vertex_index);
    @if((!(OCTREE_TRIANGLES)) && (!(OCTREE_DISCS)))
    let world = octree.world_from_local * vec4(cell.center + local * cell.half, 1.0);
    @if((!(OCTREE_TRIANGLES)) && (!(OCTREE_DISCS)))
    out.local = local;
    @if((!(OCTREE_TRIANGLES)) && (!(OCTREE_DISCS)))
    out.normal = normalize((octree.world_from_local * vec4(face_normal(vertex_index / 6u), 0.0)).xyz);
    @if((!(OCTREE_TRIANGLES)) && (!(OCTREE_DISCS)))
    out.cell = vec4((octree.world_from_local * vec4(cell.center, 1.0)).xyz,
                    cell.half * length(octree.world_from_local[1].xyz));
    out.position = view.clip_from_world * world;
    out.world = world.xyz;
    out.face = vertex_index / 6u;
    out.code = cell.code;
    var rgba = cell.rgba;
    if octree.extra.y > 0.5 {
        @if((OCTREE_TRIANGLES))
        rgba = colors[vertex_index];
        @if((!(OCTREE_TRIANGLES)))
        rgba = colors[instance_index];
    }
    out.rgba = scene_color(rgba);
    return out;
}

/// Two-sided light from above with a little side fill.
fn lighting(n: vec3<f32>) -> f32 {
    return 0.58 + 0.30 * abs(n.y) + 0.12 * abs(n.x);
}

// Grazing-angle weight of a surface point, 0 face-on and 1 edge-on.
fn fresnel(n: vec3<f32>, world: vec3<f32>) -> f32 {
    let k = 1.0 - abs(dot(n, normalize(view.world_position - world)));
    return k * k * k;
}

// Occupancy, after the voxel map: stepped blocks with dark side seams and a lit bevel on top
// edges; free space as a dot lattice at cell corners; unexplored space as faint hexagons.
// A read-out plane rises through the world, lighting each cell slab it crosses.
fn box_layers(in: OctreeOutput, base: vec3<f32>, style: f32) -> Layers {
    let axis = in.face / 2u;
    let uv = vec2(in.local[(axis + 1u) % 3u], in.local[(axis + 2u) % 3u]);
    let fp = max(vec2(fwidth(uv.x), fwidth(uv.y)), vec2(1e-6));
    let fh = max(fwidth(in.world.y), 1e-6);
    let gap = (vec2(1.0) - abs(uv)) / fp;
    let edge_px = min(gap.x, gap.y);
    let detail = smoothstep(6.0, 18.0, 2.0 / max(fp.x, fp.y));
    let edge = hud_rule(edge_px, 2.0 * HUD_EDGE_PX);
    let seam = hud_rule(edge_px, 3.0);
    let lattice = hud_rule(length(gap), 3.5);
    let hex = hud_rule(hud_hex_border(uv * 1.6) / (1.6 * max(fp.x, fp.y)), HUD_EDGE_PX) * detail;
    let top = select(0.0, 1.0, in.normal.y > 0.5);
    let shade = 0.5 + 0.42 * max(in.normal.y, 0.0) + 0.14 * abs(in.normal.x);
    let plane = fract(globals.time * READOUT_SPEED_MPS / READOUT_PERIOD_M) * READOUT_PERIOD_M;
    let rel = in.cell.y - plane - READOUT_PERIOD_M * round((in.cell.y - plane) / READOUT_PERIOD_M);
    let crossing = 1.0 - step(in.cell.w, abs(rel));
    let afterglow = (1.0 - smoothstep(0.0, READOUT_AFTERGLOW_M, -rel - in.cell.w)) * step(in.cell.w, -rel);
    let lit = max(crossing, afterglow);
    let trace = hud_rule(hud_iso_distance(in.world.y - plane, READOUT_PERIOD_M) / fh, 1.5) * (1.0 - top);
    let state = in.code & 15u;
    var out = Layers(base * shade, 1.0, 0.0, fresnel(in.normal, in.world), 1.0);
    if state == FREE {
        out.fill = 0.0;
        out.line = lattice * (0.7 + 0.6 * lit) + 0.35 * edge * lit + 0.5 * trace;
    } else if state == UNKNOWN_STATE || (in.code & IMPLICIT) != 0u {
        out.fill = 0.1;
        out.line = (0.35 + 0.4 * lit) * hex + 0.45 * edge + 0.6 * trace;
    } else if style > 1.5 {
        out.fill = 0.0;
        out.line = edge + 0.8 * lattice + trace;
    } else {
        // Under glow the block is hologram glass: every edge glows instead of a dark seam.
        let holo = clamp(2.0 * max(octree.params.y, 0.0), 0.0, 1.0);
        out.body = base * shade * (1.0 - 0.55 * seam * detail * (1.0 - top) * (1.0 - holo)) * (1.0 + 0.3 * lit);
        out.line = max(max(max(0.85 * top * edge * detail, select(0.0, 0.5, style > 0.5) * edge), lit * edge), holo * edge)
            + 0.9 * trace;
    }
    return out;
}

// Surfel disc, after map markers: crisp rim, inner datum ring, centre pip and four inward ticks.
fn disc_layers(in: OctreeOutput, base: vec3<f32>, style: f32, r: f32, fp: f32) -> Layers {
    let radius_px = 1.0 / fp;
    let ring = hud_rule((1.0 - r) / fp, 2.5) * smoothstep(3.0, 7.0, radius_px);
    let inner = hud_rule((r - 0.62) / fp, HUD_EDGE_PX) * smoothstep(14.0, 30.0, radius_px);
    let pip = hud_rule(r / fp, 2.5) * smoothstep(8.0, 16.0, radius_px);
    let p = abs(in.local.xy);
    let tick = hud_rule(min(p.x, p.y) / fp, HUD_EDGE_PX) * step(0.8, r) * smoothstep(18.0, 36.0, radius_px);
    let rim = fresnel(in.normal, in.world);
    var out = Layers(base * lighting(in.normal) * (1.0 + 0.4 * rim), 1.0, 0.0, rim, 1.0);
    out.line = ring + 0.45 * inner + 0.8 * pip + tick;
    if style > 1.5 {
        out.fill = 0.0;
        out.line = ring + 0.6 * inner + pip + tick;
    }
    return out;
}

// Triangle records: slice frames as rulers, slice cells with their measured grid, normal pins,
// terrain as a tactical map, and meshes as projected holograms.
fn triangle_layers(in: OctreeOutput, base: vec3<f32>, style: f32) -> Layers {
    let low = min(in.local.x, min(in.local.y, in.local.z));
    let fl = max(fwidth(low), 1e-6);
    let fq = max(fwidth(in.quad), vec2(1e-6));
    let fh = max(fwidth(in.world.y), 1e-6);
    let fg = max(fwidth(in.world.xz), vec2(1e-6));
    let c = cross(dpdx(in.world), dpdy(in.world));
    let edge_q = min(min(in.quad.x, 1.0 - in.quad.x) / fq.x, min(in.quad.y, 1.0 - in.quad.y) / fq.y);
    let cell_px = 1.0 / max(fq.x, fq.y);
    let scan = hud_scan(in.world, globals.time, fwidth(dot(in.world, HUD_SCAN_AXIS)));
    var n = in.normal;
    if (in.code & NORMAL_MISSING) != 0u {
        n = c / max(length(c), 1e-12);
        n = select(n, -n, dot(n, view.world_position - in.world) < 0.0);
    }
    let slope = sqrt(max(1.0 - n.y * n.y, 0.0));
    var out = Layers(base, 1.0, 0.0, 0.0, 0.0);
    if (in.code & FRAME) != 0u {
        let ten = hud_rule(hud_iso_distance(in.quad.x, 0.1) / fq.x, 1.25);
        let twenty = hud_rule(hud_iso_distance(in.quad.x, 0.05) / fq.x, HUD_EDGE_PX)
            * (1.0 - smoothstep(0.5, 0.6, in.quad.y)) * smoothstep(3.0, 6.0, 0.05 / fq.x);
        let rail = 1.0 - smoothstep(0.26, 0.26 + fq.y, in.quad.y);
        let reach = max(0.1, CORNER_PX * fq.x);
        let corner = 1.0 - smoothstep(reach, reach + fq.x, min(in.quad.x, 1.0 - in.quad.x));
        out.fill = 0.0;
        out.line = max(max(rail, corner), max(ten * smoothstep(3.0, 6.0, 0.1 / fq.x), twenty));
        return out;
    }
    if (in.code & SLICE) != 0u {
        var line = 0.22 * hud_rule(edge_q, HUD_EDGE_PX) * smoothstep(10.0, 24.0, cell_px);
        if (in.code & SLICE_UNKNOWN) != 0u {
            let hatch = hud_rule(hud_iso_distance(in.quad.x + in.quad.y, 0.5) / (fq.x + fq.y), HUD_EDGE_PX);
            line = max(line, 0.45 * hatch * smoothstep(10.0, 24.0, cell_px));
        }
        out.line = line * (1.0 + scan.y) + 0.6 * scan.x;
        return out;
    }
    if (in.code & ARROW) != 0u { return out; }
    let rim = fresnel(n, in.world);
    if (in.code & TERRAIN) != 0u {
        let h = in.world.y;
        let mpp = max(fg.x, fg.y);
        let relief = smoothstep(0.01, 0.05, slope);
        let s = hud_ladder(STRATUM_M, mpp * STRATUM_SLOPE, 7.0);
        let open = smoothstep(2.0, 4.0, s / fh);
        let stratum = hud_rule(hud_iso_distance(h, s) / fh, HUD_EDGE_PX) * open * relief;
        let index = hud_rule(hud_iso_distance(h, 4.0 * s) / fh, 1.6) * smoothstep(2.0, 4.0, 4.0 * s / fh) * relief;
        let terrace = 1.0 + 0.16 * (fract(h / s) - 0.5) * open * relief;
        let g = hud_ladder(GRID_M, mpp, 16.0);
        let grid_d = vec2(hud_iso_distance(in.world.x, g), hud_iso_distance(in.world.z, g));
        let grid = hud_rule(min(grid_d.x / fg.x, grid_d.y / fg.y), HUD_EDGE_PX);
        let major = vec2(hud_iso_distance(in.world.x, 4.0 * g), hud_iso_distance(in.world.z, 4.0 * g));
        let junction = hud_rule(max(major.x, major.y) / mpp - JUNCTION_PX, HUD_EDGE_PX);
        let hill = 0.55 + 0.6 * max(dot(n, SUN), 0.0);
        out.body = base * hill * terrace * (1.0 - 0.4 * stratum) + hud_ink(base) * 0.3 * rim * relief;
        out.line = (0.85 * index + 0.2 * grid + 0.7 * junction) * (1.0 + scan.y) + 0.5 * stratum * scan.y + 0.7 * scan.x;
        return out;
    }
    let s = hud_ladder(RASTER_M, fh, 6.0);
    let phase = in.world.y / s - globals.time * RASTER_CRAWL;
    let raster = hud_rule(abs(fract(phase + 0.5) - 0.5) * s / fh, HUD_EDGE_PX) * smoothstep(0.15, 0.4, slope);
    let topo = hud_rule(low / fl, HUD_EDGE_PX) * smoothstep(6.0, 16.0, 0.33 / fl);
    out.body = base * lighting(n) * 0.8 + hud_ink(base) * 0.6 * rim;
    out.line = select(0.55, 0.7, style > 0.5) * raster + 0.22 * topo;
    out.rim = rim;
    out.glass = 1.0;
    if style > 1.5 {
        out.fill = 0.0;
        out.line = hud_rule(low / fl, 2.0 * HUD_EDGE_PX) + 0.4 * raster;
    }
    return out;
}

@fragment
fn fragment(in: OctreeOutput) -> @location(0) vec4<f32> {
    let style = octree.params.z;
    let opacity = in.rgba.a * octree.params.x;
    let base = in.rgba.rgb;
    var layers = Layers(base, 1.0, 0.0, 0.0, 0.0);
    @if((OCTREE_DISCS))
    let r = length(in.local.xy);
    @if((OCTREE_DISCS))
    if r > 1.0 { discard; }
    @if((OCTREE_DISCS))
    layers = disc_layers(in, base, style, r, max(fwidth(r), 1e-6));
    @if((!(OCTREE_DISCS)) && (OCTREE_TRIANGLES))
    layers = triangle_layers(in, base, style);
    @if((!(OCTREE_DISCS)) && (!(OCTREE_TRIANGLES)))
    layers = box_layers(in, base, style);
    let glow = max(octree.params.y, 0.0);
    let holo = clamp(2.0 * glow, 0.0, 1.0) * layers.glass;
    let interlace = 0.5 + 0.5 * sin((in.position.y + globals.time * INTERLACE_ROLL_PX) * 6.2831853 / INTERLACE_PERIOD_PX);
    layers.body = layers.body * (1.0 - holo * (0.32 + 0.2 * interlace));
    let emission = glow * (HOLO_LINE_GAIN * clamp(layers.line, 0.0, 1.0) + HOLO_RIM_GAIN * layers.rim * layers.glass);
    @if((OCTREE_OPAQUE))
    let solid = hud_compose(base, layers.body, layers.fill, layers.line, emission, 1.0);
    @if((OCTREE_OPAQUE))
    if solid.a < 0.5 { discard; }
    @if((OCTREE_OPAQUE))
    return vec4(solid.rgb / solid.a, 1.0);
    @if((!(OCTREE_OPAQUE)))
    let color = hud_compose(base, layers.body, layers.fill, layers.line, emission, opacity);
    @if((!(OCTREE_OPAQUE)))
    if color.a <= 0.002 { discard; }
    @if((!(OCTREE_OPAQUE)))
    return background_frost(color, in.position.xy, octree.extra.x);
}
