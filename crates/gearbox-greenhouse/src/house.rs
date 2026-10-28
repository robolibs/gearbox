//! A greenhouse: rooms of air, trays of plants under lamps, the tanks the
//! trays drink from, one power bus, and the faults that break them.
//!
//! [`House::step`] runs them together in substeps of a few minutes. Lamps ask
//! the bus for power after life support has had its share; trays read their
//! room's air, their lamps and their tank; plants grow and breathe into the
//! room; the room's equipment answers; tanks give water and take condensate.
//! Faults switch pieces off until they are repaired.

use crate::air::{Air, Climate, Outside, Upkeep, Working};
use crate::lamp::{Channels, Lamp};
use crate::plant::{Exchange, Plant, Stage, Surroundings};
use crate::power::{Power, settled};
use crate::species::{Edible, Species};
use crate::water::{Medium, Tank};

/// Longest step the house takes in one go, seconds.
const SUBSTEP_S: f64 = 300.0;
/// Photosynthetic photons in a joule of sunlight: 45 % of it is PAR, at
/// 4.57 µmol/J (McCree 1972).
pub const PAR_PER_W: f64 = 0.45 * 4.57;
/// How long the daytime mean of the daylight takes to follow a change, seconds.
const DAYLIGHT_MEMORY_S: f64 = 2.0 * 86_400.0;
/// Share of a lamp's power its cooling loop carries away while the room's
/// cooling works; the rest heats the air (EDEN ISS's lamps were water-cooled).
const LAMP_COOLED: f64 = 0.6;
/// Every running pump and controller, W per room.
const CONTROLS_W: f64 = 120.0;

/// One room and what keeps its air.
#[derive(Clone, Debug, PartialEq)]
pub struct Room {
    pub name: String,
    pub air: Air,
    pub climate: Climate,
    /// Equipment that has failed.
    pub fans_failed: bool,
    pub hvac_failed: bool,
    pub co2_failed: bool,
    pub dehumidifier_failed: bool,
    pub makeup_failed: bool,
    /// A breach to the outside: share of the gas lost per second at full pressure difference.
    pub leak_per_s: f64,
    /// Share of the daylight its glazing lets in to the crops, 0 for a closed room.
    pub glazing: f64,
    /// Share of that daylight the dust on the glazing blocks, 0 to 1.
    pub glazing_dust: f64,
    /// What the equipment drew over the last step, W.
    pub draw_w: f64,
    pub last: Upkeep,
}

impl Room {
    pub fn new(name: &str, volume_m3: f64, climate: Climate) -> Self {
        Self {
            name: name.to_string(),
            air: Air::settled(volume_m3, &climate),
            climate,
            fans_failed: false,
            hvac_failed: false,
            co2_failed: false,
            dehumidifier_failed: false,
            makeup_failed: false,
            leak_per_s: 0.0,
            glazing: 0.0,
            glazing_dust: 0.0,
            draw_w: CONTROLS_W + climate.fan_w,
            last: Upkeep::default(),
        }
    }

    fn working(&self, powered: bool) -> Working {
        Working {
            hvac: powered && !self.hvac_failed,
            fans: powered && !self.fans_failed,
            co2_feed: powered && !self.co2_failed,
            dehumidifier: powered && !self.dehumidifier_failed,
            makeup: powered && !self.makeup_failed,
            leak_per_s: self.leak_per_s,
        }
    }
}

/// One tray of plants.
#[derive(Clone, Debug, PartialEq)]
pub struct Tray {
    pub name: String,
    pub room: usize,
    pub tank: usize,
    pub lamps: Vec<usize>,
    pub area_m2: f64,
    pub medium: Medium,
    /// Places for plants, in the tray's own order.
    pub slots: Vec<Option<Plant>>,
    /// Plants each place stands for: a clump of wheat is many plants grown as one.
    pub per_slot: f64,
}

impl Tray {
    /// A tray of `area_m2` holding as many plants of `species` as its fitted spacing allows.
    pub fn slots_for(area_m2: f64, species: &Species) -> usize {
        (area_m2 * species.density).round().max(1.0) as usize
    }
}

/// A plant by its tray and place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlantId {
    pub tray: usize,
    pub slot: usize,
}

/// Things that break.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Fault {
    /// The whole bus is down.
    PowerCut,
    Reactor,
    Fans(usize),
    Hvac(usize),
    Co2Feed(usize),
    /// A breach in a room's shell.
    Leak(usize),
    Lamp(usize),
    Pump(usize),
    WaterLeak(usize),
}

/// Something that happened in a step or by an action.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Sown(PlantId),
    Sprouted(PlantId),
    Staged(PlantId, Stage),
    Died(PlantId),
    Harvested(PlantId, Harvest),
    Pollinated(PlantId),
    Lifted(PlantId),
    Placed(PlantId),
    Faulted(Fault),
    Repaired(Fault),
}

/// Food energy a crew member eats a sol, kcal (BVAD: about 3000 for an
/// active crew member).
pub const CREW_KCAL: f64 = 3000.0;

/// What the farm has taken in and whom it feeds: food ready to eat by crop,
/// grain and dry seed waiting to be threshed and milled, the inedible biomass
/// waiting for resource recovery, and the crew eating from it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Store {
    /// Food held ready to eat, kg, by crop: produce washed and packed, grain milled.
    pub produce: Vec<(&'static Species, f64)>,
    /// Grain and dry seed as harvested, kg, waiting for the processing line.
    pub raw: Vec<(&'static Species, f64)>,
    /// What the processing line threshes and mills a sol, kg; none without one.
    pub processing_kg: f64,
    /// Inedible biomass taken off with harvests and clearings, and food
    /// spoiled, kg dry.
    pub biomass_kg: f64,
    /// Fresh food lost to spoiling since the house started, kg.
    pub spoiled_kg: f64,
    pub crew: u32,
    /// Food energy eaten from the store and needed since the house started, kcal.
    pub eaten_kcal: f64,
    pub needed_kcal: f64,
}

/// Share of a crop's fresh food lost a sol in cold store at 2 to 4 °C:
/// greens keep about two weeks, fruit three, cured roots and tubers months.
fn spoiling(species: &Species) -> f64 {
    match species.edible {
        Edible::Shoot => 0.05,
        Edible::Fruit => 0.03,
        Edible::Tuber => 0.005,
        Edible::Grain => 0.0,
    }
}

fn put(stock: &mut Vec<(&'static Species, f64)>, species: &'static Species, kg: f64) {
    match stock.iter_mut().find(|(s, _)| std::ptr::eq(*s, species)) {
        Some((_, held)) => *held += kg,
        None => stock.push((species, kg)),
    }
}

impl Store {
    /// Takes in a harvest: grain and dry seed to wait for processing, the
    /// rest ready to eat.
    pub fn add(&mut self, species: &'static Species, kg: f64) {
        let stock = if species.edible == Edible::Grain { &mut self.raw } else { &mut self.produce };
        put(stock, species, kg);
    }

    /// Grain and seed waiting for processing, kg.
    pub fn raw_kg(&self) -> f64 {
        self.raw.iter().map(|(_, kg)| kg).sum()
    }

    /// The store over `sols`: the processing line works through the grain,
    /// fresh food spoils, and the crew eats.
    pub fn step(&mut self, sols: f64) {
        self.mill(sols);
        self.spoil(sols);
        self.feed(sols);
    }

    /// Threshes and mills up to the line's capacity over `sols`, first come first.
    pub fn mill(&mut self, sols: f64) {
        let mut left = self.processing_kg * sols;
        for (species, kg) in &mut self.raw {
            let done = left.min(*kg);
            *kg -= done;
            left -= done;
            put(&mut self.produce, species, done);
        }
        self.raw.retain(|(_, kg)| *kg > 1e-9);
    }

    /// Fresh food lost over `sols`; what spoils goes to the recovery heap.
    pub fn spoil(&mut self, sols: f64) {
        for (species, kg) in &mut self.produce {
            let lost = *kg * (1.0 - (-spoiling(species) * sols).exp());
            *kg -= lost;
            self.spoiled_kg += lost;
            self.biomass_kg += lost * (1.0 - species.edible_water);
        }
    }

    /// Produce held, kg.
    pub fn kg(&self) -> f64 {
        self.produce.iter().map(|(_, kg)| kg).sum()
    }

    /// Food energy held, kcal.
    pub fn kcal(&self) -> f64 {
        self.produce.iter().map(|(s, kg)| kg * s.nutrition.kcal).sum()
    }

    /// Sols the store feeds the crew.
    pub fn sols(&self) -> f64 {
        if self.crew == 0 { 0.0 } else { self.kcal() / (self.crew as f64 * CREW_KCAL) }
    }

    /// Share of the crew's food energy the farm has given since it started.
    pub fn closure(&self) -> f64 {
        if self.needed_kcal > 0.0 { self.eaten_kcal / self.needed_kcal } else { 0.0 }
    }

    /// Feeds the crew for `sols`, eating what keeps least first: greens, then
    /// fruit, then roots and tubers, then grain.
    pub fn feed(&mut self, sols: f64) {
        let mut want = self.crew as f64 * CREW_KCAL * sols;
        self.needed_kcal += want;
        let keeps = |s: &Species| match s.edible {
            Edible::Shoot => 0,
            Edible::Fruit => 1,
            Edible::Tuber => 2,
            Edible::Grain => 3,
        };
        self.produce.sort_by_key(|(s, _)| keeps(s));
        for (species, kg) in &mut self.produce {
            let eaten = (want / species.nutrition.kcal).min(*kg);
            *kg -= eaten;
            want -= eaten * species.nutrition.kcal;
            self.eaten_kcal += eaten * species.nutrition.kcal;
        }
        self.produce.retain(|(_, kg)| *kg > 1e-9);
    }
}

/// What a harvest took in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Harvest {
    pub species: &'static Species,
    pub fresh_kg: f64,
    pub dry_g: f64,
}

/// Why an action could not be done.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    NoSuchTray,
    NoSuchPlace,
    Occupied,
    Empty,
    NotRipe,
    NotRegolith,
    NotGlazed,
    NotFlowering,
    TooOld,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoSuchTray => "no such tray",
            Self::NoSuchPlace => "the tray has no such place",
            Self::Occupied => "that place is taken",
            Self::Empty => "nothing grows there",
            Self::NotRipe => "not ripe yet",
            Self::NotRegolith => "the tray holds no regolith",
            Self::NotGlazed => "the room has no glazing",
            Self::NotFlowering => "it is not in flower",
            Self::TooOld => "it is too far grown to move",
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct House {
    pub rooms: Vec<Room>,
    pub trays: Vec<Tray>,
    pub lamps: Vec<Lamp>,
    pub tanks: Vec<Tank>,
    pub power: Power,
    pub faults: Vec<Fault>,
    pub store: Store,
    /// Length of the lamps' day, hours: the planet's solar day.
    pub cycle_hours: f64,
    /// Share of the lamps' demand the bus met over the last step.
    pub lamp_power: f64,
    /// Photosynthetic daylight on open ground now, and its mean while the sun
    /// is up, µmol/m²/s.
    pub daylight_ppfd: f64,
    pub daylight_mean: f64,
}

impl House {
    pub fn new(cycle_hours: f64) -> Self {
        Self { cycle_hours, lamp_power: 1.0, ..Default::default() }
    }

    /// Whether the sun is up at a local hour, on a day of `cycle_hours`.
    pub fn sunlit(&self, hour: f64) -> bool {
        let day = self.cycle_hours.max(1.0);
        (day * 0.25..day * 0.75).contains(&hour.rem_euclid(day))
    }

    pub fn plant(&self, id: PlantId) -> Option<&Plant> {
        self.trays.get(id.tray)?.slots.get(id.slot)?.as_ref()
    }

    /// Every plant with its place.
    pub fn plants(&self) -> impl Iterator<Item = (PlantId, &Plant)> {
        self.trays.iter().enumerate().flat_map(|(tray, t)| {
            t.slots.iter().enumerate().filter_map(move |(slot, p)| p.as_ref().map(|p| (PlantId { tray, slot }, p)))
        })
    }

    /// Photon flux on a tray now, and while lit, µmol/m²/s: its lamps', and
    /// the daylight its room's glazing lets in past the dust on it.
    pub fn tray_light(&self, tray: usize, hour: f64) -> (f64, f64) {
        let Some(t) = self.trays.get(tray) else { return (0.0, 0.0) };
        let glazing = self.rooms.get(t.room).map_or(0.0, |r| r.glazing * (1.0 - r.glazing_dust));
        let now: f64 = t.lamps.iter().map(|&l| self.lamps[l].ppfd(hour, self.cycle_hours, self.lamp_power)).sum();
        let lit: f64 = t.lamps.iter().map(|&l| self.lamps[l].lit_ppfd()).sum();
        (now + glazing * self.daylight_ppfd, lit + glazing * self.daylight_mean)
    }

    /// Hours a tray is lit each day: its lamps' photoperiod, or the day's
    /// sunlit half under glass, whichever is longer.
    pub fn lit_hours(&self, tray: usize) -> f64 {
        let Some(t) = self.trays.get(tray) else { return 0.0 };
        let lamps = t.lamps.iter().filter(|&&l| self.lamps[l].lit_ppfd() > 0.0).map(|&l| self.lamps[l].lit_hours).fold(0.0, f64::max);
        let glazed = self.rooms.get(t.room).is_some_and(|r| r.glazing > 0.0);
        if glazed { lamps.max(self.cycle_hours * 0.5) } else { lamps }
    }

    /// The surroundings a tray's plants stand in now.
    pub fn surroundings(&self, tray: usize, hour: f64) -> Option<Surroundings> {
        let t = self.trays.get(tray)?;
        let room = &self.rooms[t.room];
        let tank = &self.tanks[t.tank];
        let (ppf, lit_ppf) = self.tray_light(tray, hour);
        let lit_hours = self.lit_hours(tray);
        let powered = !self.power.cut;
        let airflow = powered && !room.fans_failed;
        // Still air keeps the water leaves give off around them: without the
        // canopy fans the leaves stand some 20 points wetter than the room.
        let humidity = if airflow { room.air.humidity } else { (room.air.humidity + 0.2).min(1.0) };
        Some(Surroundings {
            ppf,
            lit_ppf,
            lit_hours,
            cycle_hours: self.cycle_hours,
            temperature_c: room.air.temperature_c,
            humidity,
            co2_ppm: room.air.co2_ppm,
            o2_kpa: room.air.o2_kpa,
            pressure_kpa: room.air.pressure_kpa,
            airflow,
            water: tank.water() * if powered { 1.0 } else { 0.0 },
            nutrients: t.medium.fertility(tank.nutrients()),
            perchlorate: t.medium.perchlorate(),
        })
    }

    /// Runs the house `seconds` on from local hour `hour`, with `outside`
    /// beyond the walls and `solar_w_m2` on the panels and the ground.
    pub fn step(&mut self, seconds: f64, hour: f64, outside: &Outside, solar_w_m2: f64) -> Vec<Event> {
        self.daylight_ppfd = PAR_PER_W * solar_w_m2.max(0.0);
        if self.daylight_ppfd > 1.0 {
            let follow = (seconds / DAYLIGHT_MEMORY_S).min(1.0);
            self.daylight_mean += (self.daylight_ppfd - self.daylight_mean) * follow;
        }
        let mut events = Vec::new();
        let mut done = 0.0;
        while done < seconds {
            let dt = (seconds - done).min(SUBSTEP_S);
            let at = (hour + done / 3600.0).rem_euclid(self.cycle_hours.max(1.0));
            self.substep(dt, at, outside, solar_w_m2, &mut events);
            done += dt;
        }
        let sols = seconds / (self.cycle_hours.max(1.0) * 3600.0);
        self.power.settle_dust(sols);
        self.store.step(sols);
        for room in self.rooms.iter_mut().filter(|r| r.glazing > 0.0) {
            room.glazing_dust = settled(room.glazing_dust, sols);
        }
        events
    }

    /// Brushes the dust off a room's glazing.
    pub fn clean_glazing(&mut self, room: usize) -> Result<(), Refusal> {
        let room = self.rooms.get_mut(room).filter(|r| r.glazing > 0.0).ok_or(Refusal::NotGlazed)?;
        room.glazing_dust = 0.0;
        Ok(())
    }

    fn substep(&mut self, dt: f64, hour: f64, outside: &Outside, solar_w_m2: f64, events: &mut Vec<Event>) {
        let lamps_w: f64 = self.lamps.iter().map(|l| l.power_w(l.ppfd(hour, self.cycle_hours, 1.0))).sum();
        let essential_w: f64 = self.rooms.iter().map(|r| r.draw_w).sum();
        let (lamp_power, essential) = self.power.draw(dt, solar_w_m2, essential_w, lamps_w);
        self.lamp_power = lamp_power;
        let powered = essential >= 0.5;

        let mut exchange = vec![Exchange::default(); self.rooms.len()];
        let mut drawn = vec![0.0; self.tanks.len()];
        let mut grown = vec![0.0; self.tanks.len()];
        for tray in 0..self.trays.len() {
            let Some(around) = self.surroundings(tray, hour) else { continue };
            let (room, tank, many) = (self.trays[tray].room, self.trays[tray].tank, self.trays[tray].per_slot);
            for slot in 0..self.trays[tray].slots.len() {
                let Some(plant) = self.trays[tray].slots[slot].as_mut() else { continue };
                let (stage, mass) = (plant.stage(), plant.biomass_g);
                let breath = plant.grow(dt, &around);
                let (now, id) = (plant.stage(), PlantId { tray, slot });
                grown[tank] += (plant.biomass_g - mass).max(0.0) * many;
                drawn[tank] += breath.water_kg * many;
                exchange[room] += Exchange { co2_mol: breath.co2_mol * many, o2_mol: breath.o2_mol * many, water_kg: breath.water_kg * many };
                if now != stage {
                    events.push(match (stage, now) {
                        (_, Stage::Dead) => Event::Died(id),
                        (Stage::Seed, _) => Event::Sprouted(id),
                        _ => Event::Staged(id, now),
                    });
                }
            }
        }

        let mut condensate = vec![0.0; self.tanks.len()];
        for (index, room) in self.rooms.iter_mut().enumerate() {
            let lamp_heat: f64 = self
                .lamps
                .iter()
                .enumerate()
                .filter(|(l, _)| self.trays.iter().any(|t| t.room == index && t.lamps.contains(l)))
                .map(|(_, lamp)| lamp.power_w(lamp.ppfd(hour, self.cycle_hours, lamp_power)))
                .sum();
            let working = room.working(powered);
            let heat = lamp_heat * if working.hvac { 1.0 - LAMP_COOLED } else { 1.0 };
            let e = exchange[index];
            room.last = room.air.step(dt, &room.climate, working, e.co2_mol, e.o2_mol, e.water_kg, heat, outside);
            room.draw_w = room.last.energy_j / dt + CONTROLS_W;
            if let Some(tank) = self.trays.iter().find(|t| t.room == index).map(|t| t.tank) {
                condensate[tank] += room.last.condensate_kg;
            }
        }
        for (index, tank) in self.tanks.iter_mut().enumerate() {
            tank.step(dt, drawn[index], condensate[index], grown[index]);
        }
    }

    /// Sows `species` in one place of a tray.
    pub fn sow(&mut self, id: PlantId, species: &'static Species) -> Result<Event, Refusal> {
        let many = self.trays.get(id.tray).ok_or(Refusal::NoSuchTray)?.per_slot;
        let slot = self.slot(id)?;
        if slot.as_ref().is_some_and(|p| p.alive) {
            return Err(Refusal::Occupied);
        }
        // A dead plant still standing there goes to the recovery heap.
        let dead_g = slot.replace(Plant::sown(species)).map_or(0.0, |p| p.biomass_g);
        self.store.biomass_kg += dead_g * many / 1000.0;
        Ok(Event::Sown(id))
    }

    fn slot(&mut self, id: PlantId) -> Result<&mut Option<Plant>, Refusal> {
        let tray = self.trays.get_mut(id.tray).ok_or(Refusal::NoSuchTray)?;
        tray.slots.get_mut(id.slot).ok_or(Refusal::NoSuchPlace)
    }

    /// Takes in what a plant has made into the store: the ripe fruit of a
    /// fruiting crop, which bears on until it is spent; the outer leaves of a
    /// cut-and-come-again crop until its last cutting; otherwise the whole
    /// plant, its inedible part to the biomass waiting for recovery.
    pub fn harvest(&mut self, id: PlantId) -> Result<Event, Refusal> {
        let many = self.trays.get(id.tray).ok_or(Refusal::NoSuchTray)?.per_slot;
        let slot = self.slot(id)?;
        let plant = slot.as_mut().filter(|p| p.alive).ok_or(Refusal::Empty)?;
        let stage = plant.stage();
        if !matches!(stage, Stage::Ripe | Stage::Spent) {
            return Err(Refusal::NotRipe);
        }
        let species = plant.species;
        let (taken_g, whole) = if plant.fruiting() && stage == Stage::Ripe {
            let ripe = plant.edible_g;
            plant.biomass_g -= ripe;
            plant.edible_g = 0.0;
            (ripe, false)
        } else if let Some(cut) = plant.cut() {
            (cut, false)
        } else {
            (plant.edible_g, true)
        };
        let rest_g = if whole { slot.take().map_or(0.0, |p| p.biomass_g - taken_g) } else { 0.0 };
        let harvest = Harvest { species, fresh_kg: species.fresh_kg(taken_g) * many, dry_g: taken_g * many };
        self.store.add(species, harvest.fresh_kg);
        self.store.biomass_kg += rest_g.max(0.0) * many / 1000.0;
        Ok(Event::Harvested(id, harvest))
    }

    /// Lifts whatever is in a place, dead or alive, its biomass to the recovery heap.
    pub fn clear(&mut self, id: PlantId) -> Result<(), Refusal> {
        let many = self.trays.get(id.tray).ok_or(Refusal::NoSuchTray)?.per_slot;
        let plant = self.slot(id)?.take().ok_or(Refusal::Empty)?;
        self.store.biomass_kg += plant.biomass_g * many / 1000.0;
        Ok(())
    }

    /// Pollinates a flowering plant's open flowers.
    pub fn pollinate(&mut self, id: PlantId) -> Result<Event, Refusal> {
        let plant = self.slot(id)?.as_mut().filter(|p| p.alive).ok_or(Refusal::Empty)?;
        if !plant.flowering() {
            return Err(Refusal::NotFlowering);
        }
        plant.pollinate();
        Ok(Event::Pollinated(id))
    }

    /// Lifts a young plant out of its place to be set in another.
    pub fn lift(&mut self, id: PlantId) -> Result<(Plant, Event), Refusal> {
        let slot = self.slot(id)?;
        let plant = slot.as_ref().ok_or(Refusal::Empty)?;
        if !plant.transplantable() {
            return Err(Refusal::TooOld);
        }
        Ok((slot.take().expect("checked above"), Event::Lifted(id)))
    }

    /// Sets a lifted plant in an empty place.
    pub fn place(&mut self, id: PlantId, plant: Plant) -> Result<Event, Refusal> {
        let slot = self.slot(id)?;
        if slot.is_some() {
            return Err(Refusal::Occupied);
        }
        *slot = Some(plant);
        Ok(Event::Placed(id))
    }

    /// Washes the perchlorate out of a regolith tray, with the water it costs from its tank.
    pub fn wash(&mut self, tray: usize) -> Result<(), Refusal> {
        let t = self.trays.get_mut(tray).ok_or(Refusal::NoSuchTray)?;
        let Medium::Regolith { perchlorate, .. } = &mut t.medium else { return Err(Refusal::NotRegolith) };
        *perchlorate = 0.0;
        let tank = &mut self.tanks[t.tank];
        tank.water_l = (tank.water_l - 20.0 * t.area_m2).max(0.0);
        Ok(())
    }

    /// Works organic matter, as a share of the bed, into a regolith tray.
    pub fn amend(&mut self, tray: usize, share: f64) -> Result<(), Refusal> {
        let t = self.trays.get_mut(tray).ok_or(Refusal::NoSuchTray)?;
        let Medium::Regolith { organic, .. } = &mut t.medium else { return Err(Refusal::NotRegolith) };
        *organic = (*organic + share.max(0.0)).min(0.5);
        Ok(())
    }

    /// Sets the spectrum and output of the lamps over a tray.
    pub fn set_lamps(&mut self, tray: usize, channels: Option<Channels>, dimming: Option<f64>) -> Result<(), Refusal> {
        let t = self.trays.get(tray).ok_or(Refusal::NoSuchTray)?;
        for &l in &t.lamps {
            if let Some(channels) = channels {
                self.lamps[l].channels = channels;
            }
            if let Some(dimming) = dimming {
                self.lamps[l].dimming = dimming.clamp(0.0, 1.0);
            }
        }
        Ok(())
    }

    /// Breaks something.
    pub fn raise(&mut self, fault: Fault) -> Option<Event> {
        if self.faults.contains(&fault) || !self.switch(fault, true) {
            return None;
        }
        self.faults.push(fault);
        Some(Event::Faulted(fault))
    }

    /// Mends something broken.
    pub fn repair(&mut self, fault: Fault) -> Option<Event> {
        let at = self.faults.iter().position(|f| *f == fault)?;
        self.switch(fault, false);
        self.faults.remove(at);
        Some(Event::Repaired(fault))
    }

    fn switch(&mut self, fault: Fault, broken: bool) -> bool {
        match fault {
            Fault::PowerCut => self.power.cut = broken,
            Fault::Reactor => self.power.reactor_down = broken,
            Fault::Fans(r) | Fault::Hvac(r) | Fault::Co2Feed(r) | Fault::Leak(r) => {
                let Some(room) = self.rooms.get_mut(r) else { return false };
                match fault {
                    Fault::Fans(_) => room.fans_failed = broken,
                    Fault::Hvac(_) => room.hvac_failed = broken,
                    Fault::Co2Feed(_) => room.co2_failed = broken,
                    _ => room.leak_per_s = if broken { 2e-5 } else { 0.0 },
                }
            }
            Fault::Lamp(l) => {
                let Some(lamp) = self.lamps.get_mut(l) else { return false };
                lamp.failed = broken;
            }
            Fault::Pump(t) | Fault::WaterLeak(t) => {
                let Some(tank) = self.tanks.get_mut(t) else { return false };
                if matches!(fault, Fault::Pump(_)) {
                    tank.pump_failed = broken;
                } else {
                    tank.leak_l_h = if broken { 25.0 } else { 0.0 };
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::species::{KALE, LETTUCE, POTATO, TOMATO, WHEAT};

    /// A room with one tank and `trays` lettuce trays of 0.24 m², each under one lamp.
    fn chamber(trays: usize) -> House {
        let mut house = House::new(24.0);
        house.rooms.push(Room::new("grow", 31.5, Climate::growth_room()));
        house.tanks.push(Tank::new("loop", 400.0, 2.5));
        for i in 0..trays {
            house.lamps.push(Lamp::new(&format!("lamp{i}"), 300.0, 0.24, 16.0));
            house.trays.push(Tray {
                name: format!("tray{i}"),
                room: 0,
                tank: 0,
                lamps: vec![i],
                area_m2: 0.24,
                medium: Medium::Hydroponic,
                slots: vec![None; Tray::slots_for(0.24, &LETTUCE)],
                per_slot: 1.0,
            });
        }
        house
    }

    fn run_days(house: &mut House, days: usize) -> Vec<Event> {
        let mut events = Vec::new();
        for day in 0..days {
            for hour in 0..24 {
                let _ = day;
                events.extend(house.step(3600.0, hour as f64, &Outside::mars(hour as f64), 0.0));
            }
        }
        events
    }

    fn sow_all(house: &mut House, species: &'static Species) {
        for tray in 0..house.trays.len() {
            for slot in 0..house.trays[tray].slots.len() {
                house.sow(PlantId { tray, slot }, species).unwrap();
            }
        }
    }

    #[test]
    fn lettuce_grows_from_seed_to_harvest_in_a_month() {
        let mut house = chamber(4);
        sow_all(&mut house, &LETTUCE);
        let events = run_days(&mut house, 31);
        let ripe = events.iter().filter(|e| matches!(e, Event::Staged(_, Stage::Ripe))).count();
        assert_eq!(ripe, house.plants().count(), "every head ripens");
        let id = PlantId { tray: 0, slot: 0 };
        let Ok(Event::Harvested(_, harvest)) = house.harvest(id) else { panic!("harvest") };
        assert!((0.08..0.35).contains(&harvest.fresh_kg), "a head of {} kg", harvest.fresh_kg);
        assert!(house.plant(id).is_none());
        assert!(house.rooms[0].air.temperature_c > 18.0 && house.rooms[0].air.temperature_c < 24.0);
    }

    #[test]
    fn under_glass_lettuce_grows_by_daylight_alone() {
        let grown = |glazing: f64| {
            let mut house = chamber(1);
            house.lamps[0].ppfd_max = 0.0;
            house.rooms[0].glazing = glazing;
            house.daylight_mean = 700.0;
            sow_all(&mut house, &LETTUCE);
            for _ in 0..20 {
                for hour in 0..24 {
                    // A clear Martian day: the sun up from six to six, 450 W/m² at noon.
                    let sun = 450.0 * (std::f64::consts::PI * (hour as f64 - 6.0) / 12.0).sin().max(0.0);
                    house.step(3600.0, hour as f64, &Outside::mars(hour as f64), sun);
                }
            }
            house.plants().map(|(_, p)| p.biomass_g).sum::<f64>()
        };
        let (glass, closed) = (grown(0.7), grown(0.0));
        assert!(glass > 5.0 && closed < 0.5, "under glass {glass} g, in a closed room {closed} g");
        assert!(House { cycle_hours: 24.0, ..House::default() }.sunlit(12.0) && !House { cycle_hours: 24.0, ..House::default() }.sunlit(2.0));
    }

    #[test]
    fn dust_on_the_glazing_dims_the_daylight_until_it_is_brushed_off() {
        let mut house = chamber(1);
        house.lamps[0].ppfd_max = 0.0;
        house.rooms.push(Room::new("closed", 30.0, Climate::growth_room()));
        house.rooms[0].glazing = 0.7;
        house.daylight_mean = 700.0;
        let clean = house.tray_light(0, 12.0).1;
        house.step(100.0 * 24.0 * 3600.0, 12.0, &Outside::mars(12.0), 0.0);
        let dusty = house.tray_light(0, 12.0).1;
        assert!((house.rooms[0].glazing_dust - 0.181).abs() < 0.002, "{}", house.rooms[0].glazing_dust);
        assert_eq!(house.rooms[1].glazing_dust, 0.0, "a closed room gathers none");
        assert!((dusty / clean - (1.0 - house.rooms[0].glazing_dust)).abs() < 1e-9, "{dusty} against {clean}");
        assert_eq!(house.clean_glazing(1), Err(Refusal::NotGlazed));
        house.clean_glazing(0).unwrap();
        assert_eq!(house.tray_light(0, 12.0).1, clean);
    }

    #[test]
    fn a_power_cut_darkens_the_lamps_and_lets_carbon_dioxide_climb() {
        let mut house = chamber(30);
        sow_all(&mut house, &LETTUCE);
        run_days(&mut house, 20);
        let before = house.rooms[0].air.co2_ppm;
        house.raise(Fault::PowerCut).unwrap();
        let hour = 10.0;
        house.step(3.0 * 3600.0, hour, &Outside::mars(hour), 0.0);
        assert_eq!(house.tray_light(0, hour).0, 0.0);
        let rise = (house.rooms[0].air.co2_ppm - before) / 3.0;
        assert!(rise > 20.0, "CO2 rose {rise} ppm/h in the dark");
        let cooled = house.rooms[0].air.temperature_c;
        assert!(cooled < house.rooms[0].climate.temperature_c - 1.0, "three hours unheated: {cooled} °C");
        assert!(house.repair(Fault::PowerCut).is_some());
        house.step(3600.0, 13.0, &Outside::mars(13.0), 0.0);
        assert!(house.tray_light(0, 13.0).0 > 0.0);
    }

    /// Two tomatoes in a chamber at the crop's temperature.
    fn tomatoes() -> House {
        let mut house = chamber(1);
        house.rooms[0].climate.temperature_c = TOMATO.light_c;
        house.lamps[0].ppfd_max = 625.0;
        house.lamps[0].lit_hours = 12.0;
        house.trays[0].slots = vec![None; 2];
        sow_all(&mut house, &TOMATO);
        house
    }

    /// Tends the tomatoes for `days`: the tank cleaned monthly, the plants in
    /// `buzzed` pollinated daily, the first one picked whenever it has ripe fruit.
    fn tend_days(house: &mut House, days: usize, buzzed: &[usize], picked: &mut f64) {
        for day in 0..days {
            if day % 30 == 29 {
                house.tanks[0].clean();
            }
            for &slot in buzzed {
                let _ = house.pollinate(PlantId { tray: 0, slot });
            }
            if let Ok(Event::Harvested(_, h)) = house.harvest(PlantId { tray: 0, slot: 0 }) {
                *picked += h.fresh_kg;
            }
            run_days(house, 1);
        }
    }

    #[test]
    fn a_buzzed_tomato_bears_again_and_again_until_it_is_spent() {
        let mut house = tomatoes();
        let id = PlantId { tray: 0, slot: 0 };
        assert_eq!(house.harvest(id), Err(Refusal::NotRipe));
        let mut picked = 0.0;
        tend_days(&mut house, 100, &[0, 1], &mut picked);
        let first = picked;
        assert!(first > 0.2, "{first} kg by day 100");
        let plant = house.plant(id).expect("the vine stays after picking");
        assert!(plant.alive && plant.edible_g < plant.pick_g(), "only ripe fruit left the vine");
        tend_days(&mut house, 60, &[0, 1], &mut picked);
        assert!(picked > first + 0.2, "it bore on: {first} then {picked} kg");
        // Past bearing it is spent, and taking it in lifts it whole.
        tend_days(&mut house, 120, &[0, 1], &mut picked);
        assert_eq!(house.plant(id).map(Plant::stage), None, "the spent vine was taken in");
        assert!(house.store.biomass_kg > 0.05, "its vine went to recovery: {} kg", house.store.biomass_kg);
        assert!(house.store.kg() > 0.0);
    }

    #[test]
    fn an_unbuzzed_tomato_sets_a_fraction_of_the_fruit() {
        let mut house = tomatoes();
        for day in 0..90 {
            let _ = house.pollinate(PlantId { tray: 0, slot: 0 });
            if day % 30 == 29 {
                house.tanks[0].clean();
            }
            run_days(&mut house, 1);
        }
        let fruit = |slot| house.plant(PlantId { tray: 0, slot }).map_or(0.0, |p| p.edible_g + p.green_g);
        let share = fruit(1) / fruit(0);
        assert!((0.2..0.55).contains(&share), "unbuzzed fruit {share:.2} of buzzed");
    }

    #[test]
    fn kale_is_cut_and_comes_again_before_it_is_lifted() {
        let mut house = chamber(1);
        house.trays[0].slots = vec![None; 1];
        sow_all(&mut house, &KALE);
        let id = PlantId { tray: 0, slot: 0 };
        let mut cuts = Vec::new();
        for _ in 0..200 {
            run_days(&mut house, 1);
            if let Ok(Event::Harvested(_, h)) = house.harvest(id) {
                cuts.push((h.fresh_kg, house.plant(id).is_some()));
                if house.plant(id).is_none() {
                    break;
                }
            }
            if house.tanks[0].biofilm > 0.5 {
                house.tanks[0].clean();
            }
        }
        let kept: Vec<bool> = cuts.iter().map(|c| c.1).collect();
        assert_eq!(kept, [true, true, false], "two cuttings, then lifted: {cuts:?}");
        assert!(cuts.iter().all(|c| c.0 > 0.02), "{cuts:?}");
        assert!(house.store.biomass_kg > 0.0);
    }

    #[test]
    fn a_seedling_moves_to_its_place_but_a_grown_plant_does_not() {
        let mut house = chamber(2);
        house.sow(PlantId { tray: 0, slot: 0 }, &LETTUCE).unwrap();
        run_days(&mut house, 8);
        let (from, to) = (PlantId { tray: 0, slot: 0 }, PlantId { tray: 1, slot: 3 });
        let age = house.plant(from).unwrap().age_days;
        let (plant, lifted) = house.lift(from).unwrap();
        assert_eq!(lifted, Event::Lifted(from));
        assert!(house.plant(from).is_none());
        assert_eq!(house.place(to, plant), Ok(Event::Placed(to)));
        assert_eq!(house.plant(to).unwrap().age_days, age);
        run_days(&mut house, 24);
        assert_eq!(house.lift(to).err(), Some(Refusal::TooOld));
        assert_eq!(house.lift(from).err(), Some(Refusal::Empty));
    }

    #[test]
    fn the_crew_eats_greens_first_and_the_store_runs_down() {
        let mut store = Store { crew: 2, processing_kg: 5.0, ..Store::default() };
        store.add(&LETTUCE, 1.0);
        store.add(&WHEAT, 2.0);
        store.mill(1.0);
        store.feed(1.0);
        assert!(store.produce.iter().all(|(s, _)| s.name == "wheat"), "the lettuce went first");
        let wheat = 2.0 - (2.0 * CREW_KCAL - LETTUCE.nutrition.kcal) / WHEAT.nutrition.kcal;
        assert!((store.kg() - wheat).abs() < 1e-9);
        assert!((store.closure() - 1.0).abs() < 1e-9);
        store.feed(1.0);
        assert_eq!(store.kg(), 0.0);
        assert!(store.closure() < 0.6, "a second sol it fed short: {}", store.closure());
    }

    #[test]
    fn grain_feeds_no_one_until_the_line_mills_it() {
        let unmilled = |processing_kg| {
            let mut store = Store { crew: 1, processing_kg, ..Store::default() };
            store.add(&WHEAT, 10.0);
            store.step(1.0);
            store
        };
        let idle = unmilled(0.0);
        assert_eq!((idle.raw_kg(), idle.closure()), (10.0, 0.0), "no line, no bread");
        let milling = unmilled(2.0);
        assert!((milling.raw_kg() - 8.0).abs() < 1e-9, "the line took its 2 kg a sol");
        assert!((milling.closure() - 1.0).abs() < 1e-9, "2 kg of wheat is a day's food");
    }

    #[test]
    fn fresh_greens_spoil_into_the_recovery_heap_and_grain_keeps() {
        let mut store = Store::default();
        store.add(&LETTUCE, 10.0);
        store.produce.push((&WHEAT, 10.0));
        store.step(10.0);
        let held = |name: &str| store.produce.iter().find(|(s, _)| s.name == name).map_or(0.0, |(_, kg)| *kg);
        assert!((held("lettuce") - 10.0 * (-0.5f64).exp()).abs() < 1e-9, "greens lose a twentieth a sol");
        assert_eq!(held("wheat"), 10.0);
        let dry = store.spoiled_kg * (1.0 - LETTUCE.edible_water);
        assert!(store.spoiled_kg > 3.5 && (store.biomass_kg - dry).abs() < 1e-9, "{store:?}");
    }

    #[test]
    fn potatoes_in_raw_regolith_fail_until_the_bed_is_washed_and_amended() {
        let mut house = chamber(1);
        house.lamps[0].lit_hours = 12.0;
        house.trays[0].medium = Medium::RAW_REGOLITH;
        house.trays[0].slots = vec![None; 2];
        sow_all(&mut house, &POTATO);
        let events = run_days(&mut house, 12);
        assert_eq!(events.iter().filter(|e| matches!(e, Event::Died(_))).count(), 2, "no potato sprouts in raw regolith");
        house.wash(0).unwrap();
        house.amend(0, 0.15).unwrap();
        for slot in 0..2 {
            house.clear(PlantId { tray: 0, slot }).unwrap();
        }
        sow_all(&mut house, &POTATO);
        let events = run_days(&mut house, 2);
        assert_eq!(events.iter().filter(|e| matches!(e, Event::Sprouted(_))).count(), 2);
    }

    #[test]
    fn a_failed_fan_lets_mould_take_the_tray() {
        let mut house = chamber(30);
        sow_all(&mut house, &LETTUCE);
        run_days(&mut house, 12);
        house.raise(Fault::Fans(0)).unwrap();
        let leaves = house.surroundings(0, 12.0).unwrap().humidity;
        assert!(leaves > 0.85, "leaf-level RH {leaves}");
        let events = run_days(&mut house, 10);
        assert!(events.iter().any(|e| matches!(e, Event::Died(_))), "mould killed nothing");
        let mouldy = house.plants().filter(|(_, p)| p.mould > 0.5).count();
        assert!(mouldy > 0);
    }

    #[test]
    fn a_fan_mended_within_days_leaves_mould_that_fades_without_killing() {
        let mut house = chamber(30);
        sow_all(&mut house, &LETTUCE);
        run_days(&mut house, 12);
        house.raise(Fault::Fans(0)).unwrap();
        run_days(&mut house, 4);
        let infected = house.plants().map(|(_, p)| p.mould).fold(0.0, f64::max);
        assert!(infected > 0.25, "four still, wet days infect the crop: {infected}");
        house.repair(Fault::Fans(0)).unwrap();
        let events = run_days(&mut house, 15);
        assert!(!events.iter().any(|e| matches!(e, Event::Died(_))), "a mended fan still killed the crop");
        let mould = house.plants().map(|(_, p)| p.mould).fold(0.0, f64::max);
        assert!(mould < infected, "mould {mould} against {infected}");
    }

    #[test]
    fn a_breach_drops_the_room_below_what_plants_survive_without_makeup_gas() {
        let mut house = chamber(2);
        house.rooms[0].makeup_failed = true;
        house.raise(Fault::Leak(0)).unwrap();
        run_days(&mut house, 3);
        assert!(house.rooms[0].air.pressure_kpa < 10.0, "{} kPa", house.rooms[0].air.pressure_kpa);
        assert!(house.raise(Fault::Leak(0)).is_none(), "a fault is raised once");
        assert!(house.raise(Fault::Lamp(99)).is_none(), "no such lamp");
    }
}

