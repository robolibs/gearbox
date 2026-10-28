//! The crops a Mars greenhouse grows, as NASA's Modified Energy Cascade (MEC)
//! describes them: Life Support Baseline Values and Assumptions Document,
//! NASA/TP-2015-218570 Rev1 (2018), §4.14, Tables 4-97 and 4-103 to 4-128,
//! after Cavazzoni (2001). Each crop also carries cardinal temperatures, which
//! the MEC lacks: approximate crop-model values (tomato after Boote et al.
//! 2012, wheat after Porter and Gawith 1999, the legumes after CROPGRO, rice
//! after ORYZA2000).

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
    /// A bush of trifoliate leaves on branching stems, its seed in pods.
    Legume,
    /// Vines of broad leaves trailing over the bed from one crown.
    Creeper,
    /// A rosette over a swollen root whose shoulder stands at the surface.
    Root,
    /// Hollow upright leaves from a bulb.
    Allium,
    /// A branching upright bush hung with single fruit.
    Shrub,
    /// A crown of trifoliate leaves with trusses of berries hanging off it.
    Crown,
}

/// What a kilogram of the fresh edible part feeds, as harvested: dry seed for
/// grain and pulses (USDA FoodData Central, SR Legacy).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Nutrition {
    pub kcal: f64,
    pub protein_g: f64,
    pub fat_g: f64,
}

/// What is harvested.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edible {
    /// The whole shoot.
    Shoot,
    /// Fruit picked off the plant, which keeps growing.
    Fruit,
    /// Tubers, roots or pods lifted from the root zone with the plant.
    Tuber,
    /// Grain or dry seed, threshed from the cut plant.
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
    /// Share of the borrowed quantum yield a crop without its own MEC fit
    /// reaches, set so it yields its BVAD productivity; 1 with its own fit.
    pub quantum_scale: f64,
    /// 24-hour carbon use efficiency (Table 4-105), and the floor a legume's
    /// falls to once its canopy ages (Equation 4-17).
    pub carbon_use: f64,
    pub carbon_use_min: Option<f64>,
    /// Days after emergence until the canopy closes (Tables 4-120 to 4-128).
    pub closure_days: Fit,
    /// Nominal photoperiod H₀, hours (Table 4-117), and the light the crop is
    /// grown under over it, mol/m²/d (Table 4-96).
    pub photoperiod_h: f64,
    pub daily_light_mol: f64,
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
    pub nutrition: Nutrition,
    /// Share of a fruiting crop's flowers that set fruit with no help: tomato
    /// in still greenhouse air needs buzzing, strawberry pollen moved, pepper
    /// and beans mostly set their own; 1 for crops not eaten as fruit.
    pub self_set: f64,
    /// Days a set fruit takes to ripen, and days a fruiting crop bears past its
    /// harvest age before it is spent; 0 for crops taken whole.
    pub ripen_days: f64,
    pub bearing_days: f64,
    /// A leaf crop cut and come again: its outer leaves are taken and it grows on.
    pub regrows: bool,
}

impl Species {
    /// Photon flux that gives the crop its daily light over its photoperiod, µmol/m²/s.
    pub fn nominal_ppf(&self) -> f64 {
        self.daily_light_mol * 1e6 / (self.photoperiod_h * 3600.0)
    }

    /// Dry biomass one plant reaches at nominal conditions, g: the edible
    /// productivity over the growing period, over the harvest index, per plant.
    pub fn nominal_biomass_g(&self) -> f64 {
        self.edible_rate * self.growing_days / self.harvest_index / self.density
    }

    /// Fresh mass of `dry_g` of the edible part, kg.
    pub fn fresh_kg(&self, dry_g: f64) -> f64 {
        dry_g / (1.0 - self.edible_water) / 1000.0
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
    quantum_scale: 1.0,
    carbon_use: 0.625,
    carbon_use_min: None,
    closure_days: fit([
        [0.0, 0.0, 1.8760, 0.0, 0.0],
        [1.0289e4, 1.7571, 0.0, 0.0, 0.0],
        [-3.7018, 0.0, 0.0, 0.0, 0.0],
        [0.0, 2.3127e-6, 0.0, 0.0, 0.0],
        [3.6648e-7, 0.0, 0.0, 0.0, 0.0],
    ]),
    photoperiod_h: 16.0,
    daily_light_mol: 17.0,
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
    nutrition: Nutrition { kcal: 150.0, protein_g: 13.6, fat_g: 1.5 },
    self_set: 1.0,
    ripen_days: 0.0,
    bearing_days: 0.0,
    regrows: false,
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
    quantum_scale: 1.0,
    carbon_use: 0.65,
    carbon_use_min: None,
    closure_days: fit([
        [6.2774e5, 0.0, 0.44686, 0.0, 0.0],
        [3.1724e3, 24.281, 5.6276e-3, -3.0690e-6, 0.0],
        [0.0; 5],
        [0.0; 5],
        [0.0; 5],
    ]),
    photoperiod_h: 12.0,
    daily_light_mol: 27.0,
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
    nutrition: Nutrition { kcal: 180.0, protein_g: 8.8, fat_g: 2.0 },
    self_set: 0.3,
    ripen_days: 40.0,
    bearing_days: 90.0,
    regrows: false,
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
    quantum_scale: 1.0,
    carbon_use: 0.625,
    carbon_use_min: None,
    closure_days: fit([
        [6.5773e5, 0.0, 0.0, 0.0, 0.0],
        [8.5626e3, 0.0, 0.042749, -1.7905e-5, 0.0],
        [0.0, 0.0, 8.8437e-7, 0.0, 0.0],
        [0.0; 5],
        [0.0; 5],
    ]),
    photoperiod_h: 12.0,
    daily_light_mol: 28.0,
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
    nutrition: Nutrition { kcal: 770.0, protein_g: 20.5, fat_g: 0.9 },
    self_set: 1.0,
    ripen_days: 0.0,
    bearing_days: 0.0,
    regrows: false,
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
    quantum_scale: 1.0,
    carbon_use: 0.64,
    carbon_use_min: None,
    closure_days: fit([
        [9.5488e4, 0.0, 0.3419, -1.9076e-4, 0.0],
        [1.0686e3, 15.977, 1.9733e-4, 0.0, 0.0],
        [0.0; 5],
        [0.0; 5],
        [0.0; 5],
    ]),
    photoperiod_h: 20.0,
    daily_light_mol: 115.0,
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
    nutrition: Nutrition { kcal: 3290.0, protein_g: 154.0, fat_g: 19.2 },
    self_set: 1.0,
    ripen_days: 0.0,
    bearing_days: 0.0,
    regrows: false,
};

pub const SOYBEAN: Species = Species {
    name: "soybean",
    cultivar: "Hoyt",
    closure_exponent: 1.5,
    quantum_yield: fit([
        [0.0; 5],
        [0.0, 4.1513e-2, 0.0, -2.1582e-8, 0.0],
        [0.0, 5.1157e-5, 4.0864e-8, -1.0468e-10, 4.8541e-14],
        [0.0, -2.0992e-8, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 3.9259e-21],
    ]),
    quantum_yield_min: Some(0.02),
    quantum_scale: 1.0,
    carbon_use: 0.65,
    carbon_use_min: Some(0.30),
    closure_days: fit([
        [6.7978e6, -4.326e4, 112.63, -0.13637, 6.6918e-5],
        [-4.3658e3, 33.959, 0.0, 0.0, -2.1367e-8],
        [1.5573, 0.0, 0.0, 0.0, 1.5467e-11],
        [0.0, 0.0, -4.911e-9, 0.0, 0.0],
        [0.0; 5],
    ]),
    photoperiod_h: 12.0,
    daily_light_mol: 28.0,
    density: 35.0,
    light_c: 26.0,
    dark_c: 22.0,
    edible_share: 0.95,
    edible_days: 46.0,
    senescence_days: Some(48.0),
    mature_days: 86.0,
    harvest_days: 86.0,
    carbon_fraction: 0.46,
    oxygen_per_carbon: 1.16,
    erect: false,
    cardinal_c: [7.0, 30.0, 40.0],
    height_m: 0.55,
    harvest_index: 0.40,
    edible_water: 0.10,
    edible_rate: 4.54,
    growing_days: 97.0,
    ppf_limit: 1500.0,
    form: Form::Legume,
    edible: Edible::Grain,
    nutrition: Nutrition { kcal: 4460.0, protein_g: 365.0, fat_g: 199.0 },
    self_set: 1.0,
    ripen_days: 0.0,
    bearing_days: 0.0,
    regrows: false,
};

pub const DRY_BEAN: Species = Species {
    name: "dry_bean",
    cultivar: "Meso American Hab. 1",
    closure_exponent: 2.0,
    quantum_yield: fit([
        [0.0; 5],
        [0.0, 4.191e-2, -1.238e-5, 0.0, 0.0],
        [0.0, 5.3852e-5, 0.0, -1.544e-11, 0.0],
        [0.0, -2.1275e-8, 0.0, 6.469e-15, 0.0],
        [0.0; 5],
    ]),
    quantum_yield_min: Some(0.02),
    quantum_scale: 1.0,
    carbon_use: 0.65,
    carbon_use_min: Some(0.50),
    closure_days: fit([
        [2.9041e5, 0.0, 0.0, 0.0, 0.0],
        [1.5594e3, 15.840, 6.1120e-3, 0.0, 0.0],
        [0.0, 0.0, 0.0, -3.7409e-9, 0.0],
        [0.0; 5],
        [0.0, 0.0, 0.0, 0.0, 9.6484e-19],
    ]),
    photoperiod_h: 12.0,
    daily_light_mol: 24.0,
    density: 7.0,
    light_c: 26.0,
    dark_c: 22.0,
    edible_share: 0.97,
    edible_days: 40.0,
    senescence_days: Some(42.0),
    mature_days: 63.0,
    harvest_days: 63.0,
    carbon_fraction: 0.45,
    oxygen_per_carbon: 1.10,
    erect: false,
    cardinal_c: [10.0, 27.0, 37.0],
    height_m: 0.50,
    harvest_index: 0.40,
    edible_water: 0.10,
    edible_rate: 10.0,
    growing_days: 85.0,
    ppf_limit: 1500.0,
    form: Form::Legume,
    edible: Edible::Grain,
    nutrition: Nutrition { kcal: 3330.0, protein_g: 236.0, fat_g: 8.0 },
    self_set: 1.0,
    ripen_days: 0.0,
    bearing_days: 0.0,
    regrows: false,
};

pub const PEANUT: Species = Species {
    name: "peanut",
    cultivar: "Pronto",
    closure_exponent: 2.0,
    quantum_yield: SOYBEAN.quantum_yield,
    quantum_yield_min: Some(0.02),
    quantum_scale: 1.0,
    carbon_use: 0.65,
    carbon_use_min: Some(0.30),
    closure_days: fit([
        [3.7487e6, -1.8840e4, 51.256, -0.05963, 2.5969e-5],
        [2.9200e3, 23.912, 0.0, 5.5180e-6, 0.0],
        [0.0; 5],
        [0.0; 5],
        [9.4008e-8, 0.0, 0.0, 0.0, 0.0],
    ]),
    photoperiod_h: 12.0,
    daily_light_mol: 27.0,
    density: 7.0,
    light_c: 26.0,
    dark_c: 22.0,
    edible_share: 0.49,
    edible_days: 49.0,
    senescence_days: Some(65.0),
    mature_days: 110.0,
    harvest_days: 110.0,
    carbon_fraction: 0.50,
    oxygen_per_carbon: 1.19,
    erect: false,
    cardinal_c: [10.0, 29.0, 40.0],
    height_m: 0.65,
    harvest_index: 0.25,
    edible_water: 0.056,
    edible_rate: 5.63,
    growing_days: 104.0,
    ppf_limit: 1500.0,
    form: Form::Legume,
    edible: Edible::Tuber,
    nutrition: Nutrition { kcal: 5670.0, protein_g: 258.0, fat_g: 492.0 },
    self_set: 1.0,
    ripen_days: 0.0,
    bearing_days: 0.0,
    regrows: false,
};

pub const RICE: Species = Species {
    name: "rice",
    cultivar: "early maturing",
    closure_exponent: 1.5,
    quantum_yield: fit([
        [0.0; 5],
        [0.0, 3.6186e-2, 0.0, -2.6712e-9, 0.0],
        [0.0, 6.1457e-5, -9.1477e-9, 0.0, 0.0],
        [0.0, -2.4322e-8, 3.889e-12, 0.0, 0.0],
        [0.0; 5],
    ]),
    quantum_yield_min: Some(0.01),
    quantum_scale: 1.0,
    carbon_use: 0.64,
    carbon_use_min: None,
    closure_days: fit([
        [6.5914e6, -3.748e3, 0.0, 0.0, 0.0],
        [2.5776e4, 0.0, 0.0, 4.5207e-6, 0.0],
        [0.0, -0.043378, 4.562e-5, -1.4936e-8, 0.0],
        [6.4532e-3, 0.0, 0.0, 0.0, 0.0],
        [0.0; 5],
    ]),
    photoperiod_h: 12.0,
    daily_light_mol: 33.0,
    density: 200.0,
    light_c: 29.0,
    dark_c: 21.0,
    edible_share: 0.98,
    edible_days: 57.0,
    senescence_days: Some(61.0),
    mature_days: 88.0,
    harvest_days: 88.0,
    carbon_fraction: 0.44,
    oxygen_per_carbon: 1.08,
    erect: true,
    cardinal_c: [8.0, 30.0, 42.0],
    height_m: 0.80,
    harvest_index: 0.30,
    edible_water: 0.12,
    edible_rate: 9.07,
    growing_days: 85.0,
    ppf_limit: 2000.0,
    form: Form::Grass,
    edible: Edible::Grain,
    nutrition: Nutrition { kcal: 3700.0, protein_g: 79.0, fat_g: 29.0 },
    self_set: 1.0,
    ripen_days: 0.0,
    bearing_days: 0.0,
    regrows: false,
};

pub const SWEET_POTATO: Species = Species {
    name: "sweet_potato",
    cultivar: "TU-82-155",
    closure_exponent: 1.5,
    quantum_yield: fit([
        [0.0; 5],
        [0.0, 3.9317e-2, -1.3836e-5, 0.0, 0.0],
        [0.0, 5.6741e-5, -6.3397e-9, -1.3464e-11, 0.0],
        [0.0, -2.1797e-8, 0.0, 7.7362e-15, 0.0],
        [0.0; 5],
    ]),
    quantum_yield_min: None,
    quantum_scale: 1.0,
    carbon_use: 0.625,
    carbon_use_min: None,
    closure_days: fit([
        [1.2070e6, 0.0, 0.0, 0.0, 4.0109e-7],
        [4.9484e3, 4.2978, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 2.0193e-12],
        [0.0; 5],
        [0.0; 5],
    ]),
    photoperiod_h: 18.0,
    daily_light_mol: 28.0,
    density: 16.0,
    light_c: 28.0,
    dark_c: 22.0,
    edible_share: 1.0,
    edible_days: 33.0,
    senescence_days: None,
    mature_days: 120.0,
    harvest_days: 120.0,
    carbon_fraction: 0.44,
    oxygen_per_carbon: 1.02,
    erect: false,
    cardinal_c: [12.0, 28.0, 38.0],
    height_m: 0.65,
    harvest_index: 0.60,
    edible_water: 0.71,
    edible_rate: 24.7,
    growing_days: 85.0,
    ppf_limit: 1500.0,
    form: Form::Creeper,
    edible: Edible::Tuber,
    nutrition: Nutrition { kcal: 860.0, protein_g: 15.7, fat_g: 0.5 },
    self_set: 1.0,
    ripen_days: 0.0,
    bearing_days: 0.0,
    regrows: false,
};

// The crops below have no MEC fit. Each borrows the fits of a close MEC
// relative and keeps its own photoperiod, light, density, cycle, height,
// harvest index and yield (BVAD Tables 4-96 and 4-97). The densities are
// commercial and NASA practice, and the cultivars are the ones grown in
// space where there are any (Veggie and APH on the ISS).

pub const SPINACH: Species = Species {
    name: "spinach",
    cultivar: "smooth leaf",
    density: 30.0,
    mature_days: 30.0,
    harvest_days: 30.0,
    cardinal_c: [2.0, 18.0, 30.0],
    harvest_index: 0.90,
    edible_water: 0.91,
    edible_rate: 6.57,
    growing_days: 30.0,
    ppf_limit: 600.0,
    nutrition: Nutrition { kcal: 230.0, protein_g: 28.6, fat_g: 3.9 },
    ..LETTUCE
};

pub const CHARD: Species = Species {
    name: "chard",
    cultivar: "Swiss",
    quantum_scale: 0.69,
    density: 12.0,
    edible_share: 0.70,
    mature_days: 45.0,
    harvest_days: 45.0,
    height_m: 0.45,
    harvest_index: 0.65,
    edible_water: 0.92,
    edible_rate: 7.0,
    growing_days: 45.0,
    ppf_limit: 800.0,
    nutrition: Nutrition { kcal: 190.0, protein_g: 18.0, fat_g: 2.0 },
    regrows: true,
    ..LETTUCE
};

pub const KALE: Species = Species {
    name: "kale",
    cultivar: "Red Russian",
    quantum_scale: 0.47,
    density: 12.0,
    mature_days: 50.0,
    harvest_days: 50.0,
    cardinal_c: [2.0, 20.0, 30.0],
    height_m: 0.40,
    edible_water: 0.89,
    edible_rate: 6.06,
    growing_days: 50.0,
    ppf_limit: 900.0,
    nutrition: Nutrition { kcal: 490.0, protein_g: 42.8, fat_g: 9.3 },
    regrows: true,
    ..LETTUCE
};

pub const PAK_CHOI: Species = Species {
    name: "pak_choi",
    cultivar: "Extra Dwarf",
    quantum_scale: 0.65,
    density: 25.0,
    mature_days: 35.0,
    harvest_days: 35.0,
    height_m: 0.20,
    edible_water: 0.95,
    edible_rate: 6.06,
    growing_days: 35.0,
    ppf_limit: 700.0,
    nutrition: Nutrition { kcal: 130.0, protein_g: 15.0, fat_g: 2.0 },
    ..LETTUCE
};

pub const GREEN_ONION: Species = Species {
    name: "green_onion",
    cultivar: "Kincho",
    quantum_scale: 0.40,
    daily_light_mol: 26.0,
    density: 100.0,
    light_c: 25.0,
    dark_c: 25.0,
    mature_days: 50.0,
    harvest_days: 50.0,
    cardinal_c: [5.0, 24.0, 32.0],
    erect: true,
    edible_water: 0.89,
    edible_rate: 9.0,
    growing_days: 50.0,
    ppf_limit: 900.0,
    form: Form::Allium,
    nutrition: Nutrition { kcal: 320.0, protein_g: 18.3, fat_g: 1.9 },
    regrows: true,
    ..LETTUCE
};

pub const RADISH: Species = Species {
    name: "radish",
    cultivar: "Cherry Belle",
    quantum_scale: 0.50,
    daily_light_mol: 26.0,
    density: 100.0,
    edible_share: 0.70,
    edible_days: 8.0,
    mature_days: 25.0,
    harvest_days: 25.0,
    height_m: 0.20,
    harvest_index: 0.50,
    edible_water: 0.94,
    edible_rate: 5.50,
    growing_days: 25.0,
    ppf_limit: 900.0,
    form: Form::Root,
    edible: Edible::Tuber,
    nutrition: Nutrition { kcal: 160.0, protein_g: 6.8, fat_g: 1.0 },
    ..LETTUCE
};

pub const CARROT: Species = Species {
    name: "carrot",
    cultivar: "Nantes",
    quantum_scale: 0.44,
    daily_light_mol: 28.0,
    density: 100.0,
    light_c: 18.0,
    dark_c: 16.0,
    edible_share: 0.85,
    edible_days: 30.0,
    mature_days: 75.0,
    harvest_days: 75.0,
    cardinal_c: [3.0, 18.0, 30.0],
    height_m: 0.25,
    harvest_index: 0.60,
    edible_water: 0.88,
    edible_rate: 8.98,
    growing_days: 75.0,
    ppf_limit: 1000.0,
    form: Form::Root,
    edible: Edible::Tuber,
    nutrition: Nutrition { kcal: 410.0, protein_g: 9.3, fat_g: 2.4 },
    ..LETTUCE
};

pub const RED_BEET: Species = Species {
    name: "red_beet",
    cultivar: "Detroit Dark Red",
    quantum_scale: 0.63,
    density: 40.0,
    edible_share: 0.80,
    edible_days: 12.0,
    mature_days: 40.0,
    harvest_days: 40.0,
    height_m: 0.45,
    harvest_index: 0.65,
    edible_water: 0.80,
    edible_rate: 6.50,
    growing_days: 40.0,
    ppf_limit: 800.0,
    form: Form::Root,
    edible: Edible::Tuber,
    nutrition: Nutrition { kcal: 430.0, protein_g: 16.1, fat_g: 1.7 },
    ..LETTUCE
};

pub const PEPPER: Species = Species {
    name: "pepper",
    cultivar: "Española Improved",
    density: 8.0,
    edible_days: 45.0,
    senescence_days: Some(60.0),
    mature_days: 85.0,
    harvest_days: 85.0,
    cardinal_c: [10.0, 26.0, 35.0],
    edible_water: 0.93,
    nutrition: Nutrition { kcal: 400.0, protein_g: 18.7, fat_g: 4.4 },
    form: Form::Shrub,
    self_set: 0.8,
    bearing_days: 60.0,
    ..TOMATO
};

pub const STRAWBERRY: Species = Species {
    name: "strawberry",
    cultivar: "Albion",
    daily_light_mol: 22.0,
    density: 10.0,
    light_c: 20.0,
    dark_c: 16.0,
    edible_share: 0.50,
    edible_days: 45.0,
    senescence_days: Some(80.0),
    mature_days: 100.0,
    harvest_days: 100.0,
    cardinal_c: [5.0, 20.0, 30.0],
    height_m: 0.25,
    harvest_index: 0.35,
    edible_water: 0.90,
    edible_rate: 7.79,
    growing_days: 100.0,
    form: Form::Crown,
    nutrition: Nutrition { kcal: 320.0, protein_g: 6.7, fat_g: 3.0 },
    self_set: 0.4,
    ripen_days: 30.0,
    bearing_days: 120.0,
    ..TOMATO
};

pub const PEA: Species = Species {
    name: "pea",
    cultivar: "dwarf field",
    photoperiod_h: 16.0,
    density: 40.0,
    light_c: 20.0,
    dark_c: 16.0,
    edible_share: 0.90,
    mature_days: 75.0,
    harvest_days: 75.0,
    senescence_days: Some(55.0),
    cardinal_c: [4.0, 20.0, 30.0],
    edible_water: 0.12,
    edible_rate: 10.73,
    growing_days: 75.0,
    nutrition: Nutrition { kcal: 3520.0, protein_g: 238.0, fat_g: 12.0 },
    ..DRY_BEAN
};

pub const SNAP_BEAN: Species = Species {
    name: "snap_bean",
    cultivar: "Provider",
    photoperiod_h: 18.0,
    density: 12.0,
    light_c: 28.0,
    dark_c: 24.0,
    edible_share: 0.66,
    edible_days: 35.0,
    senescence_days: Some(60.0),
    mature_days: 85.0,
    harvest_days: 85.0,
    edible_water: 0.92,
    edible_rate: 11.88,
    growing_days: 85.0,
    edible: Edible::Fruit,
    nutrition: Nutrition { kcal: 310.0, protein_g: 18.3, fat_g: 2.2 },
    ripen_days: 10.0,
    bearing_days: 20.0,
    ..DRY_BEAN
};

/// Every crop, by name.
pub const ALL: [&Species; 21] = [
    &LETTUCE,
    &TOMATO,
    &POTATO,
    &WHEAT,
    &SOYBEAN,
    &DRY_BEAN,
    &PEANUT,
    &RICE,
    &SWEET_POTATO,
    &SPINACH,
    &CHARD,
    &KALE,
    &PAK_CHOI,
    &GREEN_ONION,
    &RADISH,
    &CARROT,
    &RED_BEET,
    &PEPPER,
    &STRAWBERRY,
    &PEA,
    &SNAP_BEAN,
];

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
            let days = species.closure_days.at(species.nominal_ppf(), 1200.0);
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
    fn every_crop_is_found_by_its_own_name_and_feeds_like_its_food() {
        for species in ALL {
            assert_eq!(by_name(species.name), Some(species), "{} is found by its name", species.name);
            let n = species.nutrition;
            // Protein and fat carry no more energy than the food has.
            assert!((100.0..6000.0).contains(&n.kcal), "{}: {} kcal/kg", species.name, n.kcal);
            assert!(4.0 * n.protein_g + 9.0 * n.fat_g <= 1.05 * n.kcal, "{}: {n:?}", species.name);
        }
        // Dry seed feeds far more per kilogram than fresh greens.
        assert!(SOYBEAN.nutrition.kcal > 10.0 * LETTUCE.nutrition.kcal);
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
