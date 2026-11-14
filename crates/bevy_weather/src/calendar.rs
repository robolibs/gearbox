use bevy::prelude::*;

/// The world a calendar keeps time on: how long its solar day and year are
/// and how its sun swings north and south over that year.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Planet {
    #[default]
    Earth,
    Mars,
    Moon,
}

/// Mars year in sols (686.9711 Earth days of 1.0274912517 each).
const MARS_SOLS_PER_YEAR: f64 = 668.5991;
/// Mars mean anomaly at Ls = 0, the start of the Mars year (Mars24 B-1 to B-5).
const MARS_ANOMALY_AT_YEAR_START: f64 = 98.70;
/// Mars semi-major axis, AU.
const MARS_MEAN_DISTANCE_AU: f64 = 1.523_679_34;

impl Planet {
    /// Mean solar day, in SI seconds.
    pub const fn solar_day_s(self) -> f64 {
        match self {
            Self::Earth => 86_400.0,
            Self::Mars => 88_775.244,
            Self::Moon => 2_551_442.9,
        }
    }

    /// Solar days in its year, counting a part-day as a whole one.
    pub const fn year_days(self) -> u16 {
        match self {
            Self::Earth => 365,
            Self::Mars => 669,
            Self::Moon => 13,
        }
    }

    const fn year_length(self) -> f64 {
        match self {
            Self::Earth => 365.0,
            Self::Mars => MARS_SOLS_PER_YEAR,
            Self::Moon => 12.368_7,
        }
    }

    /// What its solar day is called.
    pub const fn day_name(self) -> &'static str {
        match self {
            Self::Earth => "day",
            Self::Mars => "sol",
            Self::Moon => "lunar day",
        }
    }
}

/// Local true solar time and the day of the year, on one planet at one
/// latitude: noon is when the sun stands highest.
#[derive(Clone, Copy, Debug)]
pub struct SolarCalendar {
    pub planet: Planet,
    /// Hour of the local solar day, 0 to 24; a planet's hour is its solar day over 24.
    pub hour: f64,
    /// Day (sol) of the year, from 1. A Mars year starts at Ls 0, the northern spring equinox.
    pub day: u16,
    pub latitude: f32,
}

impl Default for SolarCalendar {
    fn default() -> Self {
        Self {
            planet: Planet::Earth,
            hour: 15.0,
            day: 172,
            latitude: 52.0,
        }
    }
}

impl SolarCalendar {
    /// Moves the calendar on by `seconds` of world time.
    pub fn advance(&mut self, seconds: f64) {
        if !seconds.is_finite() || seconds <= 0.0 {
            return;
        }
        let hours = self.hour + seconds / self.planet.solar_day_s() * 24.0;
        let days = (hours / 24.0).floor();
        self.hour = hours - days * 24.0;
        let year = self.planet.year_days() as f64;
        let day = (self.day.max(1) as f64 - 1.0 + days).rem_euclid(year);
        self.day = day as u16 + 1;
    }

    /// Share of the year gone, 0 to 1.
    fn year_fraction(self) -> f64 {
        let day = self.day.clamp(1, self.planet.year_days()) as f64;
        ((day - 1.0 + self.hour / 24.0) / self.planet.year_length()).rem_euclid(1.0)
    }

    /// Mars mean anomaly and equation of centre, degrees (Mars24 B-1, B-4).
    fn mars_anomaly(self) -> (f64, f64) {
        let mean = MARS_ANOMALY_AT_YEAR_START + 360.0 * self.year_fraction();
        let m = mean.to_radians();
        let centre = 10.691 * m.sin()
            + 0.623 * (2.0 * m).sin()
            + 0.050 * (3.0 * m).sin()
            + 0.005 * (4.0 * m).sin()
            + 0.0005 * (5.0 * m).sin();
        (mean, centre)
    }

    /// Areocentric solar longitude Ls on Mars, degrees: 0 at the northern
    /// spring equinox, 90 at the northern summer solstice.
    pub fn solar_longitude(self) -> f64 {
        let (mean, centre) = self.mars_anomaly();
        (mean + 251.0 + centre).rem_euclid(360.0)
    }

    /// Declination of the sun, radians.
    fn declination(self) -> f64 {
        match self.planet {
            Planet::Earth => {
                let year = std::f64::consts::TAU / 365.0
                    * (self.day.clamp(1, 365) as f64 - 1.0 + (self.hour - 12.0) / 24.0);
                0.006918 - 0.399912 * year.cos() + 0.070257 * year.sin()
                    - 0.006758 * (2.0 * year).cos()
                    + 0.000907 * (2.0 * year).sin()
                    - 0.002697 * (3.0 * year).cos()
                    + 0.00148 * (3.0 * year).sin()
            }
            Planet::Mars => {
                let ls = self.solar_longitude().to_radians();
                (0.42565 * ls.sin()).asin() + 0.25_f64.to_radians() * ls.sin()
            }
            Planet::Moon => 1.54_f64.to_radians() * (std::f64::consts::TAU * self.year_fraction()).sin(),
        }
    }

    /// Sunlight against its mean over the year, from the planet's distance to the sun.
    pub fn irradiance_scale(self) -> f32 {
        let scale = match self.planet {
            Planet::Mars => {
                let m = self.mars_anomaly().0.to_radians();
                let distance = MARS_MEAN_DISTANCE_AU
                    * (1.00436 - 0.09309 * m.cos() - 0.004336 * (2.0 * m).cos()
                        - 0.00031 * (3.0 * m).cos()
                        - 0.00003 * (4.0 * m).cos());
                (MARS_MEAN_DISTANCE_AU / distance).powi(2)
            }
            Planet::Earth | Planet::Moon => {
                1.0 + 0.033 * (std::f64::consts::TAU * (self.day as f64 - 3.0) / 365.0).cos()
            }
        };
        scale as f32
    }

    /// Unit vector towards the sun in a datum's frame: +X north, +Y up, +Z east.
    pub fn sun_direction(self) -> Vec3 {
        let declination = self.declination();
        let latitude = (self.latitude.clamp(-90.0, 90.0) as f64).to_radians();
        let hour_angle = ((self.hour.rem_euclid(24.0) - 12.0) * 15.0).to_radians();
        let east = -declination.cos() * hour_angle.sin();
        let up = latitude.sin() * declination.sin() + latitude.cos() * declination.cos() * hour_angle.cos();
        let north = latitude.cos() * declination.sin() - latitude.sin() * declination.cos() * hour_angle.cos();
        Vec3::new(north as f32, up as f32, east as f32).normalize()
    }

    pub fn date_label(self) -> String {
        match self.planet {
            Planet::Earth => earth_date(self.day),
            Planet::Mars => format!("sol {} · Ls {:.0}°", self.day, self.solar_longitude()),
            Planet::Moon => format!("lunar day {}", self.day),
        }
    }
}

fn earth_date(day: u16) -> String {
    let mut day = day.clamp(1, 365);
    for (month, days) in [
        ("Jan", 31),
        ("Feb", 28),
        ("Mar", 31),
        ("Apr", 30),
        ("May", 31),
        ("Jun", 30),
        ("Jul", 31),
        ("Aug", 31),
        ("Sep", 30),
        ("Oct", 31),
        ("Nov", 30),
        ("Dec", 31),
    ] {
        if day <= days {
            return format!("{day} {month}");
        }
        day -= days;
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mars(day: u16, hour: f64, latitude: f32) -> SolarCalendar {
        SolarCalendar { planet: Planet::Mars, hour, day, latitude }
    }

    #[test]
    fn a_mars_year_starts_at_the_northern_spring_equinox() {
        assert!(mars(1, 0.0, 0.0).solar_longitude().min(360.0 - mars(1, 0.0, 0.0).solar_longitude()) < 0.5);
        let mut previous = mars(1, 0.0, 0.0).solar_longitude();
        let mut turned = 0.0;
        for day in 2..=669 {
            let ls = mars(day, 0.0, 0.0).solar_longitude();
            turned += (ls - previous).rem_euclid(360.0);
            previous = ls;
        }
        assert!((turned - 360.0).abs() < 1.0, "Ls turned {turned}° over a year");
    }

    #[test]
    fn mars_seasons_are_uneven_and_tilted_by_25_degrees() {
        // Northern spring and summer are long (aphelion), autumn and winter short.
        let sol_at = |target: f64| (1..=669).find(|&d| mars(d, 0.0, 0.0).solar_longitude() >= target && mars(d, 0.0, 0.0).solar_longitude() < target + 5.0).unwrap();
        let (summer, autumn, winter) = (sol_at(90.0), sol_at(180.0), sol_at(270.0));
        assert!((190..=196).contains(&summer), "Ls 90 on sol {summer}");
        assert!((368..=375).contains(&autumn), "Ls 180 on sol {autumn}");
        assert!((511..=518).contains(&winter), "Ls 270 on sol {winter}");
        let noon = mars(summer, 12.0, 0.0).sun_direction();
        let tilt = noon.x.atan2(noon.y).to_degrees();
        assert!((tilt - 25.4).abs() < 0.5, "noon sun stands {tilt}° north at the solstice");
    }

    #[test]
    fn mars_sunlight_peaks_near_perihelion() {
        let perihelion = (1..=669).max_by(|&a, &b| mars(a, 0.0, 0.0).irradiance_scale().total_cmp(&mars(b, 0.0, 0.0).irradiance_scale())).unwrap();
        let ls = mars(perihelion, 0.0, 0.0).solar_longitude();
        assert!((ls - 251.0).abs() < 3.0, "brightest at Ls {ls}");
        let (high, low) = (mars(perihelion, 0.0, 0.0).irradiance_scale(), (1..=669).map(|d| mars(d, 0.0, 0.0).irradiance_scale()).fold(f32::MAX, f32::min));
        assert!((high / low - 1.45).abs() < 0.03, "perihelion over aphelion {}", high / low);
    }

    #[test]
    fn advancing_a_sol_keeps_the_hour_and_turns_the_day() {
        let mut calendar = mars(669, 9.5, 18.0);
        calendar.advance(Planet::Mars.solar_day_s());
        assert_eq!(calendar.day, 1);
        assert!((calendar.hour - 9.5).abs() < 1e-9);
        calendar.advance(Planet::Mars.solar_day_s() / 4.0);
        assert!((calendar.hour - 15.5).abs() < 1e-9);
        let mut earth = SolarCalendar::default();
        earth.advance(9.0 * 3600.0);
        assert_eq!((earth.day, earth.hour), (173, 0.0));
    }

    #[test]
    fn the_sun_rises_in_the_east_and_crosses_the_south() {
        // Northern latitudes: a datum's frame is x north, y up, z east.
        for planet in [Planet::Earth, Planet::Mars] {
            let calendar = |hour| SolarCalendar { planet, hour, day: 80, latitude: 45.0 };
            assert!(calendar(7.0).sun_direction().z > 0.5, "{planet:?} morning sun stands east");
            assert!(calendar(17.0).sun_direction().z < -0.5, "{planet:?} evening sun stands west");
            assert!(calendar(12.0).sun_direction().x < -0.3, "{planet:?} noon sun stands south");
        }
    }

    #[test]
    fn the_sun_sets_at_night_on_every_planet() {
        for planet in [Planet::Earth, Planet::Mars, Planet::Moon] {
            let calendar = |hour| SolarCalendar { planet, hour, day: 100, latitude: 20.0 };
            assert!(calendar(12.0).sun_direction().y > 0.5);
            assert!(calendar(0.0).sun_direction().y < -0.5);
        }
    }
}
