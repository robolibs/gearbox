//! `gearbox instance camera …` drops `fly ID`, `follow ID`, `unfollow`,
//! `goto …` or `look …` in `<registry dir>/<name>.camera`; this does what the
//! Agents pane rows do, and sets exact views for scripted captures.

use bevy::prelude::*;

use usd_bevy::UsdPrimRef;

use crate::controller::ControllerInventory;
use crate::viewer::state::{ChaseCameraFly, FlyTarget, FollowTarget, LookAround};
use crate::viewer::systems::machine_body_entity;

pub struct CameraRequestsPlugin;

impl Plugin for CameraRequestsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(RequestPoll(Timer::from_seconds(0.25, TimerMode::Repeating)))
            .add_systems(Update, serve_camera_requests);
    }
}

#[derive(Resource)]
struct RequestPoll(Timer);

fn serve_camera_requests(
    time: Res<Time>,
    mut poll: ResMut<RequestPoll>,
    inventory: Res<ControllerInventory>,
    mut follow: ResMut<FollowTarget>,
    mut fly: ResMut<ChaseCameraFly>,
    prims: Query<(Entity, &UsdPrimRef)>,
    parents: Query<&ChildOf>,
    mut view: ResMut<crate::viewer::camera::View>,
    mut goto: ResMut<crate::globe::Goto>,
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
    let mut words = text.split_whitespace();
    let action = words.next().unwrap_or("");
    let root = words.next().and_then(|id| {
        inventory
            .machines
            .iter()
            .find(|m| m.id == id)
            .and_then(|m| m.scene_root)
    });
    info!("gearbox-viewer: camera request `{}`", text.trim());
    match (action, root) {
        ("fly", Some(root)) => {
            let body = machine_body_entity(root, &inventory, &prims, &parents);
            fly.target = Some(FlyTarget::new(root, body, &view));
        }
        ("follow", Some(root)) => follow.set(Some(root)),
        ("unfollow", _) => follow.set(None),
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
        // `look [ID] bearing=DEG pitch=DEG distance=M`: an exact view, around
        // the machine when one is named, its bearing counted from behind it.
        ("look", Some(root)) => {
            let body = machine_body_entity(root, &inventory, &prims, &parents);
            fly.target = Some(FlyTarget::looking(root, body, &view, look_around(&text)));
        }
        ("look", None) if text.split_whitespace().nth(1).is_none_or(|word| word.contains('=')) => {
            let look = look_around(&text);
            fly.target = None;
            view.bearing_deg = look.bearing_deg.unwrap_or(view.bearing_deg);
            view.pitch_deg = look.pitch_deg.unwrap_or(view.pitch_deg);
            view.distance_m = look.distance_m.unwrap_or(view.distance_m);
            view.tidy();
        }
        _ => warn!(
            "gearbox-viewer: camera request `{}` not understood",
            text.trim()
        ),
    }
}

/// The `bearing=`, `pitch=` and `distance=` values of a `look` request.
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
            _ => {}
        }
    }
    look
}
