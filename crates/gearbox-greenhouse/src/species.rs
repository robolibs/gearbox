//! The crops a Mars greenhouse grows, as NASA's Modified Energy Cascade (MEC)
//! describes them: Life Support Baseline Values and Assumptions Document,
//! NASA/TP-2015-218570 Rev1 (2018), §4.14, Tables 4-97 and 4-103 to 4-128,
//! after Cavazzoni (2001). Each crop also carries cardinal temperatures, which
//! the MEC lacks: approximate crop-model values (tomato after Boote et al.
//! 2012, wheat after Porter and Gawith 1999).

/// A fit in photon flux and CO2 of the form of BVAD Table 4-106: rows are
/// 1/[CO2], 1, [CO2], [CO2]², [CO2]³; columns 1/PPF, 1, PPF, PPF², PPF³.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit(pub [[f64; 5]; 5]);

impl Fit {
    /// The fit at a photon flux (µmol/m²/s) and CO2 mole fraction (µmol/mol).
    pub fn at(&self, ppf: f64, co2: f64) -> f64 {
        let rows = [1.0 / co2, 1.0, co2, co2 * co2, co2 * co2 * co2];
        let cols = [1.0 / ppf, 1.0, ppf, ppf * ppf, ppf * ppf * ppf];
        let mut sum = 0.0;
        for (row, r) in self.0.iter().zip(rows) {
            for (c, k) in row.iter().zip(cols) {
                sum += c * r * k;
            }
        }
        sum
    }
}

/// How a crop stands: what its geometry is built as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// A head of leaves from one crown.
    Rosette,
    /// An upright stem with compound leaves and trusses of fruit.
    Vine,
    /// A bush of compound leaves over tubers in the root zone.
    Bush,
    /// Tillers of narrow leaves that end in ears.
    Grass,
}

/// What is harvested.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edible {
    /// The whole shoot.
    Shoot,
    /// Fruit picked off the plant, which keeps growing.
    Fruit,
    /// Tubers lifted from the root zone with the plant.
    Tuber,
    /// Grain in the ears.
    Grain,
}

/// One crop and cultivar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Species {
    pub name: &'static str,
    pub cultivar: &'static str,
    /// Canopy closure exponent n (Table 4-103).
    pub closure_exponent: f64,
    /// Maximum canopy quantum yield, µmol C fixed per µmol absorbed (Tables 4-107 to 4-115).
    pub quantum_yield: Fit,
    /// Quantum yield at maturity once the canopy ages (Table 4-105), when it does.
    pub quantum_yield_min: Option<f64>,
    /// 24-hour carbon use efficiency (Table 4-105).
    pub carbon_use: f64,
    /// Days after emergence until the canopy closes (Tables 4-120 to 4-128).
    pub closure_days: Fit,
    /// Nominal photoperiod H₀, hours (Table 4-117).
    pub photoperiod_h: f64,
    /// Plants per square metre the canopy model is fitted for (Table 4-117).
    pub density: f64,
    /// Nominal light and dark cycle air temperature, °C (Table 4-117).
    pub light_c: f64,
    pub dark_c: f64,
    /// Share of carbon gain going to the edible part after `edible_days` (Table 4-118).
    pub edible_share: f64,
    /// Days after emergence the edible part starts forming, the canopy starts
    /// to age, and the crop is mature (Table 4-118).
    pub edible_days: f64,
    pub senescence_days: Option<f64>,
    pub mature_days: f64,
    /// When it is taken in (white potato is lifted at 105 dAE, though modelled to 138).
    pub harvest_days: f64,
    /// Biomass carbon fraction and oxygen production fraction (Table 4-119).
    pub carbon_fraction: f64,
    pub oxygen_per_carbon: f64,
    /// Upright leaves (wheat, rice): the erectophile conductance of Equation 4-29.
    pub erect: bool,
    /// Base, optimum and ceiling air temperature for development and growth, °C.
    pub cardinal_c: [f64; 3],
    /// Mature height, m, harvest index and water share of the fresh edible part (Table 4-97).
    pub height_m: f64,
    pub harvest_index: f64,
    pub edible_water: f64,
    /// Edible dry productivity at nominal conditions, g/m²/d, and the growing period it holds over (Tables 4-96, 4-97).
    pub edible_rate: f64,
    pub growing_days: f64,
    /// Highest photon flux the crop takes without damage, µmol/m²/s (lettuce tip burn, BVAD §4.14.1.5).
    pub ppf_limit: f64,
    pub form: Form,
    pub edible: Edible,
}

impl Species {
    /// Dry biomass one plant reaches at nominal conditions, g: the edible
    /// productivity over the growing period, over the harvest index, per plant.
    pub fn nominal_biomass_g(&self) -> f64 {
        self.edible_rate * self.growing_days / self.harvest_index / self.density
    }

    /// Ground one plant takes at its fitted density, m².
    pub fn spacing_m2(&self) -> f64 {
        1.0 / self.density
    }

    /// Relative development and growth rate at an air temperature, 1 at the
    /// nominal light-cycle temperature: the beta function of Yan and Hunt
    /// (1999) over the cardinal temperatures.
    pub fn temperature_factor(&self, celsius: f64) -> f64 {
        let response = |t: f64| {
            let [low, best, high] = self.cardinal_c;
            if t <= low || t >= high {
                return 0.0;
            }
            ((high - t) / (high - best)) * ((t - low) / (best - low)).powf((best - low) / (high - best))
        };
        let nominal = response(self.light_c).max(1e-6);
        (response(celsius) / nominal).clamp(0.0, 1.2)
    }
}

const fn fit(rows: [[f64; 5]; 5]) -> Fit {
    Fit(rows)
}

pub const LETTUCE: Species = Species {
    name: "lettuce",
    cultivar: "Waldmann's Green",
    closure_exponent: 2.5,
    quantum_yield: fit([
        [0.0; 5],
        [0.0, 4.4763e-2, -1.1701e-5, 0.0, 0.0],
        [0.0, 5.163e-5, 0.0, -1.9731e-11, 0.0],
        [0.0, -2.075e-8, 0.0, 8.9265e-15, 0.0],
        [0.0; 5],
    ]),
    quantum_yield_min: None,
    carbon_use: 0.625,
    closure_days: fit([
        [0.0, 0.0, 1.8760, 0.0, 0.0],
        [1.0289e4, 1.7571, 0.0, 0.0, 0.0],
        [-3.7018, 0.0, 0.0, 0.0, 0.0],
        [0.0, 2.3127e-6, 0.0, 0.0, 0.0],
        [3.6648e-7, 0.0, 0.0, 0.0, 0.0],
    ]),
    photoperiod_h: 16.0,
    density: 19.2,
    light_c: 23.0,
    dark_c: 23.0,
    edible_share: 0.95,
    edible_days: 1.0,
    senescence_days: None,
    mature_days: 30.0,
    harvest_days: 30.0,
    carbon_fraction: 0.40,
    oxygen_per_carbon: 1.08,
    erect: false,
    cardinal_c: [4.0, 22.0, 32.0],
    height_m: 0.25,
    harvest_index: 0.90,
    edible_water: 0.95,
    edible_rate: 6.57,
    growing_days: 28.0,
    ppf_limit: 500.0,
    form: Form::Rosette,
    edible: Edible::Shoot,
};

pub const TOMATO: Species = Species {
    name: "tomato",
    cultivar: "Reinmann Philippe 75/59",
    closure_exponent: 2.5,
    quantum_yield: fit([
        [0.0; 5],
        [0.0, 4.0061e-2, 0.0, -7.1241e-9, 0.0],
        [0.0, 5.688e-5, -1.182e-8, 0.0, 0.0],
        [0.0, -2.2598e-8, 5.0264e-12, 0.0, 0.0],
        [0.0; 5],
    ]),
    quantum_yield_min: Some(0.01),
    carbon_use: 0.65,
    closure_days: fit([
        [6.2774e5, 0.0, 0.44686, 0.0, 0.0],
        [3.1724e3, 24.281, 5.6276e-3, -3.0690e-6, 0.0],
        [0.0; 5],
        [0.0; 5],
        [0.0; 5],
    ]),
    photoperiod_h: 12.0,
    density: 6.3,
    light_c: 26.0,
    dark_c: 22.0,
    edible_share: 0.70,
    edible_days: 41.0,
    senescence_days: Some(56.0),
    mature_days: 80.0,
    harvest_days: 80.0,
    carbon_fraction: 0.42,
    oxygen_per_carbon: 1.09,
    erect: false,
    cardinal_c: [10.0, 27.0, 35.0],
    height_m: 0.40,
    harvest_index: 0.45,
    edible_water: 0.94,
    edible_rate: 10.43,
    growing_days: 85.0,
    ppf_limit: 1500.0,
    form: Form::Vine,
    edible: Edible::Fruit,
};

pub const POTATO: Species = Species {
    name: "potato",
    cultivar: "Norland",
    closure_exponent: 2.0,
    quantum_yield: fit([
        [0.0; 5],
        [0.0, 4.6929e-2, 0.0, 0.0, -1.9602e-11],
        [0.0, 5.0910e-5, 0.0, -1.5272e-11, 0.0],
        [0.0, -2.1878e-8, 0.0, 0.0, 0.0],
        [0.0, 0.0, 4.3976e-15, 0.0, 0.0],
    ]),
    quantum_yield_min: Some(0.02),
    carbon_use: 0.625,
    closure_days: fit([
        [6.5773e5, 0.0, 0.0, 0.0, 0.0],
        [8.5626e3, 0.0, 0.042749, -1.7905e-5, 0.0],
        [0.0, 0.0, 8.8437e-7, 0.0, 0.0],
        [0.0; 5],
        [0.0; 5],
    ]),
    photoperiod_h: 12.0,
    density: 6.4,
    light_c: 20.0,
    dark_c: 16.0,
    edible_share: 1.0,
    edible_days: 45.0,
    senescence_days: Some(75.0),
    mature_days: 138.0,
    harvest_days: 105.0,
    carbon_fraction: 0.41,
    oxygen_per_carbon: 1.02,
    erect: false,
    cardinal_c: [4.0, 18.0, 30.0],
    height_m: 0.65,
    harvest_index: 0.70,
    edible_water: 0.80,
    edible_rate: 21.06,
    growing_days: 132.0,
    ppf_limit: 1500.0,
    form: Form::Bush,
    edible: Edible::Tuber,
};

pub const WHEAT: Species = Species {
    name: "wheat",
    cultivar: "Veery 10",
    closure_exponent: 1.0,
    quantum_yield: fit([
        [0.0; 5],
        [0.0, 4.4793e-2, -5.1946e-6, 0.0, 0.0],
        [0.0, 5.1583e-5, 0.0, -4.9303e-12, 0.0],
        [0.0, -2.0724e-8, 0.0, 2.2255e-15, 0.0],
        [0.0; 5],
    ]),
    quantum_yield_min: Some(0.01),
    carbon_use: 0.64,
    closure_days: fit([
        [9.5488e4, 0.0, 0.3419, -1.9076e-4, 0.0],
        [1.0686e3, 15.977, 1.9733e-4, 0.0, 0.0],
        [0.0; 5],
        [0.0; 5],
        [0.0; 5],
    ]),
    photoperiod_h: 20.0,
    density: 720.0,
    light_c: 23.0,
    dark_c: 23.0,
    edible_share: 1.0,
    edible_days: 34.0,
    senescence_days: Some(33.0),
    mature_days: 62.0,
    harvest_days: 62.0,
    carbon_fraction: 0.44,
    oxygen_per_carbon: 1.07,
    erect: true,
    cardinal_c: [0.0, 22.0, 35.0],
    height_m: 0.50,
    harvest_index: 0.40,
    edible_water: 0.12,
    edible_rate: 20.0,
    growing_days: 80.0,
    ppf_limit: 2000.0,
    form: Form::Grass,
    edible: Edible::Grain,
};

/// Every crop, by name.
pub const ALL: [&Species; 4] = [&LETTUCE, &TOMATO, &POTATO, &WHEAT];

pub fn by_name(name: &str) -> Option<&'static Species> {
    ALL.into_iter().find(|s| s.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantum_yields_are_plausible_over_the_fitted_range() {
        for species in ALL {
            for (ppf, co2) in [(200.0, 400.0), (300.0, 1200.0), (800.0, 1200.0)] {
                let cqy = species.quantum_yield.at(ppf, co2);
                assert!((0.01..0.08).contains(&cqy), "{} CQY {cqy} at {ppf}, {co2}", species.name);
            }
            // More CO2 lifts the yield from ambient to the enriched level.
            assert!(species.quantum_yield.at(300.0, 1200.0) > species.quantum_yield.at(300.0, 400.0));
        }
    }

    #[test]
    fn canopies_close_within_their_growing_season_at_nominal_light() {
        for species in ALL {
            let ppf = 300.0;
            let days = species.closure_days.at(ppf, 1200.0);
            assert!(days > 3.0 && days < species.mature_days, "{} closes on day {days}", species.name);
        }
        // Lettuce under brighter light closes sooner.
        assert!(LETTUCE.closure_days.at(450.0, 1200.0) < LETTUCE.closure_days.at(250.0, 1200.0));
    }

    #[test]
    fn temperature_peaks_near_the_optimum_and_stops_at_the_limits() {
        for species in ALL {
            assert!((species.temperature_factor(species.light_c) - 1.0).abs() < 1e-9);
            assert_eq!(species.temperature_factor(species.cardinal_c[0] - 1.0), 0.0);
            assert_eq!(species.temperature_factor(species.cardinal_c[2] + 1.0), 0.0);
            assert!(species.temperature_factor(species.cardinal_c[1]) >= 1.0 - 1e-9);
        }
    }

    #[test]
    fn nominal_plants_have_the_mass_of_real_ones() {
        // A lettuce head of about 200 g fresh, and a potato plant of about two kilograms of tubers.
        let head = LETTUCE.nominal_biomass_g() * LETTUCE.harvest_index / (1.0 - LETTUCE.edible_water);
        assert!((150.0..300.0).contains(&head), "lettuce head {head} g");
        let tubers = POTATO.nominal_biomass_g() * POTATO.harvest_index / (1.0 - POTATO.edible_water);
        assert!((1500.0..3000.0).contains(&tubers), "potato tubers {tubers} g");
    }
}
