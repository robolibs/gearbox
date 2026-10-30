//! `gearbox instance camera …` drops `fly ID`, `follow ID`, `unfollow`,
//! `goto …`, `look …` or `path FILE` in `<registry dir>/<name>.camera`; this
//! does what the Agents pane rows do, and sets exact views and smooth paths
//! for scripted captures.

use bevy::prelude::*;


use crate::controller::{ControllerInventory, MachineRef};
use crate::viewer::state::{ChaseCameraFly, FlyTarget, FollowTarget, LookAround, Steadied};
use crate::viewer::camera::{MoveView, View};
use crate::viewer::systems::{HOLD_FOLLOW, machine_body_entity, spring};

pub struct CameraRequestsPlugin;

impl Plugin for CameraRequestsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(RequestPoll(Timer::from_seconds(0.25, TimerMode::Repeating)))
            .init_resource::<PlaceGlide>()
            .init_resource::<CameraPath>()
            .add_systems(Update, serve_camera_requests)
            .add_systems(Update, (glide_view, fly_path).in_set(MoveView).after(serve_camera_requests));
    }
}

/// A scripted path: views to pass at set seconds and, between them, the view
/// on one smooth curve through them all, so a take never stops on its way.
#[derive(Resource, Default)]
struct CameraPath(Option<Path>);

struct Path {
    keys: Vec<Key>,
    elapsed: f64,
}

/// A view the path passes `at` seconds in.
struct Key {
    at: f64,
    aim: Aim,
}

/// A place seen from a set bearing, pitch and distance, or a machine seen
/// so, its place followed on a held shot's spring; bearings are compass
/// bearings.
enum Aim {
    Place(View),
    Machine { body: Entity, look: LookAround, held: Option<Steadied> },
}

impl Aim {
    fn view(&mut self, dt: f64, sites: &crate::globe::Sites, transforms: &Query<&GlobalTransform>) -> View {
        match self {
            Aim::Place(view) => *view,
            Aim::Machine { body, look, held } => {
                if let Ok(gt) = transforms.get(*body) {
                    let mut at = sites.place_of(gt.translation());
                    at.altitude += look.above_m.unwrap_or(0.0);
                    *held = Some(match *held {
                        Some(s) => {
                            let (lat, lat_v) = spring(s.at.latitude, s.rate[0], at.latitude, HOLD_FOLLOW, dt);
                            let (lon, lon_v) = spring(s.at.longitude, s.rate[1], at.longitude, HOLD_FOLLOW, dt);
                            let (alt, alt_v) = spring(s.at.altitude, s.rate[2], at.altitude, HOLD_FOLLOW, dt);
                            Steadied { at: gearbox_globe::Geodetic::new(lat, lon, alt), rate: [lat_v, lon_v, alt_v], behind: 0.0 }
                        }
                        None => Steadied { at, rate: [0.0; 3], behind: 0.0 },
                    });
                }
                let mut view = View {
                    at: held.map_or(View::default().at, |s| s.at),
                    bearing_deg: look.bearing_deg.unwrap_or(0.0),
                    pitch_deg: look.pitch_deg.unwrap_or(30.0),
                    distance_m: look.distance_m.unwrap_or(10.0),
                };
                view.tidy();
                view
            }
        }
    }
}

/// Carries the view along a scripted path by the frame's sim time, then
/// holds its last view.
fn fly_path(
    time: Res<Time>,
    mut path: ResMut<CameraPath>,
    sites: Res<crate::globe::Sites>,
    transforms: Query<&GlobalTransform>,
    mut view: ResMut<View>,
) {
    let Some(p) = path.0.as_mut() else {
        return;
    };
    let dt = time.delta_secs_f64();
    p.elapsed += dt;
    let views: Vec<View> = p.keys.iter_mut().map(|k| k.aim.view(dt, &sites, &transforms)).collect();
    let times: Vec<f64> = p.keys.iter().map(|k| k.at).collect();
    *view = along(&times, &views, p.elapsed);
}

/// The view `t` seconds along a path through `views` reached at `times`: a
/// curve through each, its pace changing smoothly, at rest at either end,
/// and never past a view in any term (place, height, bearing, pitch or
/// distance) where the path turns back in that term.
fn along(times: &[f64], views: &[View], t: f64) -> View {
    let n = views.len().min(times.len());
    let tidied = |mut v: View| {
        v.tidy();
        v
    };
    if n == 0 {
        return View::default();
    }
    if n == 1 || t <= times[0] {
        return tidied(views[0]);
    }
    if t >= times[n - 1] {
        return tidied(views[n - 1]);
    }
    let mut terms: Vec<[f64; 6]> = views[..n]
        .iter()
        .map(|v| [v.at.latitude, v.at.longitude, v.at.altitude, v.bearing_deg, v.pitch_deg, v.distance_m.max(1e-3).ln()])
        .collect();
    for k in 1..n {
        let turn = terms[k][3] - terms[k - 1][3];
        terms[k][3] -= 360.0 * (turn / 360.0).round();
    }
    let i = (0..n - 1).find(|&k| t < times[k + 1]).unwrap_or(n - 2);
    let h = (times[i + 1] - times[i]).max(1e-6);
    let s = ((t - times[i]) / h).clamp(0.0, 1.0);
    let tangent = |k: usize, c: usize| -> f64 {
        if k == 0 || k == n - 1 {
            return 0.0;
        }
        let (h0, h1) = ((times[k] - times[k - 1]).max(1e-6), (times[k + 1] - times[k]).max(1e-6));
        let (d0, d1) = ((terms[k][c] - terms[k - 1][c]) / h0, (terms[k + 1][c] - terms[k][c]) / h1);
        if d0 * d1 <= 0.0 {
            return 0.0;
        }
        let m = (h1 * d0 + h0 * d1) / (h0 + h1);
        m.signum() * m.abs().min(3.0 * d0.abs().min(d1.abs()))
    };
    let (s2, s3) = (s * s, s * s * s);
    let (h00, h10, h01, h11) = (2.0 * s3 - 3.0 * s2 + 1.0, s3 - 2.0 * s2 + s, -2.0 * s3 + 3.0 * s2, s3 - s2);
    let term = |c: usize| h00 * terms[i][c] + h10 * h * tangent(i, c) + h01 * terms[i + 1][c] + h11 * h * tangent(i + 1, c);
    tidied(View {
        at: gearbox_globe::Geodetic::new(term(0), term(1), term(2)),
        bearing_deg: term(3),
        pitch_deg: term(4),
        distance_m: term(5).exp(),
    })
}

/// The keys of a path file: one `SECONDS look [MACHINE [link=LINK]]
/// key=value…` a line, a place's missing terms taken from the key before it.
fn path_keys(
    text: &str,
    view: &View,
    inventory: &ControllerInventory,
    index: &crate::prim_index::PrimIndex,
) -> Vec<Key> {
    let mut keys = Vec::new();
    let mut last = *view;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let Some((at, look_line)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let Ok(at) = at.parse::<f64>() else {
            warn!("gearbox-viewer: path line `{line}` has no seconds");
            continue;
        };
        let look = look_around(look_line);
        let named = look_line.split_whitespace().nth(1).filter(|word| !word.contains('='));
        let aim = match named {
            Some(id) => {
                let spec = inventory.machines.iter().find(|m| m.id == id);
                let Some(machine) = spec.and_then(MachineRef::of) else {
                    warn!("gearbox-viewer: path names no machine `{id}`");
                    continue;
                };
                // `link=NAME` follows that link of the machine, not its base.
                let link = look_line.split_whitespace().find_map(|w| w.strip_prefix("link="));
                let body = link
                    .and_then(|name| spec?.links.get(name))
                    .and_then(|link| index.find(machine.scene_root, &link.prim_path))
                    .unwrap_or_else(|| machine_body_entity(&machine, inventory, index));
                Aim::Machine { body, look, held: None }
            }
            None => {
                let mut to = last;
                to.bearing_deg = look.bearing_deg.unwrap_or(to.bearing_deg);
                to.pitch_deg = look.pitch_deg.unwrap_or(to.pitch_deg);
                to.distance_m = look.distance_m.unwrap_or(to.distance_m);
                to.at.altitude = look.above_m.unwrap_or(to.at.altitude);
                to.at.latitude = look.latitude.unwrap_or(to.at.latitude);
                to.at.longitude = look.longitude.unwrap_or(to.at.longitude);
                to.tidy();
                last = to;
                Aim::Place(to)
            }
        };
        keys.push(Key { at, aim });
    }
    keys.sort_by(|a, b| a.at.total_cmp(&b.at));
    keys
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
    mut path: ResMut<CameraPath>,
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
    path.0 = None;
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
        // `path FILE`: one smooth flight through the views FILE lists.
        ("path", _) => {
            let file = text.split_whitespace().nth(1).unwrap_or("");
            match std::fs::read_to_string(file) {
                Ok(lines) => {
                    let keys = path_keys(&lines, &view, &inventory, &index);
                    if keys.is_empty() {
                        warn!("gearbox-viewer: camera path {file} lists no views");
                    } else {
                        fly.target = None;
                        follow.set(None);
                        path.0 = Some(Path { keys, elapsed: 0.0 });
                    }
                }
                Err(error) => warn!("gearbox-viewer: cannot read camera path {file}: {error}"),
            }
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

    fn path_views() -> (Vec<f64>, Vec<View>) {
        let mut a = View::default();
        a.bearing_deg = 350.0;
        a.pitch_deg = 30.0;
        a.distance_m = 10.0;
        let mut b = a;
        b.walk(90.0, 60.0);
        b.bearing_deg = 20.0;
        b.distance_m = 40.0;
        let mut c = b;
        c.walk(90.0, 60.0);
        c.bearing_deg = 50.0;
        c.distance_m = 8.0;
        let mut d = c;
        d.walk(90.0, 10.0);
        d.bearing_deg = 70.0;
        (vec![0.0, 6.0, 12.0, 16.0], vec![a, b, c, d])
    }

    #[test]
    fn a_path_meets_each_view_on_time_and_never_stops_between_its_ends() {
        let (times, views) = path_views();
        for (t, v) in times.iter().zip(&views) {
            let mut want = *v;
            want.tidy();
            let got = along(&times, &views, *t);
            assert!((got.at.longitude - want.at.longitude).abs() < 1e-9 && (got.distance_m - want.distance_m).abs() < 1e-6, "missed the view at {t}");
        }
        let eye = |t: f64| {
            let v = along(&times, &views, t);
            DVecLike(v.at.longitude, v.bearing_deg, v.distance_m.ln())
        };
        let rate = |t: f64| {
            let (a, b) = (eye(t - 1e-4), eye(t + 1e-4));
            ((b.0 - a.0) / 2e-4, (b.1 - a.1) / 2e-4, (b.2 - a.2) / 2e-4)
        };
        let cruise = rate(3.0);
        let at_rest = |r: (f64, f64, f64)| {
            r.0.abs() < 0.01 * cruise.0.abs() && r.1.abs() < 0.01 * cruise.1.abs() && r.2.abs() < 0.01 * cruise.2.abs()
        };
        assert!(at_rest(rate(1e-4)) && at_rest(rate(16.0 - 1e-4)), "not at rest at its ends");
        for k in 1..=159 {
            let t = f64::from(k) * 0.1;
            let r = rate(t);
            assert!(r.0 > 0.0 && r.1 > 0.0, "stopped or turned back at {t:.1} s: {r:?}");
        }
        for t in [6.0, 12.0] {
            let (before, after) = (rate(t - 1e-3), rate(t + 1e-3));
            assert!((before.0 - after.0).abs() < 0.02 * before.0.abs(), "pace jumps at the view at {t}");
        }
        let farthest = (0..=1600).map(|k| along(&times, &views, f64::from(k) * 0.01).distance_m).fold(0.0, f64::max);
        assert!(farthest <= 40.0 + 1e-6, "drew back past the farthest view: {farthest}");
    }

    #[derive(Clone, Copy)]
    struct DVecLike(f64, f64, f64);

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
