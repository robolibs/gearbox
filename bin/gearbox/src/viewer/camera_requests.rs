//! `gearbox instance camera …` drops `fly ID`, `follow ID` or `unfollow` in
//! `<registry dir>/<name>.camera`; this does what the Agents pane rows do.

use bevy::prelude::*;

use usd_bevy::UsdPrimRef;

use crate::controller::ControllerInventory;
use crate::viewer::state::{ChaseCameraFly, FlyTarget, FollowTarget};
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
    view: Res<crate::viewer::camera::View>,
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
        _ => warn!(
            "gearbox-viewer: camera request `{}` not understood",
            text.trim()
        ),
    }
}
