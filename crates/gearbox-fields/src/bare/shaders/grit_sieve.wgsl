// Sieves a chunk of road metal: one invocation per candidate, the vertex
// shader's own first tests run once, and every candidate that may still show
// is listed for the chunk's indirect draw. The vertex shader keeps its tests,
// so a candidate kept here and dropped there costs only its vertices.

#import bevy_pbr::mesh_view_bindings::view
#import "embedded://gearbox_fields/shaders/interaction.wgsl"::{WheelMapParams, wheel_scar}
#import "embedded://gearbox_fields/bare/shaders/cover.wgsl"::{pcg, rand, worn, WAY_METALLED}

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

struct SieveDispatch {
    // candidates issued
    counts: vec4<u32>,
};

struct SieveArgs {
    index_count: u32,
    instance_count: atomic<u32>,
    first_index: u32,
    base_vertex: i32,
    first_instance: u32,
};

@group(1) @binding(0) var<uniform> job: SieveDispatch;
@group(1) @binding(1) var<storage, read_write> sieve: array<u32>;
@group(1) @binding(2) var<storage, read_write> args: SieveArgs;

@group(3) @binding(2) var<uniform> field: VegetationParams;
@group(3) @binding(3) var trample: texture_2d<u32>;

@compute @workgroup_size(64)
fn sieve_grit(@builtin(global_invocation_id) gid: vec3<u32>) {
    let index = gid.x;
    if (index >= job.counts.x) {
        return;
    }
    let chunk_seed = pcg(bitcast<u32>(i32(field.corner.x)) * 73856093u
        ^ bitcast<u32>(i32(field.corner.y)) * 19349663u);
    let id = pcg(index ^ chunk_seed ^ 0x51EDu);
    let base = field.corner + vec2<f32>(rand(id, 1u), rand(id, 2u)) * field.chunk_size;
    let rank = f32(index) / max(field.blades_per_chunk, 1.0);
    let end = mix(field.fade_end, field.fade_start, sqrt(rank));
    if (length(base - view.world_position.xz) >= end) {
        return;
    }
    let bared = max(
        worn(base, field.bounds, field.tread, field.way, field.way_more, field.way_shape),
        wheel_scar(trample, field.wheels, base) * 0.8,
    );
    if (rand(id, 12u) > smoothstep(WAY_METALLED, 0.95, bared)) {
        return;
    }
    sieve[atomicAdd(&args.instance_count, 1u)] = index;
}
