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

// A way is not one thing with a dial on it. As it is used harder it passes
// through stages, and these are where one gives way to the next — shared, so
// the ground, the grit lying on it and what still grows in it all change
// together rather than each on its own schedule.
//
//   below BRUISED   the sward is only crushed and darkened; no earth shows
//   BRUISED..METALLED   earth comes through, still a soft track
//   above METALLED  stone comes up through the fines: a made road
//
// A tractor that has been over a field twice leaves the first; a lane the
// milk lorry uses leaves the last. Nothing but `wear` chooses between them.
const WAY_BRUISED: f32 = 0.25;
const WAY_METALLED: f32 = 0.55;

// The taller of two surfaces wins the pixel outright, and they mix only within
// a shallow band. Fading them instead leaves a grey halo round every patch —
// and where a way meets a field, a plain fade is what makes a road read as
// paint laid on the ground rather than as ground the cover has come off. Each
// side brings its own micro-relief, so the earth shows first through the
// hollows of the sward and the sward holds last on the high ground.
fn height_blend(a: vec3<f32>, a_height: f32, a_share: f32,
    b: vec3<f32>, b_height: f32, b_share: f32) -> vec4<f32> {
    let band = 0.2;
    let tallest = max(a_height + a_share, b_height + b_share) - band;
    let weight_a = max(a_height + a_share - tallest, 0.0);
    let weight_b = max(b_height + b_share - tallest, 0.0);
    let total = max(weight_a + weight_b, 0.0001);
    return vec4<f32>((a * weight_a + b * weight_b) / total, weight_b / total);
}

// A ground washed into whatever lies across each of its sides, so two covers
// meet in a blend and not on a line. `sides` holds the four neighbours' colours
// a column each — west, east, south, north — and `reach` how far each carries.
// Shared, because both grounds either side of a join have to agree where it
// falls: held as two copies they drifted, one edge ragged and one ruled.
fn washed_into(place: vec2<f32>, colour: vec3<f32>, extent: vec4<f32>,
               sides: mat4x4<f32>, reach: vec4<f32>) -> vec3<f32> {
    var out = colour;
    let past = vec4<f32>(
        extent.x - place.x, place.x - extent.z,
        extent.y - place.y, place.y - extent.w);
    for (var i = 0; i < 4; i = i + 1) {
        if (reach[i] <= 0.0) {
            continue;
        }
        let edge = past[i] / reach[i] + 1.0 + (lattice(place, 1.7) - 0.5) * 0.55;
        out = mix(out, sides[i].rgb, clamp(edge, 0.0, 1.0) * 0.85);
    }
    return out;
}

// One point of a way: sixteen of them live two to a column of two matrices,
// which spares both uniforms an array and the shared file a struct to import.
// The column is worked out before either matrix is read, so neither is ever
// indexed past its fourth column.
fn way_point(way: mat4x4<f32>, more: mat4x4<f32>, i: i32) -> vec2<f32> {
    let late = i >= 8;
    let column = select(i, i - 8, late) / 2;
    let pair = select(way[column], more[column], late);
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
fn way_offset(place: vec2<f32>, way: mat4x4<f32>, more: mat4x4<f32>,
              begin: i32, count: i32) -> vec4<f32> {
    var found = vec4<f32>(0.0, 0.0, 0.0, 1e30);
    for (var i = begin; i < begin + count - 1; i = i + 1) {
        let a = way_point(way, more, i);
        let leg = way_point(way, more, i + 1) - a;
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
// point, across, distance), `half` how wide the worn part of it is and `wear`
// how hard this line in particular is used. Two lines crossing a field each
// bring their own, so a farm track may join a metalled road.
fn worn_along(place: vec2<f32>, against: vec4<f32>, half: f32, tread: vec4<f32>, wear: f32) -> f32 {
    // No stretch of a way is worn quite like the next, and without that a road
    // is one flat tone end to end, which is what reads as paint at a distance.
    // Drawn from the nearest point of the line, so both sides of a boundary see
    // the same stretch; widest in the middle of the range, since neither an
    // untouched field nor a made road has much room to vary.
    let along = (lattice(against.xy, 11.0) - 0.5) * 0.44 * wear * (1.0 - wear * 0.55);
    let used = clamp(wear + along, 0.0, 1.0);
    // The ruts go bare long before the crown does, and never quite let it catch
    // up: at one the two meet and a made road goes flat across its width,
    // losing the wheel lines that say it is driven.
    let in_rut = min(0.55 + used * 0.45, 1.0);
    let between = max(used - 0.35, 0.0) / 0.65 * 0.9;
    // The ruts run at a tractor's gauge, but a way narrower than that would
    // carry them outside its own verge and show almost nothing. So on a narrow
    // path they close towards its middle and meet as one worn strip, which is
    // what a path too narrow for a tractor is worn into anyway.
    let gauge = min(tread.x, max(half - tread.y, 0.0));
    // No driver holds a line to the centimetre, so the ruts wander along it —
    // but never so far that they leave the way and the verge cuts them off,
    // which on a narrow path broke it into a dotted line.
    let sway = (lattice(against.xy, 23.0) - 0.5) * min(1.1, half * 0.7);
    let rut = abs(abs(against.z + sway) - gauge);
    let wheel = 1.0 - smoothstep(tread.y * 0.55, tread.y * 1.6, rut);
    // Ragged by moving where the edge falls, not by scaling the wear: as a
    // multiplier it took a road bare across its width down to two thirds. Both
    // how far the edge softens and how far it wanders are held within the way
    // itself, or a path narrower than the softening never wears at all — its
    // own verge closes over its middle.
    let soften = min(1.15, half * 0.9);
    let wander = (lattice(place, 4.0) - 0.5) * min(0.9, half * 0.6);
    let verge = smoothstep(0.0, soften, half - against.w + wander);
    return clamp(mix(between, in_rut, wheel) * verge, 0.0, 1.0);
}

// How far the wheels have worn a ground back to bare earth. `tread` is half the
// gauge between the ruts, half a rut's width, then how hard each of the two
// ways is worn; all nought for a ground nothing has worn. `shape` is how many
// points the first way has and half its width, then the same for a second way
// sharing the rest of the sixteen points: where two roads cross, the ground is
// worn by whichever of them has taken more of it. Fewer than two points in the
// first and the wear runs down the field's own long axis, which is every
// straight way.
fn worn(place: vec2<f32>, extent: vec4<f32>, tread: vec4<f32>,
        way: mat4x4<f32>, more: mat4x4<f32>, shape: vec4<f32>) -> f32 {
    if (max(tread.z, tread.w) <= 0.0) {
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
        return worn_along(place, against, abs(dot(span, across)) * 0.5, tread, tread.z);
    }
    let along_first = way_offset(place, way, more, 0, first);
    let down = worn_along(place, along_first, max(shape.y, 0.1), tread, tread.z);
    let second = i32(shape.z);
    if (second < 2) {
        return down;
    }
    let crossing = way_offset(place, way, more, first, second);
    let across = worn_along(place, crossing, max(shape.w, 0.1), tread, tread.w);
    // Where two ways meet, the ground is worn worse than either takes alone:
    // everything turning off one onto the other churns the same few metres. So
    // the lesser adds to the greater instead of hiding under it, and a
    // crossroads goes bare while the two roads either side of it keep their ruts.
    return clamp(max(down, across) + min(down, across) * 0.6, 0.0, 1.0);
}
