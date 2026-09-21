//! Where the view is, and everything that moves it.
//!
//! **The view is a place on the planet, not a point in a site.** It is a
//! latitude and longitude, a height above the ground there, a compass bearing
//! to look from, a pitch above that horizon and a distance to stand back. None
//! of those mean anything different from one datum to the next, so when the
//! ground is re-anchored underneath — which happens whenever the view wanders
//! more than a datum's reach — the view does not have to be told, corrected, or
//! caught up. It is already right.
//!
//! That is the whole point of the rewrite. Before, the camera was a rig in the
//! current site's flat frame and nine systems wrote to it: the pointer, the
//! wheel, the keys, the follow, two cinematics, the gamepad, the bookmarks, and
//! the site change itself, which moved the ground out from under all the
//! others. Asking it to stand six metres up could leave it two hundred
//! kilometres out over the wrong ocean, because each writer was correcting a
//! frame a different writer had already moved.
//!
//! So: **one state, one writer.** Everything that wants to move the view edits
//! [`View`], in planet terms. [`place`] alone turns [`View`] into the rig the
//! renderer reads, once a frame, after the datum for this frame has been
//! chosen. Nothing else writes `ChaseCamera`, and `View` never mentions a site.
//!
//! Distances are geometric, never linear in metres: a notch of the wheel is a
//! *factor*, so it is centimetres near a furrow and hundreds of kilometres from
//! orbit, and there is no scale at which the controls do nothing.

use bevy::prelude::*;
use gearbox_globe::{Datum, Geodetic};
use mara::ui::modules::bevy::{BevyViewportInput, ChaseCamera, apply_rig};

use crate::globe::Sites;

/// What one notch of the wheel multiplies the distance by.
const ZOOM_PER_NOTCH: f64 = 1.35;
/// The viewport reports scroll already divided into 120-point units.
const SCROLL_NOTCHES_PER_UNIT: f64 = 2.4;
/// The eye never comes nearer the ground than this.
const CLEARANCE_M: f64 = 0.4;
/// Closest and furthest the eye may stand from what it looks at.
const CLOSEST_M: f64 = 0.5;
const FURTHEST_M: f64 = gearbox_globe::PLANET_RADIUS_M * 12.0;
/// Pitch is kept off both poles: straight down has no bearing, and below the
/// horizon puts the eye under the ground.
const PITCH_MIN_DEG: f64 = 2.0;
const PITCH_MAX_DEG: f64 = 88.0;
/// How much of its height above ground a held key crosses each second, the
/// crawl it keeps at ground level, and what Shift multiplies it by.
const FLY_PER_S: f64 = 0.9;
const FLY_FLOOR_MPS: f64 = 1.5;
const FLY_BOOST: f64 = 5.0;
/// The crawl a pan keeps when the view is drawn right up against what it
/// orbits, so a drag still answers there.
const PAN_FLOOR_M: f64 = 0.01;

/// Where the view is. The one piece of camera state; everything else is
/// derived from it every frame.
#[derive(Resource, Clone, Copy, Debug)]
pub struct View {
    /// The place being looked at: latitude and longitude in degrees, and
    /// `altitude` as metres above the ground there rather than the ellipsoid,
    /// so the view keeps its height over hills.
    pub at: Geodetic,
    /// Compass bearing from that place out to the eye, in degrees.
    pub bearing_deg: f64,
    /// How far above that place's horizon the eye stands, in degrees.
    pub pitch_deg: f64,
    /// Eye to the place it is looking at, in metres.
    pub distance_m: f64,
}

impl Default for View {
    fn default() -> Self {
        Self {
            at: Geodetic::new(52.370216, 4.895168, 0.0),
            bearing_deg: 205.0,
            pitch_deg: 20.0,
            distance_m: 14.0,
        }
    }
}

impl View {
    /// How far above the ground the eye itself stands.
    pub fn eye_height_m(&self) -> f64 {
        self.at.altitude + self.distance_m * self.pitch_deg.to_radians().sin()
    }

    /// Stands the eye this far above the ground, by backing off rather than by
    /// lifting what it is looking at.
    pub fn set_eye_height_m(&mut self, height: f64) {
        let lift = self.pitch_deg.to_radians().sin().max(1.0e-3);
        self.distance_m = ((height - self.at.altitude) / lift).clamp(CLOSEST_M, FURTHEST_M);
    }

    /// Moves the place being looked at `metres` along the ground on `bearing`,
    /// around the planet rather than across a flat frame, so it stays right
    /// however far it goes.
    pub fn walk(&mut self, bearing_deg: f64, metres: f64) {
        if !metres.is_finite() || metres == 0.0 {
            return;
        }
        let stepped = Datum::at(self.at.latitude, self.at.longitude).travelled(bearing_deg, metres);
        self.at.latitude = stepped.latitude;
        self.at.longitude = stepped.longitude;
    }

    /// A view part of the way from one to another, for the cinematics. The
    /// distance travels geometrically, like the wheel, so a move that pulls
    /// back a thousandfold looks even rather than arriving all at the end.
    /// `turn` says whether the bearing and pitch come along too.
    pub fn between(from: &Self, to: &Self, t: f64, turn: bool) -> Self {
        let t = t.clamp(0.0, 1.0);
        let mix = |a: f64, b: f64| a + (b - a) * t;
        let swing = |a: f64, b: f64| {
            let step = (b - a + 540.0).rem_euclid(360.0) - 180.0;
            a + step * t
        };
        let mut view = Self {
            at: Geodetic::new(
                mix(from.at.latitude, to.at.latitude),
                mix(from.at.longitude, to.at.longitude),
                mix(from.at.altitude, to.at.altitude),
            ),
            bearing_deg: match turn {
                true => swing(from.bearing_deg, to.bearing_deg),
                false => from.bearing_deg,
            },
            pitch_deg: match turn {
                true => mix(from.pitch_deg, to.pitch_deg),
                false => from.pitch_deg,
            },
            distance_m: from.distance_m.max(CLOSEST_M)
                * (to.distance_m.max(CLOSEST_M) / from.distance_m.max(CLOSEST_M)).powf(t),
        };
        view.tidy();
        view
    }

    /// The direction from the place being looked at out to the eye, in that
    /// place's own north/up/east axes.
    fn eye_direction(&self) -> DVec3 {
        let (pitch, bearing) = (self.pitch_deg.to_radians(), self.bearing_deg.to_radians());
        DVec3::new(
            pitch.cos() * bearing.cos(),
            pitch.sin(),
            pitch.cos() * bearing.sin(),
        )
    }

    fn tidy(&mut self) {
        if !self.at.latitude.is_finite() || !self.at.longitude.is_finite() {
            self.at = Self::default().at;
        }
        self.at.latitude = self.at.latitude.clamp(-89.9, 89.9);
        self.at.longitude = (self.at.longitude + 540.0).rem_euclid(360.0) - 180.0;
        self.at.altitude = if self.at.altitude.is_finite() {
            self.at.altitude.max(0.0)
        } else {
            0.0
        };
        self.bearing_deg = (self.bearing_deg + 540.0).rem_euclid(360.0) - 180.0;
        self.pitch_deg = self.pitch_deg.clamp(PITCH_MIN_DEG, PITCH_MAX_DEG);
        self.distance_m = if self.distance_m.is_finite() {
            self.distance_m.clamp(CLOSEST_M, FURTHEST_M)
        } else {
            Self::default().distance_m
        };
    }
}

use bevy::math::DVec3;

pub struct CameraPlugin;

/// Everything that edits [`View`] runs in here, so that [`place`] — the only
/// writer of the rig — can run once, after all of them, and after the datum for
/// this frame has been settled.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct MoveView;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<View>()
            .add_systems(Update, (orbit, pan, zoom, fly).chain().in_set(MoveView));
    }
}

/// A primary drag turns the view about what it is looking at.
fn orbit(
    input: Res<BevyViewportInput>,
    grab: Res<crate::viewer::systems::GizmoGrab>,
    context: Res<crate::viewer::machine_context::MachineHover>,
    mut view: ResMut<View>,
) {
    let delta = Vec2::from(input.drag_delta);
    if grab.0 || context.captures_pointer || delta == Vec2::ZERO {
        return;
    }
    // Pixels to degrees: a drag across the frame swings most of the way round.
    const PER_PIXEL_DEG: f64 = 0.25;
    view.bearing_deg += delta.x as f64 * PER_PIXEL_DEG;
    view.pitch_deg += delta.y as f64 * PER_PIXEL_DEG;
    view.tidy();
}

/// A middle drag carries the view across the ground; with Shift it lifts what
/// the view is looking at instead.
fn pan(
    input: Res<BevyViewportInput>,
    keys: Res<ButtonInput<KeyCode>>,
    context: Res<crate::viewer::machine_context::MachineHover>,
    mut follow: ResMut<crate::viewer::state::FollowTarget>,
    mut view: ResMut<View>,
    cameras: Query<(&Projection, &Camera)>,
) {
    let delta = Vec2::from(input.pan_delta);
    if context.captures_pointer || delta == Vec2::ZERO {
        return;
    }
    let Ok((lens, camera)) = cameras.single() else {
        return;
    };
    let across_px = camera.physical_target_size().map(|size| size.x as f32).unwrap_or(0.0);
    let pace = pan_pace(&view, lens, across_px);
    // Going somewhere of one's own lets a followed machine go, or the follow
    // would write the view back onto it before the drag is ever drawn.
    follow.set(None);
    if keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight) {
        view.at.altitude = (view.at.altitude + delta.y as f64 * pace).max(0.0);
        view.tidy();
        return;
    }
    // The ground travels with the cursor, so the view travels against it: drag
    // right and the ground comes right while the view goes left, drag down and
    // the ground comes toward you. Both along the bearing the eye looks down.
    let look = view.bearing_deg + 180.0;
    view.walk(look, delta.y as f64 * pace);
    view.walk(look + 90.0, -delta.x as f64 * pace);
    view.tidy();
}

/// How many metres one pixel of a pan carries, at the depth the view is looking
/// at: the width the frame covers there, divided by the pixels across it. That
/// rule needs no scale of its own — whatever is under the cursor stays under
/// the cursor, in a furrow and from orbit alike.
fn pan_pace(view: &View, lens: &Projection, across_px: f32) -> f64 {
    let Projection::Perspective(perspective) = lens else {
        return (view.distance_m * 0.002).max(PAN_FLOOR_M);
    };
    if !(across_px > 1.0) {
        return (view.distance_m * 0.002).max(PAN_FLOOR_M);
    }
    let spread = 2.0 * (perspective.fov as f64 * 0.5).tan();
    (view.distance_m * spread / across_px as f64).max(PAN_FLOOR_M)
}

/// The wheel. Multiplies the distance rather than adding to it.
fn zoom(input: Res<BevyViewportInput>, mut view: ResMut<View>) {
    if input.scroll_delta == 0.0 {
        return;
    }
    let notches = input.scroll_delta as f64 * SCROLL_NOTCHES_PER_UNIT;
    view.distance_m *= ZOOM_PER_NOTCH.powf(-notches);
    view.tidy();
}

/// W/A/S/D walk the view over the ground, Q/E raise and lower what it looks at,
/// each at a share of how high the eye is standing.
fn fly(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    panel: Res<crate::viewer::drive::MachinePanel>,
    mut follow: ResMut<crate::viewer::state::FollowTarget>,
    mut view: ResMut<View>,
) {
    if panel.keyboard.is_some() {
        return;
    }
    let axis = |neg: KeyCode, pos: KeyCode| (keys.pressed(pos) as i8 - keys.pressed(neg) as i8) as f64;
    let ahead = axis(KeyCode::KeyS, KeyCode::KeyW);
    let aside = axis(KeyCode::KeyA, KeyCode::KeyD);
    let lift = axis(KeyCode::KeyQ, KeyCode::KeyE);
    if ahead == 0.0 && aside == 0.0 && lift == 0.0 {
        return;
    }
    let boost = match keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight) {
        true => FLY_BOOST,
        false => 1.0,
    };
    let pace = (view.eye_height_m() * FLY_PER_S).max(FLY_FLOOR_MPS) * boost * time.delta_secs_f64();
    follow.set(None);
    // Forward is where the eye is looking, which is opposite where it stands.
    let look = view.bearing_deg + 180.0;
    view.walk(look, ahead * pace);
    view.walk(look + 90.0, aside * pace);
    view.at.altitude = (view.at.altitude + lift * pace).max(0.0);
    view.tidy();
}

/// Turns [`View`] into the rig the renderer reads. The only writer of
/// `ChaseCamera`, and the only place a site is ever mentioned.
pub fn place(
    keys: Res<ButtonInput<KeyCode>>,
    sites: Res<Sites>,
    mut view: ResMut<View>,
    mut cameras: Query<(&mut ChaseCamera, &mut Transform, &mut Projection)>,
) {
    view.tidy();
    let Ok((mut cam, mut transform, mut projection)) = cameras.single_mut() else {
        return;
    };
    let frame = sites.current().frame;
    let here = Datum::at(view.at.latitude, view.at.longitude);
    // Where the view is looking, in the flat frame of the site holding it.
    let local = frame.from_ecef(here.origin);
    let (x, z) = (local.x as f32, local.z as f32);
    let ground = sites.height(sites.current, x, z) as f64;

    // The eye's direction is carried through ECEF, so it means the same thing
    // whichever datum happens to be current.
    let toward_eye = frame.rotation.inverse() * (here.rotation * view.eye_direction());
    let mut distance = view.distance_m;
    // The eye never sinks into the ground. Alt lets it, for looking underneath.
    //
    // How far the eye rises per metre of distance is the view's own pitch, and
    // nothing else: taken from the direction in the site's frame instead, it
    // collapses toward zero whenever the view is far from the datum serving it
    // — which it is for the one frame after a jump across the planet — and a
    // clearance divided by nearly nothing threw the eye into orbit, where it
    // stayed. Pitch is never less than a couple of degrees, so this cannot.
    if !(keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight)) {
        let lift = view.pitch_deg.to_radians().sin().max(PITCH_MIN_DEG.to_radians().sin());
        let lowest = ((CLEARANCE_M - view.at.altitude) / lift).max(CLOSEST_M);
        distance = distance.max(lowest).clamp(CLOSEST_M, FURTHEST_M);
        if distance != view.distance_m {
            view.distance_m = distance;
        }
    }

    cam.min_distance = CLOSEST_M as f32;
    cam.max_distance = FURTHEST_M as f32;
    cam.focus = Vec3::new(x, (ground + view.at.altitude) as f32, z);
    cam.yaw = toward_eye.x.atan2(toward_eye.z) as f32;
    cam.elevation = toward_eye.y.clamp(-1.0, 1.0).asin() as f32;
    cam.distance = distance as f32;

    // Depth precision falls with the square of distance over the near plane, so
    // the near plane backs off as the view does; nothing is ever that close to
    // an eye that far out.
    let near = (distance / 200.0).clamp(0.05, 100_000.0) as f32;
    if let Projection::Perspective(lens) = projection.as_mut()
        && (lens.near - near).abs() > near * 0.05
    {
        lens.near = near;
    }
    apply_rig(&cam, &mut transform);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walking_a_kilometre_north_moves_a_kilometre_north() {
        let mut view = View::default();
        let from = view.at;
        view.walk(0.0, 1_000.0);
        let north = Geodetic::new(from.latitude, from.longitude, 0.0)
            .ecef()
            .distance(Geodetic::new(view.at.latitude, view.at.longitude, 0.0).ecef());
        assert!((north - 1_000.0).abs() < 5.0, "walked {north:.1} m");
        assert!(view.at.latitude > from.latitude, "north raises the latitude");
        assert!((view.at.longitude - from.longitude).abs() < 1.0e-6);
    }

    #[test]
    fn walking_east_and_back_returns_to_the_same_place() {
        let mut view = View::default();
        let from = view.at;
        view.walk(90.0, 5_000.0);
        assert!(view.at.longitude > from.longitude);
        view.walk(270.0, 5_000.0);
        assert!((view.at.latitude - from.latitude).abs() < 1.0e-4);
        assert!((view.at.longitude - from.longitude).abs() < 1.0e-4);
    }

    #[test]
    fn a_height_asked_for_is_the_height_the_eye_stands_at() {
        let mut view = View::default();
        for height in [1.0, 6.0, 300.0, 25_000.0] {
            view.set_eye_height_m(height);
            let got = view.eye_height_m();
            assert!((got - height).abs() < height * 0.01, "asked {height}, stood at {got}");
        }
    }

    #[test]
    fn the_eye_stands_where_its_bearing_and_pitch_say() {
        let mut view = View::default();
        view.bearing_deg = 0.0;
        view.pitch_deg = 0.0;
        // Due north of what it looks at, level with it.
        let direction = view.eye_direction();
        assert!((direction.x - 1.0).abs() < 1.0e-9, "north");
        assert!(direction.y.abs() < 1.0e-9, "level");
        assert!(direction.z.abs() < 1.0e-9, "not east");
        view.bearing_deg = 90.0;
        let direction = view.eye_direction();
        assert!((direction.z - 1.0).abs() < 1.0e-9, "east");
        view.pitch_deg = 90.0;
        let direction = view.eye_direction();
        assert!((direction.y - 1.0).abs() < 1.0e-9, "overhead");
    }

    /// The clearance the eye keeps must never depend on which datum happens to
    /// be serving the ground, or a jump across the planet throws it into orbit
    /// on the frame before the datum catches up.
    #[test]
    fn standing_clear_of_the_ground_costs_the_same_wherever_the_datum_is() {
        for pitch in [PITCH_MIN_DEG, 20.0, 45.0, PITCH_MAX_DEG] {
            let lift = pitch.to_radians().sin();
            let lowest = ((CLEARANCE_M - 0.0) / lift).max(CLOSEST_M);
            assert!(
                lowest < 20.0,
                "a pitch of {pitch} deg asked for {lowest:.1} m of distance to clear {CLEARANCE_M} m"
            );
        }
    }

    #[test]
    fn nothing_the_view_is_given_can_leave_it_unusable() {
        let mut view = View::default();
        view.at.latitude = f64::NAN;
        view.distance_m = f64::INFINITY;
        view.pitch_deg = 900.0;
        view.tidy();
        assert!(view.at.latitude.is_finite());
        assert!(view.distance_m.is_finite() && view.distance_m <= FURTHEST_M);
        assert!(view.pitch_deg <= PITCH_MAX_DEG);
        view.at.longitude = 370.0;
        view.tidy();
        assert!((-180.0..=180.0).contains(&view.at.longitude));
    }
}
