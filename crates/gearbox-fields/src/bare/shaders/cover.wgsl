// What decides where a bare ground is bare. The ground material draws the soil
// and the vegetation shader stands the grass, stones and crumbs in it, and the
// two have to agree pixel for pixel: grass must stop exactly where the soil the
// material draws starts showing. They used to agree by holding identical copies
// of these, which nothing enforced — a sway noise that differed between them
// once put the ruts of the ground and the ruts of the grass in different
// places. One copy now, imported by both.

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

// How far the wheels have worn a ground back to bare earth. `extent` is the
// field's rectangle, `tread` how it is worn: half the gauge between the ruts,
// half a rut's width, how bare the rut is, how bare the rest is. A ground that
// wears evenly has a tread of nought and gets nought here.
fn worn(place: vec2<f32>, extent: vec4<f32>, tread: vec4<f32>) -> f32 {
    if (tread.z <= 0.0) {
        return 0.0;
    }
    let span = extent.zw - extent.xy;
    let middle = (extent.xy + extent.zw) * 0.5;
    // A road runs down the long side of its field, so across it is the short.
    let across = select(vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), span.x < span.y);
    // No driver holds a line to the centimetre, so the ruts wander along it.
    let along = dot(place - middle, vec2<f32>(-across.y, across.x));
    let sway = (lattice(vec2<f32>(along, 0.0), 23.0) - 0.5) * 1.1;
    let off = dot(place - middle, across) + sway;
    let rut = abs(abs(off) - tread.x);
    let wheel = 1.0 - smoothstep(tread.y * 0.55, tread.y * 1.6, rut);
    // Ragged by moving where the edge falls, not by scaling the wear: as a
    // multiplier this took a road bare across its width down to two thirds.
    let half = abs(dot(span, across)) * 0.5;
    let inside = half - abs(dot(place - middle, across));
    let verge = smoothstep(0.0, 1.15, inside + (lattice(place, 4.0) - 0.5) * 0.9);
    return clamp(mix(tread.w, tread.z, wheel) * verge, 0.0, 1.0);
}
