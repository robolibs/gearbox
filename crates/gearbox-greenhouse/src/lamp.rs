//! Grow lamps: LED panels over trays, on a daily schedule, in red, blue,
//! far-red and white channels like EDEN ISS's Heliospectra lamps (630, 450,
//! 735 nm and 5700 K; Zabel et al. 2017).

/// Shares of a lamp's photon flux per channel; they add up to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Channels {
    pub red: f64,
    pub blue: f64,
    pub far_red: f64,
    pub white: f64,
}

impl Channels {
    /// Growth light: mostly red and blue, a little far-red and white, the
    /// white enough to make out leaf colour (Signify: about 5 % green).
    pub const GROWTH: Self = Self { red: 0.70, blue: 0.20, far_red: 0.04, white: 0.06 };
    /// White only, to look at the crop by.
    pub const INSPECTION: Self = Self { red: 0.0, blue: 0.0, far_red: 0.0, white: 1.0 };

    fn shares(&self) -> [f64; 4] {
        let sum = (self.red + self.blue + self.far_red + self.white).max(1e-9);
        [self.red / sum, self.blue / sum, self.far_red / sum, self.white / sum]
    }

    /// CIE 1931 XYZ of one µmol/m²/s of the mix. The narrow channels are
    /// their colour-matching values at 630, 450 and 735 nm times the photon
    /// energy (0.1196/λ[µm] J/µmol); the white is a 5700 K chromaticity
    /// (x 0.328, y 0.338) giving 72 lux per µmol/m²/s, as white LEDs do.
    fn xyz(&self) -> [f64; 3] {
        const CHANNELS: [[f64; 3]; 4] = [
            [0.6424 * 0.1899, 0.2650 * 0.1899, 0.0],
            [0.3362 * 0.2658, 0.0380 * 0.2658, 1.7721 * 0.2658],
            [0.0011 * 0.1628, 0.0004 * 0.1628, 0.0],
            [0.970 * 72.0 / 683.0, 72.0 / 683.0, 0.988 * 72.0 / 683.0],
        ];
        let mut xyz = [0.0; 3];
        for (share, channel) in self.shares().iter().zip(CHANNELS) {
            for (out, c) in xyz.iter_mut().zip(channel) {
                *out += share * c;
            }
        }
        xyz
    }

    /// Illuminance per µmol/m²/s, lux: 683 lm/W times the luminance Y.
    pub fn lux_per_ppfd(&self) -> f64 {
        683.0 * self.xyz()[1]
    }

    /// The light's colour in linear sRGB, out-of-gamut parts clipped, brightest channel 1.
    pub fn colour(&self) -> [f32; 3] {
        let [x, y, z] = self.xyz();
        let rgb = [
            (3.2406 * x - 1.5372 * y - 0.4986 * z).max(0.0),
            (-0.9689 * x + 1.8758 * y + 0.0415 * z).max(0.0),
            (0.0557 * x - 0.2040 * y + 1.0570 * z).max(0.0),
        ];
        let top = rgb.iter().cloned().fold(1e-12, f64::max);
        rgb.map(|c| (c / top) as f32)
    }
}

/// One LED panel over one tray.
#[derive(Clone, Debug, PartialEq)]
pub struct Lamp {
    pub name: String,
    /// Photon flux at the canopy at full output, µmol/m²/s.
    pub ppfd_max: f64,
    /// Area it lights, m².
    pub area_m2: f64,
    pub channels: Channels,
    /// Output commanded, 0 to 1.
    pub dimming: f64,
    /// Local hour it switches on, and how long it stays on.
    pub on_hour: f64,
    pub lit_hours: f64,
    /// Photons per joule of electricity, µmol/J.
    pub efficacy: f64,
    pub failed: bool,
}

impl Lamp {
    pub fn new(name: &str, ppfd_max: f64, area_m2: f64, lit_hours: f64) -> Self {
        Self {
            name: name.to_string(),
            ppfd_max,
            area_m2,
            channels: Channels::GROWTH,
            dimming: 1.0,
            on_hour: 5.0,
            lit_hours,
            efficacy: 2.5,
            failed: false,
        }
    }

    /// Whether the schedule has it on at a local hour of a `cycle_hours` day.
    pub fn scheduled(&self, hour: f64, cycle_hours: f64) -> bool {
        let since = (hour - self.on_hour).rem_euclid(cycle_hours.max(1.0));
        since < self.lit_hours
    }

    /// Photon flux it gives now with `power` of what it asks for, µmol/m²/s.
    pub fn ppfd(&self, hour: f64, cycle_hours: f64, power: f64) -> f64 {
        if self.failed || !self.scheduled(hour, cycle_hours) {
            return 0.0;
        }
        self.ppfd_max * self.dimming.clamp(0.0, 1.0) * power.clamp(0.0, 1.0)
    }

    /// Flux it gives while lit, whatever the hour.
    pub fn lit_ppfd(&self) -> f64 {
        if self.failed { 0.0 } else { self.ppfd_max * self.dimming.clamp(0.0, 1.0) }
    }

    /// Electric power it draws to give `ppfd`, W.
    pub fn power_w(&self, ppfd: f64) -> f64 {
        ppfd * self.area_m2 / self.efficacy.max(0.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn red_and_blue_light_looks_magenta_and_white_light_white() {
        let growth = Channels::GROWTH.colour();
        assert!(growth[0] > 0.9 && growth[2] > 0.25 && growth[1] < 0.2, "{growth:?}");
        let white = Channels::INSPECTION.colour();
        assert!(white.iter().all(|c| *c > 0.75), "a 5700 K white is a little warm: {white:?}");
        // Growth light holds less light for the eye per photon than white light does.
        assert!(Channels::GROWTH.lux_per_ppfd() < 0.5 * Channels::INSPECTION.lux_per_ppfd());
    }

    #[test]
    fn the_schedule_wraps_past_midnight() {
        let mut lamp = Lamp::new("l", 400.0, 0.24, 17.0);
        lamp.on_hour = 20.0;
        assert!(lamp.scheduled(23.0, 24.0) && lamp.scheduled(12.0, 24.0));
        assert!(!lamp.scheduled(14.0, 24.0));
        assert_eq!(lamp.ppfd(14.0, 24.0, 1.0), 0.0);
        assert_eq!(lamp.ppfd(2.0, 24.0, 0.5), 200.0);
        lamp.failed = true;
        assert_eq!(lamp.ppfd(2.0, 24.0, 1.0), 0.0);
    }

    #[test]
    fn a_panel_draws_what_eden_iss_did_per_square_metre() {
        // 400 µmol/m²/s at 2.5 µmol/J over a square metre: 160 W.
        let lamp = Lamp::new("l", 400.0, 1.0, 17.0);
        assert!((lamp.power_w(400.0) - 160.0).abs() < 1e-9);
    }
}
