//! Greenhouses in the scene: growth rooms read from a loaded USD, their crops
//! grown on world time by `gearbox_greenhouse`, drawn as crop prefabs under
//! their trays' lamps.
//!
//! A world USD declares the greenhouse with four API schemas (specs/GREENHOUSE_SPEC.md):
//! `GearboxZoneAPI` rooms, `GearboxTrayAPI` trays with the lamp over each,
//! `GearboxPowerAPI` for the bus, `GearboxServiceAPI` spots where equipment
//! is mended. Every greenhouse found joins the one [`Greenhouse`] house.

mod api;
pub mod crops;
mod looks;
mod sense;

use std::collections::HashSet;

use bevy::prelude::*;
use gearbox_greenhouse::air::Climate;
use gearbox_greenhouse::house::{Event as HouseEvent, Fault};
use gearbox_greenhouse::{House, Lamp, Medium, Outside, PlantId, Room, Species, Surroundings, Tank, Tray};
use openusd::sdf::{Path as SdfPath, Value};
use usd_bevy::instance::UsdInstances;
use usd_bevy::{UsdSceneRoot, UsdSceneState};

use crate::controller::{read_attr, read_float, read_token};
use crate::environment::{WeatherSettings, WorldStep, WorldTime};
use crate::usd_ext::StageExt;



pub struct GreenhousePlugin;

impl Plugin for GreenhousePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Greenhouse>()
            .init_resource::<crate::sensor_generic::AirField>()
            .add_systems(Startup, looks::build_looks)
            .add_systems(
                Update,
                (
                    discover_greenhouses,
                    step_greenhouse,
                    api::serve_greenhouse_api,
                    looks::sync_plants,
                    sense::drop_plant_hulls,
                    sense::sync_plant_hulls,
                    sense::fill_air_field,
                    looks::sync_lamps,
                    looks::expose_indoors,
                )
                    .chain()
                    .after(WorldTime),
            );
    }
}

/// Where a tray of the house stands in the scene.
#[derive(Clone, Debug)]
pub struct TrayPlace {
    /// The tray prim's entity: plants stand on its origin plane, its +Z up.
    pub entity: Entity,
    pub path: String,
    pub cols: usize,
    pub rows: usize,
    pub size: Vec2,
    /// How far the lamp hangs above the tray, metres, and its prim entities.
    pub lamp_height: f32,
    pub lamp: Option<Entity>,
    pub lamp_face: Option<Entity>,
}

impl TrayPlace {
    /// Where place `slot` is on the tray, in the tray's own frame.
    pub fn slot_at(&self, slot: usize) -> Vec2 {
        let (i, j) = (slot % self.cols.max(1), slot / self.cols.max(1));
        Vec2::new(
            ((i as f32 + 0.5) / self.cols as f32 - 0.5) * self.size.x,
            ((j as f32 + 0.5) / self.rows as f32 - 0.5) * self.size.y,
        )
    }
}

/// A spot where a fault is mended.
#[derive(Clone, Debug)]
pub struct ServicePoint {
    pub entity: Entity,
    pub name: String,
    pub fault: String,
    pub room: Option<usize>,
}

/// Where a room of the house stands: the box of its air in its zone prim's
/// frame (Z up), the white light it is worked in, and how lit it is.
#[derive(Clone, Debug)]
pub struct RoomPlace {
    pub room: usize,
    pub parent: Entity,
    pub centre: Vec3,
    pub size: Vec3,
    /// Illuminance of the work light on the floor, lux; none when 0.
    pub work_lux: f32,
    pub work_light: Option<Entity>,
    /// Mean illuminance over the room from its lamps and work light, lux.
    pub lux: f32,
}

/// Every greenhouse in the scene as one house.
#[derive(Resource, Default)]
pub struct Greenhouse {
    pub house: House,
    pub trays: Vec<TrayPlace>,
    pub services: Vec<ServicePoint>,
    pub rooms: Vec<RoomPlace>,
    /// What happened since the API last took it.
    pub events: Vec<HouseEvent>,
    scanned: HashSet<Entity>,
}

impl Greenhouse {
    pub fn is_empty(&self) -> bool {
        self.house.trays.is_empty()
    }

    /// A plant's name: its tray's and its place in it, `TrayN21a/3`.
    pub fn plant_name(&self, id: PlantId) -> String {
        format!("{}/{}", self.house.trays[id.tray].name, id.slot)
    }

    /// The plant a name means.
    pub fn plant_id(&self, name: &str) -> Option<PlantId> {
        let (tray, slot) = name.rsplit_once('/')?;
        let tray = self.house.trays.iter().position(|t| t.name == tray)?;
        let slot = slot.parse().ok().filter(|s| *s < self.house.trays[tray].slots.len())?;
        Some(PlantId { tray, slot })
    }

    pub fn tray_index(&self, name: &str) -> Option<usize> {
        self.house.trays.iter().position(|t| t.name == name)
    }

    pub fn room_index(&self, name: &str) -> Option<usize> {
        self.house.rooms.iter().position(|r| r.name == name)
    }

    /// The fault a service spot's kind and room mean.
    pub fn fault(&self, kind: &str, room: Option<usize>, index: Option<usize>) -> Option<Fault> {
        let room = room.unwrap_or(0);
        Some(match kind {
            "power" => Fault::PowerCut,
            "reactor" => Fault::Reactor,
            "fans" => Fault::Fans(room),
            "hvac" => Fault::Hvac(room),
            "co2" => Fault::Co2Feed(room),
            "leak" => Fault::Leak(room),
            "pump" => Fault::Pump(room),
            "water_leak" => Fault::WaterLeak(room),
            "lamp" => Fault::Lamp(index?),
            _ => return None,
        })
    }
}

/// The name a fault goes by, with what it is in.
pub fn fault_name(house: &House, fault: Fault) -> String {
    let room = |r: usize| house.rooms.get(r).map_or(String::new(), |room| format!(":{}", room.name));
    match fault {
        Fault::PowerCut => "power".into(),
        Fault::Reactor => "reactor".into(),
        Fault::Fans(r) => format!("fans{}", room(r)),
        Fault::Hvac(r) => format!("hvac{}", room(r)),
        Fault::Co2Feed(r) => format!("co2{}", room(r)),
        Fault::Leak(r) => format!("leak{}", room(r)),
        Fault::Pump(t) => format!("pump{}", room(t)),
        Fault::WaterLeak(t) => format!("water_leak{}", room(t)),
        Fault::Lamp(l) => format!("lamp:{}", house.lamps.get(l).map_or("", |l| l.name.as_str())),
    }
}

fn float2(stage: &openusd::usd::Stage, prim: &SdfPath, name: &str) -> Option<Vec2> {
    match read_attr(stage, prim, name)? {
        Value::Vec2f(v) => Some(Vec2::new(v.x, v.y)),
        Value::Vec2d(v) => Some(Vec2::new(v.x as f32, v.y as f32)),
        _ => None,
    }
}

fn float3(stage: &openusd::usd::Stage, prim: &SdfPath, name: &str) -> Option<Vec3> {
    match read_attr(stage, prim, name)? {
        Value::Vec3f(v) => Some(Vec3::new(v.x, v.y, v.z)),
        Value::Vec3d(v) => Some(Vec3::new(v.x as f32, v.y as f32, v.z as f32)),
        _ => None,
    }
}

fn int2(stage: &openusd::usd::Stage, prim: &SdfPath, name: &str) -> Option<(usize, usize)> {
    match read_attr(stage, prim, name)? {
        Value::Vec2i(v) if v.x > 0 && v.y > 0 => Some((v.x as usize, v.y as usize)),
        _ => None,
    }
}

/// A repeatable 0..1 number per plant, for sowing dates and looks.
pub(crate) fn scatter(tray: usize, slot: usize, salt: u32) -> f32 {
    let mut h = (tray as u32).wrapping_mul(0x9E37_79B9) ^ (slot as u32).wrapping_mul(0x85EB_CA6B) ^ salt.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    (h & 0xFFFF) as f32 / 65_535.0
}

/// Reads every greenhouse a newly projected stage declares into the house.
fn discover_greenhouses(world: &mut World) {
    let ready: Vec<Entity> = world
        .query_filtered::<(Entity, &UsdSceneState), With<UsdSceneRoot>>()
        .iter(world)
        .filter(|(_, state)| **state == UsdSceneState::Ready)
        .map(|(e, _)| e)
        .filter(|e| !world.resource::<Greenhouse>().scanned.contains(e))
        .collect();
    if ready.is_empty() {
        return;
    }
    let Some(instances) = world.remove_non_send::<UsdInstances>() else { return };
    let cycle_hours = world.resource::<WeatherSettings>().calendar.planet.solar_day_s() / 3600.0;
    for root in ready {
        world.resource_mut::<Greenhouse>().scanned.insert(root);
        let Some(stage) = instances.stage(root) else { continue };
        let prims: Vec<(String, Entity)> = instances.prims(root).map(|(p, e)| (p.to_string(), e)).collect();
        let entity_of = |path: &str| prims.iter().find(|(p, _)| p == path).map(|(_, e)| *e);
        let mut found = Vec::new();
        for (path, entity) in &prims {
            let Ok(sdf) = SdfPath::new(path) else { continue };
            let names = stage.prim_properties(&sdf).unwrap_or_default();
            let has = |ns: &str| names.iter().any(|n| n.starts_with(ns));
            if has("gearbox:zone:") || has("gearbox:tray:") || has("gearbox:power:") || has("gearbox:service:") {
                found.push((path.clone(), sdf, *entity));
            }
        }
        if found.is_empty() {
            continue;
        }
        let mut greenhouse = world.resource_mut::<Greenhouse>();
        if greenhouse.house.cycle_hours <= 0.0 {
            greenhouse.house = House::new(cycle_hours);
        }
        let first_room = greenhouse.house.rooms.len();
        let mut room_names = Vec::new();
        for (path, sdf, entity) in &found {
            let float = |name: &str| read_float(stage, sdf, name).map(f64::from);
            if let Some(name) = read_token(stage, sdf, "gearbox:zone:name") {
                let base = Climate::growth_room();
                let climate = Climate {
                    temperature_c: float("gearbox:zone:temperature").unwrap_or(base.temperature_c),
                    humidity: float("gearbox:zone:humidity").unwrap_or(base.humidity),
                    co2_ppm: float("gearbox:zone:co2").unwrap_or(base.co2_ppm),
                    pressure_kpa: float("gearbox:zone:pressure").unwrap_or(base.pressure_kpa),
                    ..base
                };
                let volume = float("gearbox:zone:volume").unwrap_or(30.0).max(1.0);
                let house = &mut greenhouse.house;
                house.rooms.push(Room::new(&name, volume, climate));
                house.tanks.push(Tank::new(&name, float("gearbox:zone:tank").unwrap_or(300.0), float("gearbox:zone:ec").unwrap_or(2.5)));
                room_names.push(name.clone());
                let room = greenhouse.house.rooms.len() - 1;
                let size = float3(stage, sdf, "gearbox:zone:size").unwrap_or(Vec3::splat((volume as f32).cbrt()));
                greenhouse.rooms.push(RoomPlace {
                    room,
                    parent: *entity,
                    centre: float3(stage, sdf, "gearbox:zone:centre").unwrap_or(Vec3::new(0.0, 0.0, size.z / 2.0)),
                    size,
                    work_lux: float("gearbox:zone:workLight").unwrap_or(0.0) as f32,
                    work_light: None,
                    lux: 0.0,
                });
                info!("greenhouse: room `{name}` {volume:.1} m³ at {path}");
            }
            if read_float(stage, sdf, "gearbox:power:reactor").is_some() {
                let power = &mut greenhouse.house.power;
                power.reactor_w = float("gearbox:power:reactor").unwrap_or(power.reactor_w);
                power.solar_m2 = float("gearbox:power:solarArea").unwrap_or(power.solar_m2);
                power.battery_wh = float("gearbox:power:battery").unwrap_or(power.battery_wh);
                power.charge_wh = power.battery_wh * 0.75;
            }
        }
        for (path, sdf, entity) in &found {
            let float = |name: &str| read_float(stage, sdf, name).map(f64::from);
            let room_of = |zone: Option<String>| {
                zone.and_then(|z| room_names.iter().position(|n| *n == z)).map(|i| first_room + i)
            };
            if let Some(fault) = read_token(stage, sdf, "gearbox:service:fault") {
                let room = room_of(read_token(stage, sdf, "gearbox:service:zone"));
                let name = path.rsplit('/').next().unwrap_or_default().to_string();
                world.resource_mut::<Greenhouse>().services.push(ServicePoint { entity: *entity, name, fault, room });
                continue;
            }
            let Some(species_name) = read_token(stage, sdf, "gearbox:tray:species") else { continue };
            let Some(species) = gearbox_greenhouse::species::by_name(&species_name) else {
                warn!("greenhouse: {path}: no crop `{species_name}`");
                continue;
            };
            let zone = room_of(read_token(stage, sdf, "gearbox:tray:zone"));
            let Some(room) = zone.or_else(|| (!room_names.is_empty()).then_some(first_room)) else {
                warn!("greenhouse: {path}: a tray needs a room");
                continue;
            };
            let size = float2(stage, sdf, "gearbox:tray:size").unwrap_or(Vec2::new(0.6, 0.4));
            let (cols, rows) = int2(stage, sdf, "gearbox:tray:slots").unwrap_or((2, 1));
            let medium = match read_token(stage, sdf, "gearbox:tray:medium").as_deref() {
                Some("regolith") => Medium::Regolith { perchlorate: float("gearbox:tray:perchlorate").unwrap_or(0.5), organic: 0.0 },
                _ => Medium::Hydroponic,
            };
            let name = path.rsplit('/').next().unwrap_or_default().to_string();
            let area = (size.x * size.y) as f64;
            let mut greenhouse = world.resource_mut::<Greenhouse>();
            let house = &mut greenhouse.house;
            let mut lamp = Lamp::new(&name, float("gearbox:tray:ppfd").unwrap_or(400.0), area, float("gearbox:tray:lampHours").unwrap_or(species.photoperiod_h));
            lamp.on_hour = float("gearbox:tray:lampOn").unwrap_or(5.0);
            house.lamps.push(lamp);
            let slots = cols * rows;
            house.trays.push(Tray {
                name: name.clone(),
                room,
                tank: room,
                lamps: vec![house.lamps.len() - 1],
                area_m2: area,
                medium,
                slots: vec![None; slots],
                per_slot: (area * species.density / slots as f64).max(1.0),
            });
            let tray = house.trays.len() - 1;
            let sown = float("gearbox:tray:sownDays").unwrap_or(0.0);
            if sown > 0.0 {
                pre_grow(house, tray, species, sown);
            }
            greenhouse.trays.push(TrayPlace {
                entity: *entity,
                path: path.clone(),
                cols,
                rows,
                size,
                lamp_height: float("gearbox:tray:lampHeight").unwrap_or(0.5) as f32,
                lamp: entity_of(&format!("{path}/lamp")),
                lamp_face: entity_of(&format!("{path}/lamp/face")),
            });
        }
        let greenhouse = world.resource::<Greenhouse>();
        let lamps = greenhouse.trays.iter().filter(|t| t.lamp.is_some()).count();
        info!(
            "greenhouse: {} room(s), {} tray(s) ({lamps} with a lamp prim), {} plant(s), {} service spot(s) from {root:?}",
            greenhouse.house.rooms.len(),
            greenhouse.house.trays.len(),
            greenhouse.house.plants().count(),
            greenhouse.services.len()
        );
        if lamps < greenhouse.trays.len()
            && let Some(tray) = greenhouse.trays.iter().find(|t| t.lamp.is_none())
        {
            let near: Vec<&str> = prims.iter().map(|(p, _)| p.as_str()).filter(|p| p.contains(tray.path.rsplit('/').next().unwrap_or("-"))).take(4).collect();
            warn!("greenhouse: no lamp prim under `{}`; its prims: {near:?}", tray.path);
        }
    }
    world.insert_non_send(instances);
}

/// Sows a tray `days` ago and grows it to now at its lamp's nominal light,
/// each place a little apart so the crop is not one clone.
fn pre_grow(house: &mut House, tray: usize, species: &'static Species, days: f64) {
    let slots = house.trays[tray].slots.len();
    let lamp = house.trays[tray].lamps[0];
    let cycle = house.cycle_hours;
    for slot in 0..slots {
        let id = PlantId { tray, slot };
        if house.sow(id, species).is_err() {
            continue;
        }
        let age = (days * (0.92 + 0.16 * scatter(tray, slot, 3) as f64)).max(0.0);
        let Some(mut around) = house.surroundings(tray, 12.0) else { continue };
        around.water = 1.0;
        let hours = (age * 24.0).round() as usize;
        let plant = house.trays[tray].slots[slot].as_mut().expect("just sown");
        for h in 0..hours {
            let hour = (h as f64).rem_euclid(cycle);
            let lit = house.lamps[lamp].scheduled(hour, cycle);
            let here = Surroundings { ppf: if lit { around.lit_ppf } else { 0.0 }, ..around };
            plant.grow(3600.0, &here);
        }
    }
}

/// The house runs on world time: the outside of the planet it stands on and
/// the sunlight on its panels come from the weather.
fn step_greenhouse(
    step: Res<WorldStep>,
    weather: Res<WeatherSettings>,
    mut greenhouse: ResMut<Greenhouse>,
) {
    if step.seconds <= 0.0 || greenhouse.is_empty() {
        return;
    }
    let calendar = weather.calendar;
    let elevation = weather.sun_position.normalize_or_zero().y.clamp(-1.0, 1.0).asin() as f64;
    let (top, tau, outside) = outside(&weather);
    let solar = gearbox_greenhouse::power::surface_irradiance(top * calendar.irradiance_scale() as f64, elevation, tau);
    // The hour the step began at: the calendar has already moved on by it.
    let hour = (calendar.hour - step.seconds / (calendar.planet.solar_day_s() / 24.0)).rem_euclid(24.0) * greenhouse.house.cycle_hours / 24.0;
    let events = greenhouse.house.step(step.seconds, hour, &outside, solar);
    for event in &events {
        match event {
            HouseEvent::Died(id) => info!("greenhouse: {} died", greenhouse.plant_name(*id)),
            HouseEvent::Staged(id, stage) if *stage == gearbox_greenhouse::Stage::Ripe => {
                info!("greenhouse: {} is ripe", greenhouse.plant_name(*id));
            }
            _ => {}
        }
    }
    greenhouse.events.extend(events);
}

/// Sunlight at the top of the air (W/m²), how much the air dims it, and the
/// air beyond the walls, on the planet the weather is on.
pub fn outside(weather: &WeatherSettings) -> (f64, f64, Outside) {
    let calendar = weather.calendar;
    let elevation = weather.sun_position.normalize_or_zero().y.clamp(-1.0, 1.0).asin() as f64;
    match calendar.planet {
        bevy_weather::Planet::Mars => (586.2, weather.dust_tau as f64, Outside::mars(calendar.hour)),
        bevy_weather::Planet::Moon => {
            let day = elevation > 0.0;
            (1361.0, 0.0, Outside { temperature_c: if day { 100.0 } else { -170.0 }, pressure_kpa: 0.0 })
        }
        bevy_weather::Planet::Earth => (1361.0, 0.3 + 2.5 * weather.clouds.clouds_coverage as f64, Outside { temperature_c: 12.0, pressure_kpa: 101.3 }),
    }
}
