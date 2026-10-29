//! The sim without a window: `gearbox run --headless 1920x1080` (or
//! `GEARBOX_HEADLESS=WxH` on the launch) renders the chase camera into an
//! offscreen image on a GPU device of its own, so it needs no display,
//! compositor or login session. Everything that talks to the Bevy world
//! works as it does with a window — the bus, camera requests, viewport
//! screenshots and the recorder — while the host's panes do not exist.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bevy::app::AppExit;
use bevy::ecs::message::Messages;
use mara::ui::modules::bevy::{BevyViewportRenderer, BevyViewportTexture};

use crate::viewer::recorder::Recorder;

/// Frames a second the headless sim renders while nothing is recorded; a
/// recording renders as fast as the GPU goes, the clock stepped per frame.
const IDLE_FPS: f64 = 30.0;

/// The headless viewport's size from `GEARBOX_HEADLESS=WxH`, if set.
pub fn size() -> Option<Result<(u32, u32), String>> {
    let value = std::env::var("GEARBOX_HEADLESS").ok()?;
    Some(parse_size(&value).ok_or_else(|| format!("GEARBOX_HEADLESS={value:?} is not WIDTHxHEIGHT")))
}

fn parse_size(value: &str) -> Option<(u32, u32)> {
    let (w, h) = value.trim().split_once('x')?;
    let (w, h) = (w.parse().ok()?, h.parse().ok()?);
    (w > 0 && h > 0).then_some((w, h))
}

pub fn run(cli_paths: Vec<PathBuf>, (width, height): (u32, u32)) -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!(target: "gearbox", "gearbox-sim headless: rendering {width}x{height} offscreen");
    let mut renderer = BevyViewportRenderer::new(
        BevyViewportTexture::new(width, height),
        None,
        Some(Arc::new(crate::app::configure_plugins)),
        Some(Arc::new(move |app: &mut bevy::prelude::App| crate::app::configure(app, cli_paths.clone(), true))),
    );
    loop {
        let started = Instant::now();
        renderer.render_next_texture();
        let world = renderer.world_mut();
        if world.get_resource::<Messages<AppExit>>().is_some_and(|exit| !exit.is_empty()) {
            tracing::info!(target: "gearbox", "gearbox-sim headless: shutting down");
            return Ok(());
        }
        if !world.get_resource::<Recorder>().is_some_and(Recorder::is_recording) {
            let frame = Duration::from_secs_f64(1.0 / IDLE_FPS);
            std::thread::sleep(frame.saturating_sub(started.elapsed()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_size;

    #[test]
    fn a_headless_size_is_width_by_height() {
        assert_eq!(parse_size("1920x1080"), Some((1920, 1080)));
        assert_eq!(parse_size(" 640x360 "), Some((640, 360)));
        assert_eq!(parse_size("1920"), None);
        assert_eq!(parse_size("0x1080"), None);
        assert_eq!(parse_size("wide x tall"), None);
    }
}
