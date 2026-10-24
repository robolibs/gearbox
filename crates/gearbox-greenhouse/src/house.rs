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
use crate::power::Power;
use crate::species::{Edible, Species};
use crate::water::{Medium, Tank};

/// Longest step the house takes in one go, seconds.
const SUBSTEP_S: f64 = 300.0;
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
    Faulted(Fault),
    Repaired(Fault),
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
    /// Length of the lamps' day, hours: the planet's solar day.
    pub cycle_hours: f64,
    /// Share of the lamps' demand the bus met over the last step.
    pub lamp_power: f64,
}

impl House {
    pub fn new(cycle_hours: f64) -> Self {
        Self { cycle_hours, lamp_power: 1.0, ..Default::default() }
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

    /// Photon flux on a tray now, and while lit, µmol/m²/s.
    pub fn tray_light(&self, tray: usize, hour: f64) -> (f64, f64) {
        let Some(t) = self.trays.get(tray) else { return (0.0, 0.0) };
        let now = t.lamps.iter().map(|&l| self.lamps[l].ppfd(hour, self.cycle_hours, self.lamp_power)).sum();
        let lit = t.lamps.iter().map(|&l| self.lamps[l].lit_ppfd()).sum();
        (now, lit)
    }

    /// The surroundings a tray's plants stand in now.
    pub fn surroundings(&self, tray: usize, hour: f64) -> Option<Surroundings> {
        let t = self.trays.get(tray)?;
        let room = &self.rooms[t.room];
        let tank = &self.tanks[t.tank];
        let (ppf, lit_ppf) = self.tray_light(tray, hour);
        let lit_hours = t.lamps.first().map_or(0.0, |&l| self.lamps[l].lit_hours);
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
    /// beyond the walls and `solar_w_m2` on the panels.
    pub fn step(&mut self, seconds: f64, hour: f64, outside: &Outside, solar_w_m2: f64) -> Vec<Event> {
        let mut events = Vec::new();
        let mut done = 0.0;
        while done < seconds {
            let dt = (seconds - done).min(SUBSTEP_S);
            let at = (hour + done / 3600.0).rem_euclid(self.cycle_hours.max(1.0));
            self.substep(dt, at, outside, solar_w_m2, &mut events);
            done += dt;
        }
        self.power.settle_dust(seconds / (self.cycle_hours.max(1.0) * 3600.0));
        events
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
            let (room, tank) = (self.trays[tray].room, self.trays[tray].tank);
            for slot in 0..self.trays[tray].slots.len() {
                let Some(plant) = self.trays[tray].slots[slot].as_mut() else { continue };
                let (stage, mass) = (plant.stage(), plant.biomass_g);
                let breath = plant.grow(dt, &around);
                let (now, id) = (plant.stage(), PlantId { tray, slot });
                grown[tank] += (plant.biomass_g - mass).max(0.0);
                drawn[tank] += breath.water_kg;
                exchange[room] += breath;
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
        let tray = self.trays.get_mut(id.tray).ok_or(Refusal::NoSuchTray)?;
        let slot = tray.slots.get_mut(id.slot).ok_or(Refusal::NoSuchPlace)?;
        if slot.as_ref().is_some_and(|p| p.alive) {
            return Err(Refusal::Occupied);
        }
        *slot = Some(Plant::sown(species));
        Ok(Event::Sown(id))
    }

    /// Takes in what a plant has made: the whole plant for leaf, tuber and
    /// grain crops, the fruit alone for fruiting ones, which grow on.
    pub fn harvest(&mut self, id: PlantId) -> Result<Event, Refusal> {
        let tray = self.trays.get_mut(id.tray).ok_or(Refusal::NoSuchTray)?;
        let slot = tray.slots.get_mut(id.slot).ok_or(Refusal::NoSuchPlace)?;
        let plant = slot.as_mut().filter(|p| p.alive).ok_or(Refusal::Empty)?;
        let fruit = plant.species.edible == Edible::Fruit;
        let ready = plant.stage() == Stage::Ripe || (fruit && plant.stage() == Stage::Forming && plant.edible_g > 0.0);
        if !ready {
            return Err(Refusal::NotRipe);
        }
        let harvest = Harvest { species: plant.species, fresh_kg: plant.edible_fresh_kg(), dry_g: plant.edible_g };
        if fruit {
            plant.biomass_g -= plant.edible_g;
            plant.edible_g = 0.0;
        } else {
            *slot = None;
        }
        Ok(Event::Harvested(id, harvest))
    }

    /// Lifts whatever is in a place, dead or alive.
    pub fn clear(&mut self, id: PlantId) -> Result<(), Refusal> {
        let tray = self.trays.get_mut(id.tray).ok_or(Refusal::NoSuchTray)?;
        let slot = tray.slots.get_mut(id.slot).ok_or(Refusal::NoSuchPlace)?;
        slot.take().map(|_| ()).ok_or(Refusal::Empty)
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
    use crate::species::{LETTUCE, POTATO, TOMATO};

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

    #[test]
    fn fruit_comes_off_a_tomato_that_keeps_growing() {
        let mut house = chamber(1);
        house.lamps[0].ppfd_max = 625.0;
        house.lamps[0].lit_hours = 12.0;
        house.trays[0].slots = vec![None; 2];
        sow_all(&mut house, &TOMATO);
        let id = PlantId { tray: 0, slot: 0 };
        assert_eq!(house.harvest(id), Err(Refusal::NotRipe));
        run_days(&mut house, 35);
        house.tanks[0].clean();
        run_days(&mut house, 35);
        let state = house.plant(id).cloned();
        let Ok(Event::Harvested(_, picked)) = house.harvest(id) else { panic!("tomato harvest: {:?} {:?}", house.harvest(id), state.map(|p| (p.stage(), p.age_days, p.edible_g, p.health, p.alive))) };
        assert!(picked.fresh_kg > 0.2, "{picked:?}");
        let plant = house.plant(id).expect("the vine stays");
        assert_eq!(plant.edible_g, 0.0);
        assert!(plant.alive);
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

