//! The power bus: a fission reactor, solar arrays and a battery against
//! everything that draws. When supply falls short, lamps are dimmed before
//! life support goes without.

/// Sunlight reaching level ground through dust, W/m²: the direct beam
/// thinned by Beer–Lambert, `exp(-tau / mu)`, plus the share of what the
/// dust scatters that still reaches the ground. Dust scatters forward with a
/// single-scattering albedo near 0.9, so about half arrives as sky light —
/// close to the normalised net flux of Appelbaum and Flood (NASA TM-102299).
pub fn surface_irradiance(top_w_m2: f64, sun_elevation_rad: f64, dust_tau: f64) -> f64 {
    let mu = sun_elevation_rad.sin();
    if mu <= 0.0 {
        return 0.0;
    }
    let path = dust_tau.max(0.0) / mu.max(0.05);
    let direct = (-path).exp();
    let diffuse = 0.5 * (1.0 - direct) * (-0.25 * dust_tau.max(0.0)).exp();
    top_w_m2 * mu * (direct + diffuse)
}

/// Dust settling on solar panels costs about 0.2 % of their output a sol
/// (Lorenz et al. 2021, from Mars landers).
pub const PANEL_DUST_PER_SOL: f64 = 0.002;

#[derive(Clone, Debug, PartialEq)]
pub struct Power {
    /// A fission surface power unit, W (NASA's Kilopower gives 10 kWe).
    pub reactor_w: f64,
    pub solar_m2: f64,
    pub solar_efficiency: f64,
    /// Share of the panels' light the dust on them blocks, 0 to 1.
    pub panel_dust: f64,
    pub battery_wh: f64,
    pub charge_wh: f64,
    /// The bus is down: nothing draws, nothing charges.
    pub cut: bool,
    pub reactor_down: bool,
    /// Supply and demand over the last step, W.
    pub supply_w: f64,
    pub demand_w: f64,
}

impl Default for Power {
    fn default() -> Self {
        Self {
            reactor_w: 10_000.0,
            solar_m2: 40.0,
            solar_efficiency: 0.28,
            panel_dust: 0.0,
            battery_wh: 40_000.0,
            charge_wh: 30_000.0,
            cut: false,
            reactor_down: false,
            supply_w: 0.0,
            demand_w: 0.0,
        }
    }
}

impl Power {
    /// What the sources give now, W, under `irradiance` on the panels.
    pub fn generation_w(&self, irradiance_w_m2: f64) -> f64 {
        if self.cut {
            return 0.0;
        }
        let reactor = if self.reactor_down { 0.0 } else { self.reactor_w };
        reactor + irradiance_w_m2 * self.solar_m2 * self.solar_efficiency * (1.0 - self.panel_dust)
    }

    /// Meets `essential_w` first and then as much of `lamps_w` as it can over
    /// `seconds`, from generation and the battery; says what share of the
    /// lamps' demand and of the essential load it met.
    pub fn draw(&mut self, seconds: f64, irradiance_w_m2: f64, essential_w: f64, lamps_w: f64) -> (f64, f64) {
        self.demand_w = essential_w + lamps_w;
        if self.cut {
            self.supply_w = 0.0;
            return (0.0, 0.0);
        }
        let generated = self.generation_w(irradiance_w_m2);
        let stored_w = self.charge_wh * 3600.0 / seconds.max(1e-9);
        let available = generated + stored_w;
        let essential = (available / essential_w.max(1e-9)).min(1.0);
        let left = (available - essential_w).max(0.0);
        let lamps = if lamps_w > 0.0 { (left / lamps_w).min(1.0) } else { 1.0 };
        let used = essential_w * essential + lamps_w * lamps;
        self.charge_wh = (self.charge_wh + (generated - used) * seconds / 3600.0).clamp(0.0, self.battery_wh);
        self.supply_w = used;
        (lamps, essential)
    }

    /// Dust settles on the panels over `sols`.
    pub fn settle_dust(&mut self, sols: f64) {
        self.panel_dust = (1.0 - (1.0 - self.panel_dust) * (1.0 - PANEL_DUST_PER_SOL).powf(sols)).min(1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dust_storm_cuts_the_sunlight_on_the_ground() {
        let clear = surface_irradiance(586.0, 60f64.to_radians(), 0.4);
        let storm = surface_irradiance(586.0, 60f64.to_radians(), 5.0);
        assert!((300.0..500.0).contains(&clear), "clear noon {clear} W/m²");
        assert!(storm < 0.25 * clear, "storm {storm} against clear {clear}");
        assert_eq!(surface_irradiance(586.0, -0.1, 0.4), 0.0);
    }

    #[test]
    fn lamps_go_dark_before_life_support_does() {
        let mut power = Power { reactor_w: 2_000.0, charge_wh: 0.0, solar_m2: 0.0, ..Power::default() };
        let (lamps, essential) = power.draw(60.0, 0.0, 1_500.0, 2_000.0);
        assert_eq!(essential, 1.0);
        assert!((lamps - 0.25).abs() < 1e-9);
        power.cut = true;
        assert_eq!(power.draw(60.0, 0.0, 1_500.0, 2_000.0), (0.0, 0.0));
    }

    #[test]
    fn the_battery_carries_the_night_until_it_runs_down() {
        let mut power = Power { reactor_w: 0.0, ..Power::default() };
        let mut hours = 0;
        while power.draw(3600.0, 0.0, 3_000.0, 0.0).1 >= 1.0 && hours < 48 {
            hours += 1;
        }
        assert_eq!(hours, 10, "30 kWh carries 3 kW for ten hours");
    }

    #[test]
    fn panels_lose_a_fifth_of_a_percent_a_sol_to_dust() {
        let mut power = Power::default();
        power.settle_dust(100.0);
        assert!((power.panel_dust - 0.181).abs() < 0.002, "{}", power.panel_dust);
    }
}
