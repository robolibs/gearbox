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

// How much of what a way has worn down to is the stone under it rather than the
// field's own earth. It rises faster than the wear itself: the two sides of a
// road running along a boundary wear to the *same* stone and differ only in
// their earth, so left proportional such a road reads as two roads meeting down
// its middle — ploughed earth is nearly three times darker than stubble's. A
// faint mark still shows the field it is in, which is all a faint mark is.
fn settled(bared: f32) -> f32 {
    return smoothstep(0.0, 0.85, bared);
}

// The edge of a way is neither field nor road but the damp churned line
// between: water stands in the lip of the hollow, mud comes off the tyres, and
// nothing there dries. Without it the two meet cleanly however well they are
// blended, which is what reads as a road laid on top of a field. Taken from the
// wear the *way* has done and not the field's own mottle, or a meadow darkens
// in patches nothing has driven over. Narrow on purpose: every way's wear
// passes through this little range in a strip at its outer edge and nowhere
// else, so widening it reaches the crown between the ruts instead, and a track
// half worn flattens into one dark band rather than two ruts with grass up the
// middle — which is the shape of a farm track and the thing worth keeping.
fn verge_damp(worn_here: f32) -> f32 {
    return smoothstep(0.01, 0.07, worn_here) * (1.0 - smoothstep(0.07, 0.20, worn_here));
}

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
    // The authored line is a tolerance, not a rule: the edge strays off it by
    // up to `EDGE_STRAY_M` and the two fields either side stray *together*.
    // West and south take the stray as given, east and north take it negated,
    // because a shared line is one field's east and the other's west: added
    // with one sign to both, the two would move apart and open a gap between
    // them instead of the boundary moving.
    let past = vec4<f32>(
        extent.x - place.x, place.x - extent.z,
        extent.y - place.y, place.y - extent.w)
        + vec4<f32>(
            edge_stray(vec2<f32>(extent.x, place.y)),
            -edge_stray(vec2<f32>(extent.z, place.y)),
            edge_stray(vec2<f32>(place.x, extent.y)),
            -edge_stray(vec2<f32>(place.x, extent.w)));
    // West and east are held by the field's width, south and north by its depth.
    let across = extent.zw - extent.xy;
    let room = vec4<f32>(across.x, across.x, across.y, across.y);
    // Ragged at two scales at once: a long wander that swings the join by
    // metres and a short one that frays it. Drawn from the world and not from
    // either field, so both sides see the same curve and the two halves of the
    // join agree on where it falls.
    let wander = (lattice(place, 13.0) - 0.5) * 0.8 + (lattice(place, 2.9) - 0.5) * 0.45;
    for (var i = 0; i < 4; i = i + 1) {
        if (reach[i] <= 0.0) {
            continue;
        }
        // A blade of grass spills a step past its own field; the *soil* of two
        // fields meets over a headland — scuffed, turned at the ends of the
        // passes, and never on a ruled line. Reading one distance for both put
        // the whole colour change inside a metre, which at any distance a field
        // is looked at from is a line drawn on the ground. Held within the
        // field as well, or a six-metre lane is washed from both sides until
        // none of its own colour is left anywhere in it.
        let span = max(min(reach[i] * GROUND_WASH, room[i] * 0.3), 0.05);
        let edge = past[i] / span + 1.0 + wander;
        // Half at the shared line and no more. Each side carries the other's
        // colour the same amount there, so the two meet at one value; taken
        // further, each field wears mostly its neighbour's and the join
        // inverts — a swap across the line rather than a blend through it.
        out = mix(out, sides[i].rgb, clamp(edge, 0.0, 1.0) * 0.5);
    }
    return out;
}

/// How much further the ground blends than what grows on it.
const GROUND_WASH: f32 = 1.0;

/// How far a field's edge may stray from the line it was authored on. No
/// border in a landscape is straight: a hedge wanders, a headland is turned
/// where the plough could reach, and a fence is put up where the ground let it
/// go. So an authored border — a rectangle here, a ring of cadastral points
/// from a GeoJSON later — is a line the edge must stay *within* half a metre
/// of, not one it must lie on.
const EDGE_STRAY_M: f32 = 0.5;

/// Where an edge really falls, as metres off the line it was authored on.
/// Taken from the line's own coordinate and how far along it this point lies,
/// so every border strays differently and both fields sharing one read the
/// same two numbers and so the same answer.
/// Three scales, and the finest is the one that shows. Half a metre spent on a
/// long smooth wander is a *straight line* at any distance a field is looked at
/// from — the eye reads the trend, not the deviation. The same half metre spent
/// on a fray a hand's breadth across reads as one ground interlocking with the
/// next, which is what the edge of a crop actually is.
fn edge_stray(on: vec2<f32>) -> f32 {
    return ((lattice(on, 17.0) - 0.5) * 0.7
        + (lattice(on + 53.0, 3.1) - 0.5) * 0.7
        + (lattice(on + 131.0, 0.55) - 0.5) * 0.85) * EDGE_STRAY_M;
}

/// How far inside its own field a point lies, in metres, every edge taken where
/// it really falls rather than where it was authored; negative outside. What
/// grows in a field is cut on this and so is the wash under it, or the plants
/// would stop on the ruled line while the soil changed on the crooked one.
/// Shared, because the two fields either side of a border must place it
/// identically: one reading the authored line and the other the strayed one
/// leaves a bald strip between them, or two covers growing through one another.
/// `soft` says, side by side, whether there is anything soft across it — a
/// ground passes its neighbours' reach, what grows passes its own border. A
/// side with nothing soft across it keeps the line it was authored on: a
/// concrete yard ends where its slab ends, and the field beside one does not
/// wander half a metre over it.
fn inside_field(place: vec2<f32>, bounds: vec4<f32>, soft: vec4<f32>) -> f32 {
    let on = step(vec4<f32>(0.0001), soft);
    return min(
        min(place.x - bounds.x - edge_stray(vec2<f32>(bounds.x, place.y)) * on.x,
            bounds.z + edge_stray(vec2<f32>(bounds.z, place.y)) * on.y - place.x),
        min(place.y - bounds.y - edge_stray(vec2<f32>(place.x, bounds.y)) * on.z,
            bounds.w + edge_stray(vec2<f32>(place.x, bounds.w)) * on.w - place.y));
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
fn worn_along(place: vec2<f32>, against: vec4<f32>, half: f32, tread: vec4<f32>, wear: f32) -> vec3<f32> {
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
    let across_way = against.z + sway;
    // The two wheels do not run one line apart: each wanders on its own and
    // each finds firmer ground in different places, so one fades where the
    // other is at its deepest. Taken together they were a mirrored pair, which
    // is what makes ruts read as ruled geometry rather than as something driven.
    // Both fade out as the ruts close together on a narrow way: there the two
    // are one strip and telling them apart would seam it down its middle.
    let apart = smoothstep(0.25, 0.7, gauge);
    let side = select(vec2<f32>(37.0, -11.0), vec2<f32>(-91.0, 53.0), across_way > 0.0);
    let own = (lattice(against.xy + side, 7.0) - 0.5) * min(0.3, half * 0.2) * apart;
    let firm = 1.0 - (1.0 - lattice(against.xy + side, 5.0)) * 0.3 * apart;
    // Signed, so which side of the rut's middle a point falls on is known and
    // not only how far off it.
    let off_middle = abs(across_way) - gauge + own;
    let rut = abs(off_middle);
    let wheel = (1.0 - smoothstep(tread.y * 0.55, tread.y * 1.6, rut)) * firm;
    // A rut's floor is not flat across it. The tyre pushes material to one side
    // and leaves a channel down the other, and it is that channel — not the
    // middle of the rut — that a wheel walks beside and water runs along. Which
    // side is the rut's own business and holds for a long stretch together.
    let lean = select(-1.0, 1.0, lattice(against.xy + side, 21.0) > 0.5);
    let channel = 1.0 - smoothstep(0.0, tread.y * 0.75,
        abs(off_middle - lean * tread.y * 0.42));
    // Ragged by moving where the edge falls, not by scaling the wear: as a
    // multiplier it took a road bare across its width down to two thirds. Both
    // how far the edge softens and how far it wanders are held within the way
    // itself, or a path narrower than the softening never wears at all — its
    // own verge closes over its middle.
    let soften = min(1.15, half * 0.9);
    let wander = (lattice(place, 4.0) - 0.5) * min(0.9, half * 0.6);
    let verge = smoothstep(0.0, soften, half - against.w + wander);
    // How worn, and — separately — how much the wheels themselves sweep here.
    // The two are not the same thing and the second cannot be recovered from
    // the first: the floor of a rut and a road worn bare across its width both
    // read as fully worn, but only one of them has a tyre going down it.
    return vec3<f32>(clamp(mix(between, in_rut, wheel) * verge, 0.0, 1.0), wheel * verge, channel * verge);
}

// How far out from a way's own edge this point stands, in metres, taking the
// nearer of the two lines; a large number where the field has no way. A verge
// is a band *outside* a way and the wear cannot locate it — two metres out,
// where a verge still is one, the wear is already nought and reads the same as
// open field. Only what is scattered rather than shaded wants this, so it is a
// second walk of the line and not a wider answer from `worn()`: a clump asks
// once, where a blade of grass would ask hundreds of thousands of times.
fn way_beyond(place: vec2<f32>, tread: vec4<f32>,
              way: mat4x4<f32>, more: mat4x4<f32>, shape: vec4<f32>) -> f32 {
    let first = i32(shape.x);
    if (max(tread.z, tread.w) <= 0.0 || first < 2) {
        return 1e30;
    }
    var out = way_offset(place, way, more, 0, first).w - max(shape.y, 0.1);
    let second = i32(shape.z);
    if (second >= 2) {
        out = min(out, way_offset(place, way, more, first, second).w - max(shape.w, 0.1));
    }
    return max(out, 0.0);
}

// Ground is never one tone for a metre together. There is a damp patch a hand
// across, a paler place where the fines have blown off, a darker one where a
// clod was crushed into it. Three scales, the finest a few centimetres, so
// however close the eye gets it never finds a flat area — which is the thing
// that reads as a texture rather than as ground.
fn earth_mottle(place: vec2<f32>) -> f32 {
    return 1.0
        + (lattice(place, 0.85) - 0.5) * 0.22
        + (lattice(place + 17.0, 0.29) - 0.5) * 0.16
        + (lattice(place + 71.0, 0.10) - 0.5) * 0.10;
}

// Two things about a way that its wear cannot tell you.
//
// `.x` — how much the **wheels themselves** sweep here, which is the floor of a
// rut and nothing else. Traffic shoves the loose coarse material off that
// floor, out to the shoulder and in to the strip between the two ruts, so the
// stones on a track lie everywhere on it *except* where the tyres run and the
// floor is left as fines. Not recoverable from the wear: a rut floor and a road
// worn bare across its width read alike, and only one has a tyre down it.
//
// `.y` — where water will stand. A rut does not fall evenly along its length;
// it is deeper where the ground was softest when it was made, and a rut sheds
// nothing sideways, so the water finds those low places and stays. Nothing
// draws water yet; this is the ground being ready for it, and it is already
// worth having as the damp, dark, silted patches of a used track.
fn rut_of(place: vec2<f32>, extent: vec4<f32>, tread: vec4<f32>,
          way: mat4x4<f32>, more: mat4x4<f32>, shape: vec4<f32>) -> vec2<f32> {
    if (max(tread.z, tread.w) <= 0.0) {
        return vec2<f32>(0.0, 0.0);
    }
    var swept = 0.0;
    var channel = 0.0;
    let first = i32(shape.x);
    if (first < 2) {
        let span = extent.zw - extent.xy;
        let middle = (extent.xy + extent.zw) * 0.5;
        let across = select(vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), span.x < span.y);
        let off = dot(place - middle, across);
        let along = dot(place - middle, vec2<f32>(-across.y, across.x));
        let against = vec4<f32>(middle + vec2<f32>(-across.y, across.x) * along, off, abs(off));
        let one = worn_along(place, against, abs(dot(span, across)) * 0.5, tread, tread.z);
        swept = one.y;
        channel = one.z;
    } else {
        let along_first = way_offset(place, way, more, 0, first);
        let one = worn_along(place, along_first, max(shape.y, 0.1), tread, tread.z);
        swept = one.y;
        channel = one.z;
        let second = i32(shape.z);
        if (second >= 2) {
            let crossing = way_offset(place, way, more, first, second);
            let two = worn_along(place, crossing, max(shape.w, 0.1), tread, tread.w);
            swept = max(swept, two.y);
            channel = max(channel, two.z);
        }
    }
    // Long dips with short ones inside them: a stretch of channel that holds
    // water, and within it the few feet that hold it longest.
    let dip = lattice(place, 7.5) * 0.62 + lattice(place + 29.0, 1.9) * 0.38;
    return vec2<f32>(swept, channel * smoothstep(0.46, 0.86, dip));
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
        return worn_along(place, against, abs(dot(span, across)) * 0.5, tread, tread.z).x;
    }
    let along_first = way_offset(place, way, more, 0, first);
    let down = worn_along(place, along_first, max(shape.y, 0.1), tread, tread.z).x;
    let second = i32(shape.z);
    if (second < 2) {
        return down;
    }
    let crossing = way_offset(place, way, more, first, second);
    let across = worn_along(place, crossing, max(shape.w, 0.1), tread, tread.w).x;
    // Where two ways meet, the ground is worn worse than either takes alone:
    // everything turning off one onto the other churns the same few metres. So
    // the lesser adds to the greater instead of hiding under it, and a
    // crossroads goes bare while the two roads either side of it keep their ruts.
    return clamp(max(down, across) + min(down, across) * 0.6, 0.0, 1.0);
}
