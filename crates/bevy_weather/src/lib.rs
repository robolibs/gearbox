//! Weather for a Bevy scene: the sun by calendar and hour, daylight colour,
//! atmospheric haze, volumetric clouds and the wind, independent of whatever
//! terrain lies under them.

mod calendar;
pub mod clouds;
mod daylight;
mod sky;

pub use calendar::SolarCalendar;

use bevy::prelude::*;
use clouds::config::CloudsConfig;

#[derive(Resource, Clone)]
pub struct WeatherSettings {
    pub calendar: SolarCalendar,
    pub sun_override: bool,
    pub sun_position: Vec3,
    pub sun_color: Color,
    pub illuminance: f32,
    pub indirect_intensity: f32,
    pub exposure_ev100: f32,
    pub bloom_intensity: f32,
    pub fog_color: Color,
    /// How far the air lets you see: distant land fades into the sky by it.
    pub haze_visibility_km: f32,
    pub clouds: CloudsConfig,
    /// How fast the clouds drift, downwind, in m/s.
    pub cloud_drift_mps: f32,
    /// Turns the frame the sky is drawn in into the planet's own. The sun and
    /// the weather belong to the planet: elsewhere on it they stand differently.
    pub planet_from_site: Quat,
    /// Where the wind blows towards, degrees from +X towards +Z.
    pub wind_heading_deg: f32,
    pub wind_speed_mps: f32,
    /// How far the push drops between gusts: 0 steady, 1 gusty.
    pub wind_gustiness: f32,
}

impl Default for WeatherSettings {
    fn default() -> Self {
        let calendar = SolarCalendar::default();
        let mut sun_position = Vec3::new(-4.0, 7.0, 5.0);
        let elevation = sun_position.normalize().y.asin().to_degrees();
        let azimuth = sun_position.x.atan2(sun_position.z).to_degrees();
        let angle = |name: &str, fallback: f32| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .filter(|v| v.is_finite())
                .unwrap_or(fallback)
        };
        let elevation = angle("GEARBOX_SUN_ELEVATION", elevation)
            .clamp(-90.0, 90.0)
            .to_radians();
        let azimuth = angle("GEARBOX_SUN_AZIMUTH", azimuth)
            .rem_euclid(360.0)
            .to_radians();
        sun_position = Vec3::new(
            elevation.cos() * azimuth.sin(),
            elevation.sin(),
            elevation.cos() * azimuth.cos(),
        );
        let sun_override = ["GEARBOX_SUN_ELEVATION", "GEARBOX_SUN_AZIMUTH"]
            .iter()
            .any(|name| {
                std::env::var(name)
                    .ok()
                    .and_then(|v| v.parse::<f32>().ok())
                    .is_some_and(f32::is_finite)
            });
        if !sun_override {
            sun_position = calendar.sun_direction();
        }
        Self {
            calendar,
            sun_override,
            sun_position,
            sun_color: Color::srgb(1.0, 0.97, 0.92),
            illuminance: bevy::light::light_consts::lux::RAW_SUNLIGHT,
            indirect_intensity: 3_000.0,
            exposure_ev100: 13.5,
            bloom_intensity: 0.035,
            fog_color: Color::srgb(0.55, 0.70, 0.86),
            // Six kilometres is a humid, hazy day, and it washed every acre
            // past the near field into flat sky-blue long before the horizon —
            // the ground kept its texture only as far as the next farm. Clear
            // air is the ordinary case for looking at land, and it is what the
            // distant backdrop's own colour was measured against.
            haze_visibility_km: angle("GEARBOX_HAZE_KM", 45.0).clamp(0.5, 200.0),
            clouds: CloudsConfig {
                clouds_raymarch_steps_count: 96,
                clouds_coverage: angle("GEARBOX_CLOUD_COVER", 0.55).clamp(0.0, 1.0),
                cloud_shadow_opacity: angle("GEARBOX_CLOUD_SHADOWS", 0.9).clamp(0.0, 1.0),
                clouds_base_scale: 0.7,
                clouds_detail_strength: 0.18,
                clouds_base_edge_softness: 0.14,
                clouds_bottom_height: 3800.0,
                clouds_top_height: 5300.0,
                reprojection_strength: 0.95,
                ..default()
            },
            cloud_drift_mps: 18.0,
            planet_from_site: Quat::IDENTITY,
            wind_heading_deg: 36.87,
            wind_speed_mps: 4.0,
            wind_gustiness: 0.7,
        }
    }
}

impl WeatherSettings {
    pub fn apply_calendar(&mut self) {
        self.sun_override = false;
        self.sun_position = self.calendar.sun_direction();
    }

    /// The wind as the vegetation shaders read it: downwind x and z, speed in
    /// m/s and gustiness.
    pub fn wind_vector(&self) -> Vec4 {
        let (sin, cos) = self.wind_heading_deg.to_radians().sin_cos();
        Vec4::new(cos, sin, self.wind_speed_mps.max(0.0), self.wind_gustiness.clamp(0.0, 1.0))
    }

    /// Beaufort force of the wind speed, 0 calm to 12 hurricane.
    pub fn beaufort(&self) -> usize {
        const LIMITS: [f32; 12] = [0.5, 1.6, 3.4, 5.5, 8.0, 10.8, 13.9, 17.2, 20.8, 24.5, 28.5, 32.7];
        LIMITS.iter().filter(|limit| self.wind_speed_mps >= **limit).count()
    }
}

pub struct WeatherPlugin;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct DaylightUpdate;

impl Plugin for WeatherPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WeatherSettings>()
            .init_resource::<SkyLight>()
            .add_plugins(sky::SkyPlugin);
    }
}

/// A directional light's illuminance before anything scales it; the sun
/// carries one so viewers can dim or boost it without losing the original.
#[derive(Component, Debug, Copy, Clone)]
pub struct OriginalIlluminance(pub f32);

/// How the sky's cloud cover splits daylight. The split follows the
/// cloud-cover radiation models fitted to weather-station records (Kasten and
/// Czeplak, 1980): clear air passes a fifth of its light as sky light and the
/// rest as a direct beam, and cloud trades beam for sky light until an
/// overcast has no beam at all. The total is held well above what those
/// records measure, because a viewer's exposure is fixed where an eye adapts:
/// an overcast field should read flat and soft, not dark.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct SkyLight {
    /// Direct sun averaged over the land, against a clear sky's.
    pub mean_direct: f32,
    /// Share of the light reaching the ground that is diffuse, 0.2 to 1.
    pub diffuse_fraction: f32,
    /// Sky light against a clear sky's. It only ever grows with cover, to a
    /// little over three times under a full overcast, so shadows fill in as
    /// the sun goes.
    pub diffuse_gain: f32,
}

impl SkyLight {
    /// `cover` is the fraction of sky under cloud, 0 clear to 1 overcast.
    pub fn from_cover(cover: f32) -> Self {
        let cover = cover.clamp(0.0, 1.0);
        // No steeper than this, or sky light would dip again near full cover.
        let global = 1.0 - 0.33 * cover.powi(3);
        let diffuse_fraction = 0.2 + 0.8 * cover * cover;
        Self {
            mean_direct: global * (1.0 - diffuse_fraction) / 0.8,
            diffuse_fraction,
            diffuse_gain: global * diffuse_fraction / 0.2,
        }
    }
}

impl Default for SkyLight {
    fn default() -> Self {
        Self::from_cover(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::SkyLight;

    #[test]
    fn a_clear_sky_is_the_reference() {
        let clear = SkyLight::from_cover(0.0);
        assert!((clear.mean_direct - 1.0).abs() < 1e-6 && (clear.diffuse_gain - 1.0).abs() < 1e-6);
    }

    #[test]
    fn overcast_is_all_sky_light_and_not_dark() {
        let overcast = SkyLight::from_cover(1.0);
        assert!(overcast.mean_direct.abs() < 1e-6 && (overcast.diffuse_fraction - 1.0).abs() < 1e-6);
        // Two thirds of a clear sky's total light, all of it from the sky.
        assert!((overcast.diffuse_gain - 0.67 / 0.2).abs() < 1e-5);
    }

    #[test]
    fn cloud_trades_direct_light_for_sky_light_all_the_way() {
        let mut last = SkyLight::from_cover(0.0);
        for step in 1..=20 {
            let sky = SkyLight::from_cover(step as f32 / 20.0);
            assert!(sky.mean_direct < last.mean_direct);
            assert!(sky.diffuse_gain > last.diffuse_gain && sky.diffuse_fraction > last.diffuse_fraction);
            last = sky;
        }
    }
}
