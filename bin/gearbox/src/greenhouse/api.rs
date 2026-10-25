//! The greenhouse over the tool API: its plants and rooms as they stand, the
//! actions a crew or a robot takes on them, and what happens as scene events.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use gearbox_api::{
    GearboxBus, GreenhouseAction, PlantRecord, Props, RoomRecord, SceneEvent, Status, code, event_kind,
    greenhouse_op as op,
};
use gearbox_greenhouse::house::{Event as HouseEvent, Fault, Refusal};
use gearbox_greenhouse::{Channels, PlantId, Stage, species};

use super::{Greenhouse, fault_name};
use crate::controller::ControllerInventory;
use crate::prim_index::PrimIndex;
use crate::globe::{Site, transform_in_site};

/// How close a machine must come to a service spot to mend what it serves, metres.
const SERVICE_REACH_M: f32 = 1.2;
/// How close a machine must come to a plant's middle to sow, harvest or lift it, metres.
const PLANT_REACH_M: f32 = 0.3;
/// How close a machine must come to a room's box to clean its glazing, metres.
const GLAZING_REACH_M: f32 = 1.5;

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
                ("glazing_dust", &number(room.glazing_dust)),
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

/// The service spots a fault is mended at.
fn service_spots(greenhouse: &Greenhouse, fault: Fault) -> Vec<Entity> {
    let (kind, room) = match fault {
        Fault::PowerCut | Fault::Reactor => ("power", None),
        Fault::Fans(r) => ("fans", Some(r)),
        Fault::Hvac(r) => ("hvac", Some(r)),
        Fault::Co2Feed(r) => ("co2", Some(r)),
        Fault::Leak(r) => ("leak", Some(r)),
        Fault::Pump(r) => ("pump", Some(r)),
        Fault::WaterLeak(r) => ("water_leak", Some(r)),
        Fault::Lamp(_) => return Vec::new(),
    };
    greenhouse
        .services
        .iter()
        .filter(|s| s.fault == kind && (room.is_none() || s.room.is_none() || s.room == room))
        .map(|s| s.entity)
        .collect()
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

/// Where machines and their parts stand, for `by` reach checks.
#[derive(SystemParam)]
pub struct Machines<'w, 's> {
    inventory: Option<Res<'w, ControllerInventory>>,
    index: Option<Res<'w, PrimIndex>>,
    children: Query<'w, 's, &'static Children>,
    globals: Query<'w, 's, &'static GlobalTransform>,
}

/// Where every part under a machine's own prim stands in the scene.
fn machine_parts(machine: &str, machines: &Machines) -> Result<Vec<Vec3>, Status> {
    let spec = machines
        .inventory
        .as_deref()
        .and_then(|inventory| inventory.machines.iter().find(|m| m.id == machine))
        .ok_or_else(|| Status::err(code::NOT_FOUND, &format!("no machine `{machine}`")))?;
    let root = spec
        .scene_root
        .and_then(|scene| machines.index.as_deref()?.find(scene, &spec.prim_path))
        .ok_or_else(|| Status::err(code::NOT_FOUND, &format!("machine `{machine}` stands nowhere")))?;
    Ok(std::iter::once(root)
        .chain(machines.children.iter_descendants(root))
        .filter_map(|e| machines.globals.get(e).ok())
        .map(|g| g.translation())
        .collect())
}

/// How far the nearest part under a machine's own prim is from a point, metres.
fn machine_distance(machine: &str, to: Vec3, machines: &Machines) -> Result<f32, Status> {
    Ok(machine_parts(machine, machines)?.iter().map(|at| at.distance(to)).fold(f32::MAX, f32::min))
}

/// How far the nearest part of a machine is from a room's box of air, metres.
fn room_distance(greenhouse: &Greenhouse, room: usize, machine: &str, machines: &Machines) -> Result<f32, Status> {
    let place = greenhouse.rooms.iter().find(|p| p.room == room).ok_or_else(|| Status::err(code::ERROR, "the room stands nowhere"))?;
    let frame = machines.globals.get(place.parent).map_err(|_| Status::err(code::ERROR, "the room stands nowhere"))?.affine().inverse();
    let half = place.size * 0.5;
    Ok(machine_parts(machine, machines)?
        .into_iter()
        .map(|at| (frame.transform_point3(at) - place.centre).abs() - half)
        .map(|out| out.max(Vec3::ZERO).length())
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
    machines: &Machines,
) -> Result<Vec<PlantId>, Status> {
    let Some(machine) = keys.get("by") else { return Ok(ids) };
    let single = ids.len() == 1;
    let mut near = Vec::new();
    for id in ids {
        let at = plant_middle(greenhouse, id, &machines.globals).ok_or_else(|| Status::err(code::ERROR, "the tray has no place"))?;
        let away = machine_distance(&machine, at, machines)?;
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
    machines: &Machines,
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
            let places = within_reach(greenhouse, places, &keys, machines)?;
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
            let ids = within_reach(greenhouse, ids, &keys, machines)?;
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
            within_reach(greenhouse, vec![id], &keys, machines)?;
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
        op::CLEAN_GLAZING => {
            let room = room_of(greenhouse, &keys)?;
            if let Some(machine) = keys.get("by") {
                let away = room_distance(greenhouse, room, &machine, machines)?;
                if away > GLAZING_REACH_M {
                    return Err(Status::err(code::REFUSED, &format!("`{machine}` is {away:.2} m from the room")));
                }
            }
            greenhouse.house.clean_glazing(room).map_err(refused)?;
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
                let spots = service_spots(greenhouse, fault);
                if spots.is_empty() {
                    return Err(Status::err(code::REFUSED, "nothing to stand at for that fault"));
                }
                let mut away = f32::MAX;
                for spot in spots {
                    let at = machines.globals.get(spot).map_err(|_| Status::err(code::ERROR, "service spot has no place"))?.translation();
                    away = away.min(machine_distance(&machine, at, machines)?);
                }
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
pub fn serve_greenhouse_api(
    bus: Option<ResMut<GearboxBus>>,
    mut greenhouse: ResMut<Greenhouse>,
    parents: Query<&ChildOf>,
    transforms: Query<&Transform>,
    sites: Query<&Site>,
    machines: Machines,
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
    bus.host.serve_greenhouse_act(|action| {
        act(&mut greenhouse, &action, &machines).unwrap_or_else(|status| {
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
    use usd_bevy::UsdPrimRef;

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
            .run_system_once(move |mut g: ResMut<Greenhouse>, machines: Machines| act(&mut g, &action, &machines).unwrap_or_else(|s| s))
            .unwrap();
        *greenhouse = world.remove_resource().unwrap();
        status
    }

    /// Machines along X, each a prim with one part, all in one stage.
    fn place_machines(world: &mut World, at: &[(&str, f32)]) {
        PrimIndex::track(world);
        let stage = world.spawn(GlobalTransform::IDENTITY).id();
        let mut inventory = ControllerInventory::default();
        for &(name, x) in at {
            let prim = format!("/Fleet/{name}");
            let root = world.spawn((UsdPrimRef::new(&prim), GlobalTransform::from_translation(Vec3::X * x), ChildOf(stage))).id();
            world.spawn((GlobalTransform::from_translation(Vec3::new(x, 0.0, 0.1)), ChildOf(root)));
            inventory.machines.push(crate::controller::MachineInstanceSpec {
                tracks: Vec::new(),
                devices: Vec::new(),
                scene_root: Some(stage),
                asset_label: "fleet".into(),
                source_path: String::new(),
                prim_path: prim,
                id: name.into(),
                kind: None,
                interface_version: None,
                id_policy: String::new(),
                body: None,
                sensors: Vec::new(),
                powered_wheel_joints: Vec::new(),
                passive_wheel_joints: Vec::new(),
                steering_joints: Vec::new(),
                brake_joints: Vec::new(),
                suspension_joints: Vec::new(),
                tool_joints: Vec::new(),
                controllers: Vec::new(),
                links: Default::default(),
                grants: Vec::new(),
                model: None,
            });
        }
        world.insert_resource(inventory);
    }

    fn act_in(world: &mut World, op: u32, keys: &[(&str, &str)]) -> Status {
        let action = GreenhouseAction::new(op, &Props::from_pairs(keys));
        world.run_system_once(move |mut g: ResMut<Greenhouse>, machines: Machines| act(&mut g, &action, &machines).unwrap_or_else(|s| s)).unwrap()
    }

    #[test]
    fn a_machine_reaches_with_its_own_parts_though_others_share_its_stage() {
        let mut world = World::new();
        place_machines(&mut world, &[("near", 0.1), ("far", 20.0)]);
        let tray = world.spawn(GlobalTransform::IDENTITY).id();
        let mut g = greenhouse();
        g.trays.push(super::super::TrayPlace {
            entity: tray,
            path: "/bed".into(),
            cols: 2,
            rows: 2,
            size: Vec2::new(0.6, 0.4),
            lamp_height: 0.5,
            lamp: None,
            lamp_face: None,
        });
        world.insert_resource(g);
        let mut sow = |by: &str| act_in(&mut world, op::SOW, &[("tray", "bed"), ("species", "lettuce"), ("by", by)]);
        assert_eq!(sow("far").code, code::REFUSED, "the far machine reaches nothing, near as its stage-mate stands");
        assert_eq!(sow("near").props().get("sown").as_deref(), Some("4"));
    }

    #[test]
    fn a_fault_is_mended_from_any_of_its_service_spots() {
        let mut world = World::new();
        place_machines(&mut world, &[("west", 0.1), ("east", 10.1), ("away", 5.0)]);
        let mut g = greenhouse();
        for x in [0.0, 10.0] {
            let entity = world.spawn(GlobalTransform::from_translation(Vec3::X * x)).id();
            g.services.push(super::super::ServicePoint { entity, name: format!("service_fans_{x}"), fault: "fans".into(), room: Some(0) });
        }
        world.insert_resource(g);
        let mut mend = |by: &str| {
            act_in(&mut world, op::FAULT, &[("fault", "fans:grow")]);
            act_in(&mut world, op::REPAIR, &[("fault", "fans:grow"), ("by", by)]).code
        };
        assert_eq!(mend("away"), code::REFUSED);
        assert_eq!(mend("west"), code::OK);
        assert_eq!(mend("east"), code::OK, "the far spot serves as well as the first");
    }

    #[test]
    fn glazing_is_cleaned_only_by_a_machine_at_the_room() {
        let mut world = World::new();
        place_machines(&mut world, &[("washer", 3.0), ("away", 20.0)]);
        let mut g = greenhouse();
        g.house.rooms[0].glazing = 0.5;
        let parent = world.spawn(GlobalTransform::IDENTITY).id();
        g.rooms.push(super::super::RoomPlace {
            room: 0,
            parent,
            centre: Vec3::new(0.0, 0.0, 1.0),
            size: Vec3::new(4.0, 4.0, 2.0),
            work_lux: 0.0,
            work_light: None,
            lux: 0.0,
            glazed: true,
        });
        world.insert_resource(g);
        let clean = |world: &mut World, by: &str| {
            world.resource_mut::<Greenhouse>().house.rooms[0].glazing_dust = 0.3;
            let code = act_in(world, op::CLEAN_GLAZING, &[("room", "grow"), ("by", by)]).code;
            (code, world.resource::<Greenhouse>().house.rooms[0].glazing_dust)
        };
        assert_eq!(clean(&mut world, "away"), (code::REFUSED, 0.3));
        assert_eq!(clean(&mut world, "washer"), (code::OK, 0.0), "a metre outside the box is near enough");
        assert_eq!(act_in(&mut world, op::CLEAN_GLAZING, &[("room", "service")]).code, code::REFUSED, "a room with no glazing");
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
