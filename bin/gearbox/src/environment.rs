//! Gearbox's side of the weather: `bevy_weather` owns the sky, this hands its
//! wind to the field cover and keeps the viewer's lens.

pub use bevy_weather::{DaylightUpdate, WeatherPlugin, WeatherSettings};

use bevy::prelude::*;
use bevy_weather::clouds::CloudsCamera;

/// Vertical field of view of the viewer's lens, in degrees.
#[derive(Resource, Clone, Copy, Debug)]
pub struct ViewerLens {
    pub fov_deg: f32,
    /// The share of a frame the shutter is open for: how far a thing that
    /// moves smears. 0 is a still every frame.
    pub shutter: f32,
    /// Colour split at the corners, the glass never quite bringing every
    /// wavelength to the same place. It is a smear along the way out from the
    /// middle, so any of it at all softens the edges of the frame: off unless asked.
    pub fringing: f32,
    /// How much darker the corners are than the middle.
    pub vignette: f32,
    /// How soft the far ground goes. The blur to hand takes the near ground
    /// with it, so there is little of it.
    pub softness: f32,
}

impl Default for ViewerLens {
    fn default() -> Self {
        let fov_deg = std::env::var("GEARBOX_FOV_DEG")
            .ok()
            .and_then(|value| value.parse::<f32>().ok())
            .filter(|value| value.is_finite())
            .unwrap_or(58.0);
        Self {
            fov_deg: fov_deg.clamp(20.0, 100.0),
            shutter: 0.12,
            fringing: 0.005,
            vignette: 0.22,
            softness: 1.0,
        }
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

/// The lens itself: what a camera does to a scene that a bare projection
/// does not. There is no blur in it: the blur to hand softens the ground at
/// one's feet as much as the horizon, which no lens pointed at a field does.
pub fn sync_camera_lens(
    lens: Res<ViewerLens>,
    cameras: Query<Entity, With<CloudsCamera>>,
    chase: Query<&mara::ui::modules::bevy::ChaseCamera>,
    mut commands: Commands,
    mut fitted: Query<(
        &mut bevy::post_process::effect_stack::ChromaticAberration,
        &mut bevy::post_process::effect_stack::Vignette,
        &mut bevy::post_process::dof::DepthOfField,
    )>,
) {
    use bevy::post_process::dof::{DepthOfField, DepthOfFieldMode};
    use bevy::post_process::effect_stack::{ChromaticAberration, Vignette};

    let focus = chase.iter().next().map_or(14.0, |chase| chase.distance);
    let softness = if std::env::var_os("GEARBOX_NO_DOF").is_some() { 0.0 } else { lens.softness };

    for entity in &cameras {
        if fitted.get(entity).is_err() {
            commands.entity(entity).insert((
                ChromaticAberration { intensity: lens.fringing, max_samples: 3, ..default() },
                Vignette {
                    intensity: lens.vignette,
                    radius: 0.85,
                    smoothness: 2.2,
                    ..default()
                },
                DepthOfField {
                    mode: DepthOfFieldMode::Gaussian,
                    focal_distance: focus,
                    aperture_f_stops: 45.0,
                    max_circle_of_confusion_diameter: softness,
                    ..default()
                },
            ));
        }
    }
    for (mut fringing, mut vignette, mut dof) in &mut fitted {
        fringing.intensity = lens.fringing;
        vignette.intensity = lens.vignette;
        // The lens takes a moment to find focus, as a lens does.
        dof.focal_distance += (focus - dof.focal_distance) * 0.15;
        dof.max_circle_of_confusion_diameter = softness;
    }
}
