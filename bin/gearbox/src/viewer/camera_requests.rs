//! `gearbox instance camera …` drops `fly ID`, `follow ID`, `unfollow`,
//! `goto …` or `look …` in `<registry dir>/<name>.camera`; this does what the
//! Agents pane rows do, and sets exact views for scripted captures.

use bevy::prelude::*;


use crate::controller::{ControllerInventory, MachineRef};
use crate::viewer::state::{ChaseCameraFly, FlyTarget, FollowTarget, LookAround};
use crate::viewer::camera::{MoveView, View};
use crate::viewer::systems::machine_body_entity;

pub struct CameraRequestsPlugin;

impl Plugin for CameraRequestsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(RequestPoll(Timer::from_seconds(0.25, TimerMode::Repeating)))
            .init_resource::<PlaceGlide>()
            .add_systems(Update, serve_camera_requests)
            .add_systems(Update, glide_view.in_set(MoveView).after(serve_camera_requests));
    }
}

#[derive(Resource)]
struct RequestPoll(Timer);

/// A view of a place gliding to another over `seconds` of sim time.
#[derive(Resource, Default)]
struct PlaceGlide(Option<Glide>);

struct Glide {
    from: View,
    to: View,
    seconds: f32,
    elapsed: f32,
    /// How fast the place set out from keeps moving, degrees and metres a
    /// second: the pace of a held shot the glide took over from.
    drift: [f64; 3],
}

impl Glide {
    /// Where the glide has the view after `elapsed` seconds.
    fn at(&self, elapsed: f32) -> View {
        let s = f64::from(elapsed);
        let mut from = self.from;
        from.at.latitude += self.drift[0] * s;
        from.at.longitude += self.drift[1] * s;
        from.at.altitude += self.drift[2] * s;
        View::over(&from, &self.to, ease(elapsed / self.seconds))
    }
}

/// Carries the view along a place glide by the frame's sim time, eased at
/// both ends and craned over a long move; anything else moving the view
/// ends it.
fn glide_view(time: Res<Time>, mut glide: ResMut<PlaceGlide>, mut view: ResMut<View>) {
    let Some(g) = glide.0.as_mut() else {
        return;
    };
    if g.elapsed > 0.0 && !same_view(&view, &g.at(g.elapsed)) {
        glide.0 = None;
        return;
    }
    g.elapsed = (g.elapsed + time.delta_secs()).min(g.seconds);
    *view = g.at(g.elapsed);
    if g.elapsed >= g.seconds {
        glide.0 = None;
    }
}

fn ease(t: f32) -> f64 {
    crate::viewer::camera::ease(f64::from(t))
}

fn same_view(a: &View, b: &View) -> bool {
    let close = |x: f64, y: f64| (x - y).abs() < 1e-9;
    close(a.at.latitude, b.at.latitude)
        && close(a.at.longitude, b.at.longitude)
        && close(a.at.altitude, b.at.altitude)
        && close(a.bearing_deg, b.bearing_deg)
        && close(a.pitch_deg, b.pitch_deg)
        && close(a.distance_m, b.distance_m)
}

fn serve_camera_requests(
    time: Res<Time>,
    mut poll: ResMut<RequestPoll>,
    inventory: Res<ControllerInventory>,
    mut follow: ResMut<FollowTarget>,
    mut fly: ResMut<ChaseCameraFly>,
    index: Res<crate::prim_index::PrimIndex>,
    mut view: ResMut<View>,
    mut goto: ResMut<crate::globe::Goto>,
    mut glide: ResMut<PlaceGlide>,
) {
    if !poll.0.tick(time.delta()).just_finished() {
        return;
    }
    let name = std::env::var("GEARBOX_NAME").unwrap_or_else(|_| "gearbox".to_string());
    let request = gearbox_api::registry::registry_dir().join(format!("{name}.camera"));
    let Ok(text) = std::fs::read_to_string(&request) else {
        return;
    };
    let _ = std::fs::remove_file(&request);
    glide.0 = None;
    let mut words = text.split_whitespace();
    let action = words.next().unwrap_or("");
    let machine = words.next().and_then(|id| {
        inventory
            .machines
            .iter()
            .find(|m| m.id == id)
            .and_then(MachineRef::of)
    });
    info!("gearbox-viewer: camera request `{}`", text.trim());
    match (action, machine) {
        ("fly", Some(machine)) => {
            let body = machine_body_entity(&machine, &inventory, &index);
            fly.target = Some(FlyTarget::new(machine, body, &view));
        }
        ("follow", Some(machine)) => follow.set(Some(machine)),
        ("unfollow", _) => {
            follow.set(None);
            if fly.target.as_ref().is_some_and(|target| target.hold) {
                fly.target = None;
            }
        }
        // `goto LAT LON [HEIGHT_M]`: a place on Earth, in degrees, and how far
        // above its ground to stand.
        ("goto", _) => {
            let mut numbers = text.split_whitespace().skip(1).filter_map(|w| w.parse::<f64>().ok());
            if let (Some(latitude), Some(longitude)) = (numbers.next(), numbers.next()) {
                follow.set(None);
                goto.0 = Some(crate::globe::Jump {
                    latitude,
                    longitude,
                    height_m: numbers.next(),
                });
            }
        }
        // `look [ID] bearing=DEG pitch=DEG distance=M [over=S]`: an exact
        // view, around the machine when one is named, its bearing counted
        // from behind it. With `over`, the view glides there over S seconds
        // and holds it, tracking the machine, until something else moves it.
        ("look", Some(machine)) => {
            let body = machine_body_entity(&machine, &inventory, &index);
            let look = look_around(&text);
            let next = match glide_seconds(&text) {
                Some(seconds) => FlyTarget::gliding(machine, body, &view, look, seconds),
                None => FlyTarget::looking(machine, body, &view, look),
            };
            fly.target = Some(next.following(fly.target.as_ref()));
        }
        // `look bearing=… [lat=… lon=…] [over=S]` without a machine: a view
        // of a place, jumped to, or glided to over S seconds of sim time.
        ("look", None) if text.split_whitespace().nth(1).is_none_or(|word| word.contains('=')) => {
            let look = look_around(&text);
            let drift = fly.target.take().and_then(|held| held.anchor.filter(|_| held.hold)).map_or([0.0; 3], |a| a.rate);
            let mut to = *view;
            to.bearing_deg = look.bearing_deg.unwrap_or(to.bearing_deg);
            to.pitch_deg = look.pitch_deg.unwrap_or(to.pitch_deg);
            to.distance_m = look.distance_m.unwrap_or(to.distance_m);
            to.at.altitude = look.above_m.unwrap_or(to.at.altitude);
            to.at.latitude = look.latitude.unwrap_or(to.at.latitude);
            to.at.longitude = look.longitude.unwrap_or(to.at.longitude);
            to.tidy();
            match glide_seconds(&text).filter(|seconds| *seconds > 0.0) {
                Some(seconds) => glide.0 = Some(Glide { from: *view, to, seconds, elapsed: 0.0, drift }),
                None => *view = to,
            }
        }
        _ => warn!(
            "gearbox-viewer: camera request `{}` not understood",
            text.trim()
        ),
    }
}

/// The `over=` of a `look` request: how long its glide takes, in seconds.
fn glide_seconds(text: &str) -> Option<f32> {
    text.split_whitespace()
        .filter_map(|word| word.strip_prefix("over="))
        .find_map(|value| value.parse::<f32>().ok())
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
}

/// The `bearing=`, `pitch=`, `distance=`, `above=`, `lat=`, `lon=` and `compass=` values of a `look` request.
fn look_around(text: &str) -> LookAround {
    let mut look = LookAround::default();
    for (key, value) in text.split_whitespace().filter_map(|word| word.split_once('=')) {
        let Ok(value) = value.parse::<f64>() else {
            continue;
        };
        match key {
            "bearing" => look.bearing_deg = Some(value),
            "pitch" => look.pitch_deg = Some(value),
            "distance" => look.distance_m = Some(value),
            "above" => look.above_m = Some(value),
            "lat" => look.latitude = Some(value),
            "lon" => look.longitude = Some(value),
            "compass" => look.compass = value != 0.0,
            _ => {}
        }
    }
    look
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_glide_taking_over_from_a_moving_shot_sets_off_at_its_pace() {
        let from = View::default();
        let mut to = from;
        to.distance_m = 40.0;
        to.tidy();
        let glide = Glide { from, to, seconds: 6.0, elapsed: 0.0, drift: [0.0, 0.0, 0.3] };
        let frame = 1.0 / 60.0;
        let climbed = (glide.at(frame).at.altitude - glide.at(0.0).at.altitude) / f64::from(frame);
        assert!((climbed - 0.3).abs() < 0.01, "set off at {climbed} m/s, not 0.3");
        assert!(same_view(&glide.at(6.0), &to), "ended off its target");
    }

    #[test]
    fn a_place_glide_turns_the_short_way_and_zooms_by_ratio() {
        let mut from = View::default();
        from.bearing_deg = 170.0;
        from.pitch_deg = 20.0;
        from.distance_m = 10.0;
        let mut to = from;
        to.bearing_deg = -170.0;
        to.pitch_deg = 60.0;
        to.distance_m = 1000.0;
        let half = View::over(&from, &to, 0.5);
        assert!((half.bearing_deg.abs() - 180.0).abs() < 1e-9, "turned the long way: {}", half.bearing_deg);
        assert!((half.distance_m - 100.0).abs() < 1e-6, "zoomed by difference: {}", half.distance_m);
        assert!((half.pitch_deg - 40.0).abs() < 1e-9);
        assert!(same_view(&View::over(&from, &to, 1.0), &to), "ends off its target");
        assert!(same_view(&View::over(&from, &to, 0.0), &from), "starts off where it was");
        assert_eq!(ease(0.5), 0.5);
        assert_eq!(ease(3.0), 1.0);
        let rate = |t: f32| (ease(t + 1e-3) - ease(t)) / 1e-3;
        let pull = |t: f32| (rate(t + 1e-3) - rate(t)) / 1e-3;
        assert!(rate(0.0).abs() < 1e-4 && rate(0.999).abs() < 1e-4, "not at rest at its ends");
        assert!(pull(0.0).abs() < 0.5 && pull(0.998).abs() < 0.5, "jolts as it sets off or stops");
    }
}
