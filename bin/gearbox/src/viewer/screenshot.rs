//! Viewport screenshots on request from outside the window: `gearbox
//! instance screenshot --viewport OUT.png` drops a request file next to the
//! registry entry, the Bevy side captures the chase camera's render target
//! into that path and removes the request. Whole-window captures are the
//! host's job (`crate::host`).

use std::path::PathBuf;

use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use mara::ui::modules::bevy::ChaseCamera;

pub struct ScreenshotPlugin;

impl Plugin for ScreenshotPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(RequestPoll(Timer::from_seconds(0.25, TimerMode::Repeating)))
            .add_systems(Update, serve_screenshot_requests);
    }
}

#[derive(Resource)]
struct RequestPoll(Timer);

/// Where a request for this instance lands: `<registry dir>/<name>.<ext>`.
pub fn request_path(ext: &str) -> PathBuf {
    let name = std::env::var("GEARBOX_NAME").unwrap_or_else(|_| "gearbox".to_string());
    gearbox_api::registry::registry_dir().join(format!("{name}.{ext}"))
}

/// Read and remove a request file; its content is the output path.
pub fn take_request(ext: &str) -> Option<PathBuf> {
    let request = request_path(ext);
    let out = std::fs::read_to_string(&request).ok()?;
    let _ = std::fs::remove_file(&request);
    let out = out.trim();
    (!out.is_empty()).then(|| PathBuf::from(out))
}

fn serve_screenshot_requests(
    time: Res<Time>,
    mut poll: ResMut<RequestPoll>,
    cameras: Query<&RenderTarget, With<ChaseCamera>>,
    mut commands: Commands,
) {
    if !poll.0.tick(time.delta()).just_finished() {
        return;
    }
    let Some(out) = take_request("shotvp") else {
        return;
    };
    let Ok(target) = cameras.single() else {
        warn!("gearbox-viewer: screenshot requested without a chase camera");
        return;
    };
    info!("gearbox-viewer: viewport screenshot -> {}", out.display());
    commands
        .spawn(Screenshot(target.clone()))
        .observe(save_to_disk(out));
}
