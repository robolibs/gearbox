//! The greenhouse over the tool API: its plants and rooms as they stand, the
//! actions a crew or a robot takes on them, and what happens as scene events.

use bevy::prelude::*;
use gearbox_api::{
    GearboxBus, GreenhouseAction, PlantRecord, Props, RoomRecord, SceneEvent, Status, code, event_kind,
    greenhouse_op as op,
};
use gearbox_greenhouse::house::{Event as HouseEvent, Fault, Refusal};
use gearbox_greenhouse::{Channels, PlantId, Stage, species};

use super::{Greenhouse, fault_name};
use crate::controller::ControllerInventory;
use crate::globe::{Site, transform_in_site};

/// How close a machine must come to a service spot to mend what it serves, metres.
const SERVICE_REACH_M: f32 = 1.2;
/// How close a machine must come to a plant's middle to sow, harvest or lift it, metres.
const PLANT_REACH_M: f32 = 0.3;

fn number(v: f64) -> String {
    format!("{v:.4}")
}

fn record(greenhouse: &Greenhouse, id: PlantId, at: Vec3) -> Option<PlantRecord> {
    let plant = greenhouse.house.plant(id)?;
    let tray = &greenhouse.house.trays[id.tray];
    let props = Props::from_pairs(&[
        ("plant", &greenhouse.plant_name(id)),
        ("tray", &tray.name),
        ("species", plant.species.name),
        ("stage", plant.stage().name()),
        ("edible_kg", &number(plant.edible_fresh_kg() * tray.per_slot)),
        ("biomass_g", &number(plant.biomass_g * tray.per_slot)),
        ("water_stress", &number(plant.water_stress)),
        ("nutrient_stress", &number(plant.nutrient_stress)),
        ("root_stress", &number(plant.root_stress)),
        ("mould", &number(plant.mould)),
        ("tip_burn", &number(plant.tip_burn)),
        ("sown_days", &number(plant.sown_days)),
        ("medium", tray.medium.name()),
    ]);
    Some(PlantRecord {
        x: at.x,
        y: at.y,
        z: at.z,
        maturity: plant.maturity() as f32,
        health: plant.health as f32,
        height_m: (plant.species.height_m * plant.size()) as f32,
        alive: plant.alive as u32,
        ripe: (plant.stage() == Stage::Ripe) as u32,
        props: props.into_bytes(),
    })
}

fn rooms(greenhouse: &Greenhouse) -> Vec<RoomRecord> {
    let house = &greenhouse.house;
    let power = &house.power;
    let faults: Vec<String> = house.faults.iter().map(|f| fault_name(house, *f)).collect();
    let state = |failed: bool| if failed { "failed" } else { "ok" };
    house
        .rooms
        .iter()
        .enumerate()
        .map(|(index, room)| {
            let tank = &house.tanks[index];
            let lux = greenhouse.rooms.iter().find(|p| p.room == index).map_or(0.0, |p| p.lux);
            let props = Props::from_pairs(&[
                ("room", &room.name),
                ("tank_l", &number(tank.water_l)),
                ("ec", &number(tank.ec)),
                ("biofilm", &number(tank.biofilm)),
                ("pump", state(tank.pump_failed)),
                ("fans", state(room.fans_failed)),
                ("hvac", state(room.hvac_failed)),
                ("co2_feed", state(room.co2_failed)),
                ("leak_per_s", &format!("{:e}", room.leak_per_s)),
                ("draw_w", &number(room.draw_w)),
                ("supply_w", &number(power.supply_w)),
                ("demand_w", &number(power.demand_w)),
                ("battery_wh", &number(power.charge_wh)),
                ("panel_dust", &number(power.panel_dust)),
                ("lamp_power", &number(house.lamp_power)),
                ("faults", &faults.join(",")),
            ]);
            RoomRecord {
                temperature_c: room.air.temperature_c as f32,
                humidity: room.air.humidity as f32,
                co2_ppm: room.air.co2_ppm as f32,
                o2_kpa: room.air.o2_kpa as f32,
                pressure_kpa: room.air.pressure_kpa as f32,
                lux,
                props: props.into_bytes(),
            }
        })
        .collect()
}

/// The fault a name means: `power`, `reactor`, `KIND:ROOM` or `lamp:TRAY`.
fn parse_fault(greenhouse: &Greenhouse, name: &str) -> Result<Fault, Status> {
    let unknown = || Status::err(code::USAGE, &format!("no fault `{name}`"));
    let (kind, of) = name.split_once(':').unwrap_or((name, ""));
    if kind == "lamp" {
        let tray = greenhouse.tray_index(of).ok_or_else(unknown)?;
        return greenhouse.fault("lamp", None, greenhouse.house.trays[tray].lamps.first().copied()).ok_or_else(unknown);
    }
    let room = if of.is_empty() { None } else { Some(greenhouse.room_index(of).ok_or_else(unknown)?) };
    greenhouse.fault(kind, room, None).ok_or_else(unknown)
}

/// The service spot a fault is mended at, if the greenhouse has one for it.
fn service_spot(greenhouse: &Greenhouse, fault: Fault) -> Option<Entity> {
    let (kind, room) = match fault {
        Fault::PowerCut | Fault::Reactor => ("power", None),
        Fault::Fans(r) => ("fans", Some(r)),
        Fault::Hvac(r) => ("hvac", Some(r)),
        Fault::Co2Feed(r) => ("co2", Some(r)),
        Fault::Leak(r) => ("leak", Some(r)),
        Fault::Pump(r) => ("pump", Some(r)),
        Fault::WaterLeak(r) => ("water_leak", Some(r)),
        Fault::Lamp(_) => return None,
    };
    greenhouse
        .services
        .iter()
        .find(|s| s.fault == kind && (room.is_none() || s.room.is_none() || s.room == room))
        .map(|s| s.entity)
}

fn tray_of(greenhouse: &Greenhouse, keys: &Props) -> Result<usize, Status> {
    let name = keys.get("tray").unwrap_or_default();
    greenhouse.tray_index(&name).ok_or_else(|| Status::err(code::NOT_FOUND, &format!("no tray `{name}`")))
}

fn plant_of(greenhouse: &Greenhouse, keys: &Props) -> Result<PlantId, Status> {
    let name = keys.get("plant").unwrap_or_default();
    greenhouse.plant_id(&name).ok_or_else(|| Status::err(code::NOT_FOUND, &format!("no plant `{name}`")))
}

fn room_of(greenhouse: &Greenhouse, keys: &Props) -> Result<usize, Status> {
    let name = keys.get("room").unwrap_or_default();
    greenhouse.room_index(&name).ok_or_else(|| Status::err(code::NOT_FOUND, &format!("no room `{name}`")))
}

fn refused(why: Refusal) -> Status {
    Status::err(code::REFUSED, &why.to_string())
}

/// How far the nearest part of a machine is from a point, metres.
fn machine_distance(
    machine: &str,
    to: Vec3,
    inventory: &ControllerInventory,
    children: &Query<&Children>,
    globals: &Query<&GlobalTransform>,
) -> Result<f32, Status> {
    let root = inventory
        .machines
        .iter()
        .find(|m| m.id == machine)
        .and_then(|m| m.scene_root)
        .ok_or_else(|| Status::err(code::NOT_FOUND, &format!("no machine `{machine}`")))?;
    Ok(std::iter::once(root)
        .chain(children.iter_descendants(root))
        .filter_map(|e| globals.get(e).ok())
        .map(|g| g.translation().distance(to))
        .fold(f32::MAX, f32::min))
}

/// The middle of a plant, or of its empty place, in the scene.
fn plant_middle(greenhouse: &Greenhouse, id: PlantId, globals: &Query<&GlobalTransform>) -> Option<Vec3> {
    let place = &greenhouse.trays[id.tray];
    let at = place.slot_at(id.slot);
    let height = greenhouse.house.plant(id).map_or(0.0, |p| (p.species.height_m * p.size()) as f32);
    Some(globals.get(place.entity).ok()?.transform_point(Vec3::new(at.x, at.y, height * 0.5)))
}

/// The places among `ids` the machine `by` names can reach, all of them without
/// one; a single place out of reach is refused.
fn within_reach(
    greenhouse: &Greenhouse,
    ids: Vec<PlantId>,
    keys: &Props,
    inventory: &ControllerInventory,
    children: &Query<&Children>,
    globals: &Query<&GlobalTransform>,
) -> Result<Vec<PlantId>, Status> {
    let Some(machine) = keys.get("by") else { return Ok(ids) };
    let single = ids.len() == 1;
    let mut near = Vec::new();
    for id in ids {
        let at = plant_middle(greenhouse, id, globals).ok_or_else(|| Status::err(code::ERROR, "the tray has no place"))?;
        let away = machine_distance(&machine, at, inventory, children, globals)?;
        if away <= PLANT_REACH_M {
            near.push(id);
        } else if single {
            let name = greenhouse.plant_name(id);
            return Err(Status::err(code::REFUSED, &format!("`{machine}` is {away:.2} m from {name}")));
        }
    }
    if near.is_empty() {
        return Err(Status::err(code::REFUSED, &format!("nothing within reach of `{machine}`")));
    }
    Ok(near)
}

fn act(
    greenhouse: &mut Greenhouse,
    action: &GreenhouseAction,
    inventory: &ControllerInventory,
    children: &Query<&Children>,
    globals: &Query<&GlobalTransform>,
) -> Result<Status, Status> {
    let keys = action.props();
    match action.op {
        op::SOW => {
            let tray = tray_of(greenhouse, &keys)?;
            let name = keys.get("species").unwrap_or_default();
            let species = species::by_name(&name).ok_or_else(|| Status::err(code::USAGE, &format!("no crop `{name}`")))?;
            let slots: Vec<usize> = match keys.get("slot") {
                Some(slot) => vec![slot.parse().map_err(|_| Status::err(code::USAGE, "slot is a number"))?],
                None => (0..greenhouse.house.trays[tray].slots.len())
                    .filter(|s| greenhouse.house.plant(PlantId { tray, slot: *s }).is_none_or(|p| !p.alive))
                    .collect(),
            };
            let places = slots.into_iter().map(|slot| PlantId { tray, slot }).collect();
            let places = within_reach(greenhouse, places, &keys, inventory, children, globals)?;
            for &id in &places {
                let event = greenhouse.house.sow(id, species).map_err(refused)?;
                greenhouse.events.push(event);
            }
            Ok(Status::ok_with(&Props::from_pairs(&[("sown", &places.len().to_string())])))
        }
        op::HARVEST => {
            let ids: Vec<PlantId> = if keys.contains("plant") {
                vec![plant_of(greenhouse, &keys)?]
            } else {
                let tray = tray_of(greenhouse, &keys)?;
                (0..greenhouse.house.trays[tray].slots.len())
                    .map(|slot| PlantId { tray, slot })
                    .filter(|id| greenhouse.house.plant(*id).is_some_and(|p| p.alive))
                    .collect()
            };
            let single = keys.contains("plant");
            let ids = within_reach(greenhouse, ids, &keys, inventory, children, globals)?;
            let (mut count, mut kg) = (0, 0.0);
            for id in ids {
                match greenhouse.house.harvest(id) {
                    Ok(event) => {
                        if let HouseEvent::Harvested(_, harvest) = &event {
                            kg += harvest.fresh_kg;
                            count += 1;
                        }
                        greenhouse.events.push(event);
                    }
                    Err(why) if single => return Err(refused(why)),
                    Err(_) => {}
                }
            }
            Ok(Status::ok_with(&Props::from_pairs(&[("harvested", &count.to_string()), ("fresh_kg", &number(kg))])))
        }
        op::CLEAR => {
            let id = plant_of(greenhouse, &keys)?;
            within_reach(greenhouse, vec![id], &keys, inventory, children, globals)?;
            greenhouse.house.clear(id).map_err(refused)?;
            Ok(Status::ok())
        }
        op::WASH => {
            let tray = tray_of(greenhouse, &keys)?;
            greenhouse.house.wash(tray).map_err(refused)?;
            Ok(Status::ok())
        }
        op::AMEND => {
            let tray = tray_of(greenhouse, &keys)?;
            greenhouse.house.amend(tray, keys.get_f64("share").unwrap_or(0.15)).map_err(refused)?;
            Ok(Status::ok())
        }
        op::LAMPS => {
            let channels = match keys.get("spectrum").as_deref() {
                None => None,
                Some("growth") => Some(Channels::GROWTH),
                Some("inspection" | "white") => Some(Channels::INSPECTION),
                Some(other) => return Err(Status::err(code::USAGE, &format!("spectrum `{other}` is growth or inspection"))),
            };
            let dimming = keys.get_f64("dimming").map(|d| d.clamp(0.0, 1.0));
            let trays: Vec<usize> = if keys.get("tray").as_deref() == Some("all") {
                (0..greenhouse.house.trays.len()).collect()
            } else {
                vec![tray_of(greenhouse, &keys)?]
            };
            for tray in trays {
                greenhouse.house.set_lamps(tray, channels, dimming).map_err(refused)?;
            }
            Ok(Status::ok())
        }
        op::CLEAN_TANK => {
            let room = room_of(greenhouse, &keys)?;
            greenhouse.house.tanks[room].clean();
            Ok(Status::ok())
        }
        op::CLEAN_PANELS => {
            greenhouse.house.power.panel_dust = 0.0;
            Ok(Status::ok())
        }
        op::FAULT => {
            let fault = parse_fault(greenhouse, &keys.get("fault").unwrap_or_default())?;
            let event = greenhouse.house.raise(fault).ok_or_else(|| Status::err(code::REFUSED, "already broken"))?;
            greenhouse.events.push(event);
            Ok(Status::ok())
        }
        op::REPAIR => {
            let fault = parse_fault(greenhouse, &keys.get("fault").unwrap_or_default())?;
            if !greenhouse.house.faults.contains(&fault) {
                return Err(Status::err(code::REFUSED, "not broken"));
            }
            if let Some(machine) = keys.get("by") {
                let spot = service_spot(greenhouse, fault).ok_or_else(|| Status::err(code::REFUSED, "nothing to stand at for that fault"))?;
                let at = globals.get(spot).map_err(|_| Status::err(code::ERROR, "service spot has no place"))?.translation();
                let away = machine_distance(&machine, at, inventory, children, globals)?;
                if away > SERVICE_REACH_M {
                    return Err(Status::err(code::REFUSED, &format!("`{machine}` is {away:.2} m from the service spot")));
                }
            }
            let event = greenhouse.house.repair(fault).ok_or_else(|| Status::err(code::REFUSED, "not broken"))?;
            greenhouse.events.push(event);
            Ok(Status::ok())
        }
        op::SETPOINT => {
            let room = room_of(greenhouse, &keys)?;
            let climate = &mut greenhouse.house.rooms[room].climate;
            for (key, value) in keys.iter() {
                let Ok(value) = value.parse::<f64>() else { continue };
                match key.as_str() {
                    "temperature" => climate.temperature_c = value.clamp(0.0, 40.0),
                    "humidity" => climate.humidity = value.clamp(0.2, 1.0),
                    "co2" => climate.co2_ppm = value.clamp(0.0, 5000.0),
                    "pressure" => climate.pressure_kpa = value.clamp(5.0, 110.0),
                    _ => {}
                }
            }
            Ok(Status::ok())
        }
        other => Err(Status::err(code::UNSUPPORTED, &format!("greenhouse op {other}"))),
    }
}

fn scene_event(greenhouse: &Greenhouse, happened: &HouseEvent, at: Vec3) -> SceneEvent {
    let house = &greenhouse.house;
    let (what, id) = match happened {
        HouseEvent::Sown(id) => ("sown", Some(*id)),
        HouseEvent::Sprouted(id) => ("sprouted", Some(*id)),
        HouseEvent::Staged(id, Stage::Ripe) => ("ripe", Some(*id)),
        HouseEvent::Staged(id, _) => ("staged", Some(*id)),
        HouseEvent::Died(id) => ("died", Some(*id)),
        HouseEvent::Harvested(id, _) => ("harvested", Some(*id)),
        HouseEvent::Faulted(_) => ("faulted", None),
        HouseEvent::Repaired(_) => ("repaired", None),
    };
    let name = id.map_or_else(String::new, |id| greenhouse.plant_name(id));
    let mut event = SceneEvent::new(event_kind::GREENHOUSE, &name).at(at.x, at.y, at.z).with_prop("what", what);
    if let Some(plant) = id.and_then(|id| house.plant(id)) {
        event = event.with_prop("species", plant.species.name);
    }
    match happened {
        HouseEvent::Staged(_, stage) => event.with_prop("stage", stage.name()),
        HouseEvent::Harvested(_, harvest) => {
            event.with_prop("species", harvest.species.name).with_prop("fresh_kg", &number(harvest.fresh_kg))
        }
        HouseEvent::Faulted(fault) | HouseEvent::Repaired(fault) => event.with_prop("fault", &fault_name(house, *fault)),
        _ => event,
    }
}

/// Answers the greenhouse topics and publishes what happened since last frame.
#[allow(clippy::too_many_arguments)]
pub fn serve_greenhouse_api(
    bus: Option<ResMut<GearboxBus>>,
    mut greenhouse: ResMut<Greenhouse>,
    inventory: Option<Res<ControllerInventory>>,
    parents: Query<&ChildOf>,
    transforms: Query<&Transform>,
    sites: Query<&Site>,
    children: Query<&Children>,
    globals: Query<&GlobalTransform>,
) {
    let Some(mut bus) = bus else { return };
    if greenhouse.is_empty() {
        return;
    }
    let place = |greenhouse: &Greenhouse, id: PlantId| {
        let tray = &greenhouse.trays[id.tray];
        let at = tray.slot_at(id.slot);
        transform_in_site(tray.entity, &parents, &transforms, &sites).transform_point(Vec3::new(at.x, at.y, 0.0))
    };
    let shown = &*greenhouse;
    bus.host.serve_greenhouse_plants(|query| {
        let filter = query.props();
        let (tray, crop) = (filter.get("tray"), filter.get("species"));
        shown
            .house
            .plants()
            .filter(|(id, _)| tray.as_ref().is_none_or(|t| shown.house.trays[id.tray].name == *t))
            .filter(|(_, p)| crop.as_ref().is_none_or(|c| p.species.name == c))
            .filter(|(_, p)| query.ripe == 0 || p.stage() == Stage::Ripe)
            .filter_map(|(id, _)| record(shown, id, place(shown, id)))
            .collect()
    });
    bus.host.serve_greenhouse_rooms(|_| rooms(shown));
    let empty = ControllerInventory::default();
    let inventory = inventory.as_deref().unwrap_or(&empty);
    bus.host.serve_greenhouse_act(|action| {
        act(&mut greenhouse, &action, inventory, &children, &globals).unwrap_or_else(|status| {
            warn!("greenhouse: {} refused: {}", op::name(action.op), status.message());
            status
        })
    });
    for happened in std::mem::take(&mut greenhouse.events) {
        let at = match &happened {
            HouseEvent::Sown(id) | HouseEvent::Sprouted(id) | HouseEvent::Staged(id, _) | HouseEvent::Died(id) | HouseEvent::Harvested(id, _) => {
                place(&greenhouse, *id)
            }
            _ => Vec3::ZERO,
        };
        let event = scene_event(&greenhouse, &happened, at);
        bus.publish_event(event);
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use gearbox_greenhouse::species::LETTUCE;
    use gearbox_greenhouse::{Climate, House, Lamp, Medium, Room, Tank, Tray};

    use super::*;

    fn greenhouse() -> Greenhouse {
        let mut house = House::new(24.0);
        for name in ["grow", "service"] {
            house.rooms.push(Room::new(name, 30.0, Climate::growth_room()));
            house.tanks.push(Tank::new(name, 300.0, 2.5));
        }
        house.lamps.push(Lamp::new("bed", 300.0, 0.24, 16.0));
        house.trays.push(Tray {
            name: "bed".into(),
            room: 0,
            tank: 0,
            lamps: vec![0],
            area_m2: 0.24,
            medium: Medium::Hydroponic,
            slots: vec![None; 4],
            per_slot: 1.0,
        });
        Greenhouse { house, ..default() }
    }

    fn run(greenhouse: &mut Greenhouse, op: u32, keys: &[(&str, &str)]) -> Status {
        let mut world = World::new();
        world.insert_resource(std::mem::take(greenhouse));
        let action = GreenhouseAction::new(op, &Props::from_pairs(keys));
        let status = world
            .run_system_once(move |mut g: ResMut<Greenhouse>, children: Query<&Children>, globals: Query<&GlobalTransform>| {
                act(&mut g, &action, &ControllerInventory::default(), &children, &globals).unwrap_or_else(|s| s)
            })
            .unwrap();
        *greenhouse = world.remove_resource().unwrap();
        status
    }

    #[test]
    fn fault_names_read_back() {
        let g = greenhouse();
        let faults =
            [Fault::PowerCut, Fault::Reactor, Fault::Fans(1), Fault::Hvac(0), Fault::Co2Feed(1), Fault::Leak(0), Fault::Pump(1), Fault::WaterLeak(0), Fault::Lamp(0)];
        for fault in faults {
            let name = fault_name(&g.house, fault);
            assert_eq!(parse_fault(&g, &name).ok(), Some(fault), "{name}");
        }
        assert!(parse_fault(&g, "fans:attic").is_err());
    }

    #[test]
    fn actions_change_the_house() {
        let mut g = greenhouse();
        let sown = run(&mut g, op::SOW, &[("tray", "bed"), ("species", "lettuce")]);
        assert_eq!(sown.props().get("sown").as_deref(), Some("4"));
        assert_eq!(g.house.plants().filter(|(_, p)| p.species.name == LETTUCE.name).count(), 4);
        assert_eq!(g.events.len(), 4);
        assert_eq!(run(&mut g, op::SOW, &[("tray", "bed"), ("species", "lettuce"), ("slot", "0")]).code, code::REFUSED);
        assert_eq!(run(&mut g, op::HARVEST, &[("plant", "bed/0")]).code, code::REFUSED);
        assert_eq!(run(&mut g, op::HARVEST, &[("tray", "bed")]).props().get("harvested").as_deref(), Some("0"));

        assert!(run(&mut g, op::FAULT, &[("fault", "fans:service")]).is_ok());
        assert!(g.house.rooms[1].fans_failed && !g.house.rooms[0].fans_failed);
        assert_eq!(run(&mut g, op::FAULT, &[("fault", "fans:service")]).code, code::REFUSED);
        assert!(run(&mut g, op::REPAIR, &[("fault", "fans:service")]).is_ok());
        assert!(!g.house.rooms[1].fans_failed);
        assert_eq!(run(&mut g, op::REPAIR, &[("fault", "fans:service")]).code, code::REFUSED);

        assert!(run(&mut g, op::SETPOINT, &[("room", "grow"), ("co2", "9000"), ("temperature", "23")]).is_ok());
        assert_eq!((g.house.rooms[0].climate.co2_ppm, g.house.rooms[0].climate.temperature_c), (5000.0, 23.0));
        assert_eq!(run(&mut g, op::WASH, &[("tray", "bed")]).code, code::REFUSED);
        assert_eq!(run(&mut g, op::LAMPS, &[("tray", "nowhere"), ("dimming", "0.5")]).code, code::NOT_FOUND);
    }
}
