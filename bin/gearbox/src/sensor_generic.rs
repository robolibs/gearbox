//! Generic sensor links: inertial unit, compass, GPS, distance, light,
//! position, radar and touch (accelerometer and gyro ride on the IMU batch;
//! receivers and emitters on [`crate::sensors`]' radio network).
//!
//! Molla's sensor models compute every reading. Gearbox supplies what only
//! the environment knows: its site frame and where each site sits on the
//! planet, the sun, and this step's contact forces. Distance, light and
//! radar rays are cast on the CPU against the live rigid scene.

use std::collections::HashMap;

use gearbox_api::{Measurement, Props, measurement_kind as kind};
use molla_core::BodyId as MollaBodyId;
use molla_math::{Real, Transform as MTransform, Vec3 as MVec3};
use molla_sensors::{
    ContactForceSample, CpuRaycaster, DistanceKind, DistanceSensors, DistanceSpec, GeoReference,
    Geodetic, Gps, LightEnvironment, LightSensors, RadarSpec, RadarTarget, Radars,
    RecognizedObject, RigidRays, TouchKind, TouchSensor, WorldFrame, compass, inertial_unit,
    joint_reading,
};
use molla_sim::runtime::RigidScene;

use crate::links::{SensorKind, SensorSpec, SensorVariant};

/// Gearbox sites are laid out north along +X, up along +Y, east along +Z.
pub const SITE_FRAME: WorldFrame = WorldFrame {
    up: MVec3::Y,
    north: MVec3::X,
};

/// Whether a kind is handled here rather than by the IMU, LiDAR, camera or
/// radio paths.
pub fn is_generic(kind: SensorKind) -> bool {
    matches!(
        kind,
        SensorKind::InertialUnit
            | SensorKind::Compass
            | SensorKind::Gps
            | SensorKind::Distance
            | SensorKind::Light
            | SensorKind::Position
            | SensorKind::Radar
            | SensorKind::Touch
    )
}

/// What the environment knows this frame.
#[derive(Default)]
pub struct SensorEnvironment {
    pub light: LightEnvironment,
    /// Contact forces of the last physics step on Molla bodies.
    pub contacts: Vec<ContactForceSample>,
}

/// A measurement before its publish stamp.
pub fn measurement(
    name: &str,
    link_index: u32,
    measurement_kind: u32,
    sim_time_s: f64,
    sample: u64,
    values: Vec<f64>,
) -> Measurement {
    let width = kind::width(measurement_kind).max(1);
    Measurement {
        sim_time_s,
        link_index,
        kind: measurement_kind,
        stamp_ms: 0,
        sample: sample as u32,
        count: (values.len() / width) as u32,
        _pad: 0,
        values,
        data: Vec::new(),
        props: Props::from_pairs(&[("name", name)]).into_bytes(),
    }
}

/// A camera's recognised objects, with each object's label in props
/// `names`, one line per record.
pub fn recognition(
    name: &str,
    link_index: u32,
    sim_time_s: f64,
    sample: u64,
    seen: &[RecognizedObject],
    labels: &[String],
) -> Measurement {
    let mut values = Vec::with_capacity(seen.len() * kind::width(kind::RECOGNITION));
    let mut names = Vec::with_capacity(seen.len());
    for o in seen {
        let ([l, t, r, b], p, s) = (o.bbox, o.position, o.size);
        values.extend([o.id as f64, o.pixels as f64, l as f64, t as f64, r as f64, b as f64]);
        values.extend([p.x, p.y, p.z, s.x, s.y, s.z]);
        names.push(labels.get(o.id as usize).map_or("", String::as_str));
    }
    let mut m = measurement(name, link_index, kind::RECOGNITION, sim_time_s, sample, values);
    m.props = Props::from_pairs(&[("name", name), ("names", &names.join("\n"))]).into_bytes();
    m
}

/// A mount's due sample, as the generic sensors need it.
pub struct GenericSample<'a> {
    pub mount: usize,
    pub name: &'a str,
    pub link_index: u32,
    pub spec: SensorSpec,
    pub body: MollaBodyId,
    pub world: MTransform,
    pub sample: u64,
}

/// Molla sensor models and per-mount state for one rig.
pub struct GenericSensors {
    gps: HashMap<usize, (usize, Gps)>,
    distance: HashMap<usize, DistanceSensors>,
    light: LightSensors,
    radar: Radars,
    radar_targets: Vec<RadarTarget>,
    joints: HashMap<usize, usize>,
    /// Prepared collider geometry, reused between samples.
    caster: CpuRaycaster,
}

fn distance_spec(spec: &SensorSpec) -> DistanceSpec {
    DistanceSpec {
        kind: match spec.variant {
            SensorVariant::InfraRed => DistanceKind::InfraRed,
            SensorVariant::Sonar => DistanceKind::Sonar,
            _ => DistanceKind::Laser,
        },
        rays: spec.rays.max(1),
        aperture: spec.aperture as Real,
        max_range: spec.range_m as Real,
        gaussian_width: 1.0,
        lookup: Vec::new(),
    }
}

fn radar_spec(spec: &SensorSpec) -> RadarSpec {
    RadarSpec {
        min_range: spec.min_range_m as Real,
        max_range: spec.range_m as Real,
        horizontal_fov: spec.hfov as Real,
        vertical_fov: spec.vfov as Real,
        ..Default::default()
    }
}

impl GenericSensors {
    /// Batches for the generic mounts among `specs`. `joints` gives the
    /// Molla joint index of each position mount; `targets` are the bodies a
    /// radar may detect.
    pub fn build(
        specs: &[SensorSpec],
        joints: HashMap<usize, usize>,
        targets: Vec<MollaBodyId>,
    ) -> Result<Self, String> {
        let mut distance = HashMap::new();
        for (mount, spec) in specs.iter().enumerate().filter(|(_, s)| s.kind == SensorKind::Distance) {
            let sensor = DistanceSensors::host(vec![distance_spec(spec)])
                .map_err(|e| format!("distance: {e}"))?;
            distance.insert(mount, sensor);
        }
        Ok(Self {
            gps: HashMap::new(),
            distance,
            light: LightSensors::host(vec![Vec::new()]),
            radar: Radars::host(),
            radar_targets: targets
                .into_iter()
                .map(|body| RadarTarget {
                    body,
                    cross_section: 1.0,
                })
                .collect(),
            joints,
            caster: CpuRaycaster::default(),
        })
    }

    /// Readings of the due generic samples at simulated time `sim_time`,
    /// casting their rays on `rigid`.
    pub fn sample(
        &mut self,
        rigid: &RigidScene,
        environment: &SensorEnvironment,
        due: &[GenericSample<'_>],
        sim_time: f64,
    ) -> Result<Vec<(usize, Measurement)>, String> {
        let mut out = Vec::new();
        let make = |s: &GenericSample<'_>, k: u32, values: Vec<f64>| {
            (s.mount, measurement(s.name, s.link_index, k, sim_time, s.sample, values))
        };
        let mut rays = RigidRays {
            scene: rigid,
            caster: &mut self.caster,
        };
        for s in due {
            let rotation = s.world.rotation;
            match s.spec.kind {
                SensorKind::Distance => {
                    let Some(sensor) = self.distance.get(&s.mount) else { continue };
                    let r = sensor
                        .update_with(&mut rays, &[s.world])
                        .map_err(|e| format!("distance `{}`: {e}", s.name))?[0];
                    out.push(make(s, kind::DISTANCE, vec![r.distance, r.value]));
                }
                SensorKind::Light => {
                    let r = self
                        .light
                        .update_with(&mut rays, &[s.world], &environment.light)
                        .map_err(|e| format!("light `{}`: {e}", s.name))?[0];
                    out.push(make(s, kind::LIGHT, vec![r.irradiance, r.direct, r.sky, r.visible as f64, r.value]));
                }
                SensorKind::InertialUnit => {
                    let r = inertial_unit(&SITE_FRAME, rotation);
                    let q = r.orientation;
                    out.push(make(s, kind::INERTIAL_UNIT, vec![r.roll, r.pitch, r.yaw, q.x, q.y, q.z, q.w]));
                }
                SensorKind::Compass => {
                    let r = compass(&SITE_FRAME, rotation);
                    out.push(make(s, kind::COMPASS, vec![r.north.x, r.north.y, r.north.z, r.heading_rad]));
                }
                SensorKind::Gps => {
                    let p = s.world.position;
                    let (region, local) = crate::globe::site_local(p.x, p.y, p.z);
                    let Some(origin) = crate::globe::earth_place(region, [0.0; 3]) else {
                        continue;
                    };
                    let reference = GeoReference {
                        origin: Geodetic {
                            latitude_deg: origin.geodetic.latitude,
                            longitude_deg: origin.geodetic.longitude,
                            altitude_m: origin.geodetic.altitude,
                        },
                    };
                    let entry = self
                        .gps
                        .entry(s.mount)
                        .or_insert_with(|| (region, Gps::new(SITE_FRAME, reference)));
                    if entry.0 != region {
                        *entry = (region, Gps::new(SITE_FRAME, reference));
                    }
                    let fix = entry.1.sample(MVec3::from_array(local), sim_time);
                    let g = fix.geodetic;
                    out.push(make(
                        s,
                        kind::GPS,
                        vec![
                            g.latitude_deg,
                            g.longitude_deg,
                            g.altitude_m,
                            fix.enu.x,
                            fix.enu.y,
                            fix.enu.z,
                            fix.speed,
                            fix.velocity_enu.x,
                            fix.velocity_enu.y,
                            fix.velocity_enu.z,
                        ],
                    ));
                }
                SensorKind::Position => {
                    let Some(&joint) = self.joints.get(&s.mount) else { continue };
                    let r = joint_reading(rigid.model(), rigid.state(), joint)
                        .map_err(|e| format!("position `{}`: {e}", s.name))?;
                    out.push(make(s, kind::POSITION, vec![r.position, r.velocity]));
                }
                SensorKind::Touch => {
                    let touch_kind = match s.spec.variant {
                        SensorVariant::Force => TouchKind::Force,
                        SensorVariant::Force3d => TouchKind::Force3d,
                        _ => TouchKind::Bumper,
                    };
                    let r = TouchSensor::new(s.body, touch_kind).sample(rotation, &environment.contacts);
                    let f = r.force_sensor;
                    out.push(make(
                        s,
                        kind::TOUCH,
                        vec![f64::from(u8::from(r.touching)), r.contacts as f64, r.force, f.x, f.y, f.z],
                    ));
                }
                SensorKind::Radar => {
                    let detections = self
                        .radar
                        .update_with(
                            &mut rays,
                            rigid.model(),
                            rigid.state(),
                            &radar_spec(&s.spec),
                            s.world,
                            s.body,
                            &self.radar_targets,
                        )
                        .map_err(|e| format!("radar `{}`: {e}", s.name))?;
                    let values = detections
                        .iter()
                        .flat_map(|d| {
                            [d.distance, d.azimuth, d.elevation, d.radial_speed, d.received_power_dbm]
                        })
                        .collect();
                    out.push(make(s, kind::RADAR, values));
                }
                _ => {}
            }
        }
        Ok(out)
    }

    /// Forget GPS history, e.g. after a teleport.
    pub fn reset(&mut self) {
        self.gps.clear();
    }
}

/// This step's contact forces on Molla bodies: each manifold point's normal
/// impulse over the step, pushing collider 2 along the normal and collider 1
/// against it.
pub fn contact_forces(
    physics: &dyn crate::physics::backend::PhysicsBackend,
) -> Vec<ContactForceSample> {
    let dt = physics.settings().dt.max(1e-6);
    let manifolds = physics.contacts();
    // Molla handles of each manifold's collider bodies, outside the scene lock.
    let handle_of = |collider| {
        physics
            .collider(collider)
            .and_then(|c| c.parent())
            .and_then(|b| physics.molla_body_handle(b))
    };
    let handles: Vec<_> = manifolds
        .iter()
        .map(|m| (handle_of(m.collider1), handle_of(m.collider2)))
        .collect();
    let mut out = Vec::new();
    physics.with_molla_scene(&mut |scene| {
        let index = |handle: Option<_>| handle.and_then(|h| scene.body_index(h));
        for (manifold, &(h1, h2)) in manifolds.iter().zip(&handles) {
            let n = manifold.normal;
            let normal = MVec3::new(n.x, n.y, n.z);
            let (a, b) = (index(h1), index(h2));
            for point in &manifold.points {
                let at = MVec3::new(point.point.x, point.point.y, point.point.z);
                let force = normal * (point.impulse / dt);
                if let Some(body) = b {
                    out.push(ContactForceSample { body, point: at, force });
                }
                if let Some(body) = a {
                    out.push(ContactForceSample { body, point: at, force: -force });
                }
            }
        }
    });
    out
}

/// Luminous efficacy of daylight (lm/W): lux to W/m².
pub const DAYLIGHT_LUMENS_PER_WATT: f64 = 93.0;

/// The sun and sky as the weather has them this frame.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Daylight<'w, 's> {
    suns: bevy::prelude::Query<
        'w,
        's,
        (
            &'static bevy::prelude::DirectionalLight,
            &'static bevy_weather::OriginalIlluminance,
            &'static bevy::prelude::GlobalTransform,
        ),
    >,
    sky: Option<bevy::prelude::Res<'w, bevy_weather::SkyLight>>,
}

impl Daylight<'_, '_> {
    /// The brightest directional light as the sun, split by the weather's
    /// cloud cover (none at night).
    pub fn light(&self) -> LightEnvironment {
        let sun = self
            .suns
            .iter()
            .filter(|(light, _, _)| light.illuminance > 0.0)
            .max_by(|a, b| a.1.0.total_cmp(&b.1.0));
        let Some((_, lux, transform)) = sun else {
            return daylight(0.0, SITE_FRAME.up, bevy_weather::SkyLight::default());
        };
        let towards = transform.back().as_vec3();
        let towards = MVec3::new(towards.x as f64, towards.y as f64, towards.z as f64);
        let sky = self.sky.as_deref().copied().unwrap_or_default();
        daylight(lux.0 as f64 / DAYLIGHT_LUMENS_PER_WATT, towards, sky)
    }
}

/// Clear-sky diffuse light on the ground against the direct beam's share.
const CLEAR_SKY_DIFFUSE: f64 = 0.2 / 0.8;

/// Sun and sky from the clear-sky beam `beam` (W/m² facing the sun) towards
/// `towards`, dimmed and diffused as `sky` has the weather.
pub fn daylight(beam: f64, towards: MVec3, sky: bevy_weather::SkyLight) -> LightEnvironment {
    let elevation = towards.normalize_or_zero().dot(SITE_FRAME.up).max(0.0);
    if beam <= 0.0 || elevation == 0.0 {
        return LightEnvironment {
            sources: Vec::new(),
            sky: 0.0,
            up: SITE_FRAME.up,
        };
    }
    LightEnvironment {
        sources: vec![molla_sensors::LightSource::Directional {
            towards,
            irradiance: beam * sky.mean_direct as f64,
        }],
        sky: CLEAR_SKY_DIFFUSE * beam * elevation * sky.diffuse_gain as f64,
        up: SITE_FRAME.up,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_frame_is_north_x_up_y_east_z() {
        let [east, north, up] = SITE_FRAME.enu();
        assert_eq!((east, north, up), (MVec3::Z, MVec3::X, MVec3::Y));
    }

    #[test]
    fn daylight_splits_the_beam_as_the_weather_does() {
        let overhead = MVec3::Y;
        let clear = daylight(1000.0, overhead, bevy_weather::SkyLight::from_cover(0.0));
        let direct = match clear.sources[..] {
            [molla_sensors::LightSource::Directional { irradiance, .. }] => irradiance,
            _ => panic!("{:?}", clear.sources),
        };
        assert!((direct - 1000.0).abs() < 1e-6 && (clear.sky - 250.0).abs() < 1e-6);
        assert!((clear.sky / (direct + clear.sky) - 0.2).abs() < 1e-9, "a fifth is diffuse");
        let low = daylight(1000.0, MVec3::new(1.0, 0.5, 0.0), bevy_weather::SkyLight::from_cover(0.0));
        assert!((low.sky - 250.0 * 0.5 / 1.25f64.sqrt()).abs() < 1e-6);
        let overcast = daylight(1000.0, overhead, bevy_weather::SkyLight::from_cover(1.0));
        assert!(matches!(overcast.sources[..], [molla_sensors::LightSource::Directional { irradiance, .. }] if irradiance.abs() < 1e-6));
        assert!((overcast.sky - 250.0 * 0.67 / 0.2).abs() < 1e-3);
        assert!(daylight(1000.0, -MVec3::Y, bevy_weather::SkyLight::default()).sources.is_empty());
    }

    #[test]
    fn gps_geodesy_matches_the_site_datum() {
        let datum = gearbox_globe::Datum::at(51.98, 5.66);
        let reference = GeoReference {
            origin: Geodetic {
                latitude_deg: datum.latitude,
                longitude_deg: datum.longitude,
                altitude_m: 0.0,
            },
        };
        let local = [1200.0, 35.0, -800.0];
        let fix = Gps::new(SITE_FRAME, reference).sample(MVec3::from_array(local), 0.0);
        let expected = datum.geodetic(local.into());
        let g = fix.geodetic;
        assert!((g.latitude_deg - expected.latitude).abs() < 1e-9, "{g:?} {expected:?}");
        assert!((g.longitude_deg - expected.longitude).abs() < 1e-9, "{g:?} {expected:?}");
        assert!((g.altitude_m - expected.altitude).abs() < 1e-6, "{g:?} {expected:?}");
        assert!((fix.enu - MVec3::new(-800.0, 1200.0, 35.0)).length() < 1e-9);
    }

    #[test]
    fn recognition_lists_objects_with_their_labels() {
        let object = |id| RecognizedObject {
            id,
            pixels: 4,
            bbox: [1, 2, 3, 4],
            position: MVec3::new(0.5, -0.5, -3.0),
            size: MVec3::splat(0.5),
        };
        let labels = ["ground".to_string(), "crate".to_string()];
        let m = recognition("cam", 2, 1.0, 9, &[object(1), object(0)], &labels);
        assert_eq!((m.kind, m.count), (kind::RECOGNITION, 2));
        assert_eq!(m.props().get("names").as_deref(), Some("crate\nground"));
        assert_eq!(m.records().next().unwrap()[..6], [1.0, 4.0, 1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn measurements_count_their_records() {
        let m = measurement("radar", 3, kind::RADAR, 1.5, 7, vec![1.0; 10]);
        assert_eq!((m.count, m.kind, m.link_index, m.sample), (2, kind::RADAR, 3, 7));
        assert_eq!(m.records().count(), 2);
        assert_eq!(m.name(), "radar");
        assert_eq!(measurement("r", 0, kind::RADAR, 0.0, 0, Vec::new()).count, 0);
    }
}
