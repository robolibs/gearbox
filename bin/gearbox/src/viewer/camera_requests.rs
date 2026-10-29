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
}

/// Where a glide has the view after `eased` of the way (0 to 1): bearing the
/// short way round, distance by ratio so a zoom keeps its pace.
fn glided(from: &View, to: &View, eased: f64) -> View {
    let lerp = |a: f64, b: f64| a + (b - a) * eased;
    let mut view = *to;
    view.at.latitude = lerp(from.at.latitude, to.at.latitude);
    view.at.longitude = lerp(from.at.longitude, to.at.longitude);
    view.at.altitude = lerp(from.at.altitude, to.at.altitude);
    view.bearing_deg = from.bearing_deg + ((to.bearing_deg - from.bearing_deg + 540.0).rem_euclid(360.0) - 180.0) * eased;
    view.pitch_deg = lerp(from.pitch_deg, to.pitch_deg);
    view.distance_m = from.distance_m * (to.distance_m / from.distance_m.max(1e-3)).powf(eased);
    view.tidy();
    view
}

/// Carries the view along a place glide by the frame's sim time, eased at
/// both ends; anything else moving the view ends it.
fn glide_view(time: Res<Time>, mut glide: ResMut<PlaceGlide>, mut view: ResMut<View>) {
    let Some(g) = glide.0.as_mut() else {
        return;
    };
    let last = glided(&g.from, &g.to, smoothstep(g.elapsed / g.seconds));
    if g.elapsed > 0.0 && !same_view(&view, &last) {
        glide.0 = None;
        return;
    }
    g.elapsed = (g.elapsed + time.delta_secs()).min(g.seconds);
    *view = glided(&g.from, &g.to, smoothstep(g.elapsed / g.seconds));
    if g.elapsed >= g.seconds {
        glide.0 = None;
    }
}

fn smoothstep(t: f32) -> f64 {
    let t = f64::from(t.clamp(0.0, 1.0));
    t * t * (3.0 - 2.0 * t)
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
            fly.target = Some(match glide_seconds(&text) {
                Some(seconds) => FlyTarget::gliding(machine, body, &view, look, seconds),
                None => FlyTarget::looking(machine, body, &view, look),
            });
        }
        // `look bearing=… [lat=… lon=…] [over=S]` without a machine: a view
        // of a place, jumped to, or glided to over S seconds of sim time.
        ("look", None) if text.split_whitespace().nth(1).is_none_or(|word| word.contains('=')) => {
            let look = look_around(&text);
            fly.target = None;
            let mut to = *view;
            to.bearing_deg = look.bearing_deg.unwrap_or(to.bearing_deg);
            to.pitch_deg = look.pitch_deg.unwrap_or(to.pitch_deg);
            to.distance_m = look.distance_m.unwrap_or(to.distance_m);
            to.at.altitude = look.above_m.unwrap_or(to.at.altitude);
            to.at.latitude = look.latitude.unwrap_or(to.at.latitude);
            to.at.longitude = look.longitude.unwrap_or(to.at.longitude);
            to.tidy();
            match glide_seconds(&text).filter(|seconds| *seconds > 0.0) {
                Some(seconds) => glide.0 = Some(Glide { from: *view, to, seconds, elapsed: 0.0 }),
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
    fn a_place_glide_turns_the_short_way_and_zooms_by_ratio() {
        let mut from = View::default();
        from.bearing_deg = 170.0;
        from.pitch_deg = 20.0;
        from.distance_m = 10.0;
        let mut to = from;
        to.bearing_deg = -170.0;
        to.pitch_deg = 60.0;
        to.distance_m = 1000.0;
        to.at.latitude += 0.001;
        let half = glided(&from, &to, 0.5);
        assert!((half.bearing_deg.abs() - 180.0).abs() < 1e-9, "turned the long way: {}", half.bearing_deg);
        assert!((half.distance_m - 100.0).abs() < 1e-6, "zoomed by difference: {}", half.distance_m);
        assert!((half.pitch_deg - 40.0).abs() < 1e-9);
        assert!(same_view(&glided(&from, &to, 1.0), &to), "ends off its target");
        assert!(same_view(&glided(&from, &to, 0.0), &from), "starts off where it was");
        assert_eq!(smoothstep(0.5), 0.5);
        assert_eq!(smoothstep(3.0), 1.0);
    }
}
