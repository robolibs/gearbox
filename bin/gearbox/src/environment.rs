//! Gearbox's side of the weather: `bevy_weather` owns the sky, this hands its
//! wind to the field cover and keeps the viewer's lens.

pub use bevy_weather::{DaylightUpdate, WeatherPlugin, WeatherSettings};

use bevy::prelude::*;
use bevy_weather::clouds::CloudsCamera;

pub fn planetary_weather() -> WeatherSettings {
    weather_for(gearbox_globe::Body::active())
}

/// The calendar a body keeps.
pub fn planet_of(body: gearbox_globe::Body) -> bevy_weather::Planet {
    match body {
        gearbox_globe::Body::Earth => bevy_weather::Planet::Earth,
        gearbox_globe::Body::Mars => bevy_weather::Planet::Mars,
        gearbox_globe::Body::Moon => bevy_weather::Planet::Moon,
    }
}

/// World time for the active body, its day as long as the body's own.
pub fn world_clock() -> gearbox_api::WorldClock {
    let calendar = planetary_weather().calendar;
    gearbox_api::WorldClock {
        day_s: calendar.planet.solar_day_s(),
        hour: calendar.hour,
        day: calendar.day as u32,
        ..default()
    }
}

/// How much world time one frame of a skip may jump: a twelfth of a day,
/// so anything that integrates over it still gets its hours one by one.
const SKIP_SLICES_PER_DAY: f64 = 12.0;

/// The systems that move world time on; whatever runs on world time runs after them.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorldTime;

/// World seconds this frame moved on by.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct WorldStep {
    pub seconds: f64,
}

/// World time runs `rate` times the physics clock, plus a slice of any skip.
pub fn advance_world_clock(
    physics: Res<crate::physics::PhysicsWorld>,
    mut clock: ResMut<gearbox_api::WorldClock>,
    mut step: ResMut<WorldStep>,
    mut last: Local<Option<f64>>,
) {
    let simulated = physics.simulated_seconds;
    let ran = last.map_or(0.0, |was| (simulated - was).max(0.0));
    *last = Some(simulated);
    let slice = clock.skip.min(clock.day_s / SKIP_SLICES_PER_DAY);
    clock.skip -= slice;
    step.seconds = ran * clock.rate + slice;
    clock.seconds += step.seconds;
}

/// Dust in the weather thickens the Martian air the sky is scattered from:
/// the medium is rescaled in place, which keeps its resolved phase function.
pub fn sync_martian_dust(
    settings: Res<WeatherSettings>,
    mut planets: Query<(&mut gearbox_planet::PlanetConfig, &gearbox_planet::planet::quadtree::PlanetRenderer)>,
    mut media: ResMut<Assets<bevy::light::atmosphere::ScatteringMedium>>,
) {
    if !settings.is_changed() {
        return;
    }
    for (mut config, renderer) in &mut planets {
        let Some(gearbox_planet::Air::Martian { dust_tau }) = config.atmosphere else { continue };
        if (dust_tau - settings.dust_tau).abs() < 1e-4 {
            continue;
        }
        if let Some((_, handle)) = &renderer.atmosphere
            && let Some(mut medium) = media.get_mut(handle)
        {
            gearbox_planet::set_dust(&mut medium, dust_tau, settings.dust_tau);
            config.atmosphere = Some(gearbox_planet::Air::Martian { dust_tau: settings.dust_tau });
        }
    }
}

/// Weather keys the API sent, set on the weather. Clouds belong to Earth's sky only.
pub fn apply_weather_requests(
    mut requests: MessageReader<gearbox_api::WeatherRequest>,
    mut settings: ResMut<WeatherSettings>,
) {
    for request in requests.read() {
        for (key, text) in request.0.iter() {
            let Some(value) = text.parse::<f32>().ok().filter(|v| v.is_finite()) else {
                warn!("weather: `{key}` = `{text}` is not a number");
                continue;
            };
            match key.as_str() {
                "dust_tau" => settings.dust_tau = value.clamp(0.0, 12.0),
                "cloud_cover" if settings.sky_kind == bevy_weather::SkyKind::Earth => {
                    settings.clouds.clouds_coverage = value.clamp(0.0, 1.0);
                }
                "cloud_cover" => warn!("weather: this sky carries no clouds"),
                "wind_speed_mps" => settings.wind_speed_mps = value.clamp(0.0, 60.0),
                "wind_heading_deg" => settings.wind_heading_deg = value.rem_euclid(360.0),
                "wind_gustiness" => settings.wind_gustiness = value.clamp(0.0, 1.0),
                "haze_km" => settings.haze_visibility_km = value.clamp(0.5, 200.0),
                other => warn!("weather: unknown key `{other}`"),
            }
        }
    }
}

/// The weather as the API reports it.
pub fn report_weather(settings: Res<WeatherSettings>, mut report: ResMut<gearbox_api::WeatherReport>) {
    if !settings.is_changed() && report.0.contains("dust_tau") {
        return;
    }
    let value = |v: f32| format!("{v:.4}");
    report.0 = gearbox_api::Props::from_pairs(&[
        ("dust_tau", &value(settings.dust_tau)),
        ("cloud_cover", &value(settings.clouds.clouds_coverage)),
        ("wind_speed_mps", &value(settings.wind_speed_mps)),
        ("wind_heading_deg", &value(settings.wind_heading_deg)),
        ("wind_gustiness", &value(settings.wind_gustiness)),
        ("haze_km", &value(settings.haze_visibility_km)),
    ]);
}

/// The calendar follows world time, and the clock reports where it stands.
pub fn advance_sun(
    step: Res<WorldStep>,
    mut settings: ResMut<WeatherSettings>,
    mut clock: ResMut<gearbox_api::WorldClock>,
) {
    if step.seconds > 0.0 && settings.bypass_change_detection().advance_clock(step.seconds) {
        settings.set_changed();
    }
    clock.hour = settings.calendar.hour;
    clock.day = settings.calendar.day as u32;
}

/// Mean sunlight above Mars's air: Earth's 130 klx at 586.2 / 1361 W/m²
/// (NASA Mars fact sheet).
pub const MARS_SUNLIGHT_LUX: f32 = 130_000.0 * 586.2 / 1361.0;

fn weather_for(body: gearbox_globe::Body) -> WeatherSettings {
    let mut settings = WeatherSettings::default();
    settings.calendar.planet = planet_of(body);
    settings.calendar.latitude = body.home().0 as f32;
    if body != gearbox_globe::Body::Earth {
        // A clear northern-spring morning: Mars at Ls 72°, the Moon a quarter-day after sunrise.
        (settings.calendar.day, settings.calendar.hour) =
            if body == gearbox_globe::Body::Mars { (150, 10.0) } else { (1, 9.0) };
    }
    if !settings.sun_override {
        settings.sun_position = settings.calendar.sun_direction();
    }
    if body == gearbox_globe::Body::Earth { return settings; }
    let moon = body == gearbox_globe::Body::Moon;
    settings.sky_kind = if moon { bevy_weather::SkyKind::Airless } else { bevy_weather::SkyKind::Mars };
    settings.clouds.planet_radius = body.radius_m() as f32;
    settings.clouds.clouds_coverage = 0.0;
    settings.clouds.cloud_shadow_opacity = 0.0;
    settings.cloud_drift_mps = 0.0;
    settings.wind_speed_mps = if moon { 0.0 } else { 3.0 };
    settings.wind_gustiness = if moon { 0.0 } else { 0.25 };
    // Sunlight above the air: Mars's air reddens and dims it on the way down.
    settings.sun_color = Color::WHITE;
    settings.illuminance = if moon { 127_000.0 } else { MARS_SUNLIGHT_LUX };
    settings.indirect_intensity = if moon { 450.0 } else { 2_200.0 };
    settings.haze_visibility_km = if moon { 1.0e9 } else { 18.0 };
    settings.fog_color = if moon { Color::BLACK } else { Color::srgb(0.74, 0.55, 0.40) };
    settings.exposure_ev100 = if moon { 14.0 } else { 13.0 };
    settings
}

pub fn planetary_fields() -> gearbox_fields::FieldsPlugin {
    fields_for(gearbox_globe::Body::active())
}

fn fields_for(body: gearbox_globe::Body) -> gearbox_fields::FieldsPlugin {
    if body == gearbox_globe::Body::Earth { return gearbox_fields::FieldsPlugin::default(); }
    gearbox_fields::FieldsPlugin {
        layout: gearbox_fields::FieldLayout {
            default: body.name().into(), default_friction: Some(0.75), fields: Vec::new(), ways: Vec::new(),
        },
        grass_density: 0.0, stubble_density: 0.0, concrete_weed_density: 0.0,
    }
}

#[cfg(test)]
mod planet_tests {
    use super::*;
    use gearbox_globe::Body;

    #[test]
    fn planet_skies_and_surfaces_are_distinct_and_barren() {
        assert_eq!(weather_for(Body::Earth).sky_kind, bevy_weather::SkyKind::Earth);
        assert_eq!(weather_for(Body::Mars).sky_kind, bevy_weather::SkyKind::Mars);
        let moon = weather_for(Body::Moon);
        assert_eq!(moon.sky_kind, bevy_weather::SkyKind::Airless);
        assert_eq!(moon.wind_speed_mps, 0.0);
        assert_eq!(moon.fog_color, Color::BLACK);
        for body in [Body::Mars, Body::Moon] {
            let weather = weather_for(body);
            assert_eq!(weather.clouds.clouds_coverage, 0.0);
            assert_eq!(weather.clouds.cloud_shadow_opacity, 0.0);
            assert_eq!(weather.clouds.planet_radius, body.radius_m() as f32);
            let fields = fields_for(body);
            assert_eq!(fields.layout.default, body.name());
            assert!(fields.layout.fields.is_empty() && fields.layout.ways.is_empty());
            assert_eq!(fields.grass_density + fields.stubble_density + fields.concrete_weed_density, 0.0);
        }
    }

    #[test]
    fn every_planet_keeps_its_own_calendar_at_home() {
        for body in [Body::Earth, Body::Mars, Body::Moon] {
            let weather = weather_for(body);
            assert_eq!(weather.calendar.planet, planet_of(body));
            assert_eq!(weather.calendar.latitude, body.home().0 as f32);
            assert!(!weather.sun_override || std::env::var_os("GEARBOX_SUN_ELEVATION").is_some());
            assert!(weather.sun_position.y > 0.3, "{body:?} starts in daylight");
        }
        assert_eq!(world_clock().day_s, planet_of(Body::active()).solar_day_s());
    }

    #[test]
    fn a_skip_jumps_a_slice_a_frame_and_rate_scales_physics_time() {
        let mut app = App::new();
        app.insert_resource(crate::physics::PhysicsWorld::default())
            .insert_resource(gearbox_api::WorldClock { rate: 60.0, skip: 3.0 * 86_400.0, ..default() })
            .init_resource::<WorldStep>()
            .add_systems(Update, advance_world_clock);
        app.update();
        app.world_mut().resource_mut::<crate::physics::PhysicsWorld>().simulated_seconds += 0.5;
        app.update();
        let clock = *app.world().resource::<gearbox_api::WorldClock>();
        let slice = 86_400.0 / SKIP_SLICES_PER_DAY;
        assert!((app.world().resource::<WorldStep>().seconds - (30.0 + slice)).abs() < 1e-9);
        assert!((clock.seconds - (30.0 + 2.0 * slice)).abs() < 1e-9);
        assert!((clock.skip - (3.0 * 86_400.0 - 2.0 * slice)).abs() < 1e-6);
    }
}

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
