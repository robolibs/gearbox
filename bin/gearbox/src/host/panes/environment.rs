use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::{pane::PaneBody, pod::Pod};

use super::{PaneCtx, button_clicked, cid, pid, pod_response};
use crate::environment::EnvironmentSettings;
use crate::host::PANE_ENVIRONMENT as P;

pub fn show(body: &mut PaneBody<'_, '_>, world: &mut World, ctx: &PaneCtx) {
    let settings = world.resource::<EnvironmentSettings>();
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
    body.add_normal(
        cid(P, "weather"),
        "Weather",
        "cloud",
        vec![
            Pod::new(weather)
                .with_slider("Cloud cover", coverage, 0.0..=100.0, 0, " %", ctx.accent)
                .with_readout("range", "Clear sky → overcast")
                .with_button("Clear", ctx.accent)
                .with_button("Broken clouds", ctx.accent)
                .with_button("Overcast", ctx.accent),
        ],
    );
    let minutes = (calendar.hour * 60.0).round() as u32;
    let elevation = settings.sun_position.normalize().y.asin().to_degrees();
    body.add_normal(
        cid(P, "sun"),
        "Sun and season",
        "weather-sunny",
        vec![
            Pod::new(sun)
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
                .with_button("Use calendar", ctx.accent),
        ],
    );
    let responses = body.render();
    if let Some(resp) = pod_response(&responses, cid(P, "weather"), 0) {
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
                .resource_mut::<EnvironmentSettings>()
                .clouds
                .clouds_coverage = value;
        }
    }
    if let Some(resp) = pod_response(&responses, cid(P, "sun"), 0)
        && (resp.sliders.iter().any(|s| s.changed)
            || button_clicked(&responses, cid(P, "sun"), 0, 0))
    {
        let mut settings = world.resource_mut::<EnvironmentSettings>();
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
