//! The air of one room: its carbon dioxide, oxygen and water vapour, how warm
//! and how dense it is, and the equipment that holds it where the crops want it.
//!
//! Gases are kept as moles in an ideal gas at the room's pressure and
//! temperature. Heat flows in from lamps and equipment, out through the walls
//! to the cold outside, and is moved by the air conditioning towards its
//! setpoint; evaporation off leaves cools the air. Each piece of equipment
//! can be off — failed, or without power — and then the air drifts.

/// Gas constant, J/(mol K).
const GAS: f64 = 8.314;
const KELVIN: f64 = 273.15;
/// Latent heat of water evaporating off leaves, J/kg.
const LATENT: f64 = 2.45e6;
const WATER_G: f64 = 18.015;

/// The air of a room.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Air {
    pub volume_m3: f64,
    pub temperature_c: f64,
    /// Relative humidity, 0 to 1.
    pub humidity: f64,
    pub co2_ppm: f64,
    pub o2_kpa: f64,
    pub pressure_kpa: f64,
}

/// Where the room's equipment holds the air, and how much it can do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Climate {
    pub temperature_c: f64,
    /// Highest relative humidity the dehumidifier lets stand.
    pub humidity: f64,
    /// CO2 the enrichment feeds up to, ppm.
    pub co2_ppm: f64,
    /// Oxygen above this partial pressure is drawn off to the habitat, kPa.
    pub o2_max_kpa: f64,
    /// Total pressure the make-up gas holds, kPa.
    pub pressure_kpa: f64,
    pub heating_w: f64,
    pub cooling_w: f64,
    /// Water the dehumidifier can pull, kg/s.
    pub dehumidify_kg_s: f64,
    /// CO2 the enrichment can feed, mol/s.
    pub co2_feed_mol_s: f64,
    /// Make-up gas the pressure control can feed, mol/s.
    pub makeup_mol_s: f64,
    /// Fans' electric draw, W.
    pub fan_w: f64,
    /// Heat lost through the walls per kelvin to the outside, W/K.
    pub walls_w_k: f64,
    /// Heat the room and what stands in it hold per kelvin, J/K.
    pub heat_capacity_j_k: f64,
}

impl Climate {
    /// An EDEN ISS-like growth room (Zabel et al. 2017): 21 °C, 1000 ppm,
    /// 75 % relative humidity at sea-level pressure, behind insulated walls
    /// bermed in regolith, with its water tanks and racks to hold the heat.
    pub fn growth_room() -> Self {
        Self {
            temperature_c: 21.0,
            humidity: 0.75,
            co2_ppm: 1000.0,
            o2_max_kpa: 23.0,
            pressure_kpa: 101.3,
            heating_w: 6_000.0,
            cooling_w: 6_000.0,
            dehumidify_kg_s: 2.0e-4,
            co2_feed_mol_s: 5.0e-3,
            makeup_mol_s: 0.05,
            fan_w: 300.0,
            walls_w_k: 22.0,
            heat_capacity_j_k: 6.0e6,
        }
    }
}

/// What is beyond the walls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Outside {
    pub temperature_c: f64,
    pub pressure_kpa: f64,
}

impl Outside {
    /// The Martian surface at a local solar hour: 0.64 kPa, and air that swings
    /// between −89 and −31 °C over a sol at a Viking-like site, warmest at 14:00.
    pub fn mars(hour: f64) -> Self {
        let phase = (hour - 14.0) / 24.0 * std::f64::consts::TAU;
        Self { temperature_c: -60.0 + 29.0 * phase.cos(), pressure_kpa: 0.636 }
    }
}

/// Which equipment works this step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Working {
    pub hvac: bool,
    pub fans: bool,
    pub co2_feed: bool,
    pub dehumidifier: bool,
    pub makeup: bool,
    /// The room leaks to the outside: the share of its gas lost per second at full pressure difference.
    pub leak_per_s: f64,
}

impl Working {
    pub const ALL: Self = Self { hvac: true, fans: true, co2_feed: true, dehumidifier: true, makeup: true, leak_per_s: 0.0 };
    pub const NONE: Self = Self { hvac: false, fans: false, co2_feed: false, dehumidifier: false, makeup: false, leak_per_s: 0.0 };
}

/// What the room's air took over a step from its equipment.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Upkeep {
    /// Electric energy drawn, J.
    pub energy_j: f64,
    pub co2_fed_mol: f64,
    /// Water the dehumidifier and the walls took out, kg: it goes back to the tank.
    pub condensate_kg: f64,
    pub o2_drawn_mol: f64,
    pub makeup_mol: f64,
    pub leaked_mol: f64,
}

/// Saturated water vapour pressure, kPa (Tetens, as BVAD Equation 4-24).
pub fn saturation_kpa(celsius: f64) -> f64 {
    0.611 * (17.4 * celsius / (celsius + 239.0)).exp()
}

impl Air {
    /// The air a room starts with at its setpoints, oxygen at a fifth of the pressure.
    pub fn settled(volume_m3: f64, climate: &Climate) -> Self {
        Self {
            volume_m3,
            temperature_c: climate.temperature_c,
            humidity: climate.humidity - 0.05,
            co2_ppm: climate.co2_ppm,
            o2_kpa: climate.pressure_kpa * 0.2095,
            pressure_kpa: climate.pressure_kpa,
        }
    }

    /// Moles of gas the room holds.
    pub fn moles(&self) -> f64 {
        self.pressure_kpa * 1000.0 * self.volume_m3 / (GAS * (self.temperature_c + KELVIN))
    }

    /// Steps the air `seconds` on: `fixed_co2_mol`, `given_o2_mol` and
    /// `transpired_kg` from the crops, `heat_w` from lamps and equipment.
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        seconds: f64,
        climate: &Climate,
        working: Working,
        fixed_co2_mol: f64,
        given_o2_mol: f64,
        transpired_kg: f64,
        heat_w: f64,
        outside: &Outside,
    ) -> Upkeep {
        let mut upkeep = Upkeep::default();
        let n = self.moles();
        let vapour_mol = |humidity: f64, celsius: f64| humidity * saturation_kpa(celsius) / self.pressure_kpa * n;

        // Carbon dioxide: the crops fix it, the feed tops it up.
        let mut co2 = (self.co2_ppm * 1e-6 * n - fixed_co2_mol).max(0.0);
        let wanted = climate.co2_ppm * 1e-6 * n;
        if working.co2_feed && co2 < wanted {
            let fed = (wanted - co2).min(climate.co2_feed_mol_s * seconds);
            co2 += fed;
            upkeep.co2_fed_mol = fed;
            upkeep.energy_j += 50.0 * seconds;
        }

        // Oxygen: the crops give it off, the habitat draws off any surplus.
        let mut o2 = (self.o2_kpa / self.pressure_kpa * n + given_o2_mol).max(0.0);
        let most = climate.o2_max_kpa / self.pressure_kpa * n;
        if working.makeup && o2 > most {
            upkeep.o2_drawn_mol = o2 - most;
            o2 = most;
        }

        // Water vapour: leaves give it, the dehumidifier pulls it out.
        let mut water = vapour_mol(self.humidity, self.temperature_c) + transpired_kg * 1000.0 / WATER_G;
        let ceiling = vapour_mol(climate.humidity, self.temperature_c);
        if working.dehumidifier && water > ceiling {
            let pulled = ((water - ceiling) * WATER_G / 1000.0).min(climate.dehumidify_kg_s * seconds);
            water -= pulled * 1000.0 / WATER_G;
            upkeep.condensate_kg += pulled;
            upkeep.energy_j += pulled * LATENT / 3.0;
        }
        let mut rest = (n - self.co2_ppm * 1e-6 * n - self.o2_kpa / self.pressure_kpa * n
            - vapour_mol(self.humidity, self.temperature_c))
        .max(0.0);

        // Heat: lamps and equipment in, walls out, evaporation off leaves, and the air conditioning.
        let mut flow = heat_w - climate.walls_w_k * (self.temperature_c - outside.temperature_c) - transpired_kg * LATENT / seconds.max(1e-9);
        if working.hvac {
            let needed = (climate.temperature_c - self.temperature_c) * climate.heat_capacity_j_k / seconds.max(1.0) - flow;
            let moved = needed.clamp(-climate.cooling_w, climate.heating_w);
            flow += moved;
            upkeep.energy_j += moved.abs() / 3.0 * seconds;
        }
        if working.fans {
            upkeep.energy_j += climate.fan_w * seconds;
            flow += climate.fan_w;
        }
        self.temperature_c += flow * seconds / climate.heat_capacity_j_k;

        // Pressure: gas leaks out towards the outside, taking its share of
        // every part with it; make-up gas is air.
        let mut parts = [co2, o2, water, rest];
        if working.leak_per_s > 0.0 {
            let difference = ((self.pressure_kpa - outside.pressure_kpa) / self.pressure_kpa).max(0.0);
            let kept = (-working.leak_per_s * difference * seconds).exp();
            upkeep.leaked_mol = parts.iter().sum::<f64>() * (1.0 - kept);
            parts.iter_mut().for_each(|part| *part *= kept);
        }
        let target = climate.pressure_kpa * 1000.0 * self.volume_m3 / (GAS * (self.temperature_c + KELVIN));
        let total: f64 = parts.iter().sum();
        if working.makeup && total < target {
            let fed = (target - total).min(climate.makeup_mol_s * seconds);
            parts[1] += 0.2095 * fed;
            parts[3] += 0.7905 * fed;
            upkeep.makeup_mol = fed;
        }
        [co2, o2, water, rest] = parts;
        let total = (co2 + o2 + water + rest).max(1e-9);
        self.pressure_kpa = total * GAS * (self.temperature_c + KELVIN) / (self.volume_m3 * 1000.0);

        // Vapour past saturation condenses on the walls and runs back to the tank.
        let saturated = saturation_kpa(self.temperature_c) / self.pressure_kpa * total;
        if water > saturated {
            upkeep.condensate_kg += (water - saturated) * WATER_G / 1000.0;
            water = saturated;
        }
        self.co2_ppm = co2 / total * 1e6;
        self.o2_kpa = o2 / total * self.pressure_kpa;
        self.humidity = (water / saturated.max(1e-12)).clamp(0.0, 1.0);
        upkeep
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room() -> (Air, Climate) {
        let climate = Climate::growth_room();
        (Air::settled(31.5, &climate), climate)
    }

    #[test]
    fn the_room_holds_its_setpoints_against_the_martian_cold() {
        let (mut air, climate) = room();
        for hour in 0..48 {
            air.step(3600.0, &climate, Working::ALL, 0.0, 0.0, 0.0, 800.0, &Outside::mars(hour as f64 % 24.0));
        }
        assert!((air.temperature_c - climate.temperature_c).abs() < 0.5, "{air:?}");
        assert!((air.pressure_kpa - climate.pressure_kpa).abs() < 0.5);
    }

    #[test]
    fn without_heating_the_room_cools_towards_mars_over_hours() {
        let (mut air, climate) = room();
        let mut temperatures = Vec::new();
        for _ in 0..24 {
            air.step(3600.0, &climate, Working::NONE, 0.0, 0.0, 0.0, 0.0, &Outside::mars(4.0));
            temperatures.push(air.temperature_c);
        }
        assert!(temperatures[0] > 18.0, "an hour in it is {}", temperatures[0]);
        assert!(temperatures[23] < 5.0, "a sol in it is {}", temperatures[23]);
    }

    #[test]
    fn crops_breathing_in_a_sealed_dark_room_raise_its_carbon_dioxide() {
        let (mut air, climate) = room();
        // 10 m² of lettuce respiring 5.6 µmol CO2 per m² per second.
        let released = -10.0 * 5.6e-6 * 3600.0;
        air.step(3600.0, &climate, Working::NONE, released, released * 1.08, 0.0, 0.0, &Outside { temperature_c: 21.0, pressure_kpa: 0.636 });
        let rise = air.co2_ppm - climate.co2_ppm;
        assert!((120.0..200.0).contains(&rise), "rose {rise} ppm in an hour");
    }

    #[test]
    fn a_leak_empties_the_room_unless_makeup_gas_keeps_up() {
        let (mut air, climate) = room();
        let leaking = Working { leak_per_s: 1e-4, ..Working::NONE };
        for _ in 0..12 {
            air.step(3600.0, &climate, leaking, 0.0, 0.0, 0.0, 0.0, &Outside { temperature_c: 21.0, pressure_kpa: 0.636 });
        }
        assert!(air.pressure_kpa < 10.0, "{} kPa", air.pressure_kpa);
        assert!(air.o2_kpa < 2.0);
        let (mut held, _) = room();
        let fed = Working { leak_per_s: 2e-5, ..Working::ALL };
        for _ in 0..12 {
            held.step(3600.0, &climate, fed, 0.0, 0.0, 0.0, 800.0, &Outside::mars(12.0));
        }
        assert!(held.pressure_kpa > 95.0, "{} kPa with make-up gas", held.pressure_kpa);
    }

    #[test]
    fn transpiration_wets_the_air_until_the_dehumidifier_pulls_it_back() {
        let (mut air, climate) = room();
        let mut condensate = 0.0;
        for _ in 0..24 {
            condensate += air.step(3600.0, &climate, Working::ALL, 0.0, 0.0, 0.5, 800.0, &Outside::mars(12.0)).condensate_kg;
        }
        assert!(air.humidity <= climate.humidity + 0.01, "RH {}", air.humidity);
        assert!((condensate - 12.0).abs() < 1.0, "recovered {condensate} kg of 12");
        let (mut wet, _) = room();
        for _ in 0..6 {
            wet.step(3600.0, &climate, Working { dehumidifier: false, ..Working::ALL }, 0.0, 0.0, 0.5, 800.0, &Outside::mars(12.0));
        }
        assert!(wet.humidity > 0.95, "without the dehumidifier RH {}", wet.humidity);
    }
}
