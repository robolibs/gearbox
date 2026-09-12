//! Screenshots on request from outside the window: `gearbox instance
//! screenshot OUT.png` drops a request file next to the registry entry, the
//! viewer captures the primary window into that path and removes the request.

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

pub struct ScreenshotPlugin;

impl Plugin for ScreenshotPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(RequestPoll(Timer::from_seconds(0.25, TimerMode::Repeating)))
            .add_systems(Update, serve_screenshot_requests);
    }
}

#[derive(Resource)]
struct RequestPoll(Timer);

/// Where a request for this instance lands: `<registry dir>/<name>.shot`.
pub fn request_path() -> PathBuf {
    let name = std::env::var("GEARBOX_NAME").unwrap_or_else(|_| "gearbox".to_string());
    gearbox_api::registry::registry_dir().join(format!("{name}.shot"))
}

fn serve_screenshot_requests(
    time: Res<Time>,
    mut poll: ResMut<RequestPoll>,
    mut commands: Commands,
) {
    if !poll.0.tick(time.delta()).just_finished() {
        return;
    }
    let request = request_path();
    let Ok(target) = std::fs::read_to_string(&request) else {
        return;
    };
    let _ = std::fs::remove_file(&request);
    let target = target.trim();
    if target.is_empty() {
        return;
    }
    info!("gearbox-viewer: screenshot requested -> {target}");
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(PathBuf::from(target)));
}
