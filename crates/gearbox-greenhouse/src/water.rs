//! Root zones: the nutrient tank a room's trays drink from, and what the
//! roots stand in.
//!
//! EDEN ISS ran two nutrient loops at pH 5.8 and EC 2.5 or 3.5 mS/cm and had
//! to drain, clean and refill its tanks seven times in a season for biofilm
//! (Zabel et al. 2019). A tank here loses water to transpiration and gains the
//! condensate back, is dosed back to its EC as roots take nutrients, and grows
//! biofilm that clogs the roots until someone cleans it.

/// Days for biofilm to grow from a clean tank to fully fouled: a season's seven cleanings.
const BIOFILM_DAYS: f64 = 45.0;
/// Fouling above which roots start to suffer.
pub const BIOFILM_HARM: f64 = 0.6;

/// What roots stand in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Medium {
    /// Rock wool over a nutrient film: what EDEN ISS and every built system used.
    Hydroponic,
    /// Martian regolith in a bed, as Watney farmed it: perchlorate in weight
    /// % (Phoenix measured 0.4 to 0.6), and organic matter worked in.
    Regolith { perchlorate: f64, organic: f64 },
}

impl Medium {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Hydroponic => "hydroponic",
            Self::Regolith { .. } => "regolith",
        }
    }

    /// Martian regolith as it is dug up.
    pub const RAW_REGOLITH: Self = Self::Regolith { perchlorate: 0.5, organic: 0.0 };

    pub fn perchlorate(&self) -> f64 {
        match self {
            Self::Hydroponic => 0.0,
            Self::Regolith { perchlorate, .. } => *perchlorate,
        }
    }

    /// Nutrients the medium holds for roots, against what the crop needs:
    /// raw regolith holds almost no nitrogen; compost or frass at 5 to 30 %
    /// brings it up (Wamelink et al. 2014, 2019; Frontiers 2025).
    pub fn fertility(&self, tank_nutrients: f64) -> f64 {
        match self {
            Self::Hydroponic => tank_nutrients,
            Self::Regolith { organic, .. } => (0.15 + organic / 0.15).min(1.0) * (0.5 + 0.5 * tank_nutrients),
        }
    }
}

/// One nutrient tank.
#[derive(Clone, Debug, PartialEq)]
pub struct Tank {
    pub name: String,
    pub water_l: f64,
    pub capacity_l: f64,
    /// Electrical conductivity, mS/cm, and what dosing holds it at.
    pub ec: f64,
    pub ec_target: f64,
    /// Concentrate left for dosing, litres.
    pub stock_l: f64,
    /// Fouling, 0 clean to 1.
    pub biofilm: f64,
    pub pump_failed: bool,
    /// Water lost to a leak, litres per hour.
    pub leak_l_h: f64,
}

impl Tank {
    pub fn new(name: &str, capacity_l: f64, ec_target: f64) -> Self {
        Self {
            name: name.to_string(),
            water_l: capacity_l * 0.9,
            capacity_l,
            ec: ec_target,
            ec_target,
            stock_l: 40.0,
            biofilm: 0.0,
            pump_failed: false,
            leak_l_h: 0.0,
        }
    }

    /// Water the roots can draw, 0 to 1: none once the pump stops or the tank runs low.
    pub fn water(&self) -> f64 {
        if self.pump_failed {
            return 0.0;
        }
        let level = self.water_l / self.capacity_l.max(1e-9);
        let clogged = ((self.biofilm - BIOFILM_HARM) / (1.0 - BIOFILM_HARM)).clamp(0.0, 1.0);
        (level / 0.1).clamp(0.0, 1.0) * (1.0 - 0.15 * clogged)
    }

    /// Nutrients against what the crop needs, 0 to 1.
    pub fn nutrients(&self) -> f64 {
        let clogged = ((self.biofilm - BIOFILM_HARM) / (1.0 - BIOFILM_HARM)).clamp(0.0, 1.0);
        (self.ec / self.ec_target.max(1e-9)).clamp(0.0, 1.0) * (1.0 - 0.6 * clogged)
    }

    /// Steps the tank `seconds` on: roots drew `drawn_kg` and took up
    /// nutrients for `grown_g` of dry matter; `returned_kg` of condensate came back.
    pub fn step(&mut self, seconds: f64, drawn_kg: f64, returned_kg: f64, grown_g: f64) {
        let days = seconds / 86_400.0;
        let leaked = self.leak_l_h * seconds / 3600.0;
        self.water_l = (self.water_l - drawn_kg + returned_kg - leaked).clamp(0.0, self.capacity_l);
        // About 8 % of plant dry matter is mineral; at EC 2.5 a litre carries some 1.6 g of salts.
        let salts_g = grown_g.max(0.0) * 0.08;
        let carried = self.water_l.max(1.0) * 1.6 / 2.5;
        self.ec = (self.ec - salts_g / carried * self.ec_target).max(0.0);
        if self.ec < self.ec_target && self.stock_l > 0.0 {
            let dose_l = ((self.ec_target - self.ec) * self.water_l / 200.0).min(self.stock_l);
            self.stock_l -= dose_l;
            self.ec += dose_l * 200.0 / self.water_l.max(1.0);
        }
        self.biofilm = (self.biofilm + days / BIOFILM_DAYS).min(1.0);
    }

    /// Drain, scrub and refill: clean, full, at its EC.
    pub fn clean(&mut self) {
        self.biofilm = 0.0;
        self.water_l = self.capacity_l * 0.9;
        self.ec = self.ec_target;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn biofilm_builds_over_weeks_and_starves_roots_until_cleaned() {
        let mut tank = Tank::new("t", 400.0, 2.5);
        let fresh = (tank.water(), tank.nutrients());
        for _ in 0..40 {
            tank.step(86_400.0, 0.0, 0.0, 0.0);
        }
        assert!(tank.biofilm > 0.8 && tank.nutrients() < 0.8 * fresh.1, "{tank:?}");
        tank.clean();
        assert_eq!((tank.water(), tank.nutrients()), fresh);
    }

    #[test]
    fn a_stopped_pump_or_an_empty_tank_gives_no_water() {
        let mut tank = Tank::new("t", 400.0, 2.5);
        tank.pump_failed = true;
        assert_eq!(tank.water(), 0.0);
        tank.pump_failed = false;
        tank.leak_l_h = 50.0;
        tank.step(8.0 * 3600.0, 0.0, 0.0, 0.0);
        assert_eq!(tank.water(), 0.0);
    }

    #[test]
    fn dosing_holds_the_conductivity_while_stock_lasts() {
        let mut tank = Tank::new("t", 400.0, 2.5);
        tank.step(3600.0, 2.0, 0.0, 200.0);
        assert!((tank.ec - 2.5).abs() < 0.05, "dosed to {}", tank.ec);
        tank.stock_l = 0.0;
        for _ in 0..20 {
            tank.step(3600.0, 2.0, 0.0, 200.0);
        }
        assert!(tank.nutrients() < 0.9, "undosed {}", tank.ec);
    }

    #[test]
    fn raw_regolith_feeds_roots_poorly_until_amended() {
        assert!(Medium::RAW_REGOLITH.fertility(1.0) < 0.2);
        let amended = Medium::Regolith { perchlorate: 0.0, organic: 0.15 };
        assert!(amended.fertility(1.0) > 0.95);
    }
}
