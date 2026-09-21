//! Wheel-contact collection shared by field surface packages.

use bevy::prelude::*;
use bevy::render::extract_resource::ExtractResource;

/// One wheel on the ground this frame: where, which way it rolls, how wide.
#[derive(Clone, Copy, Debug)]
pub struct WheelContact {
    pub position: Vec3,
    pub direction: Vec2,
    pub width: f32,
    /// How hard the tyre works the ground, 0..1: faint rolling straight,
    /// strong when it spins, slides sideways or twists through a turn.
    pub scrub: f32,
    /// Metres the *machine* has rolled; it places the tread along the track.
    pub travelled: f32,
    /// Where the machine was when it had rolled that far, in the same metres as
    /// `position`. The tread is measured from here and not from the wheel,
    /// because a tractor's wheels reach the same ground at different moments —
    /// the rear one a wheelbase later — and measured from each wheel they stamp
    /// phases a wheelbase apart onto the same texels. Measured from one place
    /// that moves with the machine, every wheel writes the same number for the
    /// same ground. The arm is short, a wheelbase at most, which matters: it
    /// multiplies any wobble of the heading, and from a far-off origin a half
    /// degree of steering correction would slide the whole tread by pitches.
    pub anchor: Vec2,
    /// The line this wheel runs down, as a place on it, with the frame-to-frame
    /// jitter of the contact taken out of it sideways. What the tread is
    /// measured across from, and it has to be steady: the contact itself moves
    /// a centimetre or two each frame as the physics settles, every texel keeps
    /// whichever frame last touched it, and since the bars are placed from the
    /// distance off the tyre's middle, the whole pattern steps sideways along
    /// the line where one frame's stamp meets the next's.
    pub centreline: Vec2,
    /// Physical rolling-direction footprint length; None uses the field default.
    pub length: Option<f32>,
}

impl WheelContact {
    pub fn footprint_length(&self, fallback: f32) -> f32 {
        self.length
            .filter(|v| v.is_finite() && *v >= 0.0)
            .unwrap_or(fallback)
    }
}

#[test]
fn explicit_wheel_footprint_overrides_field_default() {
    let mut wheel = WheelContact {
        position: Vec3::ZERO,
        direction: Vec2::X,
        width: 0.4,
        scrub: 0.0,
        travelled: 0.0,
        anchor: Vec2::ZERO,
        centreline: Vec2::ZERO,
        length: None,
    };
    assert_eq!(wheel.footprint_length(0.3), 0.3);
    for length in [0.0, 0.2, 0.6] {
        wheel.length = Some(length);
        assert_eq!(wheel.footprint_length(0.3), length);
    }
    wheel.length = Some(f32::NAN);
    assert_eq!(wheel.footprint_length(0.3), 0.3);
}

/// How many points of driven line the covers can read. The tread is only drawn
/// near to — it fades out by its own pitch against the pixel long before this
/// runs out — so this is the *recent* line and not the whole day's driving.
pub const TRAIL_POINTS: usize = 512;

/// Metres between the points a trail is kept as. Close enough that a turn is a
/// smooth curve rather than a polygon, far enough that a hundred and twenty
/// eight of them hold a useful stretch of it.
pub const TRAIL_STEP_M: f32 = 0.5;

/// The line a wheel has actually driven, as a line and not as a raster.
///
/// The tread used to be worked out from coordinates stamped into the wheel map,
/// one frame's snapshot of a turning, moving frame per texel. That cannot be
/// made right: a lug is a fifth of a metre and a texel an eighth, so the
/// pattern was always finer than the grid it was stored on; every texel held a
/// different frame's idea of where the tyre's middle was, which wavered through
/// turns; the four texels round a point had to be blended, which is meaningless
/// when each is measured in its own rotated frame; and two wheels crossing
/// contested the same texels.
///
/// A tyre track is a curve. Kept as one — the points the wheel passed through,
/// with the distance along the line at each — every one of those problems goes
/// away: where a point stands against the line is exact at any range, in a turn
/// as much as on a straight, and no two wheels can disagree because each has
/// its own line. It is what the ways laid out in a layout have always done,
/// which is why those print cleanly and this did not.
#[derive(Clone, Copy, Debug, Default)]
pub struct TrailPoint {
    /// Where the wheel's middle was, in world XZ.
    pub at: Vec2,
    /// Metres of line before this point.
    pub along: f32,
    /// Half the tyre's width. Negative marks the first point of a run, so a
    /// wheel lifted and set down elsewhere does not join the two with a line it
    /// never drove.
    pub half_width: f32,
    /// When the wheel was here. Kept on this side only — the covers are handed
    /// the line, not the clock — and it is what a point is dropped on.
    pub seen: f32,
}

/// How long a wheel's line is kept. A mark on the ground outlasts the machine
/// that made it, so the tread is held by age and not by how far the machine has
/// since driven: standing still or crawling, the line behind stays as long as
/// it would at speed. `TRAIL_POINTS` is still the ceiling — a wheel at working
/// speed fills its share before this runs out — so this governs the slow end,
/// which is where the old rule dropped a line almost at once.
pub const TRAIL_KEEP_S: f32 = 90.0;

/// Every wheel's recent line, ready for the covers.
#[derive(Resource, ExtractResource, Clone)]
pub struct WheelTrails {
    pub points: Vec<TrailPoint>,
    /// The tyre printing them, as `TyreTread::packed`.
    pub bar: Vec4,
}

impl Default for WheelTrails {
    fn default() -> Self {
        Self { points: Vec::new(), bar: crate::layout::TyreTread::default().packed() }
    }
}

/// Wheel contacts collected by the controllers during one frame, stamped
/// into the trample map by the render world.
#[derive(Resource, ExtractResource, Clone, Default)]
pub struct WheelContacts {
    pub contacts: Vec<WheelContact>,
    /// `Time::elapsed_secs_wrapped` this frame, the clock the stamps carry.
    pub now: f32,
}

/// Wheel contacts are stamped into a wrapped-time clock with this period;
/// it has to match `Time`'s wrap period. It is also the ceiling on how long any
/// mark can be read for: past it a stamp wraps and an old mark reads as a fresh
/// one. `SCAR_SECONDS` in `shaders/interaction.wgsl`, the slow clock a wheel's
/// bare scar greens over on, is held under this with room to spare for that
/// reason — a scar wanting longer than an hour needs a map of its own, not a
/// larger number here.
pub const TRAMPLE_CLOCK_S: f32 = 3600.0;

/// Encodes a stamp: time on the wrapped clock, then one channel packing the
/// roll direction (8 bits), where across the tyre the texel lies (4 bits,
/// -1..1) and the scrub (4 bits, never 0, so 0 means never stamped).
pub fn trample_texel(now: f32, direction: Vec2, across: f32, scrub: f32) -> [u16; 2] {
    // Floored so a stamp never lands ahead of the shader clock: a stamp in
    // the future would read as a whole clock period old and spring back up.
    let time = ((now.rem_euclid(TRAMPLE_CLOCK_S) / TRAMPLE_CLOCK_S) * 65535.0).floor() as u16;
    let angle = (direction.y.atan2(direction.x) + std::f32::consts::PI) / std::f32::consts::TAU;
    let angle = (angle * 255.0).round() as u16 & 255;
    let across = ((across.clamp(-1.0, 1.0) * 0.5 + 0.5) * 15.0).round() as u16;
    let scrub = ((scrub.clamp(0.0, 1.0) * 15.0).round() as u16).max(1);
    [time, angle << 8 | across << 4 | scrub]
}

pub(super) fn begin_wheel_contacts(mut contacts: ResMut<WheelContacts>, time: Res<Time>) {
    contacts.contacts.clear();
    contacts.now = time.elapsed_secs_wrapped();
}

/// Keeps each wheel's line as it drives: a point every `TRAIL_STEP_M`, the
/// oldest dropped when there is no room. Held per wheel, so two wheels can
/// cross without either taking the other's line for its own.
#[derive(Resource, Default)]
pub struct TrailKeeper {
    runs: std::collections::HashMap<String, Vec<TrailPoint>>,
    seen: std::collections::HashMap<String, f32>,
}

impl TrailKeeper {
    /// One wheel's place this frame. `now` is the clock the gaps are judged on:
    /// a wheel unseen for a moment starts a new run rather than joining across
    /// wherever it went in between.
    pub fn saw(&mut self, wheel: &str, at: Vec2, half_width: f32, now: f32) {
        let broken = self.seen.get(wheel).is_none_or(|last| now - last > 0.5);
        self.seen.insert(wheel.to_owned(), now);
        // Each wheel keeps its own share of the room, so one wheel driving hard
        // cannot crowd the others' lines out of the picture. Counted before the
        // run is taken out, since taking it borrows the whole set.
        let share = (TRAIL_POINTS / self.runs.len().max(1)).max(4);
        let run = self.runs.entry(wheel.to_owned()).or_default();
        if broken {
            run.push(TrailPoint { at, along: 0.0, half_width: -half_width.abs(), seen: now });
        } else {
            let Some(last) = run.last().copied() else { return };
            let step = at.distance(last.at);
            if step < TRAIL_STEP_M {
                return;
            }
            run.push(TrailPoint {
                at,
                along: last.along + step,
                half_width: half_width.abs(),
                seen: now,
            });
        }
        // Old points go first, then any still over the share; a run whose head
        // is cut has to say so, or the covers join it to whatever precedes it.
        let stale = run.iter().take_while(|point| now - point.seen > TRAIL_KEEP_S).count();
        let over = run.len().saturating_sub(share);
        let cut = stale.max(over);
        if cut > 0 {
            run.drain(..cut);
            if let Some(first) = run.first_mut() {
                first.half_width = -first.half_width.abs();
            }
        }
    }

    /// All the lines, flattened for the covers.
    pub fn flattened(&self) -> Vec<TrailPoint> {
        self.runs.values().flatten().copied().take(TRAIL_POINTS).collect()
    }

    /// Drops whole runs whose wheel has not been seen for `TRAIL_KEEP_S`, so a
    /// machine driven away and deleted does not hold its room for ever.
    pub fn forget_older_than(&mut self, now: f32) {
        self.runs.retain(|wheel, run| {
            run.retain(|point| now - point.seen <= TRAIL_KEEP_S);
            if let Some(first) = run.first_mut() {
                first.half_width = -first.half_width.abs();
            }
            !run.is_empty() && self.seen.get(wheel).is_some_and(|last| now - last <= TRAIL_KEEP_S)
        });
    }
}

/// Tread coordinates are stored in these steps; a lug pitch is 48 of the
/// first, and the rolled distance wraps on a whole number of pitches.
pub const TREAD_ALONG_STEP_M: f32 = 0.004;
pub const TREAD_ACROSS_STEP_M: f32 = 0.005;
const TREAD_ALONG_WRAP: f32 = 65520.0;

/// The two extra channels of a tread-printing map: how far the wheel had
/// rolled at this texel, then its signed offset from the tyre's centreline
/// (high byte, biased by 128) with the tyre's half width (low byte).
pub fn tread_texel(along_m: f32, across_m: f32, half_width_m: f32) -> [u16; 2] {
    let along = (along_m / TREAD_ALONG_STEP_M).round().rem_euclid(TREAD_ALONG_WRAP) as u16;
    let across = ((across_m / TREAD_ACROSS_STEP_M).round() + 128.0).clamp(0.0, 255.0) as u16;
    let half_width = (half_width_m / TREAD_ACROSS_STEP_M).round().clamp(1.0, 255.0) as u16;
    [along, across << 8 | half_width]
}

/// Gathers this frame's contacts into each wheel's line. Keyed by where the
/// contacts sit in the list, which is the order the controller walks a
/// machine's wheels and so holds from frame to frame.
pub(super) fn keep_wheel_trails(
    contacts: Res<WheelContacts>,
    mut keeper: ResMut<TrailKeeper>,
    mut trails: ResMut<WheelTrails>,
    time: Res<Time>,
) {
    if contacts.contacts.is_empty() {
        return;
    }
    let now = time.elapsed_secs();
    for (index, contact) in contacts.contacts.iter().enumerate() {
        keeper.saw(
            &index.to_string(),
            Vec2::new(contact.position.x, contact.position.z),
            contact.width * 0.5,
            now,
        );
    }
    keeper.forget_older_than(now);
    trails.points = keeper.flattened();
}

// A line is kept by age, so a machine that crawls or stands keeps the tread it
// laid; held by count alone, a slow wheel's line was dropped almost at once
// while a fast one's reached across the field.
#[test]
fn a_line_is_dropped_by_age_and_not_by_how_far_it_has_since_driven() {
    let mut keeper = TrailKeeper::default();
    for step in 0..40 {
        let along = step as f32 * TRAIL_STEP_M;
        keeper.saw("near", Vec2::new(along, 0.0), 0.3, step as f32);
    }
    let laid = keeper.flattened().len();
    assert!(laid > 30, "kept only {laid} points of a line just driven");

    // Ten seconds on and barely moved: nothing is dropped.
    keeper.saw("near", Vec2::new(20.0, 0.0), 0.3, 49.0);
    keeper.forget_older_than(49.0);
    assert_eq!(keeper.flattened().len(), laid + 1, "a slow wheel lost its line");

    // Past the keep, the head goes and what is left still says it is a head.
    keeper.saw("near", Vec2::new(40.0, 0.0), 0.3, 40.0 + TRAIL_KEEP_S);
    keeper.forget_older_than(40.0 + TRAIL_KEEP_S);
    let left = keeper.flattened();
    assert!(left.len() < laid, "nothing aged out after {TRAIL_KEEP_S}s");
    assert!(!left.is_empty() && left[0].half_width < 0.0, "a cut head must say so");

    // A wheel gone for good takes its room with it.
    keeper.forget_older_than(40.0 + TRAIL_KEEP_S * 3.0);
    assert!(keeper.flattened().is_empty(), "a wheel long gone still holds room");
}
