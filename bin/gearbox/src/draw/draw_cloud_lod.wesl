// Level of detail for compact clouds, chosen on the GPU per view and frame.
// measure: each visible hierarchy node adds the points it would draw, for every error
//   threshold, to a histogram over thresholds;
// choose: the finest threshold no coarser than the view's error that fits its point budget;
// emit: per page, the refinement-order range each node draws at that threshold, as a table
//   the vertex stage searches, and the page's indirect draw;
// accept: counts the listed points the clip volume keeps.
import gearbox::draw::draw_cloud_types::{CloudPosition, CloudPageUniform, cloud_clip_accept};

struct TreeNode {
    min: vec3<f32>,
    first: u32,
    max: vec3<f32>,
    count: u32,
    parent: u32,
    start: u32,
    leaf: u32,
    pad: u32,
    errors_m: array<vec4<f32>, 2>,
};

struct LodView {
    clip_from_world: mat4x4<f32>,
    viewport: vec2<f32>,
    error_px: f32,
    budget: u32,
};

// Thresholds 2^(b/4 - 8) px for b in 0..=BINS; a difference array of points per threshold.
const BINS: u32 = 160u;
const UNBOUNDED: u32 = 161u;
struct Bins {
    diff: array<atomic<i32>, 161>,
    chosen: u32,
    points: i32,
};

// One page's draw: indirect draw arguments, then what it selected.
struct Head {
    draw: vec4<u32>,
    nodes: u32,
    selected: u32,
    represented: u32,
    accepted: atomic<u32>,
    chosen: u32,
    pad: array<u32, 7>,
};

const NO_PARENT: u32 = 0xffffffffu;
const HUGE: f32 = 3.0e38;
const NODES: u32 = 512u;
const WORKGROUP: u32 = 256u;

@group(0) @binding(0) var<uniform> view: LodView;
@group(0) @binding(1) var<storage, read_write> bins: Bins;
@group(1) @binding(0) var<uniform> page: CloudPageUniform;
@group(1) @binding(1) var<storage, read> tree: array<TreeNode>;
@group(1) @binding(2) var<storage, read> order: array<u32>;
@group(1) @binding(3) var<storage, read> positions: array<CloudPosition>;
@group(2) @binding(0) var<storage, read_write> head: Head;
@group(2) @binding(1) var<storage, read_write> table: array<vec2<u32>, 512>;
// The accept dispatch's indirect arguments; only `emit` binds them.
@group(2) @binding(2) var<storage, read_write> dispatch: vec4<u32>;

struct Measure {
    visible: bool,
    error: f32,
    // Screen pixels per metre of sample spacing inside the node.
    slope: f32,
};

fn corner(node: TreeNode, index: u32) -> vec3<f32> {
    return select(node.min, node.max, vec3((index & 1u) != 0u, (index & 2u) != 0u, (index & 4u) != 0u));
}

fn row_length(m: mat4x4<f32>, row: u32) -> f32 {
    return length(vec3(m[0][row], m[1][row], m[2][row]));
}

fn measure(node: TreeNode) -> Measure {
    var out = Measure(false, 0.0, 0.0);
    let clip_from_local = view.clip_from_world * page.world_from_local;
    let half = view.viewport * 0.5;
    var depth_outside = 7u;
    var low = vec3(HUGE);
    var high = vec3(-HUGE);
    var bounded = true;
    var rect_low = vec2(HUGE);
    var rect_high = vec2(-HUGE);
    var nearest_w = HUGE;
    var widest = vec2(0.0);
    for (var index = 0u; index < 8u; index += 1u) {
        let world = (page.world_from_local * vec4(corner(node, index), 1.0)).xyz;
        let clip = view.clip_from_world * vec4(world, 1.0);
        depth_outside &= select(0u, 1u, clip.z < 0.0) | select(0u, 2u, clip.z > clip.w) | select(0u, 4u, clip.w <= 0.0);
        low = min(low, world);
        high = max(high, world);
        if clip.w <= 0.0 || clip.z < 0.0 || clip.z > clip.w {
            bounded = false;
        } else {
            let xy = clip.xy / clip.w * half;
            rect_low = min(rect_low, xy);
            rect_high = max(rect_high, xy);
            nearest_w = min(nearest_w, clip.w);
            widest = max(widest, abs(clip.xy));
        }
    }
    if depth_outside != 0u { return out; }
    // Frustum test of the world box grown by the drawn glyph.
    let margin = vec2(1.0) + (page.lod.y + 1.0) / half;
    let grow = vec3(page.lod.x);
    var outside = 15u;
    for (var index = 0u; index < 8u; index += 1u) {
        let side = vec3((index & 1u) != 0u, (index & 2u) != 0u, (index & 4u) != 0u);
        let clip = view.clip_from_world * vec4(select(low - grow, high + grow, side), 1.0);
        outside &= select(0u, 1u, clip.x < -clip.w * margin.x) | select(0u, 2u, clip.x > clip.w * margin.x)
            | select(0u, 4u, clip.y < -clip.w * margin.y) | select(0u, 8u, clip.y > clip.w * margin.y);
    }
    if outside != 0u { return out; }
    out.visible = true;
    if !bounded {
        out.error = HUGE;
        out.slope = HUGE;
        return out;
    }
    out.error = length(rect_high - rect_low);
    let w_gradient = row_length(clip_from_local, 3u);
    let slope = vec2(
        (row_length(clip_from_local, 0u) + widest.x / nearest_w * w_gradient) / nearest_w * half.x,
        (row_length(clip_from_local, 1u) + widest.y / nearest_w * w_gradient) / nearest_w * half.y);
    out.slope = length(slope);
    return out;
}

// First threshold bin at or above `error`; UNBOUNDED for an error no threshold reaches.
fn bin_of(error: f32) -> u32 {
    if error >= HUGE { return UNBOUNDED; }
    if error <= exp2(-8.0) { return 0u; }
    return u32(clamp(ceil((log2(error) + 8.0) * 4.0), 0.0, f32(BINS)));
}

fn level_error(node: TreeNode, level: u32) -> f32 {
    return node.errors_m[level / 4u][level % 4u];
}

fn deposit(points: u32, low: u32, high: u32) {
    if points == 0u || low >= high || low > BINS { return; }
    atomicAdd(&bins.diff[low], i32(points));
    if high <= BINS { atomicAdd(&bins.diff[high], -i32(points)); }
}

fn parent_error(node: TreeNode) -> f32 {
    if node.parent == NO_PARENT { return HUGE; }
    return measure(tree[node.parent]).error;
}

// The points `node` draws at threshold bin `chosen`, or adds its whole ladder to the
// histogram when `chosen` is UNBOUNDED + 1.
fn ladder(node: TreeNode, chosen: u32) -> u32 {
    let m = measure(node);
    if !m.visible { return 0u; }
    var high = bin_of(parent_error(node));
    let accumulate = chosen > UNBOUNDED;
    if node.leaf == 0u {
        let low = bin_of(m.error);
        if accumulate { deposit(1u, low, high); return 0u; }
        return select(0u, 1u, low <= chosen && chosen < high);
    }
    var error = m.error;
    var points = 1u;
    for (var level = 0u; level <= 8u; level += 1u) {
        let full = points >= node.count;
        var low = bin_of(error);
        if full { low = 0u; }
        if accumulate {
            deposit(min(points, node.count), low, high);
        } else if low <= chosen && chosen < high {
            return min(points, node.count);
        }
        if full { break; }
        high = min(high, low);
        points *= 2u;
        let spacing = level_error(node, min(level + 1u, 7u)) + 2.0 * page.lod.z;
        error = min(error, select(m.slope * spacing, HUGE, m.slope >= HUGE));
    }
    return 0u;
}

@compute @workgroup_size(256)
fn measure_nodes(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= page.tree.x { return; }
    ladder(tree[id.x], UNBOUNDED + 1u);
}

@compute @workgroup_size(1)
fn choose() {
    let finest = bin_of(view.error_px);
    var points = 0;
    bins.chosen = BINS;
    for (var bin = 0u; bin <= BINS; bin += 1u) {
        points += atomicLoad(&bins.diff[bin]);
        if bin >= finest && points <= i32(view.budget) {
            bins.chosen = bin;
            break;
        }
    }
    bins.points = points;
}

var<workgroup> sums: array<u32, 256>;
var<workgroup> represented: atomic<u32>;

@compute @workgroup_size(256)
fn emit(@builtin(local_invocation_index) lane: u32) {
    let nodes = min(page.tree.x, NODES);
    if lane == 0u { atomicStore(&represented, 0u); }
    workgroupBarrier();
    var counts = vec2(0u);
    var starts = vec2(0u);
    for (var pair = 0u; pair < 2u; pair += 1u) {
        let index = lane * 2u + pair;
        if index < nodes {
            let node = tree[index];
            counts[pair] = ladder(node, bins.chosen);
            starts[pair] = select(node.first, node.start, node.leaf == 0u);
            if counts[pair] > 0u { atomicAdd(&represented, node.count); }
        }
    }
    sums[lane] = counts.x + counts.y;
    workgroupBarrier();
    for (var step = 1u; step < WORKGROUP; step *= 2u) {
        let other = select(0u, sums[lane - step], lane >= step);
        workgroupBarrier();
        sums[lane] += other;
        workgroupBarrier();
    }
    let before = sums[lane] - counts.x - counts.y;
    if lane * 2u < nodes { table[lane * 2u] = vec2(before, starts.x); }
    if lane * 2u + 1u < nodes { table[lane * 2u + 1u] = vec2(before + counts.x, starts.y); }
    if lane == WORKGROUP - 1u {
        let total = sums[lane];
        let clipped = page.clip_min.w != 0.0;
        head.draw = vec4(6u, total, 0u, 0u);
        dispatch = vec4(select(0u, (total + WORKGROUP - 1u) / WORKGROUP, clipped), 1u, 1u, 0u);
        head.nodes = nodes;
        head.selected = total;
        head.represented = atomicLoad(&represented);
        atomicStore(&head.accepted, select(total, 0u, clipped));
        head.chosen = bins.chosen;
    }
}

@compute @workgroup_size(256)
fn accept(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= head.selected { return; }
    var low = 0u;
    var high = max(head.nodes, 1u);
    while low + 1u < high {
        let middle = (low + high) / 2u;
        if table[middle].x <= id.x { low = middle; } else { high = middle; }
    }
    let p = positions[order[table[low].y + id.x - table[low].x]];
    if cloud_clip_accept(page.clip_rotation, page.clip_min, page.clip_max, vec3(p.x, p.y, p.z)) {
        atomicAdd(&head.accepted, 1u);
    }
}
