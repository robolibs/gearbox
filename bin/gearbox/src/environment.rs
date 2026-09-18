//! Gearbox's side of the weather: `bevy_weather` owns the sky, this hands its
//! wind to the field cover and keeps the viewer's lens.

pub use bevy_weather::{DaylightUpdate, WeatherPlugin, WeatherSettings};

use bevy::prelude::*;
use bevy_weather::clouds::CloudsCamera;

/// Vertical field of view of the viewer's lens, in degrees.
#[derive(Resource, Clone, Copy, Debug)]
pub struct ViewerLens {
    pub fov_deg: f32,
}

impl Default for ViewerLens {
    fn default() -> Self {
        let fov_deg = std::env::var("GEARBOX_FOV_DEG")
            .ok()
            .and_then(|value| value.parse::<f32>().ok())
            .filter(|value| value.is_finite())
            .unwrap_or(58.0);
        Self { fov_deg: fov_deg.clamp(20.0, 100.0) }
    }
}

/// The weather's wind, handed to the field cover.
pub fn sync_cover_wind(
    settings: Res<WeatherSettings>,
    mut wind: ResMut<gearbox_fields::CoverWind>,
) {
    if !settings.is_changed() {
        return;
    }
    wind.heading_deg = settings.wind_heading_deg;
    wind.speed_mps = settings.wind_speed_mps;
    wind.gustiness = settings.wind_gustiness;
}

/// The viewer's cameras follow the lens.
pub fn sync_lens(lens: Res<ViewerLens>, mut cameras: Query<&mut Projection, With<CloudsCamera>>) {
    let fov = lens.fov_deg.to_radians();
    for mut projection in &mut cameras {
        let stale = matches!(&*projection, Projection::Perspective(p) if (p.fov - fov).abs() > 1e-4);
        if stale && let Projection::Perspective(p) = projection.as_mut() {
            p.fov = fov;
        }
    }
}
