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
use crate::sensors::SensorLabel;

/// A plant entity whose hull stands in the physics world.
#[derive(Component)]
pub struct PlantHull;

/// Gives each new plant entity the hull of its prefab: a collider that
/// touches nothing, which sensor rays still meet, labelled with the plant's
/// name; a plant's next look brings its own hull.
#[allow(clippy::too_many_arguments)]
pub fn sync_plant_hulls(
    mut commands: Commands,
    greenhouse: Res<Greenhouse>,
    looks: Option<Res<CropLooks>>,
    mut physics: ResMut<PhysicsWorld>,
    plants: Query<(Entity, &PlantLook), Without<PlantHull>>,
    parents: Query<&ChildOf>,
    transforms: Query<&Transform>,
    sites: Query<&Site>,
) {
    let Some(looks) = looks else { return };
    for (entity, look) in &plants {
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

/// Takes the hulls of plant entities gone since last frame out of the physics world.
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
    fn mars_air_is_thin_carbon_dioxide_and_dries_by_day() {
        let dawn = outside_air(bevy_weather::Planet::Mars, gearbox_greenhouse::Outside::mars(5.0));
        let noon = outside_air(bevy_weather::Planet::Mars, gearbox_greenhouse::Outside::mars(14.0));
        assert!(noon[0] > dawn[0] && noon[1] < 0.05 && dawn[1] > 0.5);
        assert!(noon[2] > 900_000.0 && noon[3] < 0.01 && noon[4] < 1.0);
        let earth = outside_air(bevy_weather::Planet::Earth, gearbox_greenhouse::Outside { temperature_c: 12.0, pressure_kpa: 101.3 });
        assert!((earth[3] - 21.2).abs() < 0.1);
    }
}
