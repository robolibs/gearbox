#import bevy_pbr::mesh_view_bindings::globals

struct WheelMapParams {
    origin: vec2<f32>,
    texels_per_metre: f32,
    width: f32,
    height: f32,
    recovery_seconds: f32,
    bend: f32,
    darkening: f32,
    /// The tyre rolling here — pitch, lean, duty, depth — so a mark driven
    /// across open ground carries the same print as a way laid out for one.
    bar: vec4<f32>,
}

fn wheel_texel(tex: texture_2d<u32>, index: vec2<i32>, recovery: f32) -> vec3<f32> {
    let texel = textureLoad(tex, index, 0);
    if (texel.g == 0u) { return vec3<f32>(0.0); }
    let stamped = f32(texel.r) / 65535.0 * 3600.0;
    // A stamp a moment ahead of this frame is fresh, not a period old.
    let delta = globals.time - stamped;
    let age = select(delta, max(delta + 3600.0, 0.0), delta < -8.0);
    let press = 1.0 - clamp(age / max(recovery, 0.001), 0.0, 1.0);
    let angle = f32(texel.g >> 8u) / 255.0 * 6.2831853 - 3.1415927;
    return vec3<f32>(press, cos(angle) * press, sin(angle) * press);
}

// How long a wheel's bare scar takes to green over. A flattened sward springs
// back in minutes — that is what `recovery_seconds` is for — but the ground it
// was pressed into stays showing far longer, so one map is read on two clocks.
// Nothing can outlast the stamp clock itself, which spans an hour.
const SCAR_SECONDS: f32 = 720.0;

fn sample_wheels_over(tex: texture_2d<u32>, params: WheelMapParams, world_xz: vec2<f32>,
                      recovery: f32) -> vec3<f32> {
    let t = (world_xz - params.origin) * params.texels_per_metre;
    let last = vec2<i32>(i32(params.width), i32(params.height)) - vec2<i32>(1);
    if (any(t < vec2<f32>(0.0)) || any(t > vec2<f32>(last))) { return vec3<f32>(0.0); }
    let i = clamp(vec2<i32>(floor(t)), vec2<i32>(0), last - vec2<i32>(1));
    let f = clamp(t - vec2<f32>(i), vec2<f32>(0.0), vec2<f32>(1.0));
    let a = wheel_texel(tex, i, recovery);
    let b = wheel_texel(tex, i + vec2<i32>(1, 0), recovery);
    let c = wheel_texel(tex, i + vec2<i32>(0, 1), recovery);
    let d = wheel_texel(tex, i + vec2<i32>(1, 1), recovery);
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

fn sample_wheels(tex: texture_2d<u32>, params: WheelMapParams, world_xz: vec2<f32>) -> vec3<f32> {
    return sample_wheels_over(tex, params, world_xz, params.recovery_seconds);
}

// How bare a wheel has left this ground, on the slow clock. Taken together with
// whatever the layout says and the harder wins, so a mark driven into a field
// lasts like one laid out in it rather than fading while the other stays.
fn wheel_scar(tex: texture_2d<u32>, params: WheelMapParams, world_xz: vec2<f32>) -> f32 {
    if (params.recovery_seconds <= 0.0) { return 0.0; }
    return clamp(sample_wheels_over(tex, params, world_xz, SCAR_SECONDS).x, 0.0, 1.0);
}

// Unit roll direction of a wheel sample; zero where nothing has rolled.
fn wheel_roll(pressed: vec3<f32>) -> vec3<f32> {
    let size = length(pressed.yz);
    if (size < 1e-4) { return vec3<f32>(0.0); }
    return vec3<f32>(pressed.y, 0.0, pressed.z) / size;
}

// The roll direction turned by `angle` radians about the vertical.
fn scatter_roll(roll: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec3<f32>(roll.x * c - roll.z * s, 0.0, roll.x * s + roll.z * c);
}

// A tyre mark on a tread map: how fresh it is, how hard the tyre scrubbed,
// and where this point lies in the tread — lug pitches rolled along the
// track, and -1..1 across the tyre. `metres` gives the same two in metres and
// `roll` the world direction the wheel was going, which is what a cover needs
// to press the tyre's own bars into its relief rather than only tint them.
struct WheelMark {
    press: f32,
    scrub: f32,
    along: f32,
    across: f32,
    metres: vec2<f32>,
    roll: vec2<f32>,
    /// How far off the tyre's middle this point lies as a share of its half
    /// width, taken as the *nearest* of what the four texels round it say
    /// rather than their average. Through a turn each of them was stamped with
    /// the axle pointing a different way, so they disagree about where across
    /// the tyre a point is; averaged, the answer reads as outside the tyre and
    /// the mark is cut away exactly where the machine turned. The ground a
    /// turning wheel covers is the union of its poses, and the nearest reading
    /// is that union.
    inside: f32,
}

const TREAD_PITCH_M: f32 = 0.192;

// Press and scrub blend between texels for a soft edge; the tread
// coordinates come from the nearest texel alone, carried to this point
// along its own roll, so every lug of one pass lands in step.
fn sample_wheel_mark(tex: texture_2d<u32>, params: WheelMapParams, world_xz: vec2<f32>) -> WheelMark {
    let press = sample_wheels(tex, params, world_xz).x;
    let t = (world_xz - params.origin) * params.texels_per_metre;
    let last = vec2<i32>(i32(params.width), i32(params.height)) - vec2<i32>(1);
    // Where across the tyre a point lies is read from all four texels round it
    // and not from the nearest alone. Each texel was stamped on its own frame,
    // with the wheel a little further on, so each holds a slightly different
    // idea of where the tyre's middle was; taking the nearest one steps between
    // those ideas on the texel boundary and the whole tread jumps sideways a
    // few centimetres along a ruled line. Each corner is asked where this point
    // lies against *its* tyre, and the four answers are blended.
    let nearest = clamp(vec2<i32>(round(t)), vec2<i32>(0), last);
    let texel = textureLoad(tex, nearest, 0);
    let near_turn = f32(texel.g >> 8u) / 255.0 * 6.2831853 - 3.1415927;
    let near_roll = vec2<f32>(cos(near_turn), sin(near_turn));
    let near_out = (t - vec2<f32>(nearest)) / params.texels_per_metre;
    let near_across = (f32(texel.a >> 8u) - 128.0) * 0.005
        + dot(near_out, vec2<f32>(-near_roll.y, near_roll.x));
    let near_width = max(f32(texel.a & 255u) * 0.005, 0.01);
    var closest = 1e9;
    var centre_sum = vec2<f32>(0.0);
    var roll_sum = vec2<f32>(0.0);
    var width_sum = 0.0;
    let corner = clamp(vec2<i32>(floor(t)), vec2<i32>(0), last - vec2<i32>(1));
    let into = clamp(t - vec2<f32>(corner), vec2<f32>(0.0), vec2<f32>(1.0));
    for (var k = 0; k < 4; k = k + 1) {
        let step = vec2<i32>(k & 1, k >> 1);
        let at = corner + step;
        let there = textureLoad(tex, at, 0);
        let share = select(1.0 - into.x, into.x, step.x == 1)
            * select(1.0 - into.y, into.y, step.y == 1);
        // A corner with nothing stamped on it takes the nearest texel's own
        // reading rather than dropping out of the blend. Dropped and the rest
        // renormalised, the answer steps by a share of the tyre's width
        // wherever a corner falls off the end of a pass — which is a seam
        // running diagonally down the track, exactly where two passes meet.
        let known = there.g != 0u && there.a != 0u;
        let turn = f32(there.g >> 8u) / 255.0 * 6.2831853 - 3.1415927;
        let its_roll = select(near_roll, vec2<f32>(cos(turn), sin(turn)), known);
        let its_axle = vec2<f32>(-its_roll.y, its_roll.x);
        let out_by = (t - vec2<f32>(at)) / params.texels_per_metre;
        let its_across = select(near_across,
            (f32(there.a >> 8u) - 128.0) * 0.005 + dot(out_by, its_axle), known);
        let its_width = select(near_width, max(f32(there.a & 255u) * 0.005, 0.01), known);
        closest = min(closest, abs(its_across) / max(its_width, 0.01));
        // What each texel is blended by is the point on the tyre's own middle
        // that it implies — itself, stepped back across its own axle — and not
        // its reading of how far off that middle this point lies. Through a
        // turn the four were stamped with the axle pointing different ways, and
        // averaging four readings taken in four different frames is what makes
        // the middle wander: it is a mean of numbers that do not mean the same
        // thing. The points they imply all lie on one line, curved or straight,
        // so averaging *those* is sound however the frames differ.
        let its_stamp = select(near_across, (f32(there.a >> 8u) - 128.0) * 0.005, known);
        let its_place = params.origin + vec2<f32>(at) / params.texels_per_metre;
        centre_sum = centre_sum + share * (its_place - its_stamp * its_axle);
        roll_sum = roll_sum + share * its_roll;
        width_sum = width_sum + share * its_width;
    }
    let weight = 1.0;
    // `texel.a` nought means this map carries no tread channels at all — the
    // ground it belongs to never asked for them. Read anyway, the across byte
    // reads as its own bias, which is a constant offset from the tyre's middle:
    // every bar then leans the same way and the chevron comes out as half of
    // itself, drawn as parallel diagonals.
    if (press < 0.001 || texel.g == 0u || texel.a == 0u) {
        return WheelMark(0.0, 0.0, 0.0, 0.0, vec2<f32>(0.0), vec2<f32>(0.0), 1e9);
    }
    // Rolled distance stays the nearest texel's, unblended: the stamp writes a
    // place projected on the heading, so neighbours already agree to a
    // millimetre and averaging would only round the corners of that agreement.
    let along = f32(texel.b) * 0.004 + dot(near_out, near_roll);
    // A wheel that went one way and later came back leaves neighbouring texels
    // pointing opposite ways, and their blend is nothing at all; there the
    // nearest texel's own direction is the honest answer.
    let blended = roll_sum / weight;
    let roll = select(near_roll, normalize(blended), length(blended) > 0.3);
    let half_width = width_sum / weight;
    // How far off the middle, measured to the blended middle itself rather than
    // averaged out of four different frames' answers.
    let centre = centre_sum / weight;
    let across = dot(world_xz - centre, vec2<f32>(-roll.y, roll.x));
    // The mark is kept wherever *any* of the four says the tyre covers it, so a
    // turn does not cut its own track away; the middle is the well-behaved
    // reading above, so the two are taken together.
    let inside = min(closest, abs(across) / max(half_width, 0.01));
    return WheelMark(press, f32(texel.g & 15u) / 15.0, along / TREAD_PITCH_M, across / half_width,
        vec2<f32>(along, across), roll, inside);
}

// How much of a tyre's width covers this point: one in the middle of it, nought
// past its shoulder. Taken from the smooth offset across the tyre and never
// from how far the stamp reached — the stamp is whole texels of an eighth of a
// metre, and its edge gains one as a machine drifts sideways against that grid,
// then loses it and gains one on the other side metres later. That is a track
// forever getting a hand's breadth wider down one side and then the other, and
// it shows in a pressed sward as plainly as in a tread. Everything a wheel does
// to the ground is held inside this, so none of it steps.
//
// One where the map carries no tread channels to ask with: such a ground has
// only the stamp's own edge, and must live with it.
fn wheel_edge(tex: texture_2d<u32>, params: WheelMapParams, world_xz: vec2<f32>) -> f32 {
    let mark = sample_wheel_mark(tex, params, world_xz);
    if (mark.press <= 0.0) {
        return 1.0;
    }
    return 1.0 - smoothstep(0.88, 1.02, mark.inside);
}
