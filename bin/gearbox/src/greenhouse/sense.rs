//! What robots sense of the greenhouse: the hull of every plant, which depth,
//! LiDAR and recognition meet and name, and the air climate sensors read.

use bevy::math::{DAffine3, DQuat, DVec3};
use bevy::prelude::*;

use super::Greenhouse;
use super::looks::{CropLooks, PlantLook};
use crate::environment::WeatherSettings;
use crate::globe::{Site, site_of, transform_in_site};
use crate::physics::PhysicsWorld;
use crate::physics::backend::{ColliderDesc, Pose, Shape};
use crate::sensor_generic::{AirField, AirReading, AirZone};
use crate::sensors::{MachineSensorRigs, SensorLabel};

/// A plant entity whose hull stands in the physics world.
#[derive(Component)]
pub struct PlantHull;

/// A tray's plants gain hulls once a machine's sensor comes this close,
/// metres, and lose them past `HULL_LEAVE_M`.
const HULL_REACH_M: f32 = 5.0;
const HULL_LEAVE_M: f32 = 7.0;

/// Which trays are in reach of a sensor now, from which were before.
pub fn trays_in_reach(eyes: &[Vec3], trays: &[Option<Vec3>], before: &[bool]) -> Vec<bool> {
    trays
        .iter()
        .enumerate()
        .map(|(i, tray)| {
            let Some(tray) = tray else { return false };
            let reach = if before.get(i).copied().unwrap_or(false) { HULL_LEAVE_M } else { HULL_REACH_M };
            eyes.iter().any(|eye| eye.distance(*tray) < reach)
        })
        .collect()
}

/// Gives the plants of trays a machine's sensors can reach the hull of their
/// prefab: a collider that touches nothing, which sensor rays still meet,
/// labelled with the plant's name. Plants out of reach carry none; a
/// plant's next look brings its own hull.
#[allow(clippy::too_many_arguments)]
pub fn sync_plant_hulls(
    mut commands: Commands,
    greenhouse: Res<Greenhouse>,
    looks: Option<Res<CropLooks>>,
    rigs: Option<Res<MachineSensorRigs>>,
    mut physics: ResMut<PhysicsWorld>,
    mut reach: Local<Vec<bool>>,
    plants: Query<(Entity, &PlantLook, Has<PlantHull>)>,
    (parents, transforms, sites, globals): (Query<&ChildOf>, Query<&Transform>, Query<&Site>, Query<&GlobalTransform>),
) {
    let Some(looks) = looks else { return };
    let eyes: Vec<Vec3> = rigs.iter().flat_map(|r| r.sensor_entities()).filter_map(|e| globals.get(e).ok()).map(|g| g.translation()).collect();
    let trays: Vec<Option<Vec3>> = greenhouse.trays.iter().map(|t| globals.get(t.entity).ok().map(|g| g.translation())).collect();
    *reach = trays_in_reach(&eyes, &trays, &reach);
    for (entity, look, hulled) in &plants {
        let wanted = reach.get(look.id.tray).copied().unwrap_or(false);
        if hulled && !wanted {
            commands.entity(entity).remove::<(PlantHull, SensorLabel)>();
        }
        if hulled || !wanted {
            continue;
        }
        commands.entity(entity).insert(PlantHull);
        let Some(plant) = greenhouse.house.plant(look.id) else { continue };
        let Some(points) = looks.hull(plant.species.name, look.stage) else { continue };
        let at = transform_in_site(entity, &parents, &transforms, &sites).compute_transform();
        let offset = gearbox_globe::physics_offset(site_of(entity, &parents, &sites));
        let shape = Shape::ConvexHull { points: points.iter().map(|p| physics_vec((*p * at.scale).as_dvec3())).collect() };
        let pose = Pose { translation: physics_vec(at.translation.as_dvec3() + offset), rotation: physics_quat(at.rotation.as_dquat()) };
        let desc = ColliderDesc { enabled: false, ..ColliderDesc::new(shape).pose(pose).entity(entity) };
        if let Some(handle) = physics.insert_collider(desc) {
            physics.entity_to_collider.insert(entity, handle);
        }
        commands.entity(entity).insert(SensorLabel(format!("plant:{}", greenhouse.plant_name(look.id))));
    }
}

fn physics_vec(v: DVec3) -> glam::DVec3 {
    glam::DVec3::new(v.x, v.y, v.z)
}

fn physics_quat(q: DQuat) -> glam::DQuat {
    glam::DQuat::from_xyzw(q.x, q.y, q.z, q.w)
}

/// Takes the hulls of plant entities gone, or gone out of reach, since last
/// frame out of the physics world.
pub fn drop_plant_hulls(mut gone: RemovedComponents<PlantHull>, mut physics: ResMut<PhysicsWorld>) {
    for entity in gone.read() {
        physics.remove_entity_collider(entity, false);
    }
}

/// The air outside on a planet at its local solar hour.
pub fn outside_air(planet: bevy_weather::Planet, outside: gearbox_greenhouse::Outside) -> AirReading {
    let (t, p) = (outside.temperature_c, outside.pressure_kpa);
    match planet {
        // 95% CO2 and 0.17% O2; REMS saw under 5% humidity by day and up to
        // about 70% in the cold before dawn.
        bevy_weather::Planet::Mars => [t, 0.02 + 0.68 * ((-31.0 - t) / 58.0).clamp(0.0, 1.0), 949_000.0, 0.0017 * p, p],
        bevy_weather::Planet::Moon => [t, 0.0, 0.0, 0.0, 0.0],
        bevy_weather::Planet::Earth => [t, 0.6, 420.0, 0.2095 * p, p],
    }
}

/// The air a climate sensor stands in: each room's inside its box, the
/// planet's everywhere else.
pub fn fill_air_field(
    mut field: ResMut<AirField>,
    greenhouse: Res<Greenhouse>,
    weather: Res<WeatherSettings>,
    parents: Query<&ChildOf>,
    transforms: Query<&Transform>,
    sites: Query<&Site>,
) {
    field.outside = outside_air(weather.calendar.planet, super::outside(&weather).2);
    field.zones = greenhouse
        .rooms
        .iter()
        .map(|place| {
            let air = &greenhouse.house.rooms[place.room].air;
            let in_site = transform_in_site(place.parent, &parents, &transforms, &sites).affine();
            let offset = gearbox_globe::physics_offset(site_of(place.parent, &parents, &sites));
            let to_physics = DAffine3::from_translation(offset) * DAffine3::from_mat4(Mat4::from(in_site).as_dmat4());
            AirZone {
                to_local: to_physics.inverse(),
                centre: place.centre.as_dvec3(),
                half: (place.size * 0.5).as_dvec3(),
                air: [air.temperature_c, air.humidity, air.co2_ppm, air.o2_kpa, air.pressure_kpa],
            }
        })
        .collect();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_room_holds_its_air_and_outside_is_the_planets() {
        let room = [21.0, 0.7, 1000.0, 21.2, 101.3];
        let mut field = AirField { outside: [-60.0, 0.1, 949_000.0, 0.001, 0.64], zones: Vec::new() };
        // A 4 × 3 m room standing 2.5 m tall, its floor at the origin of a frame turned Z-up.
        let frame = DAffine3::from_rotation_x(-std::f64::consts::FRAC_PI_2);
        field.zones.push(AirZone {
            to_local: frame.inverse(),
            centre: DVec3::new(0.0, 0.0, 1.25),
            half: DVec3::new(2.0, 1.5, 1.25),
            air: room,
        });
        let at = |x, y, z| field.at(molla_math::Vec3::new(x, y, z));
        assert_eq!(at(0.5, 1.0, 0.5), room);
        assert_eq!(at(0.5, 3.0, 0.5), field.outside);
        assert_eq!(at(0.5, 1.0, 2.0), field.outside);
        assert_eq!(at(-1.9, 2.4, -1.4), room);
    }

    #[test]
    fn a_plant_gone_takes_its_hull_out_of_the_physics_world() {
        let mut app = App::new();
        app.insert_resource(PhysicsWorld::with_backend(Box::new(crate::physics::MollaBackend::default())));
        app.add_systems(Update, drop_plant_hulls);
        let plant = app.world_mut().spawn(PlantHull).id();
        let mut physics = app.world_mut().resource_mut::<PhysicsWorld>();
        let ball = ColliderDesc { enabled: false, ..ColliderDesc::new(Shape::Ball { radius: 0.1 }).entity(plant) };
        let hull = physics.insert_collider(ball).unwrap();
        physics.entity_to_collider.insert(plant, hull);
        app.update();
        assert!(app.world().resource::<PhysicsWorld>().collider(hull).is_some());
        app.world_mut().despawn(plant);
        app.update();
        let physics = app.world().resource::<PhysicsWorld>();
        assert!(physics.collider(hull).is_none() && !physics.entity_to_collider.contains_key(&plant));
    }

    #[test]
    fn only_trays_a_sensor_reaches_carry_hulls_and_they_keep_them_a_little_longer() {
        let trays = [Some(Vec3::new(3.0, 0.0, 0.0)), Some(Vec3::new(6.0, 0.0, 0.0)), Some(Vec3::new(20.0, 0.0, 0.0)), None];
        let eye = [Vec3::ZERO];
        let now = trays_in_reach(&eye, &trays, &[]);
        assert_eq!(now, [true, false, false, false]);
        // 9 m on, the first tray is 6 m behind: it keeps its hulls, though
        // a tray that far would not gain them.
        let on = [Vec3::new(9.0, 0.0, 0.0)];
        assert_eq!(trays_in_reach(&on, &trays, &now), [true, true, false, false]);
        assert_eq!(trays_in_reach(&on, &trays, &[]), [false, true, false, false]);
        // Without a sensor nothing carries a hull.
        assert_eq!(trays_in_reach(&[], &trays, &now), [false; 4]);
    }

    #[test]
    fn mars_air_is_thin_carbon_dioxide_and_dries_by_day() {
        let dawn = outside_air(bevy_weather::Planet::Mars, gearbox_greenhouse::Outside::mars(5.0));
        let noon = outside_air(bevy_weather::Planet::Mars, gearbox_greenhouse::Outside::mars(14.0));
        assert!(noon[0] > dawn[0] && noon[1] < 0.05 && dawn[1] > 0.5);
        assert!(noon[2] > 900_000.0 && noon[3] < 0.01 && noon[4] < 1.0);
        let earth = outside_air(bevy_weather::Planet::Earth, gearbox_greenhouse::Outside { temperature_c: 12.0, pressure_kpa: 101.3 });
        assert!((earth[3] - 21.2).abs() < 0.1);
    }
}
