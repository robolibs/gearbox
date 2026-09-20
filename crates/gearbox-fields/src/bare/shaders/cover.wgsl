// What decides where a bare ground is bare, imported by the ground material,
// by what stands in it and by the clumps. They must agree pixel for pixel —
// grass stops exactly where the soil starts showing — and held as separate
// copies they drifted: a sway noise that differed between two of them once put
// the ruts of the ground and the ruts of the grass in different places.

fn pcg(input: u32) -> u32 {
    let state = input * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn rand(seed: u32, salt: u32) -> f32 {
    return f32(pcg(seed ^ (salt * 0x9E3779B9u))) / 4294967295.0;
}

fn lattice(place: vec2<f32>, across: f32) -> f32 {
    let cell = floor(place / across);
    let f = fract(place / across);
    let ease = f * f * (3.0 - 2.0 * f);
    let corner = array<vec2<f32>, 4>(vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0), vec2(1.0, 1.0));
    var heights = array<f32, 4>();
    for (var i = 0; i < 4; i = i + 1) {
        let c = cell + corner[i];
        var h = u32(i32(c.x)) * 0x9E3779B9u ^ u32(i32(c.y)) * 0x85EBCA6Bu;
        h = h ^ (h >> 15u); h = h * 0x2C1B3C6Du; h = h ^ (h >> 12u);
        heights[i] = f32(h) / 4294967295.0;
    }
    let low = mix(heights[0], heights[1], ease.x);
    let high = mix(heights[2], heights[3], ease.x);
    return mix(low, high, ease.y);
}

fn taken(place: vec2<f32>) -> f32 {
    return lattice(place, 9.0);
}

fn clump_edge(about: f32, seed: u32) -> f32 {
    let a = rand(seed, 30u) * 6.2831853;
    let b = rand(seed, 31u) * 6.2831853;
    return max(1.0 + 0.44 * sin(about * 3.0 + a) + 0.26 * sin(about * 5.0 + b), 0.3);
}

fn clump_frame(cell: vec2<f32>, cell_m: f32, seed: u32) -> vec3<f32> {
    let run = floor(cell * cell_m / 5.0);
    var h = u32(i32(run.x)) * 0x9E3779B9u ^ u32(i32(run.y)) * 0xC2B2AE35u;
    h = h ^ (h >> 15u); h = h * 0x2C1B3C6Du; h = h ^ (h >> 13u);
    let lie = f32(h) / 4294967295.0 * 3.1415927 + (rand(seed, 32u) - 0.5) * 0.7;
    return vec3<f32>(cos(lie), sin(lie), mix(1.0, 2.8, rand(seed, 33u)));
}

// One point of a way: eight of them live two to a column of the matrix, which
// spares both uniforms an array and the shared file a struct to import.
fn way_point(way: mat4x4<f32>, i: i32) -> vec2<f32> {
    let pair = way[i / 2];
    return select(pair.xy, pair.zw, (i & 1) == 1);
}

// Where a point stands against a way: the nearest point of the line itself,
// then signed distance across and plain distance to it. A field is always a
// rectangle, so a winding track is a line through one, not its shape. The last
// two differ at a bend — across folds where two legs meet, distance rounds — so
// the ruts follow the one and the edge of the way the other. The nearest point
// is what the wander is drawn from: it is a place in the world, the same from
// either side of a field boundary, so no part of the road has to be told how
// far along it lies.
fn way_offset(place: vec2<f32>, way: mat4x4<f32>, begin: i32, count: i32) -> vec4<f32> {
    var found = vec4<f32>(0.0, 0.0, 0.0, 1e30);
    for (var i = begin; i < begin + count - 1; i = i + 1) {
        let a = way_point(way, i);
        let leg = way_point(way, i + 1) - a;
        let length_of = max(length(leg), 1e-4);
        let heading = leg / length_of;
        let at = clamp(dot(place - a, heading), 0.0, length_of);
        let near = a + heading * at;
        let gap = distance(place, near);
        if (gap < found.w) {
            found = vec4<f32>(near, dot(place - a, vec2<f32>(-heading.y, heading.x)), gap);
        }
    }
    return found;
}

// Wear from one line: `against` is where the point stands against it (nearest
// point, across, distance) and `half` how wide the worn part of it is.
fn worn_along(place: vec2<f32>, against: vec4<f32>, half: f32, tread: vec4<f32>) -> f32 {
    // No driver holds a line to the centimetre, so the ruts wander along it.
    let sway = (lattice(against.xy, 23.0) - 0.5) * 1.1;
    let rut = abs(abs(against.z + sway) - tread.x);
    let wheel = 1.0 - smoothstep(tread.y * 0.55, tread.y * 1.6, rut);
    // Ragged by moving where the edge falls, not by scaling the wear: as a
    // multiplier it took a road bare across its width down to two thirds.
    let verge = smoothstep(0.0, 1.15, half - against.w + (lattice(place, 4.0) - 0.5) * 0.9);
    return clamp(mix(tread.w, tread.z, wheel) * verge, 0.0, 1.0);
}

// How far the wheels have worn a ground back to bare earth. `tread` is half the
// gauge between the ruts, half a rut's width, how bare the rut is and how bare
// the rest is; all nought for a ground that wears evenly. `shape` is how many
// points the first way has and half its width, then the same for a second way
// sharing the rest of the eight points: where two roads cross, the ground is
// worn by whichever of them has taken more of it. Fewer than two points in the
// first and the wear runs down the field's own long axis, which is every
// straight way.
fn worn(place: vec2<f32>, extent: vec4<f32>, tread: vec4<f32>,
        way: mat4x4<f32>, shape: vec4<f32>) -> f32 {
    if (tread.z <= 0.0) {
        return 0.0;
    }
    let first = i32(shape.x);
    if (first < 2) {
        let span = extent.zw - extent.xy;
        let middle = (extent.xy + extent.zw) * 0.5;
        // A road runs down the long side of its field, so across it is the short.
        let across = select(vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), span.x < span.y);
        let off = dot(place - middle, across);
        let along = dot(place - middle, vec2<f32>(-across.y, across.x));
        let against = vec4<f32>(middle + vec2<f32>(-across.y, across.x) * along, off, abs(off));
        return worn_along(place, against, abs(dot(span, across)) * 0.5, tread);
    }
    var most = worn_along(place, way_offset(place, way, 0, first), max(shape.y, 0.1), tread);
    let second = i32(shape.z);
    if (second >= 2) {
        let crossing = way_offset(place, way, first, second);
        most = max(most, worn_along(place, crossing, max(shape.w, 0.1), tread));
    }
    return most;
}
