//! Whole-window screenshots: `gearbox instance screenshot OUT.png` drops a
//! request file next to the registry entry; the host asks mara for a frame
//! capture and writes it as PNG. Panes and ribbons are in the picture, which
//! a Bevy render-target capture cannot show.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::viewer::screenshot::take_request;

const POLL_EVERY: Duration = Duration::from_millis(250);
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Default)]
pub struct WindowCapture {
    last_poll: Option<Instant>,
    pending: Option<(PathBuf, egui::UserData, Instant)>,
}

impl WindowCapture {
    pub fn update(&mut self, egui: &egui::Context) {
        let now = Instant::now();
        if let Some((path, token, requested)) = &self.pending {
            let image = egui.input(|input| {
                input.events.iter().find_map(|event| match event {
                    egui::Event::Screenshot {
                        user_data, image, ..
                    } if user_data == token => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(image) = image {
                match write_png(path, &image) {
                    Ok(()) => tracing::info!(
                        target: "gearbox",
                        "gearbox-viewer: window screenshot -> {}",
                        path.display()
                    ),
                    Err(error) => tracing::error!(
                        target: "gearbox",
                        "gearbox-viewer: window screenshot {} failed: {error}",
                        path.display()
                    ),
                }
                self.pending = None;
            } else if now.saturating_duration_since(*requested) >= CALLBACK_TIMEOUT {
                tracing::error!(target: "gearbox", "gearbox-viewer: window screenshot timed out");
                self.pending = None;
            }
            return;
        }
        if self
            .last_poll
            .is_some_and(|last| now.saturating_duration_since(last) < POLL_EVERY)
        {
            return;
        }
        self.last_poll = Some(now);
        if let Some(path) = take_request("shot") {
            let token = egui::UserData::new(path.clone());
            egui.send_viewport_cmd(egui::ViewportCommand::Screenshot(token.clone()));
            self.pending = Some((path, token, now));
        }
    }
}

fn write_png(path: &Path, image: &egui::ColorImage) -> Result<(), Box<dyn std::error::Error>> {
    let [width, height] = image.size;
    if width == 0 || height == 0 || width.checked_mul(height) != Some(image.pixels.len()) {
        return Err("invalid screenshot dimensions".into());
    }
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, u32::try_from(width)?, u32::try_from(height)?);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let pixels: Vec<u8> = image
            .pixels
            .iter()
            .flat_map(|pixel| pixel.to_array())
            .collect();
        encoder.write_header()?.write_image_data(&pixels)?;
    }
    let mut file = std::fs::File::create(path)?;
    file.write_all(&bytes)?;
    Ok(())
}
