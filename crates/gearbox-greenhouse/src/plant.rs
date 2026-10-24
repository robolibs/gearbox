//! One plant, grown by the Modified Energy Cascade at the conditions around it.
//!
//! The MEC models a canopy per square metre at a fitted planting density; a
//! plant here is its share of that canopy, so a tray of them at the fitted
//! spacing grows as BVAD's crops do. Three things the MEC lacks are added:
//! development and growth follow air temperature through the crop's cardinal
//! temperatures, carbon is gained and lost hour by hour (Boscheri et al. 2012),
//! and stresses accumulate into health, which a camera can see.

use crate::species::{Edible, Species};

/// Fraction of light the closed canopy absorbs (BVAD Equation 4-15).
const CANOPY_MAX: f64 = 0.93;
/// Molar mass of carbon and of water, g/mol.
const CARBON_G: f64 = 12.011;
const WATER_G: f64 = 18.015;
/// Dry mass a seedling starts from, as a share of the nominal plant.
const SEEDLING_SHARE: f64 = 0.002;
/// Days a seed may sit in a medium it cannot sprout in before it is lost.
const GERMINATION_DAYS: f64 = 10.0;
/// Perchlorate in the medium, weight %, above which seeds do not sprout (Oze et al. 2021).
pub const PERCHLORATE_LIMIT: f64 = 0.1;

/// The air, light, water and medium around a plant at one moment.
#[derive(Clone, Copy, Debug)]
pub struct Surroundings {
    /// Photon flux reaching the canopy now, µmol/m²/s.
    pub ppf: f64,
    /// Photon flux while its lamps are lit, and hours lit per cycle.
    pub lit_ppf: f64,
    pub lit_hours: f64,
    /// Length of the light cycle, hours: the habitat's day.
    pub cycle_hours: f64,
    pub temperature_c: f64,
    /// Relative humidity, 0 to 1.
    pub humidity: f64,
    pub co2_ppm: f64,
    pub o2_kpa: f64,
    pub pressure_kpa: f64,
    /// Air moves over the leaves.
    pub airflow: bool,
    /// Water the root zone can give, 0 to 1.
    pub water: f64,
    /// Nutrients in the root zone against what the crop needs, 0 to 1.
    pub nutrients: f64,
    /// Perchlorate in the medium, weight %.
    pub perchlorate: f64,
}

impl Surroundings {
    /// A growth chamber at the crop's nominal conditions (BVAD Table 4-117),
    /// lit at `ppf` for its nominal photoperiod, with 1200 ppm CO2.
    pub fn nominal(species: &Species, ppf: f64) -> Self {
        Self {
            ppf,
            lit_ppf: ppf,
            lit_hours: species.photoperiod_h,
            cycle_hours: 24.0,
            temperature_c: species.light_c,
            humidity: 0.70,
            co2_ppm: 1200.0,
            o2_kpa: 21.0,
            pressure_kpa: 101.3,
            airflow: true,
            water: 1.0,
            nutrients: 1.0,
            perchlorate: 0.0,
        }
    }
}

/// What a plant took from and gave to the air over a step, moles and kilograms.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Exchange {
    /// CO2 taken up; negative when respiration gives it back.
    pub co2_mol: f64,
    /// O2 given off; negative when respiration takes it.
    pub o2_mol: f64,
    /// Water transpired into the air.
    pub water_kg: f64,
}

impl std::ops::AddAssign for Exchange {
    fn add_assign(&mut self, other: Self) {
        self.co2_mol += other.co2_mol;
        self.o2_mol += other.o2_mol;
        self.water_kg += other.water_kg;
    }
}

/// Where a plant is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// Sown, not yet up.
    Seed,
    /// Up, its canopy still open.
    Seedling,
    /// Growing leaf.
    Vegetative,
    /// Forming what is eaten: fruit, tubers, grain.
    Forming,
    /// Ready to harvest.
    Ripe,
    Dead,
}

impl Stage {
    pub fn name(self) -> &'static str {
        match self {
            Self::Seed => "seed",
            Self::Seedling => "seedling",
            Self::Vegetative => "vegetative",
            Self::Forming => "forming",
            Self::Ripe => "ripe",
            Self::Dead => "dead",
        }
    }
}

/// One plant.
#[derive(Clone, Debug, PartialEq)]
pub struct Plant {
    pub species: &'static Species,
    /// Development age, days after emergence at the nominal temperature.
    pub age_days: f64,
    /// Days since it was sown, whether or not it came up.
    pub sown_days: f64,
    pub emerged: bool,
    /// Dry biomass and its edible part, g.
    pub biomass_g: f64,
    pub edible_g: f64,
    /// 0 dead to 1 sound.
    pub health: f64,
    /// Recent shortage of water, nutrients and air, each 0 to 1: wilting, yellowing, suffocating roots.
    pub water_stress: f64,
    pub nutrient_stress: f64,
    pub root_stress: f64,
    /// Leaf margins scorched by light or still, wet air (tip burn), 0 to 1.
    pub tip_burn: f64,
    /// Fungal infection, 0 to 1; it spreads in still, wet air and kills at 1.
    pub mould: f64,
    pub alive: bool,
}

impl Plant {
    /// A seed just sown.
    pub fn sown(species: &'static Species) -> Self {
        Self {
            species,
            age_days: 0.0,
            sown_days: 0.0,
            emerged: false,
            biomass_g: species.nominal_biomass_g() * SEEDLING_SHARE,
            edible_g: 0.0,
            health: 1.0,
            water_stress: 0.0,
            nutrient_stress: 0.0,
            root_stress: 0.0,
            tip_burn: 0.0,
            mould: 0.0,
            alive: true,
        }
    }

    pub fn stage(&self) -> Stage {
        let s = self.species;
        if !self.alive {
            Stage::Dead
        } else if !self.emerged {
            Stage::Seed
        } else if self.age_days >= s.harvest_days {
            Stage::Ripe
        } else if s.edible != Edible::Shoot && self.age_days >= s.edible_days {
            Stage::Forming
        } else if self.age_days >= self.closure_days(300.0, 1200.0) * 0.5 {
            Stage::Vegetative
        } else {
            Stage::Seedling
        }
    }

    /// Share of its life gone, 0 sown to 1 at harvest.
    pub fn maturity(&self) -> f64 {
        (self.age_days / self.species.harvest_days).clamp(0.0, 1.0)
    }

    /// Size against a nominal plant at harvest, by mass: a linear scale.
    pub fn size(&self) -> f64 {
        (self.biomass_g / self.species.nominal_biomass_g()).max(0.0).cbrt()
    }

    /// Fresh mass of the edible part, kg.
    pub fn edible_fresh_kg(&self) -> f64 {
        self.edible_g / (1.0 - self.species.edible_water) / 1000.0
    }

    /// Leaf area index of the plant's own patch: from the share of light it absorbs.
    pub fn leaf_area_index(&self, lit_ppf: f64, co2: f64) -> f64 {
        let absorbed = self.absorbed(lit_ppf, co2).min(CANOPY_MAX * 0.999);
        // Beer's law with an extinction coefficient of 0.7.
        -(1.0 - absorbed).ln() / 0.7
    }

    fn closure_days(&self, lit_ppf: f64, co2: f64) -> f64 {
        self.species.closure_days.at(lit_ppf.clamp(100.0, 2000.0), co2.clamp(330.0, 1300.0)).max(1.0)
    }

    /// Fraction of the light the plant's patch absorbs (Equation 4-15).
    fn absorbed(&self, effective_ppf: f64, co2: f64) -> f64 {
        if !self.emerged {
            return 0.0;
        }
        let closes = self.closure_days(effective_ppf, co2);
        CANOPY_MAX * (self.age_days / closes).min(1.0).powf(self.species.closure_exponent)
    }

    /// Canopy quantum yield after ageing (Equation 4-16).
    fn quantum_yield(&self, lit_ppf: f64, co2: f64) -> f64 {
        let s = self.species;
        let best = s.quantum_yield.at(lit_ppf.clamp(100.0, 2000.0), co2.clamp(330.0, 1300.0)).max(0.0);
        match (s.senescence_days, s.quantum_yield_min) {
            (Some(onset), Some(low)) if self.age_days > onset => {
                let gone = ((self.age_days - onset) / (s.mature_days - onset)).clamp(0.0, 1.0);
                best - (best - low) * gone
            }
            _ => best,
        }
    }

    /// Grows the plant for `seconds` in `around`, and says what it exchanged with the air.
    pub fn grow(&mut self, seconds: f64, around: &Surroundings) -> Exchange {
        if !self.alive || seconds <= 0.0 {
            return Exchange::default();
        }
        let days = seconds / 86_400.0;
        let s = self.species;
        self.sown_days += days;
        let warmth = s.temperature_factor(around.temperature_c);
        if !self.emerged {
            let sprouts = around.perchlorate < PERCHLORATE_LIMIT && around.water > 0.3 && warmth > 0.0;
            if sprouts {
                self.emerged = true;
            } else if self.sown_days > GERMINATION_DAYS {
                self.die();
            }
            return Exchange::default();
        }

        self.stress(days, around);
        let thrive = warmth.min(1.0)
            * (1.0 - self.water_stress)
            * (1.0 - 0.6 * self.nutrient_stress)
            * (1.0 - self.root_stress)
            * (1.0 - 0.5 * self.tip_burn)
            * self.health.sqrt();
        self.age_days += days * warmth * (0.5 + 0.5 * (1.0 - self.water_stress));

        // Photoperiod and flux the canopy's growth is fitted against (Equation 4-32).
        let lit_hours = around.lit_hours.clamp(0.0, around.cycle_hours);
        let effective = around.lit_ppf * lit_hours / s.photoperiod_h;
        let absorbed = self.absorbed(effective, around.co2_ppm);
        let yield_ = self.quantum_yield(around.lit_ppf, around.co2_ppm) * co2_floor(around.co2_ppm);
        // Gross carbon while lit, and respiration spread over the whole cycle
        // (Equation 4-26), µmol C per m² of canopy per second.
        let lit_gross = absorbed * yield_ * around.lit_ppf * thrive;
        let respiration = (1.0 - s.carbon_use) * lit_gross * lit_hours / around.cycle_hours.max(1.0);
        let gross = absorbed * yield_ * around.ppf * thrive;
        let net = gross - respiration;

        let share = s.spacing_m2();
        let carbon_mol = net * 1e-6 * seconds * share;
        let grown_g = carbon_mol * CARBON_G / s.carbon_fraction;
        let floor = s.nominal_biomass_g() * SEEDLING_SHARE;
        self.biomass_g = (self.biomass_g + grown_g).max(floor);
        if grown_g > 0.0 && self.age_days >= s.edible_days {
            self.edible_g += grown_g * s.edible_share;
        }
        self.edible_g = self.edible_g.min(self.biomass_g);

        let water_kg = transpiration(s, net.max(0.0), around) * seconds * share * WATER_G / 1000.0;
        Exchange {
            co2_mol: carbon_mol,
            o2_mol: carbon_mol * s.oxygen_per_carbon,
            water_kg: water_kg * (1.0 - self.water_stress),
        }
    }

    /// Stresses move towards what the surroundings impose, health pays for
    /// them, and mould spreads where the air stands wet.
    fn stress(&mut self, days: f64, around: &Surroundings) {
        let s = self.species;
        let approach = |level: &mut f64, target: f64, per_day: f64| {
            *level += (target.clamp(0.0, 1.0) - *level) * (1.0 - (-per_day * days).exp());
        };
        let vpd = vapour_deficit(around.temperature_c, around.humidity);
        // Thin air dries leaves faster than roots can water them (NASA KSC: 6.1x at 10 kPa).
        let thin = ((15.0 - around.pressure_kpa) / 10.0).clamp(0.0, 1.0);
        let parched = ((vpd - 2.0) / 2.0).clamp(0.0, 1.0);
        approach(&mut self.water_stress, (1.0 - around.water).max(parched).max(thin), 3.0);
        approach(&mut self.nutrient_stress, 1.0 - around.nutrients, 0.15);
        // Roots and leaves suffocate below 7 kPa of oxygen, fully by 2 kPa (radish trials).
        approach(&mut self.root_stress, (7.0 - around.o2_kpa) / 5.0, 2.0);
        let glare = ((around.lit_ppf - s.ppf_limit) / s.ppf_limit).clamp(0.0, 1.0);
        let still = if around.airflow { 0.0 } else { 0.3 };
        let burn = glare.max(if vpd < 0.25 { still } else { 0.0 });
        self.tip_burn = (self.tip_burn + burn * days * 0.2).min(1.0);
        let damp = around.humidity > 0.85 && !around.airflow;
        if damp {
            self.mould = (self.mould + days * (0.04 + 0.4 * self.mould)).min(1.0);
        } else {
            self.mould = (self.mould - days * 0.02).max(0.0);
        }

        let cold_or_hot = if s.temperature_factor(around.temperature_c) == 0.0 { 1.0 } else { 0.0 };
        let harm = 0.8 * self.water_stress.powi(2)
            + 0.08 * self.nutrient_stress.powi(2)
            + 1.0 * self.root_stress.powi(2)
            + 0.5 * cold_or_hot
            + 0.6 * self.mould.powi(2);
        let mended = if harm < 0.02 { 0.05 } else { 0.0 };
        self.health = (self.health - harm * days + mended * days).min(1.0);
        if self.health <= 0.0 || self.mould >= 1.0 {
            self.die();
        }
    }

    fn die(&mut self) {
        self.alive = false;
        self.health = 0.0;
    }
}

/// Below 150 ppm a C3 canopy cannot fix more carbon than it breathes out;
/// the MEC fits start at 330 ppm, so the yield falls to nothing between.
fn co2_floor(co2: f64) -> f64 {
    ((co2 - 150.0) / 180.0).clamp(0.0, 1.0)
}

/// Saturated vapour pressure and deficit, kPa (BVAD Equation 4-24).
pub fn vapour_deficit(celsius: f64, humidity: f64) -> f64 {
    let saturated = 0.611 * (17.4 * celsius / (celsius + 239.0)).exp();
    saturated * (1.0 - humidity.clamp(0.0, 1.0))
}

/// Canopy transpiration, mol water per m² per second (BVAD Equations 4-27 to 4-30).
fn transpiration(s: &Species, net: f64, around: &Surroundings) -> f64 {
    let vpd = vapour_deficit(around.temperature_c, around.humidity);
    let co2 = around.co2_ppm.max(1.0);
    let (stomatal, aerodynamic) = if s.erect {
        ((0.1389 + 15.32 * around.humidity) * net / co2, 5.5)
    } else {
        ((1.717 * around.temperature_c - 19.96 - 10.54 * vpd).max(0.0) * net / co2, 2.5)
    };
    let aerodynamic = if around.airflow { aerodynamic } else { aerodynamic * 0.2 };
    let canopy = if stomatal + aerodynamic > 0.0 { aerodynamic * stomatal / (aerodynamic + stomatal) } else { 0.0 };
    canopy * vpd / around.pressure_kpa.max(0.5)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::species::{ALL, LETTUCE, POTATO};

    /// Grows one plant hour by hour at nominal conditions until harvest.
    fn grown(species: &'static Species, ppf: f64) -> (Plant, Exchange) {
        let mut plant = Plant::sown(species);
        let mut total = Exchange::default();
        let around = Surroundings::nominal(species, ppf);
        let mut hour = 0.0;
        while plant.alive && plant.stage() != Stage::Ripe && hour < 24.0 * 400.0 {
            let lit = (hour % 24.0) < species.photoperiod_h;
            let here = Surroundings { ppf: if lit { ppf } else { 0.0 }, ..around };
            total += plant.grow(3600.0, &here);
            hour += 1.0;
        }
        (plant, total)
    }

    /// The flux that gives each crop its BVAD daily light integral over its photoperiod.
    fn nominal_flux(species: &Species) -> f64 {
        let integral = match species.name {
            "lettuce" => 17.0,
            "tomato" => 27.0,
            "potato" => 28.0,
            _ => 115.0,
        };
        integral * 1e6 / (species.photoperiod_h * 3600.0)
    }

    #[test]
    fn nominal_crops_yield_what_nasa_measured() {
        for species in ALL {
            let (plant, _) = grown(species, nominal_flux(species));
            assert!(plant.alive, "{} died", species.name);
            let edible_per_m2 = plant.edible_g * species.density;
            let expected = species.edible_rate * species.growing_days;
            let ratio = edible_per_m2 / expected;
            assert!((0.7..1.4).contains(&ratio), "{}: {edible_per_m2:.0} g/m² edible, BVAD {expected:.0} (x{ratio:.2})", species.name);
        }
    }

    #[test]
    fn a_canopy_makes_oxygen_for_the_carbon_it_fixes() {
        let (plant, total) = grown(&LETTUCE, nominal_flux(&LETTUCE));
        assert!(total.co2_mol > 0.0 && total.water_kg > 0.0);
        assert!((total.o2_mol / total.co2_mol - LETTUCE.oxygen_per_carbon).abs() < 1e-9);
        // Every fixed gram of carbon is in the plant.
        let carbon_g = total.co2_mol * CARBON_G;
        let kept = (plant.biomass_g - LETTUCE.nominal_biomass_g() * SEEDLING_SHARE) * LETTUCE.carbon_fraction;
        assert!((carbon_g - kept).abs() < 0.01 * kept.max(1.0), "{carbon_g} g C fixed, {kept} kept");
    }

    #[test]
    fn in_the_dark_a_plant_breathes_carbon_dioxide_out() {
        let mut plant = Plant::sown(&LETTUCE);
        let around = Surroundings::nominal(&LETTUCE, 300.0);
        for _ in 0..(20 * 24) {
            plant.grow(3600.0, &around);
        }
        let dark = Surroundings { ppf: 0.0, ..around };
        let breath = plant.grow(3600.0, &dark);
        assert!(breath.co2_mol < 0.0 && breath.o2_mol < 0.0, "{breath:?}");
    }

    #[test]
    fn perchlorate_regolith_keeps_seeds_from_sprouting() {
        let mut plant = Plant::sown(&POTATO);
        let regolith = Surroundings { perchlorate: 0.5, ..Surroundings::nominal(&POTATO, 400.0) };
        for _ in 0..(12 * 24) {
            plant.grow(3600.0, &regolith);
        }
        assert!(!plant.emerged && !plant.alive);
        let mut washed = Plant::sown(&POTATO);
        washed.grow(3600.0, &Surroundings { perchlorate: 0.0, ..regolith });
        assert!(washed.emerged);
    }

    #[test]
    fn cold_low_oxygen_or_dry_roots_hold_growth_back() {
        let sound = grown(&LETTUCE, 300.0).0.biomass_g;
        let stunted = |change: fn(&mut Surroundings)| {
            let mut plant = Plant::sown(&LETTUCE);
            let mut around = Surroundings::nominal(&LETTUCE, 300.0);
            change(&mut around);
            for hour in 0..(30 * 24) {
                let here = Surroundings { ppf: if hour % 24 < 16 { 300.0 } else { 0.0 }, ..around };
                plant.grow(3600.0, &here);
            }
            plant
        };
        let cold = stunted(|a| a.temperature_c = 10.0);
        let thin = stunted(|a| a.o2_kpa = 4.0);
        let dry = stunted(|a| a.water = 0.4);
        for (what, plant) in [("cold", &cold), ("hypoxic", &thin), ("dry", &dry)] {
            assert!(plant.biomass_g < 0.6 * sound, "{what}: {} g against {sound} g", plant.biomass_g);
        }
        assert!(dry.water_stress > 0.5 && thin.root_stress > 0.4);
    }

    #[test]
    fn still_wet_air_grows_mould_that_kills() {
        let mut plant = Plant::sown(&LETTUCE);
        let damp = Surroundings { humidity: 0.95, airflow: false, ..Surroundings::nominal(&LETTUCE, 300.0) };
        let mut days = 0;
        while plant.alive && days < 30 {
            for _ in 0..24 {
                plant.grow(3600.0, &damp);
            }
            days += 1;
        }
        assert!(!plant.alive, "mould {} after {days} days", plant.mould);
        assert!(days > 3, "mould needs days to take hold, killed in {days}");
    }
}
