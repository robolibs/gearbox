use bevy::prelude::*;

#[derive(Clone, Copy)]
pub struct SolarCalendar {
    pub hour: f32,
    pub day: u16,
    pub latitude: f32,
}

impl Default for SolarCalendar {
    fn default() -> Self {
        Self {
            hour: 15.0,
            day: 172,
            latitude: 52.0,
        }
    }
}

impl SolarCalendar {
    /// Approximate NOAA solar direction; +Z is north and +X is east.
    pub fn sun_direction(self) -> Vec3 {
        let year = std::f32::consts::TAU / 365.0
            * (self.day.clamp(1, 365) as f32 - 1.0 + (self.hour - 12.0) / 24.0);
        let declination = 0.006918 - 0.399912 * year.cos() + 0.070257 * year.sin()
            - 0.006758 * (2.0 * year).cos()
            + 0.000907 * (2.0 * year).sin()
            - 0.002697 * (3.0 * year).cos()
            + 0.00148 * (3.0 * year).sin();
        let latitude = self.latitude.clamp(-90.0, 90.0).to_radians();
        let hour_angle = ((self.hour.rem_euclid(24.0) - 12.0) * 15.0).to_radians();
        Vec3::new(
            -declination.cos() * hour_angle.sin(),
            latitude.sin() * declination.sin()
                + latitude.cos() * declination.cos() * hour_angle.cos(),
            latitude.cos() * declination.sin()
                - latitude.sin() * declination.cos() * hour_angle.cos(),
        )
        .normalize()
    }

    pub fn date_label(self) -> String {
        let mut day = self.day.clamp(1, 365);
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
}
