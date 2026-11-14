//! The Farm pane: whom the greenhouse feeds and what it holds, its power bus,
//! and each room's air and crops. It only reads the house.

use bevy::prelude::*;
use mara::ui::mara_core;
use mara_core::container::{Tab, TabContainer};
use mara_core::pod::Pod;

use super::{PaneCtx, cid, pid};
use crate::greenhouse::{Greenhouse, RoomCount, RoomTally};
use crate::host::PANE_FARM as P;

/// Crops listed in the store, the ones held most of by food energy first.
const STORE_ROWS: usize = 6;

pub fn tab(world: &World, _ctx: &PaneCtx) -> Tab {
    let greenhouse = world.resource::<Greenhouse>();
    let house = &greenhouse.house;
    if greenhouse.is_empty() {
        let none = Pod::new(pid(P, "food", 0)).with_readout("farm", "no greenhouse in this world");
        return Tab::new(cid(P, "farm"), "Farm", "sprout").pods(vec![none]);
    }
    let store = &house.store;
    let mut food = Pod::new(pid(P, "food", 0))
        .with_readout("crew", store.crew.to_string())
        .with_readout("food from the farm", format!("{:.0} % of the crew's energy", store.closure() * 100.0))
        .with_readout("food held", format!("{:.0} kcal, {:.1} sols", store.kcal(), store.sols()))
        .with_readout("produce", format!("{:.1} kg", store.kg()))
        .with_readout("grain to mill", format!("{:.1} kg, {:.0} kg a sol milled", store.raw_kg(), store.processing_kg))
        .with_readout("spoiled", format!("{:.1} kg", store.spoiled_kg))
        .with_readout("biomass to recover", format!("{:.1} kg dry", store.biomass_kg));
    let mut carts: Vec<String> = greenhouse
        .carried
        .iter()
        .filter_map(|(holder, plants)| Some(format!("{} {}", holder.strip_prefix("cart:")?.replace('_', " "), plants.len())))
        .collect();
    carts.sort();
    food = food.with_readout("seedling carts", if carts.is_empty() { "none loaded".to_string() } else { carts.join(", ") });
    let mut held: Vec<_> = store.produce.iter().map(|(s, kg)| (s.name, *kg, kg * s.nutrition.kcal)).collect();
    held.sort_by(|a, b| b.2.total_cmp(&a.2));
    for (name, kg, kcal) in held.into_iter().take(STORE_ROWS) {
        food = food.with_readout(name.replace('_', " "), format!("{kg:.1} kg, {kcal:.0} kcal"));
    }

    let power = &house.power;
    let power_pod = Pod::new(pid(P, "power", 0))
        .with_readout("supply", format!("{:.1} kW", power.supply_w / 1000.0))
        .with_readout("demand", format!("{:.1} kW", power.demand_w / 1000.0))
        .with_readout("lamps lit", format!("{:.0} % of their demand", house.lamp_power * 100.0))
        .with_readout("battery", format!("{:.0} of {:.0} kWh", power.charge_wh / 1000.0, power.battery_wh / 1000.0))
        .with_readout("panel dust", format!("{:.0} %", power.panel_dust * 100.0));

    let tally = world.resource::<RoomTally>();
    let rooms: Vec<Pod> = house
        .rooms
        .iter()
        .zip(tally.0.iter().cloned().chain(std::iter::repeat_with(RoomCount::default)))
        .enumerate()
        .map(|(k, (room, RoomCount { growing, ripe, crops }))| {
            let air = &room.air;
            let mut kinds: Vec<_> = crops.into_iter().collect();
            kinds.sort_by(|a, b| b.1.cmp(&a.1));
            let names: Vec<String> = kinds.iter().take(4).map(|(name, _)| name.replace('_', " ")).collect();
            Pod::new(pid(P, "room", k))
                .with_readout(room.name.clone(), format!("{growing} plants, {ripe} ripe"))
                .with_readout("air", format!("{:.1} °C, {:.0} % RH, {:.0} ppm CO2", air.temperature_c, air.humidity * 100.0, air.co2_ppm))
                .with_readout("crops", if names.is_empty() { "none".to_string() } else { names.join(", ") })
        })
        .collect();

    Tab::new(cid(P, "farm"), "Farm", "sprout").containers(vec![
        TabContainer::new(cid(P, "food"), "Food", "food-apple", vec![food]),
        TabContainer::new(cid(P, "power"), "Power", "flash", vec![power_pod]),
        TabContainer::new(cid(P, "rooms"), "Rooms", "home-thermometer", rooms),
    ])
}
