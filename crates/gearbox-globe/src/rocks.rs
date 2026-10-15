//! Stable geodetic cells for streamed planetary boulders.

use crate::{Body, Geodetic, regolith_noise::hash};
use bevy::math::Vec3;

const CELL_M: f64 = 12.0;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct RockKey(pub i32, pub i32);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rock {
    pub key: RockKey,
    pub ground: Geodetic,
    pub scale: Vec3,
    pub yaw: f32,
    pub template: usize,
}

pub fn near(body: Body, centre: Geodetic, radius_m: f64) -> Vec<Rock> {
    if body == Body::Earth || !radius_m.is_finite() || radius_m <= 0.0 {
        return Vec::new();
    }
    let radius = body.radius_m();
    let bands = (std::f64::consts::PI * radius / CELL_M).ceil() as i32;
    let band_height = std::f64::consts::PI / bands as f64;
    let latitude = centre.latitude.to_radians();
    let longitude = centre.longitude.to_radians();
    let angular = radius_m / radius;
    let band_at = |lat: f64| ((lat + std::f64::consts::FRAC_PI_2) / band_height).floor() as i32;
    let low = band_at(latitude - angular).clamp(0, bands - 1);
    let high = band_at(latitude + angular).clamp(0, bands - 1);
    let origin = Geodetic::new(centre.latitude, centre.longitude, 0.0).ecef_on(body);
    let mut rocks = Vec::new();
    for band in low..=high {
        let mid = -std::f64::consts::FRAC_PI_2 + (band as f64 + 0.5) * band_height;
        let columns = (std::f64::consts::TAU * radius * mid.cos() / CELL_M)
            .ceil()
            .max(1.0) as i32;
        let half = ((angular / mid.cos().max(1e-10)) * columns as f64 / std::f64::consts::TAU)
            .ceil() as i32
            + 2;
        let around = ((longitude / std::f64::consts::TAU + 0.5) * columns as f64).floor() as i32;
        for offset in 0..(half.saturating_mul(2) + 1).min(columns) {
            let column = (around - half + offset).rem_euclid(columns);
            let random = |salt: i64| hash(band as i64, column as i64, body as i64 * 1009 + salt);
            if random(0) > 0.42 {
                continue;
            }
            let lat = -std::f64::consts::FRAC_PI_2
                + (band as f64 + 0.1 + 0.8 * random(1) as f64) * band_height;
            let lon = ((column as f64 + 0.1 + 0.8 * random(2) as f64) / columns as f64 - 0.5)
                * std::f64::consts::TAU;
            let ground = Geodetic::new(lat.to_degrees(), lon.to_degrees(), 0.0);
            if ground.ecef_on(body).distance(origin) > radius_m {
                continue;
            }
            let size = 0.14 + 1.5 * random(3).powi(4);
            rocks.push(Rock {
                key: RockKey(band, column),
                ground,
                scale: Vec3::new(
                    size,
                    size * (0.4 + 0.4 * random(4)),
                    size * (0.65 + 0.6 * random(5)),
                ),
                yaw: random(6) * std::f32::consts::TAU,
                template: (random(7) * 16.0) as usize,
            });
        }
    }
    rocks
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn boulders_are_stable_bounded_and_wrap_at_poles_and_dateline() {
        for body in [Body::Mars, Body::Moon] {
            for (lat, lon) in [
                body.home(),
                (89.999, 179.9999),
                (-89.999, -179.9999),
                (0.0, 180.0),
            ] {
                let centre = Geodetic::new(lat, lon, 0.0);
                let rocks = near(body, centre, 180.0);
                assert!(
                    rocks.len() > 20 && rocks.len() < 1000,
                    "{body:?} {lat}: {}",
                    rocks.len()
                );
                let unique: HashMap<_, _> = rocks.iter().map(|r| (r.key, r)).collect();
                assert_eq!(unique.len(), rocks.len());
                assert_eq!(rocks, near(body, centre, 180.0));
                for rock in &rocks {
                    assert!(rock.ground.ecef_on(body).distance(centre.ecef_on(body)) <= 180.0);
                    assert!(rock.scale.min_element() > 0.0 && rock.scale.max_element() < 2.1);
                    assert!(rock.template < 16);
                }
                let wrapped = near(body, Geodetic::new(lat, lon + 360.0, 0.0), 180.0);
                assert_eq!(rocks, wrapped);
                let moved = crate::Datum::on(body, lat, lon).travelled(90.0, 35.0);
                let neighbours = near(
                    body,
                    Geodetic::new(moved.latitude, moved.longitude, 0.0),
                    180.0,
                );
                let mut shared = 0;
                for rock in &neighbours {
                    if let Some(previous) = unique.get(&rock.key) {
                        assert_eq!(*previous, rock);
                        shared += 1;
                    }
                }
                assert!(shared > 15);
            }
        }
        assert!(near(Body::Earth, Geodetic::new(0.0, 0.0, 0.0), 180.0).is_empty());
    }
}
