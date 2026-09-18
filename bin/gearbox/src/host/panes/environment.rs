use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::container::{Tab, TabContainer};
use mara_core::pod::{Pod, PodResponse};
use mara_core::vocab::Id as MaraId;
use std::collections::HashMap;

use super::{PaneCtx, cid, pid};
use crate::environment::WeatherSettings;
use crate::host::PANE_ENVIRONMENT as P;

const BEAUFORT_NAMES: [&str; 13] = [
    "calm", "light air", "light breeze", "gentle breeze", "moderate breeze", "fresh breeze",
    "strong breeze", "near gale", "gale", "strong gale", "storm", "violent storm", "hurricane",
];

pub fn tab(world: &World, ctx: &PaneCtx) -> Tab {
    let settings = world.resource::<WeatherSettings>();
    let calendar = settings.calendar;
    let weather = pid(P, "weather", 0);
    let sun = pid(P, "sun", 0);
    let coverage = settings.clouds.clouds_coverage as f64 * 100.0;
    ctx.set_slider(weather, 0, coverage);
    for (i, value) in [
        calendar.hour as f64,
        calendar.day as f64,
        calendar.latitude as f64,
    ]
    .into_iter()
    .enumerate()
    {
        ctx.set_slider(sun, i, value);
    }
    let weather_pod = Pod::new(weather)
        .with_slider("Cloud cover", coverage, 0.0..=100.0, 0, " %", ctx.accent)
        .with_readout("range", "Clear sky → overcast")
        .with_button("Clear", ctx.accent)
        .with_button("Broken clouds", ctx.accent)
        .with_button("Overcast", ctx.accent);
    let minutes = (calendar.hour * 60.0).round() as u32;
    let elevation = settings.sun_position.normalize().y.asin().to_degrees();
    let sun_pod = Pod::new(sun)
        .with_readout(
            "sun control",
            if settings.sun_override {
                "Startup override"
            } else {
                "Solar calendar"
            },
        )
        .with_readout(
            "solar time",
            format!("{:02}:{:02}", minutes / 60, minutes % 60),
        )
        .with_slider(
            "Time of day",
            calendar.hour as f64,
            0.0..=24.0,
            2,
            " h",
            ctx.accent,
        )
        .with_readout("date", calendar.date_label())
        .with_slider(
            "Day of year",
            calendar.day as f64,
            1.0..=365.0,
            0,
            "",
            ctx.accent,
        )
        .with_slider(
            "Latitude",
            calendar.latitude as f64,
            -90.0..=90.0,
            1,
            "°",
            ctx.accent,
        )
        .with_readout("sun elevation", format!("{elevation:.1}°"))
        .with_readout("time basis", "Solar time: noon = highest sun")
        .with_readout("season", "Changes sunlight, not field crops")
        .with_button("Use calendar", ctx.accent);
    let wind = pid(P, "wind", 0);
    let (speed, heading, gust, drift) = (
        settings.wind_speed_mps as f64,
        settings.wind_heading_deg as f64,
        settings.wind_gustiness as f64 * 100.0,
        settings.cloud_drift_mps as f64,
    );
    let force = settings.beaufort();
    for (i, value) in [speed, heading, gust, drift].into_iter().enumerate() {
        ctx.set_slider(wind, i, value);
    }
    let wind_pod = Pod::new(wind)
        .with_readout("beaufort", format!("{force} · {}", BEAUFORT_NAMES[force]))
        .with_slider("Speed", speed, 0.0..=20.0, 1, " m/s", ctx.accent)
        .with_slider("Towards", heading, 0.0..=360.0, 0, "°", ctx.accent)
        .with_slider("Gustiness", gust, 0.0..=100.0, 0, " %", ctx.accent)
        .with_slider("Cloud drift", drift, 0.0..=60.0, 0, " m/s", ctx.accent)
        .with_button("Calm", ctx.accent)
        .with_button("Breeze", ctx.accent)
        .with_button("Gale", ctx.accent);

    let air = pid(P, "air", 0);
    let fov = world.resource::<crate::environment::ViewerLens>().fov_deg as f64;
    let visibility = settings.haze_visibility_km as f64;
    for (i, value) in [visibility, fov].into_iter().enumerate() {
        ctx.set_slider(air, i, value);
    }
    let air_pod = Pod::new(air)
        .with_readout("depth", "haze and lens")
        .with_slider("Visibility", visibility, 1.0..=40.0, 1, " km", ctx.accent)
        .with_slider("Field of view", fov, 35.0..=85.0, 0, "°", ctx.accent)
        .with_button("Crisp", ctx.accent)
        .with_button("Clear", ctx.accent)
        .with_button("Hazy", ctx.accent);

    // Same order as the flat list was, so `apply`'s pod indices hold.
    Tab::new(cid(P, "env"), "Environment", "weather-sunny").containers(vec![
        TabContainer::new(cid(P, "weather"), "Weather", "weather-cloudy", vec![weather_pod]),
        TabContainer::new(cid(P, "sun"), "Sun", "weather-sunny", vec![sun_pod]),
        TabContainer::new(cid(P, "wind"), "Wind", "weather-squalls", vec![wind_pod]),
        TabContainer::new(cid(P, "air"), "Air & lens", "weather-fog", vec![air_pod]),
    ])
}

// A collapsed section renders no pods, so positions in the tab's response
// list shift with what is folded; a section is told apart by how many
// sliders and buttons it has instead.
fn section(
    responses: &HashMap<MaraId, Vec<PodResponse>>,
    sliders: usize,
    buttons: usize,
) -> Option<&PodResponse> {
    responses
        .get(&cid(P, "env"))?
        .iter()
        .find(|resp| resp.sliders.len() == sliders && resp.buttons.len() == buttons)
}

pub fn apply(responses: &HashMap<MaraId, Vec<PodResponse>>, world: &mut World, _ctx: &PaneCtx) {
    if let Some(resp) = section(responses, 2, 3) {
        let fov = resp.sliders.get(1).filter(|s| s.changed).map(|s| s.value as f32);
        if let Some(fov) = fov {
            world.resource_mut::<crate::environment::ViewerLens>().fov_deg = fov.clamp(20.0, 100.0);
        }
        let preset = [25.0, 6.0, 3.0]
            .into_iter()
            .enumerate()
            .find(|(i, _)| resp.buttons.get(*i).is_some_and(|b| b.clicked))
            .map(|(_, km)| km);
        let mut settings = world.resource_mut::<WeatherSettings>();
        if let Some(km) = preset {
            settings.haze_visibility_km = km;
        }
        for (index, slider) in resp.sliders.iter().enumerate().filter(|(_, s)| s.changed) {
            match index {
                0 => settings.haze_visibility_km = (slider.value as f32).clamp(0.5, 80.0),
                _ => {}
            }
        }
    }
    if let Some(resp) = section(responses, 4, 3) {
        let preset = [(1.0, 0.3), (5.0, 0.7), (15.0, 0.9)]
            .into_iter()
            .enumerate()
            .find(|(i, _)| resp.buttons.get(*i).is_some_and(|b| b.clicked))
            .map(|(_, preset)| preset);
        let moved: Vec<(usize, f64)> = resp
            .sliders
            .iter()
            .enumerate()
            .filter(|(_, s)| s.changed)
            .map(|(i, s)| (i, s.value))
            .collect();
        if preset.is_some() || !moved.is_empty() {
            let mut settings = world.resource_mut::<WeatherSettings>();
            if let Some((speed, gust)) = preset {
                settings.wind_speed_mps = speed;
                settings.wind_gustiness = gust;
            }
            for (index, value) in moved {
                match index {
                    0 => settings.wind_speed_mps = (value as f32).clamp(0.0, 30.0),
                    1 => settings.wind_heading_deg = (value as f32).rem_euclid(360.0),
                    2 => settings.wind_gustiness = (value as f32 / 100.0).clamp(0.0, 1.0),
                    3 => settings.cloud_drift_mps = (value as f32).clamp(0.0, 100.0),
                    _ => {}
                }
            }
        }
    }
    if let Some(resp) = section(responses, 1, 3) {
        let preset = [0.0, 0.55, 1.0]
            .into_iter()
            .enumerate()
            .find(|(i, _)| resp.buttons.get(*i).is_some_and(|b| b.clicked))
            .map(|(_, value)| value);
        let coverage = preset.or_else(|| {
            resp.sliders
                .first()
                .filter(|s| s.changed)
                .map(|s| (s.value as f32 / 100.0).clamp(0.0, 1.0))
        });
        if let Some(value) = coverage {
            world
                .resource_mut::<WeatherSettings>()
                .clouds
                .clouds_coverage = value;
        }
    }
    if let Some(resp) = section(responses, 3, 1)
        && (resp.sliders.iter().any(|s| s.changed)
            || resp.buttons.first().is_some_and(|b| b.clicked))
    {
        let mut settings = world.resource_mut::<WeatherSettings>();
        for (index, slider) in resp.sliders.iter().enumerate().filter(|(_, s)| s.changed) {
            match index {
                0 => settings.calendar.hour = (slider.value as f32).clamp(0.0, 24.0),
                1 => settings.calendar.day = slider.value.round().clamp(1.0, 365.0) as u16,
                2 => settings.calendar.latitude = (slider.value as f32).clamp(-90.0, 90.0),
                _ => {}
            }
        }
        settings.apply_calendar();
    }
}
